//! E1 CUDA full-model trainer backend.
//!
//! The backend runs the provider-generic full GQA graph on one CUDA device with
//! FP32 tensors and straight-through BF16 storage quantization, while every
//! deterministic decision lives on the host: INIT-001 initialization, the frozen
//! P12 `CanonicalAdamw` master-weight arithmetic, the RNG witness chains, and
//! the closed five-artifact checkpoint codec. Gradient sums return to the host
//! per micro-batch as the `TrainerBackend` contract requires; the AdamW step is
//! exact host FP32, and refreshed BF16 parameters are re-uploaded afterward.
//! Determinism claims are backend-versus-itself (byte-identical repetition and
//! resume); conformance against the P9B oracle is the separate `PRECISION-002`
//! gate, which keeps the forward exact byte for byte and bounds the gradient.

use crate::error::{ProductError, Result};
use crate::model::accelerator::full_model::{FullModelGraph, GraphConstants, graph_constants};
use crate::train::full_state::{FullModelState, ValidationSet};
use crate::train::trainer::{
    BackendStateArtifact, BatchGradient, EVALUATION_TARGETS, EvaluationResult, TrainerBackend,
    TrainingBatch,
};
use burn::backend::{Autodiff, Cuda};
use burn_autodiff::checkpoint::strategy::BalancedCheckpointing;
use half::bf16;
use sha2::{Digest, Sha256};

/// `BalancedCheckpointing` recomputes the elementwise, transcendental, and view
/// layer during backward instead of retaining it, which covers the whole softmax
/// decomposition. Recomputation reruns the same kernels on the same inputs, so it
/// is bit-identical and neither the `PRECISION-002` gate nor determinism moves.
type Gpu = Autodiff<Cuda<bf16, i32>, BalancedCheckpointing>;
type CudaDevice = burn::backend::cuda::CudaDevice;

pub const CUDA_FULL_MODEL_BACKEND: &str = "e1-cuda-full-model-backend-v1";

/// Held-out sequences per evaluation dispatch. Evaluation is forward-only and
/// detached, so it can batch more widely than training; this is a Rust constant
/// because the frozen defaults file is byte-pinned and cannot take new fields.
pub const EVALUATION_BATCH_SEQUENCES: usize = 8;

pub struct CudaTrainerBackend {
    device: CudaDevice,
    state: FullModelState,
    graph: FullModelGraph<Gpu>,
    constants: GraphConstants<Gpu>,
    validation: ValidationSet,
}

fn graph_failure(error: anyhow::Error) -> ProductError {
    ProductError::internal(
        "E1_GRAPH_EXECUTION_FAILED",
        format!("the full-model graph failed: {error:#}"),
    )
}

impl CudaTrainerBackend {
    fn from_state(
        state: FullModelState,
        device_ordinal: usize,
        validation: ValidationSet,
    ) -> Result<Self> {
        let device = CudaDevice::new(device_ordinal);
        let graph = FullModelGraph::<Gpu>::load(*state.dimensions(), state.parameters(), &device)
            .map_err(graph_failure)?;
        let constants =
            graph_constants::<Gpu>(state.dimensions(), &device).map_err(graph_failure)?;
        Ok(Self {
            device,
            state,
            graph,
            constants,
            validation,
        })
    }

    /// INIT-001 canonical initialization on the selected device.
    pub fn canonical(device_ordinal: usize, validation: ValidationSet) -> Result<Self> {
        Self::from_state(
            FullModelState::initialize_canonical()?,
            device_ordinal,
            validation,
        )
    }

    /// The closed P9B oracle fixture at width four, for automated diagnostics.
    pub fn oracle_fixture(device_ordinal: usize, validation: ValidationSet) -> Result<Self> {
        Self::from_state(
            FullModelState::initialize_oracle_fixture()?,
            device_ordinal,
            validation,
        )
    }

    pub fn model_identity(&self) -> &str {
        self.state.model_identity()
    }

    pub fn parameter_count(&self) -> u64 {
        self.state.dimensions().parameter_count()
    }

    /// The backend's RNG witness chain states, so a launching coordinator can seed
    /// the trainer from the backend rather than from invented values.

