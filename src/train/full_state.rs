//! Provider-neutral full-model training state for the E1 accelerator backend.
//!
//! This module owns everything the concrete accelerator backend must keep exact
//! on the host: the generalized GQA dimensional contract, INIT-001 master-weight
//! initialization, the frozen AdamW state through the P12 `CanonicalAdamw`
//! arithmetic, the deterministic host/device RNG witness chains, and the closed
//! five-artifact checkpoint codec with byte-exact restore. Nothing here touches
//! an accelerator; every behavior is testable on the CPU lanes.

use crate::error::{ProductError, Result};
use crate::model::{
    CANONICAL_MODEL_ID, CPU_ORACLE_FIXTURE_ID, ModelPreset, bf16_bits_to_f32,
    canonical_parameter_specs, cpu_oracle_fixture_parameters, f32_to_bf16_bits,
    initialization_seed, sample_initial_value,
};
use crate::train::trainer::{
    AdamwParameterState, BackendStateArtifact, CanonicalAdamw, EVALUATION_TARGETS,
};
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RUNTIME_STATE_SCHEMA: &str = "python-slm-e1-runtime-state-v1";
pub const FULL_GRAPH_SEMANTICS: &str = "pre-norm-gqa-rope-swiglu-causal-cross-entropy-v1";

pub const PARAMETERS_ROLE: &str = "parameters_bf16";
pub const PARAMETERS_PATH: &str = "model/parameters.bf16";
pub const MASTER_ROLE: &str = "master_parameters_fp32";
pub const MASTER_PATH: &str = "optimizer/master.f32";
pub const FIRST_MOMENTS_ROLE: &str = "adamw_first_moments_fp32";
pub const FIRST_MOMENTS_PATH: &str = "optimizer/adamw-m1.f32";
pub const SECOND_MOMENTS_ROLE: &str = "adamw_second_moments_fp32";
pub const SECOND_MOMENTS_PATH: &str = "optimizer/adamw-m2.f32";
pub const RUNTIME_ROLE: &str = "backend_runtime_state";
pub const RUNTIME_PATH: &str = "runtime/backend.json";

const HOST_RNG_TAG: &[u8] = b"python-slm/e1-host-rng/v1\0";
const DEVICE_RNG_TAG: &[u8] = b"python-slm/e1-device-rng/v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GqaDimensions {
    pub vocabulary: usize,
    pub width: usize,
    pub ffn_width: usize,
    pub layers: usize,
    pub query_heads: usize,
    pub key_value_heads: usize,
    pub head_width: usize,
    pub max_context: usize,
}

impl GqaDimensions {
    pub const fn canonical() -> Self {
        Self {
            vocabulary: 32_000,
            width: 768,
            ffn_width: 2_432,
            layers: 12,
            query_heads: 12,
            key_value_heads: 4,
            head_width: 64,
            max_context: 2_048,
        }
    }

