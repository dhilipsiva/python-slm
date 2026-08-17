//! E1B Phase 1: canonical-scale memory and throughput measurement.
//!
//! Everything E1B does afterwards is an optimization, and optimizing without a
//! baseline is guesswork. This probe runs the real `gqa-135m-v1` model through one
//! forward and backward on the device and reports peak device memory and wall time
//! per sequence, so each later lever can be attributed to a measured change.
//!
//! It is a diagnostic, not a gate: it reports what it observed and fails only when
//! the device genuinely cannot execute the model at all.
//!
//! Run with:
//!   cargo test --release --locked --features cuda --test e1b_canonical_scale_probe -- --ignored --nocapture

#![cfg(feature = "cuda")]

use burn::backend::Cuda;
use burn::tensor::{
    DType, FloatDType, Tensor, TensorCreationOptions, TensorData, backend::Backend,
};
use half::bf16;
use rust_llm_pretrain::train::cuda_backend::CudaTrainerBackend;
use rust_llm_pretrain::train::full_state::{GqaDimensions, ValidationSet, ValidationSpan};
use rust_llm_pretrain::train::trainer::{EVALUATION_TARGETS, TrainerBackend, TrainingBatch};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Device memory in use, in mebibytes, read from the vendor tool. The CUDA driver
/// API alternative needs a current context on the calling thread, which the burn
/// runtime owns, so an out-of-process reading is the reliable option here.
fn device_memory_used_mib() -> Option<u64> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used",
            "--format=csv,noheader,nounits",
            "--id=0",
        ])
        .output()
        .ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Samples device memory on a background thread and keeps the maximum, so the
/// reading covers transient backward-pass peaks rather than only the endpoints.
struct PeakSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PeakSampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let thread_stop = stop.clone();
        let thread_peak = peak.clone();
        let handle = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                if let Some(used) = device_memory_used_mib() {
                    thread_peak.fetch_max(used, Ordering::AcqRel);
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        });
        Self {
            stop,
            peak,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.peak.load(Ordering::Acquire)
    }
}

/// Raw device backend, without autodiff: this probe times kernels, not tapes.
type Raw = Cuda<bf16, i32>;
type RawDevice = burn::backend::cuda::CudaDevice;

/// Seconds per matmul of the given shape, with either FP32 or BF16 inputs.
fn matmul_seconds(
    device: &RawDevice,
    rows: usize,
    inner: usize,
    columns: usize,
    bf16_inputs: bool,
) -> f64 {
    let build = |height: usize, width: usize, offset: f32| {
        let values = (0..height * width)
            .map(|index| ((index as f32 * 0.7 + offset) % 3.0 - 1.5) / 8.0)
            .collect::<Vec<_>>();
        let tensor = Tensor::<Raw, 2>::from_data(
            TensorData::new(values, [height, width]),
            TensorCreationOptions::new(device.clone()).with_dtype(DType::F32),
        );
        if bf16_inputs {
            tensor.cast(FloatDType::BF16)
        } else {
            tensor
        }
    };
    let left = build(rows, inner, 0.0);
    let right = build(inner, columns, 1.3);

    // One warmup so kernel compilation and autotuning are not timed.
    let mut last = Some(left.clone().matmul(right.clone()));
    Raw::sync(device).expect("warmup synchronization");

    const REPEATS: usize = 20;
    let started = Instant::now();
    for _ in 0..REPEATS {
        // Overwrite rather than collect: keeping twenty outputs of this size
        // would exhaust the device before the timing finished.
        last = Some(left.clone().matmul(right.clone()));
    }
    Raw::sync(device).expect("timed synchronization");
    let elapsed = started.elapsed().as_secs_f64() / REPEATS as f64;
    drop(last);
    elapsed
}

fn canonical_validation_set(dimensions: &GqaDimensions) -> ValidationSet {
    let mut spans = Vec::new();
    let mut remaining = EVALUATION_TARGETS;
    while remaining > 0 {
        let length = remaining.min(dimensions.max_context as u64) as usize;
        spans.push(ValidationSpan {
            input_ids: vec![7_u16; length],
            target_ids: vec![9_u16; length],
        });
        remaining -= length as u64;
    }
    ValidationSet::new(spans, dimensions).expect("canonical validation set")
}

fn sequence_batch(first_target: u64, sequence_targets: u64, sequences: u64) -> TrainingBatch {
    let total = sequence_targets * sequences;
    TrainingBatch {
        first_target,
        valid_targets: total,
        input_ids: (0..total).map(|index| (index % 32_000) as u16).collect(),
        target_ids: (0..total)
            .map(|index| ((index + 1) % 32_000) as u16)
            .collect(),
        sequence_lengths: vec![sequence_targets; sequences as usize],
    }
}

