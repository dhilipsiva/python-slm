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
