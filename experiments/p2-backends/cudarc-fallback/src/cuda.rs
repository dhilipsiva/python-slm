use std::{sync::Arc, time::Instant};

use cudarc::{
    cublaslt::{CudaBlasLT, Matmul, MatmulConfig},
    driver::{CudaContext, CudaSlice, CudaStream},
};
use half::bf16;
use p2_backend_common::{
    AllocationResult, CandidateArgs, CandidateMode, CandidateResult, MemoryResult, ResultStatus,
    ShapeResult, TimingMeasurement, TimingResult, WorkloadShape, assess_correctness,
    evaluate_oracle,
    fixture::{load, sha256_bf16},
    timing::{
        WARMUP_ITERATIONS, duration_ns, gflops, measure, nearest_rank, validate_measurement_window,
    },
};

pub fn run(args: &CandidateArgs, result: &mut CandidateResult) -> Result<(), String> {
    if args.mode == CandidateMode::CpuSmoke {
        return Err("CUDA runner cannot execute cpu-smoke".to_owned());
    }
    let fixture = load(&args.fixture_dir, args.workload).map_err(|error| error.to_string())?;
    result.fixture_hashes = Some(fixture.hashes());
    let context_started = Instant::now();
    let context = CudaContext::new(0).map_err(display)?;
    let stream = context.default_stream();
    let blas = CudaBlasLT::new(stream.clone()).map_err(display)?;
    stream.synchronize().map_err(display)?;
    let context_ns = duration_ns(context_started.elapsed());
    let memory_context = free_bytes(&context)?;

    match (args.mode, args.workload.shape()) {
        (CandidateMode::Correctness, WorkloadShape::Allocation(shape)) => {
            let device = stream.clone_htod(&fixture.a).map_err(display)?;
            stream.synchronize().map_err(display)?;
            let memory_allocation = free_bytes(&context)?;
            let output = stream.clone_dtoh(&device).map_err(display)?;
            stream.synchronize().map_err(display)?;
            let output_sha256 = sha256_bf16(&output);
            let input_sha256 = fixture.manifest.a.sha256.clone();
            result.allocation = Some(AllocationResult {
                shape: shape.map(|value| value as u64),
                elements: fixture.a.len() as u64,
                bitwise_equal: output_sha256 == input_sha256 && output == fixture.a,
                input_sha256,
                output_sha256,
            });
            result.memory = Some(MemoryResult {
                free_bytes_after_context: Some(memory_context),
                free_bytes_after_allocation: Some(memory_allocation),
                free_bytes_after_forward: None,
                free_bytes_after_backward: None,
            });
            if result
                .allocation
                .as_ref()
                .is_some_and(|allocation| allocation.bitwise_equal)
            {
                result.status = ResultStatus::Pass;
            } else {
                return Err("cudarc BF16 allocation round-trip was not bitwise equal".to_owned());
            }
        }
        (CandidateMode::Correctness, WorkloadShape::Matmul { m, k, n }) => {
            let mut graph = Graph::new(&fixture.a, &fixture.b, m, k, n, stream, blas)?;
            graph.stream.synchronize().map_err(display)?;
            let memory_allocation = free_bytes(&context)?;
            let observed = graph.evaluate(&context)?;
            let oracle = evaluate_oracle(&fixture.a, &fixture.b, m, k, n);
            let correctness = assess_correctness(
                ShapeResult {
                    m: m as u64,
                    k: k as u64,
                    n: n as u64,
                },
                &observed.y,
                observed.loss,
                &observed.grad_a,
                &observed.grad_b,
                &oracle,
            );
            if !correctness.passes() {
                result.correctness = Some(correctness);
                return Err("cuBLASLt diagnostic failed the frozen tolerances".to_owned());
            }
            result.correctness = Some(correctness);
            result.memory = Some(MemoryResult {
                free_bytes_after_context: Some(memory_context),
                free_bytes_after_allocation: Some(memory_allocation),
                free_bytes_after_forward: Some(observed.free_after_forward),
                free_bytes_after_backward: Some(observed.free_after_backward),
            });
            result.status = ResultStatus::Pass;
        }
        (CandidateMode::Benchmark, WorkloadShape::Matmul { m, k, n }) => {
            let mut graph = Graph::new(&fixture.a, &fixture.b, m, k, n, stream, blas)?;
            graph.prepare_gradients()?;
            graph.stream.synchronize().map_err(display)?;
            let memory_allocation = free_bytes(&context)?;

            let first_started = Instant::now();
            graph.forward()?;
            graph.stream.synchronize().map_err(display)?;
            let first_result_ns = duration_ns(first_started.elapsed());
            let memory_forward = free_bytes(&context)?;
            graph.forward_backward()?;
            graph.stream.synchronize().map_err(display)?;
            let memory_backward = free_bytes(&context)?;

            let stream_for_forward = graph.stream.clone();
            let (forward_samples, forward_elapsed) = measure(
                || stream_for_forward.synchronize().map_err(display),
                || graph.forward(),
            )?;
            let stream_for_backward = graph.stream.clone();
            let (backward_samples, backward_elapsed) = measure(
                || stream_for_backward.synchronize().map_err(display),
                || graph.forward_backward(),
            )?;
            validate_measurement_window(&forward_samples, forward_elapsed)?;
            validate_measurement_window(&backward_samples, backward_elapsed)?;
            let forward_flops = flop_count(2, m, k, n)?;
            let backward_flops = flop_count(6, m, k, n)?;
            result.timing = Some(TimingResult {
                shape: ShapeResult {
                    m: m as u64,
                    k: k as u64,
                    n: n as u64,
                },
                warmup_iterations: WARMUP_ITERATIONS as u64,
                forward: timing_measurement(forward_samples, forward_elapsed, forward_flops),
                forward_backward: timing_measurement(
                    backward_samples,
                    backward_elapsed,
                    backward_flops,
                ),
                context_ns,
                jit_ns: 0,
                first_result_ns,
            });
            result.memory = Some(MemoryResult {
                free_bytes_after_context: Some(memory_context),
                free_bytes_after_allocation: Some(memory_allocation),
                free_bytes_after_forward: Some(memory_forward),
                free_bytes_after_backward: Some(memory_backward),
            });
            result.status = ResultStatus::Pass;
        }
        _ => return Err("mode and workload reached an invalid CUDA branch".to_owned()),
    }
    Ok(())
}