    /// Sample continuation tokens from the trained model.
    ///
    /// Deterministic given the same prompt, settings and seed: the RNG is the
    /// pinned ChaCha12 the rest of the product uses, seeded explicitly, so a
    /// generation can be repeated exactly. Temperature zero is greedy and takes no
    /// RNG draw at all.
    ///
    /// This is a diagnostic. It reads the model and produces text; it does not
    /// touch optimizer state, and nothing it returns is qualified evidence about
    /// model quality.
    pub fn generate(
        &mut self,
        prompt_ids: &[u16],
        maximum_new_tokens: usize,
        temperature: f32,
        top_k: usize,
        seed: u64,
    ) -> Result<Vec<u16>> {
        use rand_chacha::ChaCha12Rng;
        use rand_core::SeedableRng;

        if prompt_ids.is_empty() {
            return Err(ProductError::usage(
                "E7_GENERATION_PROMPT_EMPTY",
                "generation needs at least one prompt token",
            ));
        }
        if !(temperature.is_finite() && temperature >= 0.0) || top_k == 0 {
            return Err(ProductError::usage(
                "E7_GENERATION_SETTINGS_INVALID",
                "temperature must be finite and non-negative and top_k must be positive",
            ));
        }
        let context = self.graph.dimensions().max_context;
        let mut seed_bytes = [0_u8; 32];
        seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
        let mut rng = ChaCha12Rng::from_seed(seed_bytes);
        let mut window = prompt_ids.to_vec();
        let mut produced = Vec::new();

        for _ in 0..maximum_new_tokens {
            // The model has no memory beyond its context, so once the prompt plus
            // what it has written exceeds that, the oldest tokens fall off rather
            // than the request failing.
            if window.len() > context {
                window.drain(..window.len() - context);
            }
            let logits = self
                .graph
                .untracked()
                .next_token_logits(&window, &self.constants, &self.device)
                .map_err(graph_failure)?;
            let next = sample_token(&logits, temperature, top_k, &mut rng)?;
            produced.push(next);
            window.push(next);
            if u32::from(next) == crate::tokenizer::EOS_ID {
                break;
            }
        }
        Ok(produced)
    }
    pub fn rng_states(&self) -> (Vec<u8>, Vec<u8>) {
        self.state.rng_states()
    }

    fn reload_graph(&mut self) -> Result<()> {
        self.graph = FullModelGraph::<Gpu>::load(
            *self.state.dimensions(),
            self.state.parameters(),
            &self.device,
        )
        .map_err(graph_failure)?;
        Ok(())
    }
}

impl TrainerBackend for CudaTrainerBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        let sequences = batch.sequences();
        for (inputs, targets) in &sequences {
            crate::train::full_state::validate_token_span(
                inputs,
                targets,
                self.state.dimensions(),
            )?;
        }
        // Sequences of one length go out together; the ragged final update simply
        // yields a second, shorter dispatch rather than being padded.
        let mut grouped: Vec<Vec<(&[u16], &[u16])>> = Vec::new();
        let mut lengths: Vec<usize> = Vec::new();
        for sequence in sequences {
            match lengths
                .iter()
                .position(|length| *length == sequence.0.len())
            {
                Some(index) => grouped[index].push(sequence),
                None => {
                    lengths.push(sequence.0.len());
                    grouped.push(vec![sequence]);
                }
            }
        }

        let mut loss_sum = 0.0_f64;
        let mut gradient_sums: Vec<f32> = Vec::new();
        for group in grouped {
            let output = self
                .graph
                .training_step(&group, &self.constants, 1.0, false, &self.device)
                .map_err(graph_failure)?;
            loss_sum += f64::from(output.loss_f32);
            if gradient_sums.is_empty() {
                gradient_sums = output.gradient_f32;
            } else {
                if gradient_sums.len() != output.gradient_f32.len() {
                    return Err(ProductError::integrity(
                        "E1_GRAPH_GRADIENT_LAYOUT_MISMATCH",
                        "grouped dispatches produced different gradient layouts",
                    ));
                }
                for (total, value) in gradient_sums.iter_mut().zip(output.gradient_f32) {
                    *total += value;
                }
            }
        }
        let (host_rng_state, device_rng_state) = self
            .state
            .advance_rng(batch.first_target, batch.valid_targets);
        Ok(BatchGradient {
            loss_sum,
            gradient_sums,
            host_rng_state,
            device_rng_state,
        })
    }

    fn apply_update(
        &mut self,
        normalized_clipped_gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
        _valid_targets: u64,
    ) -> Result<String> {
        let state_sha256 = self.state.apply_normalized_clipped_gradients(
            normalized_clipped_gradients,
            learning_rate,
            one_based_update,
        )?;
        self.reload_graph()?;
        // The previous update's graph tensors are unreachable once the parameters are
        // re-uploaded; return their pages to the allocator rather than holding the
        // high-water mark for the whole run.
        <Gpu as burn::tensor::backend::Backend>::memory_cleanup(&self.device);
        Ok(state_sha256)
    }

    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/e1-evaluation/v1\0");
        hasher.update(validation_span_manifest_sha256.as_bytes());
        hasher.update(self.state.adamw_state_sha256()?.as_bytes());
        let spans = self
            .validation
            .spans()
            .iter()
            .map(|span| (span.input_ids.clone(), span.target_ids.clone()))
            .collect::<Vec<_>>();
        let span_losses = self
            .graph
            .untracked()
            .validation_loss_sums(
                &spans,
                EVALUATION_BATCH_SEQUENCES,
                &self.constants,
                &self.device,
            )
            .map_err(graph_failure)?;
        let mut loss_sum = 0.0_f64;
        for span_loss in span_losses {
            if !span_loss.is_finite() {
                return Err(ProductError::gate(
                    "E1_EVALUATION_NONFINITE",
                    "a held-out span produced a nonfinite loss",
                ));
            }
            hasher.update(span_loss.to_le_bytes());
            loss_sum += f64::from(span_loss);
        }
        Ok(EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: loss_sum / EVALUATION_TARGETS as f64,
            result_sha256: hex::encode(hasher.finalize()),
        })
    }

    fn snapshot(&self) -> Result<Vec<BackendStateArtifact>> {
        self.state.snapshot_artifacts()
    }

    fn restore(&mut self, artifacts: &[BackendStateArtifact]) -> Result<()> {
        let identity = self.state.model_identity().to_owned();
        self.state =
            FullModelState::restore_artifacts(*self.state.dimensions(), &identity, artifacts)?;
        self.reload_graph()
    }
}

