use crate::{
    config::{LlamaConfig, MemoryEstimate, TrainConfig},
    data::TokenDataset,
    model::LlamaModel,
};
use anyhow::{Context, Result, ensure};
use burn::{
    module::{AutodiffModule, Module},
    nn::loss::CrossEntropyLoss,
    optim::{AdamWConfig, GradientsAccumulator, GradientsParams, Optimizer},
    record::{FullPrecisionSettings, NamedMpkFileRecorder},
    tensor::{
        ElementConversion, Int, Tensor, TensorData,
        backend::{AutodiffBackend, Backend},
    },
};
use serde::Serialize;
use std::{path::Path, time::Instant};
use tracing::{info, warn};

const TRAINING_SEED: u64 = 42;

/// Synchronized measurements from the correctness-oriented training loop.
///
/// This is deliberately not described as an optimized-training receipt. The
/// implementation uses Burn's differentiable reference attention and ordinary
/// AdamW path; both must be replaced or benchmarked before claiming the target
/// Blackwell throughput.
#[derive(Debug, Clone, Serialize)]
pub struct TrainingSummary {
    pub backend: String,
    pub device_index: Option<usize>,
    pub seed: u64,
    pub parameter_count: u64,
    pub target_tokens: u64,
    pub processed_tokens: u64,
    pub alignment_overshoot_tokens: u64,
    pub micro_steps: u64,
    pub optimizer_steps: u64,
    pub elapsed_seconds: f64,
    pub average_tokens_per_second: f64,
    pub target_tokens_per_second: f64,
    pub met_throughput_target: bool,
    pub last_loss: Option<f64>,
    pub estimated_reference_peak_gib: f64,
    pub configured_vram_budget_gib: f64,
    pub memory_estimate: MemoryEstimate,
    pub reference_attention: bool,
    pub checkpoint_path: Option<String>,
}

/// CPU correctness entry point for CI and tiny smoke configurations.
///
/// This is intentionally not exposed by the production `train` CLI. It exists so
/// callers can execute exactly the same accumulation and optimizer logic without
/// requiring CUDA on build hosts.
#[cfg(feature = "cpu-reference")]
pub fn run_cpu_reference_training(
    manifest_path: &Path,
    model: LlamaConfig,
    train: TrainConfig,
    verify_hashes: bool,
) -> Result<TrainingSummary> {
    use burn::backend::{Autodiff, Flex};

    run_reference_training::<Autodiff<Flex>>(
        manifest_path,
        model,
        train,
        Default::default(),
        None,
        verify_hashes,
        None,
    )
}

/// CUDA entry point requested by the CLI.
///
/// Burn's CUDA backend is wrapped in `Autodiff` and instantiated with bf16
/// floating-point storage. This establishes a real zero-Python CUDA training
/// path, but the attention and optimizer implementations remain reference-grade.
#[cfg(feature = "cuda")]
pub fn run_cuda_training(
    manifest_path: &Path,
    model: LlamaConfig,
    train: TrainConfig,
    device_index: usize,
    verify_hashes: bool,
) -> Result<TrainingSummary> {
    run_cuda_training_with_checkpoint(
        manifest_path,
        model,
        train,
        device_index,
        verify_hashes,
        None,
    )
}

/// CUDA entry point with an optional atomically finalized model-only checkpoint.
/// Burn appends `.mpk` to the supplied checkpoint base. Optimizer state and
/// periodic restart checkpoints remain requirements for the optimized backend.
#[cfg(feature = "cuda")]
pub fn run_cuda_training_with_checkpoint(
    manifest_path: &Path,
    model: LlamaConfig,
    train: TrainConfig,
    device_index: usize,
    verify_hashes: bool,
    checkpoint_base: Option<&Path>,
) -> Result<TrainingSummary> {
    use burn::backend::{Autodiff, Cuda, cuda::CudaDevice};

    #[cfg(feature = "cuda-msvc-link")]
    tracing::info!(
        versions = ?crate::cuda_probe::linked_cuda_versions(device_index)?,
        "validated linked MSVC/CUDA library ABIs"
    );

    type CudaAutodiff = Autodiff<Cuda<half::bf16, i32>>;
    let device = CudaDevice::new(device_index);
    run_reference_training::<CudaAutodiff>(
        manifest_path,
        model,
        train,
        device,
        Some(device_index),
        verify_hashes,
        checkpoint_base,
    )
}