    /// The closed P9B oracle shape. Its context bound is deliberately small: the
    /// fixture exists for exactness diagnostics, and quadratic attention over a
    /// 2,048-position context would dominate its runtime for no added coverage.
    pub const fn oracle_fixture() -> Self {
        Self {
            vocabulary: 4,
            width: 4,
            ffn_width: 4,
            layers: 1,
            query_heads: 2,
            key_value_heads: 1,
            head_width: 2,
            max_context: 256,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.vocabulary == 0
            || self.width == 0
            || self.ffn_width == 0
            || self.layers == 0
            || self.query_heads == 0
            || self.key_value_heads == 0
            || self.head_width == 0
            || self.max_context == 0
            || !self.head_width.is_multiple_of(2)
            || self.query_heads * self.head_width != self.width
            || !self.query_heads.is_multiple_of(self.key_value_heads)
        {
            return Err(ProductError::integrity(
                "E1_DIMENSIONS_INVALID",
                "the full-model dimensions are not a valid untied pre-norm GQA shape",
            ));
        }
        Ok(())
    }

    pub const fn key_value_width(&self) -> usize {
        self.key_value_heads * self.head_width
    }

    pub const fn query_to_key_value_head(&self, query_head: usize) -> usize {
        query_head / (self.query_heads / self.key_value_heads)
    }

    /// The stable PARAM-001 tensor order generalized over the dimensions.
    pub fn parameter_layout(&self) -> Vec<(String, Vec<usize>)> {
        let mut layout = Vec::with_capacity(2 + self.layers * 9 + 1);
        layout.push((
            "tok_embeddings.weight".to_owned(),
            vec![self.vocabulary, self.width],
        ));
        for block in 0..self.layers {
            layout.push((format!("blocks.{block}.attn_norm.weight"), vec![self.width]));
            layout.push((
                format!("blocks.{block}.attn.q.weight"),
                vec![self.width, self.width],
            ));
            for projection in ["k", "v"] {
                layout.push((
                    format!("blocks.{block}.attn.{projection}.weight"),
                    vec![self.key_value_width(), self.width],
                ));
            }
            layout.push((
                format!("blocks.{block}.attn.o.weight"),
                vec![self.width, self.width],
            ));
            layout.push((format!("blocks.{block}.ffn_norm.weight"), vec![self.width]));
            for projection in ["gate", "up"] {
                layout.push((
                    format!("blocks.{block}.ffn.{projection}.weight"),
                    vec![self.ffn_width, self.width],
                ));
            }
            layout.push((
                format!("blocks.{block}.ffn.down.weight"),
                vec![self.width, self.ffn_width],
            ));
        }
        layout.push(("final_norm.weight".to_owned(), vec![self.width]));
        layout.push((
            "lm_head.weight".to_owned(),
            vec![self.vocabulary, self.width],
        ));
        layout
    }

    pub fn parameter_count(&self) -> u64 {
        self.parameter_layout()
            .iter()
            .map(|(_, shape)| shape.iter().product::<usize>() as u64)
            .sum()
    }

    pub fn layout_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/e1-parameter-layout/v1\0");
        for (name, shape) in self.parameter_layout() {
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((shape.len() as u64).to_le_bytes());
            for dimension in shape {
                hasher.update((dimension as u64).to_le_bytes());
            }
        }
        hex::encode(hasher.finalize())
    }
}

/// AdamW decay membership by stable name: exactly the RMSNorm scales are no-decay.
pub fn weight_decay_for(name: &str) -> bool {
    !name.ends_with("norm.weight")
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeStateV1 {
    schema: String,
    model_identity: String,
    graph_semantics: String,
    layout_sha256: String,
    host_rng_state_hex: String,
    device_rng_state_hex: String,
    consumed_batches: u64,
}

#[derive(Debug)]
pub struct FullModelState {
    dimensions: GqaDimensions,
    model_identity: String,
    adamw: CanonicalAdamw,
    host_rng_state: Vec<u8>,
    device_rng_state: Vec<u8>,
    consumed_batches: u64,
}

fn chain_seed(tag: &[u8], model_identity: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(tag);
    hasher.update(model_identity.as_bytes());
    hasher.finalize().to_vec()
}

fn chain_advance(tag: &[u8], previous: &[u8], first_target: u64, valid_targets: u64) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(tag);
    hasher.update(previous);
    hasher.update(first_target.to_le_bytes());
    hasher.update(valid_targets.to_le_bytes());
    hasher.finalize().to_vec()
}

impl FullModelState {
    fn new(
        dimensions: GqaDimensions,
        model_identity: String,
        parameters: Vec<AdamwParameterState>,
        host_rng_state: Vec<u8>,
        device_rng_state: Vec<u8>,
        consumed_batches: u64,
    ) -> Result<Self> {
        dimensions.validate()?;
        let expected = dimensions.parameter_layout();
        if parameters.len() != expected.len()
            || parameters
                .iter()
                .zip(&expected)
                .any(|(state, (name, shape))| {
                    state.name != *name
                        || state.master_weights.len() != shape.iter().product::<usize>()
                        || state.weight_decay != weight_decay_for(name)
                })
        {
            return Err(ProductError::integrity(
                "E1_PARAMETER_LAYOUT_MISMATCH",
                "the optimizer state does not match the stable PARAM-001 layout",
            ));
        }
        Ok(Self {
            dimensions,
            model_identity,
            adamw: CanonicalAdamw::new(parameters)?,
            host_rng_state,
            device_rng_state,
            consumed_batches,
        })
    }

