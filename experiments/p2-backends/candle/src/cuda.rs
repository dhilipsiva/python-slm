use std::{hint::black_box, time::Instant};

use candle_core::{DType, Device, Tensor, Var};
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
    candle_core::cuda::set_gemm_reduced_precision_bf16(false);
    if candle_core::cuda::gemm_reduced_precision_bf16() {
        return Err("Candle failed to disable reduced-precision BF16 GEMM reduction".to_owned());
    }
    result.provenance.fp32_accumulation_evidence =
        "runtime_getter=candle_core::cuda::gemm_reduced_precision_bf16;observed=false;compute=CUBLAS_COMPUTE_32F"
            .to_owned();

    let fixture = load(&args.fixture_dir, args.workload).map_err(|error| error.to_string())?;
    result.fixture_hashes = Some(fixture.hashes());
    let context_started = Instant::now();
    let device = Device::new_cuda(0).map_err(display)?;
    device.synchronize().map_err(display)?;
    let context_ns = duration_ns(context_started.elapsed());
    let memory_context = free_bytes()?;

    match (args.mode, args.workload.shape()) {
        (CandidateMode::Correctness, WorkloadShape::Allocation(shape)) => {
            let tensor = Tensor::from_vec(fixture.a.clone(), &shape, &device).map_err(display)?;
            device.synchronize().map_err(display)?;
            let memory_allocation = free_bytes()?;
            let output = tensor
                .flatten_all()
                .map_err(display)?
                .to_vec1::<bf16>()
                .map_err(display)?;
            device.synchronize().map_err(display)?;
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
                return Err("Candle BF16 allocation round-trip was not bitwise equal".to_owned());
            }
        }
        (CandidateMode::Correctness, WorkloadShape::Matmul { m, k, n }) => {
            let graph = Graph::new(&fixture.a, &fixture.b, m, k, n, &device)?;
            device.synchronize().map_err(display)?;
            let memory_allocation = free_bytes()?;
            let observed = graph.evaluate()?;
            let memory_forward = observed.free_after_forward;
            let memory_backward = observed.free_after_backward;
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
                return Err("Candle correctness metrics failed the frozen tolerances".to_owned());
            }
            result.correctness = Some(correctness);
            result.memory = Some(MemoryResult {
                free_bytes_after_context: Some(memory_context),
                free_bytes_after_allocation: Some(memory_allocation),
                free_bytes_after_forward: Some(memory_forward),
                free_bytes_after_backward: Some(memory_backward),
            });
            result.status = ResultStatus::Pass;
        }
        (CandidateMode::Benchmark, WorkloadShape::Matmul { m, k, n }) => {
            let graph = Graph::new(&fixture.a, &fixture.b, m, k, n, &device)?;
            device.synchronize().map_err(display)?;
            let memory_allocation = free_bytes()?;

            let first_started = Instant::now();
            let first_y = graph.forward()?;
            device.synchronize().map_err(display)?;
            black_box(&first_y);
            let first_result_ns = duration_ns(first_started.elapsed());
            let memory_forward = free_bytes()?;

            let first_loss = first_y
                .to_dtype(DType::F32)
                .map_err(display)?
                .sqr()
                .map_err(display)?
                .mean_all()
                .map_err(display)?;
            let first_grads = first_loss.backward().map_err(display)?;
            device.synchronize().map_err(display)?;
            black_box(&first_grads);
            let memory_backward = free_bytes()?;

            let (forward_samples, forward_elapsed) = measure(
                || device.synchronize().map_err(display),
                || {
                    let tensor = graph.forward()?;
                    black_box(tensor);
                    Ok(())
                },
            )?;
            let (backward_samples, backward_elapsed) = measure(
                || device.synchronize().map_err(display),
                || graph.forward_backward(),
            )?;
            validate_measurement_window(&forward_samples, forward_elapsed)?;
            validate_measurement_window(&backward_samples, backward_elapsed)?;
            let forward_flops = flop_count(2, m, k, n)?;
            let backward_flops = flop_count(6, m, k, n)?;
            let forward = timing_measurement(forward_samples, forward_elapsed, forward_flops);
            let forward_backward =
                timing_measurement(backward_samples, backward_elapsed, backward_flops);
            result.timing = Some(TimingResult {
                shape: ShapeResult {
                    m: m as u64,
                    k: k as u64,
                    n: n as u64,
                },
                warmup_iterations: WARMUP_ITERATIONS as u64,
                forward,
                forward_backward,
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
    a: Var,
    b: Var,
    device: Device,
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
        device: &Device,
    ) -> Result<Self, String> {
        Ok(Self {
            a: Var::from_slice(a, (m, k), device).map_err(display)?,
            b: Var::from_slice(b, (k, n), device).map_err(display)?,
            device: device.clone(),
        })
    }

    fn forward(&self) -> Result<Tensor, String> {
        self.a.matmul(&self.b).map_err(display)
    }

    fn forward_backward(&self) -> Result<(), String> {
        let y = self.forward()?;
        let loss = y
            .to_dtype(DType::F32)
            .map_err(display)?
            .sqr()
            .map_err(display)?
            .mean_all()
            .map_err(display)?;
        let grads = loss.backward().map_err(display)?;
        black_box(grads);
        Ok(())
    }

    fn evaluate(&self) -> Result<Observed, String> {
        let y = self.forward()?;
        self.device.synchronize().map_err(display)?;
        let free_after_forward = free_bytes()?;
        let loss = y
            .to_dtype(DType::F32)
            .map_err(display)?
            .sqr()
            .map_err(display)?
            .mean_all()
            .map_err(display)?;
        let grads = loss.backward().map_err(display)?;
        self.device.synchronize().map_err(display)?;
        let free_after_backward = free_bytes()?;
        let grad_a = grads
            .get(&self.a)
            .ok_or_else(|| "Candle returned no gradient for A".to_owned())?;
        let grad_b = grads
            .get(&self.b)
            .ok_or_else(|| "Candle returned no gradient for B".to_owned())?;
        Ok(Observed {
            y: tensor_f64(&y)?,
            loss: loss.to_scalar::<f32>().map_err(display)? as f64,
            grad_a: tensor_f64(grad_a)?,
            grad_b: tensor_f64(grad_b)?,
            free_after_forward,
            free_after_backward,
        })
    }
}

fn tensor_f64(tensor: &Tensor) -> Result<Vec<f64>, String> {
    tensor
        .to_dtype(DType::F32)
        .map_err(display)?
        .flatten_all()
        .map_err(display)?
        .to_vec1::<f32>()
        .map_err(display)
        .map(|values| values.into_iter().map(f64::from).collect())
}

fn free_bytes() -> Result<u64, String> {
    candle_core::cuda::cudarc::driver::result::mem_get_info()
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
    fn flop_counts_match_frozen_formula() {
        assert_eq!(flop_count(2, 17, 31, 29).unwrap(), 30_566);
        assert_eq!(flop_count(6, 17, 31, 29).unwrap(), 91_698);
    }
}