struct Graph {
    stream: Arc<CudaStream>,
    blas: CudaBlasLT,
    a: CudaSlice<bf16>,
    b: CudaSlice<bf16>,
    y: CudaSlice<bf16>,
    grad_y: Option<CudaSlice<bf16>>,
    a_t: Option<CudaSlice<bf16>>,
    b_t: Option<CudaSlice<bf16>>,
    grad_a: CudaSlice<bf16>,
    grad_b: CudaSlice<bf16>,
    m: usize,
    k: usize,
    n: usize,
}

struct Observed {
    y: Vec<f64>,
    loss: f64,
    grad_a: Vec<f64>,
    grad_b: Vec<f64>,
    free_after_forward: u64,
    free_after_backward: u64,
}

impl Graph {
    fn new(
        a: &[bf16],
        b: &[bf16],
        m: usize,
        k: usize,
        n: usize,
        stream: Arc<CudaStream>,
        blas: CudaBlasLT,
    ) -> Result<Self, String> {
        Ok(Self {
            a: stream.clone_htod(a).map_err(display)?,
            b: stream.clone_htod(b).map_err(display)?,
            y: stream.alloc_zeros(m * n).map_err(display)?,
            grad_y: None,
            a_t: None,
            b_t: None,
            grad_a: stream.alloc_zeros(m * k).map_err(display)?,
            grad_b: stream.alloc_zeros(k * n).map_err(display)?,
            stream,
            blas,
            m,
            k,
            n,
        })
    }

    fn forward(&mut self) -> Result<(), String> {
        row_major_matmul(
            &self.blas,
            &self.a,
            &self.b,
            &mut self.y,
            self.m,
            self.k,
            self.n,
        )
    }

    fn prepare_gradients(&mut self) -> Result<(), String> {
        self.forward()?;
        self.stream.synchronize().map_err(display)?;
        let y = self.stream.clone_dtoh(&self.y).map_err(display)?;
        let scale = 2.0_f32 / (self.m * self.n) as f32;
        let grad_y = y
            .iter()
            .map(|value| bf16::from_f32(value.to_f32() * scale))
            .collect::<Vec<_>>();
        let a = self.stream.clone_dtoh(&self.a).map_err(display)?;
        let b = self.stream.clone_dtoh(&self.b).map_err(display)?;
        self.grad_y = Some(self.stream.clone_htod(&grad_y).map_err(display)?);
        self.a_t = Some(
            self.stream
                .clone_htod(&transpose(&a, self.m, self.k))
                .map_err(display)?,
        );
        self.b_t = Some(
            self.stream
                .clone_htod(&transpose(&b, self.k, self.n))
                .map_err(display)?,
        );
        Ok(())
    }