    /// INIT-001 initialization at canonical scale: the exact ChaCha12 and
    /// StandardNormal draw sequence, FP32 masters, and BF16 RNE storage.
    pub fn initialize_canonical() -> Result<Self> {
        let dimensions = GqaDimensions::canonical();
        let specs = canonical_parameter_specs()?;
        let mut rng = ChaCha12Rng::from_seed(initialization_seed(ModelPreset::Gqa135mV1));
        let mut parameters = Vec::with_capacity(specs.len());
        for spec in &specs {
            let mut master_weights = Vec::with_capacity(spec.elements as usize);
            let mut parameter_bf16 = Vec::with_capacity(spec.elements as usize);
            for _ in 0..spec.elements {
                let value = sample_initial_value(&mut rng, spec.initialization);
                master_weights.push(value);
                parameter_bf16.push(f32_to_bf16_bits(value));
            }
            parameters.push(AdamwParameterState {
                name: spec.name.clone(),
                weight_decay: weight_decay_for(&spec.name),
                first_moments: vec![0.0; spec.elements as usize],
                second_moments: vec![0.0; spec.elements as usize],
                master_weights,
                parameter_bf16,
            });
        }
        let host = chain_seed(HOST_RNG_TAG, CANONICAL_MODEL_ID);
        let device = chain_seed(DEVICE_RNG_TAG, CANONICAL_MODEL_ID);
        Self::new(
            dimensions,
            CANONICAL_MODEL_ID.to_owned(),
            parameters,
            host,
            device,
            0,
        )
    }

    /// The closed P9B oracle fixture as an optimizer state: masters are the
    /// exactly representable BF16 values, moments start at zero.
    pub fn initialize_oracle_fixture() -> Result<Self> {
        let dimensions = GqaDimensions::oracle_fixture();
        let parameters = cpu_oracle_fixture_parameters()
            .into_iter()
            .map(|parameter| {
                let master_weights = parameter
                    .values_bf16_bits
                    .iter()
                    .map(|bits| bf16_bits_to_f32(*bits))
                    .collect::<Vec<_>>();
                let elements = master_weights.len();
                AdamwParameterState {
                    weight_decay: weight_decay_for(&parameter.name),
                    name: parameter.name,
                    master_weights,
                    parameter_bf16: parameter.values_bf16_bits,
                    first_moments: vec![0.0; elements],
                    second_moments: vec![0.0; elements],
                }
            })
            .collect();
        let host = chain_seed(HOST_RNG_TAG, CPU_ORACLE_FIXTURE_ID);
        let device = chain_seed(DEVICE_RNG_TAG, CPU_ORACLE_FIXTURE_ID);
        Self::new(
            dimensions,
            CPU_ORACLE_FIXTURE_ID.to_owned(),
            parameters,
            host,
            device,
            0,
        )
    }