fn run_reference_training<B>(
    manifest_path: &Path,
    model_config: LlamaConfig,
    train_config: TrainConfig,
    device: B::Device,
    device_index: Option<usize>,
    verify_hashes: bool,
    checkpoint_base: Option<&Path>,
) -> Result<TrainingSummary>
where
    B: AutodiffBackend,
    LlamaModel<B>: AutodiffModule<B>,
{
    train_config.validate(&model_config)?;
    ensure!(
        train_config.log_every_micro_steps > 0,
        "log_every_micro_steps must be positive"
    );
    train_config.enforce_optimized_kernel_gate()?;

    let memory_estimate = MemoryEstimate::bf16(&model_config, &train_config);
    let estimated_reference_peak_gib = estimate_reference_peak_gib(&model_config, &memory_estimate);
    ensure!(
        estimated_reference_peak_gib <= train_config.vram_budget_gib,
        "reference-attention analytical peak estimate is {estimated_reference_peak_gib:.2} GiB, above the configured {:.2} GiB budget; reduce micro_batch_size/context_length. This preflight is conservative but is not a live CUDA allocator measurement",
        train_config.vram_budget_gib
    );

    let dataset = TokenDataset::open(manifest_path, verify_hashes)
        .with_context(|| format!("opening token dataset {}", manifest_path.display()))?;
    ensure!(
        dataset.manifest().vocabulary_size == model_config.vocab_size,
        "dataset vocabulary size {} does not match model vocabulary size {}",
        dataset.manifest().vocabulary_size,
        model_config.vocab_size
    );
    ensure!(
        dataset.len() > train_config.tokens_per_micro_step(&model_config),
        "token dataset is too small for one full next-token micro-batch"
    );

    let backend_name = B::name(&device);
    let optimizer_steps_planned = train_config.total_optimizer_steps(&model_config);
    let micro_steps_planned = optimizer_steps_planned
        .checked_mul(train_config.gradient_accumulation as u64)
        .context("planned micro-step count overflow")?;
    let processed_tokens_planned = micro_steps_planned
        .checked_mul(train_config.tokens_per_micro_step(&model_config))
        .context("planned token count overflow")?;
    let alignment_overshoot_tokens =
        processed_tokens_planned.saturating_sub(train_config.target_tokens);

    info!(
        backend = %backend_name,
        parameter_count = memory_estimate.parameter_count,
        micro_batch_size = train_config.micro_batch_size,
        sequence_length = model_config.context_length,
        gradient_accumulation = train_config.gradient_accumulation,
        optimizer_steps = optimizer_steps_planned,
        target_tokens = train_config.target_tokens,
        processed_tokens = processed_tokens_planned,
        alignment_overshoot_tokens,
        estimated_reference_peak_gib,
        configured_vram_budget_gib = train_config.vram_budget_gib,
        "starting reference-attention pre-training loop; estimate is analytical, not live VRAM telemetry"
    );
    warn!(
        "this Burn loop is a correctness/reference implementation and is not evidence for the 8-hour or 75k-token/s target"
    );

    B::seed(&device, TRAINING_SEED);
    let mut model = LlamaModel::<B>::new(&model_config, &device)?;
    let mut optimizer = AdamWConfig::new()
        .with_beta_1(train_config.beta_1)
        .with_beta_2(train_config.beta_2)
        .with_epsilon(train_config.adam_epsilon)
        .with_weight_decay(train_config.weight_decay)
        .init::<B, LlamaModel<B>>();
    let loss_fn = CrossEntropyLoss::new(None, &device);
    let mut accumulator = GradientsAccumulator::<LlamaModel<B>>::new();

    sync_backend::<B>(&device, "before timing")?;
    let started = Instant::now();
    let mut window_started = started;
    let mut window_start_tokens = 0_u64;
    let mut processed_tokens = 0_u64;
    let mut micro_steps = 0_u64;
    let mut optimizer_steps = 0_u64;
    let mut last_loss = None;
    let tokens_per_micro_step = train_config.tokens_per_micro_step(&model_config);
    let flattened_tokens = usize::try_from(tokens_per_micro_step)
        .context("micro-batch token count does not fit usize")?;

    for optimizer_step in 0..optimizer_steps_planned {
        let learning_rate = train_config.learning_rate(optimizer_step, &model_config);

        for _ in 0..train_config.gradient_accumulation {
            let (inputs, labels) = dataset.batch(
                processed_tokens,
                train_config.micro_batch_size,
                model_config.context_length,
            )?;
            let inputs = Tensor::<B, 2, Int>::from_data(
                TensorData::new(
                    inputs,
                    [train_config.micro_batch_size, model_config.context_length],
                ),
                &device,
            );
            let labels = Tensor::<B, 2, Int>::from_data(
                TensorData::new(
                    labels,
                    [train_config.micro_batch_size, model_config.context_length],
                ),
                &device,
            );

            let logits = model
                .forward(inputs)
                .reshape([flattened_tokens, model_config.vocab_size]);
            let labels = labels.reshape([flattened_tokens]);
            let loss = loss_fn.forward(logits, labels);

            micro_steps += 1;
            let should_log = micro_steps.is_multiple_of(train_config.log_every_micro_steps)
                || micro_steps == micro_steps_planned;
            if should_log {
                let value = loss
                    .clone()
                    .try_into_scalar()
                    .context("reading synchronized training loss")?
                    .elem::<f64>();
                last_loss = Some(value);
            }

            // Burn's accumulator sums parameter gradients. Scaling each mean loss
            // yields the same average gradient as one logical large batch.
            let scaled_loss = loss.div_scalar(train_config.gradient_accumulation as f64);
            let grads = scaled_loss.backward();
            let grads = GradientsParams::from_grads(grads, &model);
            accumulator.accumulate::<B>(&model, grads);
            processed_tokens += tokens_per_micro_step;

            if should_log {
                sync_backend::<B>(&device, "at throughput log boundary")?;
                let now = Instant::now();
                let window_seconds = now.duration_since(window_started).as_secs_f64();
                let overall_seconds = now.duration_since(started).as_secs_f64();
                let window_tokens = processed_tokens - window_start_tokens;
                let window_tokens_per_second =
                    window_tokens as f64 / window_seconds.max(f64::EPSILON);
                let average_tokens_per_second =
                    processed_tokens as f64 / overall_seconds.max(f64::EPSILON);

                info!(
                    micro_step = micro_steps,
                    optimizer_step = optimizer_steps,
                    processed_tokens,
                    loss = last_loss,
                    learning_rate,
                    window_tokens_per_second,
                    average_tokens_per_second,
                    target_tokens_per_second = train_config.target_tokens_per_second,
                    estimated_reference_peak_gib,
                    "synchronized training throughput"
                );
                window_started = now;
                window_start_tokens = processed_tokens;
            }
        }

        let grads = accumulator.grads();
        model = optimizer.step(learning_rate, model, grads);
        optimizer_steps += 1;
    }

    sync_backend::<B>(&device, "after final optimizer step")?;
    let elapsed_seconds = started.elapsed().as_secs_f64();
    let average_tokens_per_second = processed_tokens as f64 / elapsed_seconds.max(f64::EPSILON);
    let met_throughput_target = average_tokens_per_second >= train_config.target_tokens_per_second;
    if !met_throughput_target {
        warn!(
            average_tokens_per_second,
            target_tokens_per_second = train_config.target_tokens_per_second,
            "reference implementation missed the configured throughput target"
        );
    }
    let checkpoint_path = checkpoint_base
        .map(|base| save_model_checkpoint::<B>(model, base))
        .transpose()?;

    Ok(TrainingSummary {
        backend: backend_name,
        device_index,
        seed: TRAINING_SEED,
        parameter_count: memory_estimate.parameter_count,
        target_tokens: train_config.target_tokens,
        processed_tokens,
        alignment_overshoot_tokens,
        micro_steps,
        optimizer_steps,
        elapsed_seconds,
        average_tokens_per_second,
        target_tokens_per_second: train_config.target_tokens_per_second,
        met_throughput_target,
        last_loss,
        estimated_reference_peak_gib,
        configured_vram_budget_gib: train_config.vram_budget_gib,
        memory_estimate,
        reference_attention: true,
        checkpoint_path,
    })
}