    fn forward_backward(&mut self) -> Result<(), String> {
        if self.grad_y.is_none() {
            self.prepare_gradients()?;
        }
        self.forward()?;
        let grad_y = self.grad_y.as_ref().expect("prepared gradient");
        let b_t = self.b_t.as_ref().expect("prepared transpose");
        row_major_matmul(
            &self.blas,
            grad_y,
            b_t,
            &mut self.grad_a,
            self.m,
            self.n,
            self.k,
        )?;
        let a_t = self.a_t.as_ref().expect("prepared transpose");
        row_major_matmul(
            &self.blas,
            a_t,
            grad_y,
            &mut self.grad_b,
            self.k,
            self.m,
            self.n,
        )
    }

    fn evaluate(&mut self, context: &CudaContext) -> Result<Observed, String> {
        self.forward()?;
        self.stream.synchronize().map_err(display)?;
        let free_after_forward = free_bytes(context)?;
        let y = self.stream.clone_dtoh(&self.y).map_err(display)?;
        self.prepare_gradients()?;
        self.forward_backward()?;
        self.stream.synchronize().map_err(display)?;
        let free_after_backward = free_bytes(context)?;
        let grad_a = self.stream.clone_dtoh(&self.grad_a).map_err(display)?;
        let grad_b = self.stream.clone_dtoh(&self.grad_b).map_err(display)?;
        let y = y
            .into_iter()
            .map(|value| f64::from(value.to_f32()))
            .collect::<Vec<_>>();
        let loss = y.iter().map(|value| value * value).sum::<f64>() / y.len() as f64;
        Ok(Observed {
            y,
            loss,
            grad_a: grad_a
                .into_iter()
                .map(|value| f64::from(value.to_f32()))
                .collect(),
            grad_b: grad_b
                .into_iter()
                .map(|value| f64::from(value.to_f32()))
                .collect(),
            free_after_forward,
            free_after_backward,
        })
    }
}

fn row_major_matmul(
    blas: &CudaBlasLT,
    a: &CudaSlice<bf16>,
    b: &CudaSlice<bf16>,
    c: &mut CudaSlice<bf16>,
    m: usize,
    k: usize,
    n: usize,
) -> Result<(), String> {
    let config = MatmulConfig {
        transa: false,
        transb: false,
        transc: false,
        m: n as u64,
        n: m as u64,
        k: k as u64,
        alpha: 1.0,
        lda: n as i64,
        ldb: k as i64,
        beta: 0.0,
        ldc: n as i64,
        stride_a: None,
        stride_b: None,
        stride_c: None,
        stride_bias: None,
        batch_size: None,
    };
    // SAFETY: the slice lengths and leading dimensions are constructed from
    // the checked M/K/N workload shape, and the row-major transpose trick
    // passes B before A exactly as required by cuBLASLt's column-major API.
    unsafe { blas.matmul(config, b, a, c, None, None) }.map_err(display)
}

fn transpose(values: &[bf16], rows: usize, columns: usize) -> Vec<bf16> {
    let mut output = vec![bf16::ZERO; values.len()];
    for row in 0..rows {
        for column in 0..columns {
            output[column * rows + row] = values[row * columns + column];
        }
    }
    output
}

fn free_bytes(context: &CudaContext) -> Result<u64, String> {
    context
        .mem_get_info()
        .map_err(display)
        .and_then(|(free, _)| u64::try_from(free).map_err(|error| error.to_string()))
}

fn timing_measurement(samples_ns: Vec<u64>, elapsed_ns: u64, flop_count: u64) -> TimingMeasurement {
    let p50_ns = nearest_rank(&samples_ns, 50);
    let p95_ns = nearest_rank(&samples_ns, 95);
    TimingMeasurement {
        gflops: gflops(flop_count, p50_ns),
        sample_count: samples_ns.len() as u64,
        samples_ns,
        elapsed_ns,
        p50_ns,
        p95_ns,
        flop_count,
    }
}

fn flop_count(multiplier: u64, m: usize, k: usize, n: usize) -> Result<u64, String> {
    [m, k, n]
        .into_iter()
        .try_fold(multiplier, |value, dimension| {
            value.checked_mul(dimension as u64).ok_or(())
        })
        .map_err(|()| "FLOP count overflow".to_owned())
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_uses_row_major_layout() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].map(bf16::from_f32);
        let actual = transpose(&values, 2, 3);
        let expected = [1.0, 4.0, 2.0, 5.0, 3.0, 6.0].map(bf16::from_f32);
        assert_eq!(actual, expected);
    }

    #[test]
    fn flop_counts_match_frozen_formula() {
        assert_eq!(flop_count(2, 17, 31, 29).unwrap(), 30_566);
        assert_eq!(flop_count(6, 17, 31, 29).unwrap(), 91_698);
    }
}
