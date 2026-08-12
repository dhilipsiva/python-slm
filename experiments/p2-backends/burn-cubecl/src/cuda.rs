use std::{
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

use burn::{
    backend::{Autodiff, Cuda, cuda::CudaDevice},
    prelude::Backend,
    tensor::{FloatDType, Tensor, TensorData, backend::AutodiffBackend},
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

type Gpu = Autodiff<Cuda<bf16, i32>>;

pub fn run(args: &CandidateArgs, result: &mut CandidateResult) -> Result<(), String> {
    if args.mode == CandidateMode::CpuSmoke {
        return Err("CUDA runner cannot execute cpu-smoke".to_owned());
    }
    result.provenance.fp32_accumulation_evidence = fp32_accumulation_evidence()?;
    let fixture = load(&args.fixture_dir, args.workload).map_err(|error| error.to_string())?;
    result.fixture_hashes = Some(fixture.hashes());
    let context_started = Instant::now();
    let device = CudaDevice::new(0);
    Gpu::sync(&device).map_err(display)?;
    let context_ns = duration_ns(context_started.elapsed());
    let memory_context = free_bytes()?;

    match (args.mode, args.workload.shape()) {
        (CandidateMode::Correctness, WorkloadShape::Allocation(shape)) => {
            let tensor =
                Tensor::<Gpu, 3>::from_data(TensorData::new(fixture.a.clone(), shape), &device);
            Gpu::sync(&device).map_err(display)?;
            let memory_allocation = free_bytes()?;
            let output = tensor
                .try_into_data()
                .map_err(display)?
                .to_vec::<bf16>()
                .map_err(display)?;
            Gpu::sync(&device).map_err(display)?;
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
                return Err("Burn BF16 allocation round-trip was not bitwise equal".to_owned());
            }
        }
        (CandidateMode::Correctness, WorkloadShape::Matmul { m, k, n }) => {
            let graph = Graph::new(&fixture.a, &fixture.b, m, k, n, &device);
            Gpu::sync(&device).map_err(display)?;
            let memory_allocation = free_bytes()?;
            let observed = graph.evaluate()?;
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
                return Err("Burn correctness metrics failed the frozen tolerances".to_owned());
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
            let graph = Graph::new(&fixture.a, &fixture.b, m, k, n, &device);
            Gpu::sync(&device).map_err(display)?;
            let memory_allocation = free_bytes()?;

            let jit_started = Instant::now();
            let jit_y = graph.forward();
            Gpu::sync(&device).map_err(display)?;
            black_box(jit_y);
            let jit_ns = duration_ns(jit_started.elapsed());

            let first_started = Instant::now();
            let first_y = graph.forward();
            Gpu::sync(&device).map_err(display)?;
            black_box(&first_y);
            let first_result_ns = duration_ns(first_started.elapsed());
            let memory_forward = free_bytes()?;
            let first_loss = first_y.cast(FloatDType::F32).powf_scalar(2.0).mean();
            let first_grads = first_loss.backward();
            Gpu::sync(&device).map_err(display)?;
            black_box(&first_grads);
            let memory_backward = free_bytes()?;

            let (forward_samples, forward_elapsed) = measure(
                || Gpu::sync(&device).map_err(display),
                || {
                    black_box(graph.forward());
                    Ok(())
                },
            )?;
            let (backward_samples, backward_elapsed) = measure(
                || Gpu::sync(&device).map_err(display),
                || {
                    graph.forward_backward();
                    Ok(())
                },
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
                jit_ns,
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

fn fp32_accumulation_evidence() -> Result<String, String> {
    const ARTIFACT: &[u8] = include_bytes!("../evidence/fp32-accumulation-v1.json");
    let value: serde_json::Value = serde_json::from_slice(ARTIFACT).map_err(display)?;
    let crate_checksum = value
        .get("crate_checksum_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Burn FP32 evidence has no crate checksum".to_owned())?;
    let source_sha256 = value
        .get("source_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Burn FP32 evidence has no source_sha256".to_owned())?;
    let assertion = value
        .get("required_assertion")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Burn FP32 evidence has no accumulator assertion".to_owned())?;
    if value.get("crate").and_then(serde_json::Value::as_str) != Some("cubek-matmul")
        || value.get("version").and_then(serde_json::Value::as_str) != Some("0.2.0")
        || value
            .get("target_condition")
            .and_then(serde_json::Value::as_str)
            != Some("not(target_os=macos)")
        || value
            .get("source_relative_path")
            .and_then(serde_json::Value::as_str)
            != Some("src/definition/spec.rs")
        || value
            .get("source_lines")
            .and_then(serde_json::Value::as_str)
            != Some("85-91")
        || assertion != "BF16_MATMUL_PRECISION_ACCUMULATOR_F32"
    {
        return Err("Burn FP32 evidence identity is invalid".to_owned());
    }
    verify_experiment_lock_binding(crate_checksum)?;
    let _artifact_sha256 = p2_backend_common::sha256_bytes(ARTIFACT);
    let observed_sha256 = verify_locked_cubek_matmul_source(source_sha256)?;
    Ok(format!(
        "crate=cubek-matmul@0.2.0;crate-checksum-sha256={crate_checksum};source-sha256={observed_sha256};locator=cubek-matmul-0.2.0/src/definition/spec.rs:85-91;assertion={assertion};cfg=not-macos"
    ))
}

fn verify_experiment_lock_binding(expected_checksum: &str) -> Result<(), String> {
    const LOCK: &str = include_str!("../../Cargo.lock");
    let packages = LOCK
        .split("[[package]]")
        .filter(|section| {
            section
                .lines()
                .any(|line| line == "name = \"cubek-matmul\"")
        })
        .collect::<Vec<_>>();
    if packages.len() != 1 {
        return Err("experiment lock must contain exactly one cubek-matmul package".to_owned());
    }
    let package = packages[0];
    if !package.lines().any(|line| line == "version = \"0.2.0\"")
        || !package.lines().any(|line| {
            line == "source = \"registry+https://github.com/rust-lang/crates.io-index\""
        })
        || !package
            .lines()
            .any(|line| line == format!("checksum = \"{expected_checksum}\""))
    {
        return Err(
            "experiment lock cubek-matmul identity does not match FP32 evidence".to_owned(),
        );
    }
    Ok(())
}

fn verify_locked_cubek_matmul_source(expected_sha256: &str) -> Result<String, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|path| path.join(".cargo"))
        })
        .ok_or_else(|| "CARGO_HOME and USERPROFILE are unavailable".to_owned())?;
    let registry_sources = cargo_home.join("registry").join("src");
    let entries = std::fs::read_dir(&registry_sources)
        .map_err(|_| "Cargo registry source directory is unavailable".to_owned())?;
    let relative = Path::new("cubek-matmul-0.2.0")
        .join("src")
        .join("definition")
        .join("spec.rs");
    let mut matching_sources = Vec::<Vec<u8>>::new();
    let mut observed_count = 0_usize;
    for entry in entries {
        let entry = entry.map_err(|_| "Cargo registry source enumeration failed".to_owned())?;
        let source = entry.path().join(&relative);
        if !source.is_file() {
            continue;
        }
        let bytes = std::fs::read(&source)
            .map_err(|_| "locked cubek-matmul source could not be read".to_owned())?;
        let digest = p2_backend_common::sha256_bytes(&bytes);
        observed_count += 1;
        if digest == expected_sha256 {
            matching_sources.push(bytes);
        }
    }
    if observed_count == 0 {
        return Err("locked cubek-matmul 0.2.0 source is absent".to_owned());
    }
    if observed_count != 1 || matching_sources.len() != 1 {
        return Err(
            "locked cubek-matmul source must have exactly one matching registry copy".to_owned(),
        );
    }
    let source = matching_sources.pop().expect("exactly one matching source");
    let source = std::str::from_utf8(&source)
        .map_err(|_| "locked cubek-matmul source is not UTF-8".to_owned())?;
    let lines = source.lines().skip(84).take(7).collect::<Vec<_>>();
    let precision_matches = lines.len() == 7
        && lines[0].contains("impl MatmulPrecision for bf16")
        && lines[1].contains("type Lhs = (bf16, bf16)")
        && lines[2].contains("type Rhs = (bf16, bf16)")
        && lines[5].contains("cfg(not(target_os = \"macos\"))")
        && lines[6].contains("type Acc = (bf16, f32)");
    if !precision_matches || cfg!(target_os = "macos") {
        return Err("cubek-matmul BF16 MatmulPrecision accumulator evidence changed".to_owned());
    }
    Ok(expected_sha256.to_owned())
}

struct Graph {
    a: Tensor<Gpu, 2>,
    b: Tensor<Gpu, 2>,
    device: CudaDevice,
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
    fn new(a: &[bf16], b: &[bf16], m: usize, k: usize, n: usize, device: &CudaDevice) -> Self {
        Self {
            a: Tensor::from_data(TensorData::new(a.to_vec(), [m, k]), device).require_grad(),
            b: Tensor::from_data(TensorData::new(b.to_vec(), [k, n]), device).require_grad(),
            device: device.clone(),
        }
    }

    fn forward(&self) -> Tensor<Gpu, 2> {
        self.a.clone().matmul(self.b.clone())
    }

    fn forward_backward(&self) {
        let loss = self.forward().cast(FloatDType::F32).powf_scalar(2.0).mean();
        black_box(loss.backward());
    }

    fn evaluate(&self) -> Result<Observed, String> {
        let y = self.forward();
        Gpu::sync(&self.device).map_err(display)?;
        let free_after_forward = free_bytes()?;
        let loss = y.clone().cast(FloatDType::F32).powf_scalar(2.0).mean();
        let loss_values = loss
            .clone()
            .try_into_data()
            .map_err(display)?
            .to_vec::<f32>()
            .map_err(display)?;
        let loss_value = f64::from(
            *loss_values
                .first()
                .ok_or_else(|| "Burn returned no scalar loss value".to_owned())?,
        );
        let grads = loss.backward();
        Gpu::sync(&self.device).map_err(display)?;
        let free_after_backward = free_bytes()?;
        let grad_a = self
            .a
            .grad(&grads)
            .ok_or_else(|| "Burn returned no gradient for A".to_owned())?;
        let grad_b = self
            .b
            .grad(&grads)
            .ok_or_else(|| "Burn returned no gradient for B".to_owned())?;
        Ok(Observed {
            y: tensor_bf16_f64(y)?,
            loss: loss_value,
            grad_a: tensor_f32_f64(grad_a)?,
            grad_b: tensor_f32_f64(grad_b)?,
            free_after_forward,
            free_after_backward,
        })
    }
}

fn tensor_bf16_f64(tensor: Tensor<Gpu, 2>) -> Result<Vec<f64>, String> {
    tensor
        .try_into_data()
        .map_err(display)?
        .to_vec::<bf16>()
        .map_err(display)
        .map(|values| {
            values
                .into_iter()
                .map(|value| f64::from(value.to_f32()))
                .collect()
        })
}

fn tensor_f32_f64(
    tensor: Tensor<<Gpu as AutodiffBackend>::InnerBackend, 2>,
) -> Result<Vec<f64>, String> {
    tensor
        .cast(FloatDType::F32)
        .try_into_data()
        .map_err(display)?
        .to_vec::<f32>()
        .map_err(display)
        .map(|values| values.into_iter().map(f64::from).collect())
}

fn free_bytes() -> Result<u64, String> {
    cudarc::driver::result::mem_get_info()
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