/// What tensor cores are worth on this device, measured before committing to the
/// storage change that would reach them.
///
/// E1B Phase 5 replaces the FP32 substrate with genuine BF16 storage, which is a
/// large and numerically consequential change; it is only justified if BF16
/// matmul is materially faster than the FP32 matmul the graph runs today. The
/// accumulation question is already settled by reading cubek: `MatmulElems::from_globals`
/// promotes the accumulator to FP32 whenever the output is BF16 or FP16
/// (`cubek-matmul-0.2.0/src/definition/spec.rs:276`), so the contract's
/// `sensitive_accumulation: "fp32"` survives either way. This measures the payoff.
///
/// The shapes are the graph's own: the FFN projections and the untied LM head at
/// eight sequences per dispatch.
#[test]
#[ignore = "E1B diagnostic; run explicitly with --ignored on the prototype host"]
fn probe_bf16_versus_f32_matmul_throughput() {
    let device = RawDevice::new(0);
    let rows = 8 * 2_048;

    println!("\n=== E1B probe: BF16 versus FP32 matmul ===");
    println!("  rows: {rows} (eight 2,048-target sequences)");
    for (label, inner, columns) in [
        ("ffn gate/up  768 -> 2432", 768_usize, 2_432_usize),
        ("ffn down    2432 ->  768", 2_432, 768),
        ("lm head      768 -> 32000", 768, 32_000),
    ] {
        let f32_seconds = matmul_seconds(&device, rows, inner, columns, false);
        let bf16_seconds = matmul_seconds(&device, rows, inner, columns, true);
        let flops = 2.0 * rows as f64 * inner as f64 * columns as f64;
        println!(
            "  {label}: fp32 {:.2} TFLOP/s, bf16 {:.2} TFLOP/s, speedup {:.2}x",
            flops / f32_seconds / 1e12,
            flops / bf16_seconds / 1e12,
            f32_seconds / bf16_seconds
        );
    }
}

#[test]
#[ignore = "E1B diagnostic; run explicitly with --ignored on the prototype host"]
fn probe_canonical_scale_memory_and_throughput() {
    let dimensions = GqaDimensions::canonical();
    let sequence_targets = 2_048_u64;
    let sequences_per_dispatch: u64 = std::env::var("RUST_LLM_E1B_SEQUENCES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let baseline = device_memory_used_mib();

    println!("\n=== E1B probe: canonical scale, one sequence per dispatch ===");
    println!("  model: {} parameters", dimensions.parameter_count());
    println!("  sequences per dispatch: {sequences_per_dispatch}");
    match baseline {
        Some(used) => println!("  device memory before construction: {used} MiB"),
        None => println!("  device memory reading unavailable (nvidia-smi not found)"),
    }

    let sampler = PeakSampler::start();
    let construction_started = Instant::now();
    let mut backend = CudaTrainerBackend::canonical(0, canonical_validation_set(&dimensions))
        .expect("canonical backend construction");
    let construction = construction_started.elapsed();
    println!(
        "  construction (INIT-001 + device upload): {:.2} s",
        construction.as_secs_f64()
    );

    // One warmup dispatch so kernel compilation and autotuning are not timed.
    let warmup_started = Instant::now();
    backend
        .accumulate(&sequence_batch(0, sequence_targets, sequences_per_dispatch))
        .expect("warmup forward and backward");
    println!(
        "  first dispatch including kernel compilation: {:.2} s",
        warmup_started.elapsed().as_secs_f64()
    );

    let measured_dispatches = 5_u64;
    let steady_started = Instant::now();
    for index in 0..measured_dispatches {
        let first_target = (index + 1) * sequence_targets * sequences_per_dispatch;
        backend
            .accumulate(&sequence_batch(
                first_target,
                sequence_targets,
                sequences_per_dispatch,
            ))
            .expect("steady-state forward and backward");
    }
    let steady = steady_started.elapsed();
    let peak = sampler.finish();

    let per_sequence = steady.as_secs_f64() / measured_dispatches as f64;
    let targets_per_second = (sequence_targets * sequences_per_dispatch) as f64 / per_sequence;
    let required = 2_000_000_000.0 / 28_800.0;

    println!("  ---");
    println!("  peak device memory: {peak} MiB");
    if let Some(used) = baseline {
        println!(
            "  attributable to this process: {} MiB",
            peak.saturating_sub(used)
        );
    }
    println!("  steady-state per sequence: {:.4} s", per_sequence);
    println!("  throughput: {targets_per_second:.0} targets/s");
    println!("  SLA needs: {required:.0} targets/s over 2,000,000,000 targets");
    println!(
        "  projected wall clock for the full run: {:.1} hours",
        2_000_000_000.0 / targets_per_second / 3_600.0
    );
    println!(
        "  frozen micro-batch of 16 sequences would need roughly {} MiB of activations \
         if they scale linearly",
        peak.saturating_sub(baseline.unwrap_or(0)) * 16
    );
}