    pub fn dimensions(&self) -> &GqaDimensions {
        &self.dimensions
    }

    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }

    pub fn parameters(&self) -> &[AdamwParameterState] {
        self.adamw.parameters()
    }

    pub fn consumed_batches(&self) -> u64 {
        self.consumed_batches
    }

    /// Advance both deterministic RNG witness chains for one consumed batch and
    /// return the post-batch states. The deterministic graph consumes no
    /// randomness; the chains witness exact batch order and are resumable.
    pub fn advance_rng(&mut self, first_target: u64, valid_targets: u64) -> (Vec<u8>, Vec<u8>) {
        self.host_rng_state = chain_advance(
            HOST_RNG_TAG,
            &self.host_rng_state,
            first_target,
            valid_targets,
        );
        self.device_rng_state = chain_advance(
            DEVICE_RNG_TAG,
            &self.device_rng_state,
            first_target,
            valid_targets,
        );
        self.consumed_batches += 1;
        (self.host_rng_state.clone(), self.device_rng_state.clone())
    }

    pub fn apply_normalized_clipped_gradients(
        &mut self,
        gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
    ) -> Result<String> {
        self.adamw
            .apply_normalized_clipped_gradients(gradients, learning_rate, one_based_update)
    }

    pub fn adamw_state_sha256(&self) -> Result<String> {
        self.adamw.state_sha256()
    }

    pub fn snapshot_artifacts(&self) -> Result<Vec<BackendStateArtifact>> {
        let parameters = self.adamw.parameters();
        let mut bf16 = Vec::new();
        let mut master = Vec::new();
        let mut first_moments = Vec::new();
        let mut second_moments = Vec::new();
        for parameter in parameters {
            for bits in &parameter.parameter_bf16 {
                bf16.extend_from_slice(&bits.to_le_bytes());
            }
            for value in &parameter.master_weights {
                master.extend_from_slice(&value.to_le_bytes());
            }
            for value in &parameter.first_moments {
                first_moments.extend_from_slice(&value.to_le_bytes());
            }
            for value in &parameter.second_moments {
                second_moments.extend_from_slice(&value.to_le_bytes());
            }
        }
        let runtime = RuntimeStateV1 {
            schema: RUNTIME_STATE_SCHEMA.to_owned(),
            model_identity: self.model_identity.clone(),
            graph_semantics: FULL_GRAPH_SEMANTICS.to_owned(),
            layout_sha256: self.dimensions.layout_sha256(),
            host_rng_state_hex: hex::encode(&self.host_rng_state),
            device_rng_state_hex: hex::encode(&self.device_rng_state),
            consumed_batches: self.consumed_batches,
        };
        let runtime_bytes = serde_json::to_vec(&runtime).map_err(|error| {
            ProductError::internal(
                "E1_RUNTIME_STATE_SERIALIZATION_FAILED",
                format!("could not serialize the backend runtime state: {error}"),
            )
        })?;
        Ok(vec![
            artifact(PARAMETERS_ROLE, PARAMETERS_PATH, bf16),
            artifact(MASTER_ROLE, MASTER_PATH, master),
            artifact(FIRST_MOMENTS_ROLE, FIRST_MOMENTS_PATH, first_moments),
            artifact(SECOND_MOMENTS_ROLE, SECOND_MOMENTS_PATH, second_moments),
            artifact(RUNTIME_ROLE, RUNTIME_PATH, runtime_bytes),
        ])
    }

    pub fn restore_artifacts(
        dimensions: GqaDimensions,
        expected_identity: &str,
        artifacts: &[BackendStateArtifact],
    ) -> Result<Self> {
        dimensions.validate()?;
        let runtime_bytes = artifact_bytes(artifacts, RUNTIME_ROLE, RUNTIME_PATH)?;
        let runtime = serde_json::from_slice::<RuntimeStateV1>(runtime_bytes).map_err(|error| {
            ProductError::integrity(
                "E1_RUNTIME_STATE_INVALID",
                format!("the backend runtime state is not a closed runtime object: {error}"),
            )
        })?;
        if runtime.schema != RUNTIME_STATE_SCHEMA
            || runtime.model_identity != expected_identity
            || runtime.graph_semantics != FULL_GRAPH_SEMANTICS
            || runtime.layout_sha256 != dimensions.layout_sha256()
        {
            return Err(ProductError::integrity(
                "E1_RUNTIME_STATE_INVALID",
                "the backend runtime state does not bind this model and layout identity",
            ));
        }
        let host_rng_state = decode_rng_state(&runtime.host_rng_state_hex)?;
        let device_rng_state = decode_rng_state(&runtime.device_rng_state_hex)?;

        let layout = dimensions.parameter_layout();
        let total_elements = layout
            .iter()
            .map(|(_, shape)| shape.iter().product::<usize>())
            .sum::<usize>();
        let bf16 = artifact_bytes(artifacts, PARAMETERS_ROLE, PARAMETERS_PATH)?;
        let master = artifact_bytes(artifacts, MASTER_ROLE, MASTER_PATH)?;
        let first_moments = artifact_bytes(artifacts, FIRST_MOMENTS_ROLE, FIRST_MOMENTS_PATH)?;
        let second_moments = artifact_bytes(artifacts, SECOND_MOMENTS_ROLE, SECOND_MOMENTS_PATH)?;
        if bf16.len() != total_elements * 2
            || master.len() != total_elements * 4
            || first_moments.len() != total_elements * 4
            || second_moments.len() != total_elements * 4
        {
            return Err(ProductError::integrity(
                "E1_STATE_ARTIFACT_INVALID",
                "a checkpoint artifact does not cover the exact parameter layout",
            ));
        }

        let mut offset = 0_usize;
        let mut parameters = Vec::with_capacity(layout.len());
        for (name, shape) in layout {
            let elements = shape.iter().product::<usize>();
            let parameter_bf16 = (0..elements)
                .map(|index| {
                    let at = (offset + index) * 2;
                    u16::from_le_bytes([bf16[at], bf16[at + 1]])
                })
                .collect::<Vec<_>>();
            let read_f32 = |bytes: &[u8], index: usize| {
                let at = (offset + index) * 4;
                f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
            };
            parameters.push(AdamwParameterState {
                weight_decay: weight_decay_for(&name),
                master_weights: (0..elements).map(|index| read_f32(master, index)).collect(),
                first_moments: (0..elements)
                    .map(|index| read_f32(first_moments, index))
                    .collect(),
                second_moments: (0..elements)
                    .map(|index| read_f32(second_moments, index))
                    .collect(),
                parameter_bf16,
                name,
            });
            offset += elements;
        }
        Self::new(
            dimensions,
            expected_identity.to_owned(),
            parameters,
            host_rng_state,
            device_rng_state,
            runtime.consumed_batches,
        )
    }
}