/// Pick one token from a logit row under temperature and top-k.
///
/// Temperature zero is greedy and never draws, which keeps a deterministic
/// generation deterministic without depending on the RNG at all. Above zero the
/// row is softmaxed in f64 after subtracting its maximum — the subtraction is
/// what stops `exp` overflowing on a confident model, and f64 because summing
/// 32,000 exponentials in f32 loses more than the sampling can afford.
///
/// Ties break toward the lower token id, so a model that has learned nothing and
/// emits a flat row still produces something reproducible rather than something
/// that depends on iteration order.
fn sample_token(
    logits: &[f32],
    temperature: f32,
    top_k: usize,
    rng: &mut rand_chacha::ChaCha12Rng,
) -> Result<u16> {
    use rand_core::Rng;

    if logits.is_empty() || logits.iter().any(|value| !value.is_finite()) {
        return Err(ProductError::gate(
            "E7_GENERATION_LOGITS_INVALID",
            "the model produced an empty or non-finite logit row",
        ));
    }
    let mut ranked: Vec<(usize, f32)> = logits.iter().copied().enumerate().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(&right.0))
    });
    ranked.truncate(top_k.min(ranked.len()));

    let chosen = if temperature == 0.0 {
        ranked[0].0
    } else {
        let maximum = ranked[0].1;
        let weights: Vec<f64> = ranked
            .iter()
            .map(|(_, logit)| f64::from((logit - maximum) / temperature).exp())
            .collect();
        let total: f64 = weights.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(ProductError::gate(
                "E7_GENERATION_DISTRIBUTION_INVALID",
                "the sampling distribution summed to zero or a non-finite value",
            ));
        }
        // next_u64 rather than a distribution helper: it is the primitive every
        // rand version exposes identically, so a generation stays reproducible
        // across dependency updates that reshape the distribution API.
        let unit = rng.next_u64() as f64 / (u64::MAX as f64 + 1.0);
        let mut draw = unit * total;
        let mut selected = ranked[ranked.len() - 1].0;
        for ((index, _), weight) in ranked.iter().zip(&weights) {
            draw -= weight;
            if draw <= 0.0 {
                selected = *index;
                break;
            }
        }
        selected
    };
    u16::try_from(chosen).map_err(|_| {
        ProductError::internal(
            "E7_GENERATION_TOKEN_INVALID",
            "the sampled token id does not fit the immutable u16 shard format",
        )
    })
}