fn save_model_checkpoint<B: Backend>(model: LlamaModel<B>, base: &Path) -> Result<String> {
    let mut final_path = base.to_owned();
    final_path.set_extension("mpk");
    ensure!(
        !final_path.exists(),
        "refusing to overwrite checkpoint {}",
        final_path.display()
    );
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let mut partial_name = final_path
        .file_stem()
        .context("checkpoint base must have a file stem")?
        .to_os_string();
    partial_name.push("__partial");
    let partial_base = parent.join(partial_name);
    let mut partial_path = partial_base.clone();
    partial_path.set_extension("mpk");
    ensure!(
        !partial_path.exists(),
        "partial checkpoint {} already exists; inspect and remove it explicitly",
        partial_path.display()
    );

    model
        .save_file(
            &partial_base,
            &NamedMpkFileRecorder::<FullPrecisionSettings>::default(),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&partial_path)?
        .sync_all()?;
    std::fs::rename(&partial_path, &final_path).with_context(|| {
        format!(
            "atomically finalizing checkpoint {} as {}",
            partial_path.display(),
            final_path.display()
        )
    })?;
    Ok(final_path.display().to_string())
}

fn sync_backend<B: Backend>(device: &B::Device, context: &str) -> Result<()> {
    B::sync(device).with_context(|| format!("synchronizing backend {context}"))
}