fn artifact(role: &str, relative_path: &str, bytes: Vec<u8>) -> BackendStateArtifact {
    BackendStateArtifact {
        role: role.to_owned(),
        relative_path: relative_path.to_owned(),
        bytes,
    }
}

fn artifact_bytes<'a>(
    artifacts: &'a [BackendStateArtifact],
    role: &str,
    expected_path: &str,
) -> Result<&'a [u8]> {
    let found = artifacts
        .iter()
        .find(|artifact| artifact.role == role)
        .ok_or_else(|| {
            ProductError::integrity(
                "E1_STATE_ARTIFACT_INVALID",
                format!("the checkpoint omits the {role} artifact"),
            )
        })?;
    if found.relative_path != expected_path {
        return Err(ProductError::integrity(
            "E1_STATE_ARTIFACT_INVALID",
            format!("the {role} artifact is not at its closed relative path"),
        ));
    }
    Ok(&found.bytes)
}

fn decode_rng_state(value: &str) -> Result<Vec<u8>> {
    let bytes = hex::decode(value).map_err(|_| {
        ProductError::integrity(
            "E1_RUNTIME_STATE_INVALID",
            "an RNG witness state is not lowercase hexadecimal",
        )
    })?;
    if bytes.len() != 32 || value != hex::encode(&bytes) {
        return Err(ProductError::integrity(
            "E1_RUNTIME_STATE_INVALID",
            "an RNG witness state is not exactly 32 canonical bytes",
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationSpan {
    pub input_ids: Vec<u16>,
    pub target_ids: Vec<u16>,
}

#[derive(Debug)]
pub struct ValidationSet {
    spans: Vec<ValidationSpan>,
}

impl ValidationSet {
    pub fn new(spans: Vec<ValidationSpan>, dimensions: &GqaDimensions) -> Result<Self> {
        dimensions.validate()?;
        let mut total = 0_u64;
        for span in &spans {
            validate_token_span(&span.input_ids, &span.target_ids, dimensions)?;
            total += span.input_ids.len() as u64;
        }
        if total != EVALUATION_TARGETS {
            return Err(ProductError::integrity(
                "E1_VALIDATION_SET_INVALID",
                format!(
                    "the validation set covers {total} targets instead of the mandatory {EVALUATION_TARGETS}"
                ),
            ));
        }
        Ok(Self { spans })
    }

    pub fn spans(&self) -> &[ValidationSpan] {
        &self.spans
    }
}

pub fn validate_token_span(
    input_ids: &[u16],
    target_ids: &[u16],
    dimensions: &GqaDimensions,
) -> Result<()> {
    if input_ids.is_empty()
        || input_ids.len() != target_ids.len()
        || input_ids.len() > dimensions.max_context
        || input_ids
            .iter()
            .chain(target_ids)
            .any(|token| usize::from(*token) >= dimensions.vocabulary)
    {
        return Err(ProductError::integrity(
            "E1_BATCH_INVALID",
            "a token span is empty, oversized, misaligned, or outside the vocabulary",
        ));
    }
    Ok(())
}