/// A deliberately conservative analytical estimate for the ordinary autograd
/// graph. Passing it does not guarantee that Burn's allocator will fit the run.
fn estimate_reference_peak_gib(model: &LlamaConfig, estimate: &MemoryEstimate) -> f64 {
    let retained_per_layer_mib = estimate.hidden_mib
        + estimate.q_mib
        + 2.0 * estimate.each_kv_mib
        + 2.0 * estimate.each_ffn_mib;
    estimate.persistent_state_gib
        + estimate.reference_attention_matrices_gib
        + estimate.logits_gib
        + model.n_layers as f64 * retained_per_layer_mib / 1024.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reference_plan_fails_28_gib_preflight() {
        let model = LlamaConfig::default();
        let train = TrainConfig {
            allow_reference_attention: true,
            ..TrainConfig::default()
        };
        let estimate = MemoryEstimate::bf16(&model, &train);
        assert!(estimate_reference_peak_gib(&model, &estimate) > train.vram_budget_gib);
    }

    #[cfg(feature = "cpu-reference")]
    #[test]
    fn flex_autodiff_executes_one_accumulated_optimizer_step() {
        use burn::backend::{Autodiff, Flex};
        use sha2::{Digest, Sha256};

        let temp = tempfile::tempdir().unwrap();
        let token_file = temp.path().join("tokens-00000.u16le");
        let tokens: Vec<u16> = (0..65).map(|index| (index % 32) as u16).collect();
        let bytes: Vec<u8> = tokens
            .iter()
            .flat_map(|token| token.to_le_bytes())
            .collect();
        std::fs::write(&token_file, &bytes).unwrap();
        let digest = hex::encode(Sha256::digest(&bytes));
        let manifest = crate::data::TokenManifest {
            format_version: 1,
            dtype: "u16".into(),
            byte_order: "little".into(),
            total_tokens: tokens.len() as u64,
            vocabulary_size: 32,
            eos_id: 2,
            pad_id: 0,
            eos_between_documents: true,
            source_corpus_sha256: "00".repeat(32),
            tokenizer_sha256: "00".repeat(32),
            files: vec![crate::data::TokenFileEntry {
                file: "tokens-00000.u16le".into(),
                tokens: tokens.len() as u64,
                bytes: bytes.len() as u64,
                sha256: digest,
            }],
        };
        let manifest_path = temp.path().join("tokens.manifest.json");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let model = LlamaConfig {
            vocab_size: 32,
            d_model: 16,
            d_ff: 32,
            n_layers: 1,
            n_heads: 4,
            n_kv_heads: 2,
            context_length: 8,
            rms_epsilon: 1e-5,
            rope_theta: 10_000.0,
            tie_embeddings: false,
        };
        let train = TrainConfig {
            target_tokens: 32,
            micro_batch_size: 2,
            gradient_accumulation: 2,
            peak_learning_rate: 1e-3,
            minimum_learning_rate: 0.0,
            warmup_steps: 0,
            beta_1: 0.9,
            beta_2: 0.95,
            adam_epsilon: 1e-8,
            weight_decay: 0.1,
            log_every_micro_steps: 1,
            target_tokens_per_second: 1.0,
            vram_budget_gib: 1.0,
            allow_reference_attention: true,
        };

        type B = Autodiff<Flex>;
        let checkpoint_base = temp.path().join("final-model");
        let summary = run_reference_training::<B>(
            &manifest_path,
            model,
            train,
            Default::default(),
            None,
            true,
            Some(&checkpoint_base),
        )
        .unwrap();
        assert_eq!(summary.optimizer_steps, 1);
        assert_eq!(summary.micro_steps, 2);
        assert_eq!(summary.processed_tokens, 32);
        assert!(summary.last_loss.is_some_and(f64::is_finite));
        assert_eq!(
            summary.checkpoint_path.as_deref(),
            Some(
                temp.path()
                    .join("final-model.mpk")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(temp.path().join("final-model.mpk").is_file());
    }
}
