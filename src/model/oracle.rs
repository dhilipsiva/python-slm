use super::{
    CANONICAL_MODEL_ID, CANONICAL_PARAMETER_COUNT, ModelPreset, REFERENCE_MODEL_ID,
    REFERENCE_PARAMETER_COUNT,
};
use crate::error::{ProductError, Result};
use rand::distr::Distribution;
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use rand_distr::StandardNormal;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MODEL_CONFIG_SCHEMA: &str = "python-slm-model-config-v1";
pub const INITIALIZATION_MANIFEST_SCHEMA: &str = "python-slm-model-initialization-manifest-v1";
pub const CPU_ORACLE_SCHEMA: &str = "python-slm-cpu-oracle-result-v1";
pub const MODEL_ORACLE_RESULT_SCHEMA: &str = "python-slm-model-oracle-result-v1";
pub const CPU_ORACLE_FIXTURE_ID: &str = "gqa-scalar-oracle-v1";
pub const RMS_NORM_EPSILON: f32 = 1.0e-5;
pub const ROPE_BASE: f32 = 10_000.0;
pub const INITIALIZATION_STDDEV: f32 = 0.02;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub schema: &'static str,
    pub identity: &'static str,
    pub canonical: bool,
    pub vocabulary_size: usize,
    pub width: usize,
    pub ffn_width: usize,
    pub layers: usize,
    pub query_heads: usize,
    pub key_value_heads: usize,
    pub head_width: usize,
    pub context_length: usize,
    pub untied_lm_head: bool,
    pub biases: bool,
    pub dropout_f32_bits: String,
    pub parameter_count: u64,
}

impl ModelConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema != MODEL_CONFIG_SCHEMA
            || self.vocabulary_size != 32_000
            || self.width != 768
            || self.layers != 12
            || self.query_heads != 12
            || self.key_value_heads != 4
            || self.head_width != 64
            || self.context_length != 2_048
            || !self.untied_lm_head
            || self.biases
            || self.dropout_f32_bits != "00000000"
            || self.query_heads * self.head_width != self.width
            || !self.query_heads.is_multiple_of(self.key_value_heads)
        {
            return Err(ProductError::integrity(
                "MODEL_CONFIG_INVALID",
                "the model configuration does not match MODEL-001/PARAM-001",
            ));
        }
        let expected = match self.identity {
            CANONICAL_MODEL_ID if self.canonical && self.ffn_width == 2_432 => {
                CANONICAL_PARAMETER_COUNT
            }
            REFERENCE_MODEL_ID if !self.canonical && self.ffn_width == 2_048 => {
                REFERENCE_PARAMETER_COUNT
            }
            _ => {
                return Err(ProductError::integrity(
                    "MODEL_CONFIG_INVALID",
                    "the model identity, canonical flag, or FFN width is inconsistent",
                ));
            }
        };
        if self.parameter_count != expected {
            return Err(ProductError::integrity(
                "MODEL_PARAMETER_COUNT_MISMATCH",
                "the declared model parameter count is not canonical",
            ));
        }
        Ok(())
    }
}

pub fn model_config(preset: ModelPreset) -> ModelConfig {
    match preset {
        ModelPreset::Gqa135mV1 => ModelConfig {
            schema: MODEL_CONFIG_SCHEMA,
            identity: CANONICAL_MODEL_ID,
            canonical: true,
            vocabulary_size: 32_000,
            width: 768,
            ffn_width: 2_432,
            layers: 12,
            query_heads: 12,
            key_value_heads: 4,
            head_width: 64,
            context_length: 2_048,
            untied_lm_head: true,
            biases: false,
            dropout_f32_bits: "00000000".to_owned(),
            parameter_count: CANONICAL_PARAMETER_COUNT,
        },
        ModelPreset::Gqa124mRefV1 => ModelConfig {
            schema: MODEL_CONFIG_SCHEMA,
            identity: REFERENCE_MODEL_ID,
            canonical: false,
            vocabulary_size: 32_000,
            width: 768,
            ffn_width: 2_048,
            layers: 12,
            query_heads: 12,
            key_value_heads: 4,
            head_width: 64,
            context_length: 2_048,
            untied_lm_head: true,
            biases: false,
            dropout_f32_bits: "00000000".to_owned(),
            parameter_count: REFERENCE_PARAMETER_COUNT,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterInitialization {
    NormalStddev002,
    Ones,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizerGroup {
    Decay,
    NoDecay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterSpec {
    pub name: String,
    pub shape: Vec<usize>,
    pub elements: u64,
    pub initialization: ParameterInitialization,
    pub optimizer_group: OptimizerGroup,
}

fn checked_elements(shape: &[usize]) -> Result<u64> {
    shape.iter().try_fold(1_u64, |product, dimension| {
        let dimension = u64::try_from(*dimension).map_err(|_| {
            ProductError::internal("MODEL_SHAPE_OVERFLOW", "a model dimension exceeds u64")
        })?;
        product.checked_mul(dimension).ok_or_else(|| {
            ProductError::internal("MODEL_SHAPE_OVERFLOW", "a parameter shape exceeds u64")
        })
    })
}

fn parameter_spec(
    name: impl Into<String>,
    shape: Vec<usize>,
    initialization: ParameterInitialization,
    optimizer_group: OptimizerGroup,
) -> Result<ParameterSpec> {
    Ok(ParameterSpec {
        name: name.into(),
        elements: checked_elements(&shape)?,
        shape,
        initialization,
        optimizer_group,
    })
}

pub fn parameter_specs(config: &ModelConfig) -> Result<Vec<ParameterSpec>> {
    config.validate()?;
    let mut specs = Vec::with_capacity(111);
    specs.push(parameter_spec(
        "tok_embeddings.weight",
        vec![config.vocabulary_size, config.width],
        ParameterInitialization::NormalStddev002,
        OptimizerGroup::Decay,
    )?);
    let key_value_width = config.key_value_heads * config.head_width;
    for block in 0..config.layers {
        specs.push(parameter_spec(
            format!("blocks.{block}.attn_norm.weight"),
            vec![config.width],
            ParameterInitialization::Ones,
            OptimizerGroup::NoDecay,
        )?);
        specs.push(parameter_spec(
            format!("blocks.{block}.attn.q.weight"),
            vec![config.width, config.width],
            ParameterInitialization::NormalStddev002,
            OptimizerGroup::Decay,
        )?);
        for projection in ["k", "v"] {
            specs.push(parameter_spec(
                format!("blocks.{block}.attn.{projection}.weight"),
                vec![key_value_width, config.width],
                ParameterInitialization::NormalStddev002,
                OptimizerGroup::Decay,
            )?);
        }
        specs.push(parameter_spec(
            format!("blocks.{block}.attn.o.weight"),
            vec![config.width, config.width],
            ParameterInitialization::NormalStddev002,
            OptimizerGroup::Decay,
        )?);
        specs.push(parameter_spec(
            format!("blocks.{block}.ffn_norm.weight"),
            vec![config.width],
            ParameterInitialization::Ones,
            OptimizerGroup::NoDecay,
        )?);
        for projection in ["gate", "up"] {
            specs.push(parameter_spec(
                format!("blocks.{block}.ffn.{projection}.weight"),
                vec![config.ffn_width, config.width],
                ParameterInitialization::NormalStddev002,
                OptimizerGroup::Decay,
            )?);
        }
        specs.push(parameter_spec(
            format!("blocks.{block}.ffn.down.weight"),
            vec![config.width, config.ffn_width],
            ParameterInitialization::NormalStddev002,
            OptimizerGroup::Decay,
        )?);
    }
    specs.push(parameter_spec(
        "final_norm.weight",
        vec![config.width],
        ParameterInitialization::Ones,
        OptimizerGroup::NoDecay,
    )?);
    specs.push(parameter_spec(
        "lm_head.weight",
        vec![config.vocabulary_size, config.width],
        ParameterInitialization::NormalStddev002,
        OptimizerGroup::Decay,
    )?);

    let count = specs.iter().try_fold(0_u64, |total, spec| {
        total.checked_add(spec.elements).ok_or_else(|| {
            ProductError::internal(
                "MODEL_PARAMETER_COUNT_OVERFLOW",
                "parameter count overflowed",
            )
        })
    })?;
    if specs.len() != 111 || count != config.parameter_count {
        return Err(ProductError::integrity(
            "MODEL_PARAMETER_LAYOUT_MISMATCH",
            format!(
                "PARAM-001 produced {} tensors and {count} parameters",
                specs.len()
            ),
        ));
    }
    let mut names = specs.iter().map(|spec| &spec.name).collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if names.len() != specs.len() {
        return Err(ProductError::internal(
            "MODEL_PARAMETER_NAME_DUPLICATE",
            "PARAM-001 produced a duplicate stable name",
        ));
    }
    Ok(specs)
}

pub fn canonical_parameter_specs() -> Result<Vec<ParameterSpec>> {
    parameter_specs(&model_config(ModelPreset::Gqa135mV1))
}

pub const fn query_to_key_value_head(query_head: usize) -> usize {
    query_head / 3
}

pub fn rope_angle(position: usize, pair_index: usize, head_width: usize) -> Result<f32> {
    if head_width == 0 || !head_width.is_multiple_of(2) || pair_index >= head_width / 2 {
        return Err(ProductError::integrity(
            "MODEL_ROPE_PAIR_INVALID",
            "ROPE-001 requires a contained adjacent pair in an even head width",
        ));
    }
    let exponent = (2 * pair_index) as f32 / head_width as f32;
    Ok(position as f32 / ROPE_BASE.powf(exponent))
}

pub const fn causal_attention_allowed(query: usize, key: usize, key_is_padding: bool) -> bool {
    !key_is_padding && key <= query
}

pub fn f32_to_bf16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let rounding_bias = 0x7fff_u32 + ((bits >> 16) & 1);
    bits.wrapping_add(rounding_bias).wrapping_shr(16) as u16
}

pub fn bf16_bits_to_f32(bits: u16) -> f32 {
    f32::from_bits(u32::from(bits) << 16)
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn initialization_seed(preset: ModelPreset) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/init/v1\0");
    hasher.update(preset.identity().as_bytes());
    hasher.finalize().into()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitializedParameterArtifact {
    pub name: String,
    pub shape: Vec<usize>,
    pub elements: u64,
    pub initialization: ParameterInitialization,
    pub optimizer_group: OptimizerGroup,
    pub storage: &'static str,
    pub byte_order: &'static str,
    pub bytes: u64,
    pub bf16_le_sha256: String,
    pub first_values_bf16_le_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitializationManifest {
    pub schema: &'static str,
    pub model_identity: &'static str,
    pub seed_sha256_raw_hex: String,
    pub rng: &'static str,
    pub distribution: &'static str,
    pub stddev_f32_le_hex: String,
    pub bf16_rounding: &'static str,
    pub parameter_count: u64,
    pub artifact_count: usize,
    pub artifacts: Vec<InitializedParameterArtifact>,
    pub bundle_sha256: String,
}

fn fill_initialized_artifact(
    rng: &mut ChaCha12Rng,
    spec: &ParameterSpec,
) -> Result<InitializedParameterArtifact> {
    let mut hasher = Sha256::new();
    let mut first = Vec::with_capacity(16);
    let mut buffer = Vec::with_capacity(8_192);
    for _ in 0..spec.elements {
        let bits = match spec.initialization {
            ParameterInitialization::Ones => f32_to_bf16_bits(1.0),
            ParameterInitialization::NormalStddev002 => {
                let sample: f32 = StandardNormal.sample(rng);
                f32_to_bf16_bits(sample * INITIALIZATION_STDDEV)
            }
        };
        let bytes = bits.to_le_bytes();
        if first.len() < 16 {
            first.extend_from_slice(&bytes);
        }
        buffer.extend_from_slice(&bytes);
        if buffer.len() == buffer.capacity() {
            hasher.update(&buffer);
            buffer.clear();
        }
    }
    hasher.update(&buffer);
    let bytes = spec.elements.checked_mul(2).ok_or_else(|| {
        ProductError::internal(
            "MODEL_INITIALIZATION_SIZE_OVERFLOW",
            "initialized parameter byte length overflowed",
        )
    })?;
    Ok(InitializedParameterArtifact {
        name: spec.name.clone(),
        shape: spec.shape.clone(),
        elements: spec.elements,
        initialization: spec.initialization,
        optimizer_group: spec.optimizer_group,
        storage: "bf16",
        byte_order: "little",
        bytes,
        bf16_le_sha256: hex::encode(hasher.finalize()),
        first_values_bf16_le_hex: hex::encode(first),
    })
}

pub fn initialization_manifest(preset: ModelPreset) -> Result<InitializationManifest> {
    let config = model_config(preset);
    let specs = parameter_specs(&config)?;
    let seed = initialization_seed(preset);
    let mut rng = ChaCha12Rng::from_seed(seed);
    let mut artifacts = Vec::with_capacity(specs.len());
    for spec in &specs {
        artifacts.push(fill_initialized_artifact(&mut rng, spec)?);
    }
    let mut bundle = Sha256::new();
    bundle.update(b"python-slm/model-initialization-bundle/v1\0");
    update_length_prefixed(&mut bundle, config.identity.as_bytes());
    bundle.update(seed);
    for artifact in &artifacts {
        update_length_prefixed(&mut bundle, artifact.name.as_bytes());
        bundle.update((artifact.shape.len() as u64).to_le_bytes());
        for dimension in &artifact.shape {
            bundle.update((*dimension as u64).to_le_bytes());
        }
        bundle.update(artifact.elements.to_le_bytes());
        let digest = hex::decode(&artifact.bf16_le_sha256).map_err(|_| {
            ProductError::internal(
                "MODEL_INITIALIZATION_HASH_INVALID",
                "an internal initialization digest was not lowercase SHA-256",
            )
        })?;
        bundle.update(digest);
    }
    Ok(InitializationManifest {
        schema: INITIALIZATION_MANIFEST_SCHEMA,
        model_identity: config.identity,
        seed_sha256_raw_hex: hex::encode(seed),
        rng: "rand_chacha-0.10.0/ChaCha12Rng",
        distribution: "rand_distr-0.6.0/StandardNormal<f32>",
        stddev_f32_le_hex: hex::encode(INITIALIZATION_STDDEV.to_le_bytes()),
        bf16_rounding: "round-to-nearest-even",
        parameter_count: config.parameter_count,
        artifact_count: artifacts.len(),
        artifacts,
        bundle_sha256: hex::encode(bundle.finalize()),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizerRules {
    pub algorithm: &'static str,
    pub beta1: f32,
    pub beta2: f32,
    pub epsilon: f32,
    pub decay_weight: f32,
    pub no_decay_weight: f32,
    pub global_l2_clip: f32,
    pub master_weights: &'static str,
    pub moments: &'static str,
    pub parameter_storage: &'static str,
    pub clip_after_unscale: bool,
    pub reject_nonfinite: bool,
}

pub fn optimizer_rules() -> OptimizerRules {
    OptimizerRules {
        algorithm: "adamw-opt-001",
        beta1: 0.9,
        beta2: 0.95,
        epsilon: 1.0e-8,
        decay_weight: 0.1,
        no_decay_weight: 0.0,
        global_l2_clip: 1.0,
        master_weights: "fp32",
        moments: "fp32",
        parameter_storage: "bf16",
        clip_after_unscale: true,
        reject_nonfinite: true,
    }
}

pub fn gradient_clip_scale(gradients: &[f32]) -> Result<f32> {
    let mut sum = 0.0_f32;
    for gradient in gradients {
        if !gradient.is_finite() {
            return Err(ProductError::gate(
                "MODEL_GRADIENT_NONFINITE",
                "OPT-001 rejects non-finite unscaled gradients",
            ));
        }
        sum += gradient * gradient;
    }
    if !sum.is_finite() {
        return Err(ProductError::gate(
            "MODEL_GRADIENT_NORM_NONFINITE",
            "OPT-001 global gradient norm is non-finite",
        ));
    }
    if sum == 0.0 {
        return Ok(1.0);
    }
    Ok((1.0 / sum.sqrt()).min(1.0))
}

pub fn adamw_scalar_step(
    theta: f32,
    gradient: f32,
    moment: f32,
    variance: f32,
    learning_rate: f32,
    one_based_update: u32,
    weight_decay: f32,
) -> Result<(f32, f32, f32, u16)> {
    if one_based_update == 0
        || ![
            theta,
            gradient,
            moment,
            variance,
            learning_rate,
            weight_decay,
        ]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(ProductError::gate(
            "MODEL_OPTIMIZER_INPUT_INVALID",
            "OPT-001 requires finite state and a one-based update index",
        ));
    }
    let rules = optimizer_rules();
    let next_moment = rules.beta1 * moment + (1.0 - rules.beta1) * gradient;
    let next_variance = rules.beta2 * variance + (1.0 - rules.beta2) * gradient * gradient;
    let t = one_based_update as i32;
    let moment_hat = next_moment / (1.0 - rules.beta1.powi(t));
    let variance_hat = next_variance / (1.0 - rules.beta2.powi(t));
    let next_master = theta
        - learning_rate
            * (moment_hat / (variance_hat.sqrt() + rules.epsilon) + weight_decay * theta);
    if ![next_moment, next_variance, next_master]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(ProductError::gate(
            "MODEL_OPTIMIZER_RESULT_NONFINITE",
            "OPT-001 produced non-finite state",
        ));
    }
    Ok((
        next_master,
        next_moment,
        next_variance,
        f32_to_bf16_bits(next_master),
    ))
}

type NodeId = usize;

#[derive(Clone, Copy)]
enum Operation {
    Constant,
    Parameter,
    Add(NodeId, NodeId),
    Mul(NodeId, NodeId),
    Div(NodeId, NodeId),
    Sqrt(NodeId),
    Exp(NodeId),
    Ln(NodeId),
    Sin(NodeId),
    Cos(NodeId),
    Neg(NodeId),
    Bf16(NodeId),
}

#[derive(Clone, Copy)]
struct Node {
    value: f32,
    gradient: f32,
    operation: Operation,
}

#[derive(Default)]
struct Tape {
    nodes: Vec<Node>,
}

impl Tape {
    fn push(&mut self, value: f32, operation: Operation) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            value,
            gradient: 0.0,
            operation,
        });
        id
    }

    fn constant(&mut self, value: f32) -> NodeId {
        self.push(value, Operation::Constant)
    }

    fn parameter(&mut self, value: f32) -> NodeId {
        self.push(value, Operation::Parameter)
    }

    fn add(&mut self, left: NodeId, right: NodeId) -> NodeId {
        self.push(
            self.nodes[left].value + self.nodes[right].value,
            Operation::Add(left, right),
        )
    }

    fn mul(&mut self, left: NodeId, right: NodeId) -> NodeId {
        self.push(
            self.nodes[left].value * self.nodes[right].value,
            Operation::Mul(left, right),
        )
    }

    fn div(&mut self, numerator: NodeId, denominator: NodeId) -> NodeId {
        self.push(
            self.nodes[numerator].value / self.nodes[denominator].value,
            Operation::Div(numerator, denominator),
        )
    }

    fn sqrt(&mut self, input: NodeId) -> NodeId {
        self.push(self.nodes[input].value.sqrt(), Operation::Sqrt(input))
    }

    fn exp(&mut self, input: NodeId) -> NodeId {
        self.push(self.nodes[input].value.exp(), Operation::Exp(input))
    }

    fn ln(&mut self, input: NodeId) -> NodeId {
        self.push(self.nodes[input].value.ln(), Operation::Ln(input))
    }

    fn sin(&mut self, input: NodeId) -> NodeId {
        self.push(self.nodes[input].value.sin(), Operation::Sin(input))
    }

    fn cos(&mut self, input: NodeId) -> NodeId {
        self.push(self.nodes[input].value.cos(), Operation::Cos(input))
    }

    fn neg(&mut self, input: NodeId) -> NodeId {
        self.push(-self.nodes[input].value, Operation::Neg(input))
    }

    fn bf16(&mut self, input: NodeId) -> NodeId {
        self.push(
            bf16_bits_to_f32(f32_to_bf16_bits(self.nodes[input].value)),
            Operation::Bf16(input),
        )
    }

    fn value(&self, id: NodeId) -> f32 {
        self.nodes[id].value
    }

    fn sum_in_order(&mut self, values: &[NodeId]) -> NodeId {
        let mut sum = self.constant(0.0);
        for value in values {
            sum = self.add(sum, *value);
        }
        sum
    }

    fn backward(&mut self, output: NodeId) {
        self.nodes[output].gradient = 1.0;
        for id in (0..=output).rev() {
            let gradient = self.nodes[id].gradient;
            match self.nodes[id].operation {
                Operation::Constant | Operation::Parameter => {}
                Operation::Add(left, right) => {
                    self.nodes[left].gradient += gradient;
                    self.nodes[right].gradient += gradient;
                }
                Operation::Mul(left, right) => {
                    self.nodes[left].gradient += gradient * self.nodes[right].value;
                    self.nodes[right].gradient += gradient * self.nodes[left].value;
                }
                Operation::Div(numerator, denominator) => {
                    let denominator_value = self.nodes[denominator].value;
                    self.nodes[numerator].gradient += gradient / denominator_value;
                    self.nodes[denominator].gradient -= gradient * self.nodes[numerator].value
                        / (denominator_value * denominator_value);
                }
                Operation::Sqrt(input) => {
                    self.nodes[input].gradient += gradient * 0.5 / self.nodes[id].value;
                }
                Operation::Exp(input) => {
                    self.nodes[input].gradient += gradient * self.nodes[id].value;
                }
                Operation::Ln(input) => {
                    self.nodes[input].gradient += gradient / self.nodes[input].value;
                }
                Operation::Sin(input) => {
                    self.nodes[input].gradient += gradient * self.nodes[input].value.cos();
                }
                Operation::Cos(input) => {
                    self.nodes[input].gradient -= gradient * self.nodes[input].value.sin();
                }
                Operation::Neg(input) => {
                    self.nodes[input].gradient -= gradient;
                }
                Operation::Bf16(input) => {
                    self.nodes[input].gradient += gradient;
                }
            }
        }
    }
}

#[derive(Clone)]
struct FixtureParameter {
    shape: Vec<usize>,
    nodes: Vec<NodeId>,
}

fn fixture_parameter_value(name: &str, index: usize, cursor: usize) -> f32 {
    if name.contains("norm.weight") {
        let offset = (index % 3) as f32 - 1.0;
        1.0 + offset / 64.0
    } else {
        let centered = ((cursor * 37 + 11) % 31) as i32 - 15;
        centered as f32 / 64.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CpuOracleFixtureParameter {
    pub name: String,
    pub shape: Vec<usize>,
    pub values_bf16_bits: Vec<u16>,
}

pub fn cpu_oracle_fixture_parameters() -> Vec<CpuOracleFixtureParameter> {
    let mut cursor = 0;
    fixture_gradient_layout()
        .into_iter()
        .map(|(name, shape)| {
            let elements = shape.iter().product::<usize>();
            let values_bf16_bits = (0..elements)
                .map(|index| {
                    let value = fixture_parameter_value(&name, index, cursor);
                    cursor += 1;
                    f32_to_bf16_bits(value)
                })
                .collect();
            CpuOracleFixtureParameter {
                name,
                shape,
                values_bf16_bits,
            }
        })
        .collect()
}

fn add_fixture_parameter(
    tape: &mut Tape,
    parameters: &mut BTreeMap<String, FixtureParameter>,
    ordered_parameter_nodes: &mut Vec<NodeId>,
    cursor: &mut usize,
    name: &str,
    shape: Vec<usize>,
) {
    let elements = shape.iter().product::<usize>();
    let mut nodes = Vec::with_capacity(elements);
    for index in 0..elements {
        let value = fixture_parameter_value(name, index, *cursor);
        *cursor += 1;
        let rounded = bf16_bits_to_f32(f32_to_bf16_bits(value));
        let node = tape.parameter(rounded);
        nodes.push(node);
        ordered_parameter_nodes.push(node);
    }
    parameters.insert(name.to_owned(), FixtureParameter { shape, nodes });
}

fn fixture_parameters(tape: &mut Tape) -> (BTreeMap<String, FixtureParameter>, Vec<NodeId>) {
    const WIDTH: usize = 4;
    const FFN: usize = 4;
    const VOCAB: usize = 4;
    let mut parameters = BTreeMap::new();
    let mut ordered = Vec::new();
    let mut cursor = 0;
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "tok_embeddings.weight",
        vec![VOCAB, WIDTH],
    );
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "blocks.0.attn_norm.weight",
        vec![WIDTH],
    );
    for (projection, rows) in [("q", WIDTH), ("k", 2), ("v", 2), ("o", WIDTH)] {
        add_fixture_parameter(
            tape,
            &mut parameters,
            &mut ordered,
            &mut cursor,
            &format!("blocks.0.attn.{projection}.weight"),
            vec![rows, WIDTH],
        );
    }
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "blocks.0.ffn_norm.weight",
        vec![WIDTH],
    );
    for projection in ["gate", "up"] {
        add_fixture_parameter(
            tape,
            &mut parameters,
            &mut ordered,
            &mut cursor,
            &format!("blocks.0.ffn.{projection}.weight"),
            vec![FFN, WIDTH],
        );
    }
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "blocks.0.ffn.down.weight",
        vec![WIDTH, FFN],
    );
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "final_norm.weight",
        vec![WIDTH],
    );
    add_fixture_parameter(
        tape,
        &mut parameters,
        &mut ordered,
        &mut cursor,
        "lm_head.weight",
        vec![VOCAB, WIDTH],
    );
    (parameters, ordered)
}

fn tensor(parameters: &BTreeMap<String, FixtureParameter>, name: &str) -> FixtureParameter {
    parameters
        .get(name)
        .unwrap_or_else(|| panic!("missing internal fixture parameter {name}"))
        .clone()
}

fn matvec(tape: &mut Tape, parameter: &FixtureParameter, input: &[NodeId]) -> Vec<NodeId> {
    let rows = parameter.shape[0];
    let columns = parameter.shape[1];
    assert_eq!(columns, input.len());
    (0..rows)
        .map(|row| {
            let mut products = Vec::with_capacity(columns);
            for (column, input_node) in input.iter().enumerate() {
                products.push(tape.mul(parameter.nodes[row * columns + column], *input_node));
            }
            let sum = tape.sum_in_order(&products);
            tape.bf16(sum)
        })
        .collect()
}

fn rms_norm(tape: &mut Tape, input: &[NodeId], scale: &FixtureParameter) -> Vec<NodeId> {
    assert_eq!(input.len(), scale.nodes.len());
    let squares = input
        .iter()
        .map(|value| tape.mul(*value, *value))
        .collect::<Vec<_>>();
    let sum = tape.sum_in_order(&squares);
    let count = tape.constant(input.len() as f32);
    let mean = tape.div(sum, count);
    let epsilon = tape.constant(RMS_NORM_EPSILON);
    let stabilized = tape.add(mean, epsilon);
    let root = tape.sqrt(stabilized);
    let one = tape.constant(1.0);
    let inverse = tape.div(one, root);
    input
        .iter()
        .zip(&scale.nodes)
        .map(|(value, weight)| {
            let normalized = tape.mul(*value, inverse);
            let scaled = tape.mul(normalized, *weight);
            tape.bf16(scaled)
        })
        .collect()
}

fn rope_pair(
    tape: &mut Tape,
    left: NodeId,
    right: NodeId,
    position: usize,
    pair_index: usize,
    head_width: usize,
) -> [NodeId; 2] {
    let angle = tape.constant(
        rope_angle(position, pair_index, head_width)
            .expect("the internal fixture uses a valid RoPE pair"),
    );
    let cosine = tape.cos(angle);
    let sine = tape.sin(angle);
    let left_cos = tape.mul(left, cosine);
    let right_sin = tape.mul(right, sine);
    let neg_right_sin = tape.neg(right_sin);
    let first = tape.add(left_cos, neg_right_sin);
    let left_sin = tape.mul(left, sine);
    let right_cos = tape.mul(right, cosine);
    let second = tape.add(left_sin, right_cos);
    [tape.bf16(first), tape.bf16(second)]
}

fn apply_rope(
    tape: &mut Tape,
    input: &[NodeId],
    position: usize,
    head_width: usize,
) -> Vec<NodeId> {
    assert!(head_width > 0 && head_width.is_multiple_of(2));
    assert_eq!(input.len() % head_width, 0);
    let mut output = Vec::with_capacity(input.len());
    for head in input.chunks_exact(head_width) {
        for (pair_index, pair) in head.chunks_exact(2).enumerate() {
            output.extend(rope_pair(
                tape, pair[0], pair[1], position, pair_index, head_width,
            ));
        }
    }
    output
}

fn softmax(tape: &mut Tape, scores: &[NodeId]) -> Vec<NodeId> {
    let maximum = scores
        .iter()
        .map(|score| tape.value(*score))
        .fold(f32::NEG_INFINITY, f32::max);
    let maximum = tape.constant(maximum);
    let exponentials = scores
        .iter()
        .map(|score| {
            let neg_maximum = tape.neg(maximum);
            let shifted = tape.add(*score, neg_maximum);
            tape.exp(shifted)
        })
        .collect::<Vec<_>>();
    let denominator = tape.sum_in_order(&exponentials);
    exponentials
        .into_iter()
        .map(|value| tape.div(value, denominator))
        .collect()
}

fn residual_add(tape: &mut Tape, left: &[NodeId], right: &[NodeId]) -> Vec<NodeId> {
    left.iter()
        .zip(right)
        .map(|(left, right)| {
            let sum = tape.add(*left, *right);
            tape.bf16(sum)
        })
        .collect()
}

fn fixture_gradient_layout() -> Vec<(String, Vec<usize>)> {
    vec![
        ("tok_embeddings.weight".to_owned(), vec![4, 4]),
        ("blocks.0.attn_norm.weight".to_owned(), vec![4]),
        ("blocks.0.attn.q.weight".to_owned(), vec![4, 4]),
        ("blocks.0.attn.k.weight".to_owned(), vec![2, 4]),
        ("blocks.0.attn.v.weight".to_owned(), vec![2, 4]),
        ("blocks.0.attn.o.weight".to_owned(), vec![4, 4]),
        ("blocks.0.ffn_norm.weight".to_owned(), vec![4]),
        ("blocks.0.ffn.gate.weight".to_owned(), vec![4, 4]),
        ("blocks.0.ffn.up.weight".to_owned(), vec![4, 4]),
        ("blocks.0.ffn.down.weight".to_owned(), vec![4, 4]),
        ("final_norm.weight".to_owned(), vec![4]),
        ("lm_head.weight".to_owned(), vec![4, 4]),
    ]
}

fn scalar_oracle_graph() -> CpuOracleResult {
    const WIDTH: usize = 4;
    const QUERY_HEADS: usize = 2;
    const KEY_VALUE_HEADS: usize = 1;
    const HEAD_WIDTH: usize = 2;
    const VOCAB: usize = 4;
    const SEQUENCE: usize = 2;
    let input_token_ids = [1_usize, 2];
    let target_token_ids = [2_usize, 3];

    let mut tape = Tape::default();
    let (parameters, ordered_parameter_nodes) = fixture_parameters(&mut tape);
    let gradient_layout = fixture_gradient_layout();
    let embedding = tensor(&parameters, "tok_embeddings.weight");
    let mut hidden = input_token_ids
        .iter()
        .map(|token| embedding.nodes[token * WIDTH..(token + 1) * WIDTH].to_vec())
        .collect::<Vec<_>>();

    let attn_norm = tensor(&parameters, "blocks.0.attn_norm.weight");
    let query_weight = tensor(&parameters, "blocks.0.attn.q.weight");
    let key_weight = tensor(&parameters, "blocks.0.attn.k.weight");
    let value_weight = tensor(&parameters, "blocks.0.attn.v.weight");
    let output_weight = tensor(&parameters, "blocks.0.attn.o.weight");
    let mut queries = Vec::with_capacity(SEQUENCE);
    let mut keys = Vec::with_capacity(SEQUENCE);
    let mut values = Vec::with_capacity(SEQUENCE);
    for (position, hidden_at_position) in hidden.iter().enumerate().take(SEQUENCE) {
        let normalized = rms_norm(&mut tape, hidden_at_position, &attn_norm);
        let query = matvec(&mut tape, &query_weight, &normalized);
        let key = matvec(&mut tape, &key_weight, &normalized);
        values.push(matvec(&mut tape, &value_weight, &normalized));
        queries.push(apply_rope(&mut tape, &query, position, HEAD_WIDTH));
        keys.push(apply_rope(&mut tape, &key, position, HEAD_WIDTH));
    }

    let scale = tape.constant((HEAD_WIDTH as f32).sqrt());
    let mut attention_outputs = Vec::with_capacity(SEQUENCE);
    for (query_position, query_at_position) in queries.iter().enumerate().take(SEQUENCE) {
        let mut concatenated = Vec::with_capacity(WIDTH);
        for query_head in 0..QUERY_HEADS {
            let key_value_head = query_to_key_value_head(query_head);
            assert!(key_value_head < KEY_VALUE_HEADS);
            let query_start = query_head * HEAD_WIDTH;
            let key_start = key_value_head * HEAD_WIDTH;
            let mut scores = Vec::with_capacity(query_position + 1);
            for key_at_position in keys.iter().take(query_position + 1) {
                let mut products = Vec::with_capacity(HEAD_WIDTH);
                for dimension in 0..HEAD_WIDTH {
                    products.push(tape.mul(
                        query_at_position[query_start + dimension],
                        key_at_position[key_start + dimension],
                    ));
                }
                let dot = tape.sum_in_order(&products);
                scores.push(tape.div(dot, scale));
            }
            let probabilities = softmax(&mut tape, &scores);
            for dimension in 0..HEAD_WIDTH {
                let terms = probabilities
                    .iter()
                    .enumerate()
                    .map(|(key_position, probability)| {
                        tape.mul(*probability, values[key_position][key_start + dimension])
                    })
                    .collect::<Vec<_>>();
                let context = tape.sum_in_order(&terms);
                concatenated.push(tape.bf16(context));
            }
        }
        let projected = matvec(&mut tape, &output_weight, &concatenated);
        attention_outputs.push(projected);
    }
    for position in 0..SEQUENCE {
        hidden[position] = residual_add(&mut tape, &hidden[position], &attention_outputs[position]);
    }

    let ffn_norm = tensor(&parameters, "blocks.0.ffn_norm.weight");
    let gate_weight = tensor(&parameters, "blocks.0.ffn.gate.weight");
    let up_weight = tensor(&parameters, "blocks.0.ffn.up.weight");
    let down_weight = tensor(&parameters, "blocks.0.ffn.down.weight");
    for hidden_at_position in hidden.iter_mut().take(SEQUENCE) {
        let normalized = rms_norm(&mut tape, hidden_at_position, &ffn_norm);
        let gate = matvec(&mut tape, &gate_weight, &normalized);
        let up = matvec(&mut tape, &up_weight, &normalized);
        let mut activated = Vec::with_capacity(gate.len());
        for (gate_value, up_value) in gate.iter().zip(&up) {
            let negated = tape.neg(*gate_value);
            let exponential = tape.exp(negated);
            let one = tape.constant(1.0);
            let denominator = tape.add(one, exponential);
            let numerator = tape.constant(1.0);
            let sigmoid = tape.div(numerator, denominator);
            let silu = tape.mul(*gate_value, sigmoid);
            let product = tape.mul(silu, *up_value);
            activated.push(tape.bf16(product));
        }
        let projected = matvec(&mut tape, &down_weight, &activated);
        *hidden_at_position = residual_add(&mut tape, hidden_at_position, &projected);
    }

    let final_norm = tensor(&parameters, "final_norm.weight");
    let lm_head = tensor(&parameters, "lm_head.weight");
    let mut logits = Vec::with_capacity(SEQUENCE);
    for hidden_at_position in hidden.iter().take(SEQUENCE) {
        let normalized = rms_norm(&mut tape, hidden_at_position, &final_norm);
        logits.push(matvec(&mut tape, &lm_head, &normalized));
    }

    let mut losses = Vec::with_capacity(SEQUENCE);
    for position in 0..SEQUENCE {
        let maximum = logits[position]
            .iter()
            .map(|node| tape.value(*node))
            .fold(f32::NEG_INFINITY, f32::max);
        let maximum_node = tape.constant(maximum);
        let exponentials = logits[position]
            .iter()
            .map(|logit| {
                let neg_maximum = tape.neg(maximum_node);
                let shifted = tape.add(*logit, neg_maximum);
                tape.exp(shifted)
            })
            .collect::<Vec<_>>();
        let sum = tape.sum_in_order(&exponentials);
        let log_sum = tape.ln(sum);
        let log_partition = tape.add(log_sum, maximum_node);
        let target = logits[position][target_token_ids[position]];
        let neg_target = tape.neg(target);
        losses.push(tape.add(log_partition, neg_target));
    }
    let loss_sum = tape.sum_in_order(&losses);
    let valid_targets = tape.constant(SEQUENCE as f32);
    let loss = tape.div(loss_sum, valid_targets);
    tape.backward(loss);

    let logits_bits = logits
        .iter()
        .flatten()
        .map(|node| f32_to_bf16_bits(tape.value(*node)))
        .collect::<Vec<_>>();
    let logits_bytes = logits_bits
        .iter()
        .flat_map(|bits| bits.to_le_bytes())
        .collect::<Vec<_>>();
    let gradients = ordered_parameter_nodes
        .iter()
        .map(|node| tape.nodes[*node].gradient)
        .collect::<Vec<_>>();
    let gradient_bytes = gradients
        .iter()
        .flat_map(|gradient| gradient.to_le_bytes())
        .collect::<Vec<_>>();
    let mut byte_offset = 0_usize;
    let gradient_artifacts = gradient_layout
        .into_iter()
        .map(|(name, shape)| {
            let elements = shape.iter().product::<usize>();
            let byte_length = elements * size_of::<f32>();
            let end = byte_offset + byte_length;
            let artifact = OracleGradientArtifact {
                name,
                shape,
                elements,
                byte_offset,
                byte_length,
                f32_le_sha256: sha256_hex(&gradient_bytes[byte_offset..end]),
            };
            byte_offset = end;
            artifact
        })
        .collect::<Vec<_>>();
    assert_eq!(byte_offset, gradient_bytes.len());
    let finite = tape.value(loss).is_finite()
        && gradients.iter().all(|gradient| gradient.is_finite())
        && logits
            .iter()
            .flatten()
            .all(|node| tape.value(*node).is_finite());

    CpuOracleResult {
        schema: CPU_ORACLE_SCHEMA,
        fixture_id: CPU_ORACLE_FIXTURE_ID,
        model_semantics: "pre-norm-gqa-rope-swiglu-causal-cross-entropy-v1",
        parameter_storage: "bf16-le",
        activation_storage: "bf16",
        accumulation: "fp32-left-to-right",
        gradient_storage: "ieee754-f32-le",
        bf16_rounding: "round-to-nearest-even",
        bf16_cast_gradient: "identity",
        vocabulary_size: VOCAB,
        width: WIDTH,
        ffn_width: 4,
        layers: 1,
        query_heads: QUERY_HEADS,
        key_value_heads: KEY_VALUE_HEADS,
        head_width: HEAD_WIDTH,
        sequence_length: SEQUENCE,
        parameter_count: ordered_parameter_nodes.len(),
        input_token_ids: input_token_ids.to_vec(),
        target_token_ids: target_token_ids.to_vec(),
        logits_bf16_le_hex: hex::encode(&logits_bytes),
        logits_sha256: sha256_hex(&logits_bytes),
        loss_f32_le_hex: hex::encode(tape.value(loss).to_le_bytes()),
        gradient_f32_le_hex: hex::encode(&gradient_bytes),
        gradient_artifacts,
        gradient_sha256: sha256_hex(&gradient_bytes),
        finite,
        causal_mask: "inclusive-lower-triangular",
        gqa_mapping: "query-head-div-3",
        rope: "adjacent-pairs-base-10000-reset-at-sample-start",
        rms_norm_epsilon_f32_le_hex: hex::encode(RMS_NORM_EPSILON.to_le_bytes()),
        loss_normalized_by_valid_targets: SEQUENCE,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleGradientArtifact {
    pub name: String,
    pub shape: Vec<usize>,
    pub elements: usize,
    pub byte_offset: usize,
    pub byte_length: usize,
    pub f32_le_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CpuOracleResult {
    pub schema: &'static str,
    pub fixture_id: &'static str,
    pub model_semantics: &'static str,
    pub parameter_storage: &'static str,
    pub activation_storage: &'static str,
    pub accumulation: &'static str,
    pub gradient_storage: &'static str,
    pub bf16_rounding: &'static str,
    pub bf16_cast_gradient: &'static str,
    pub vocabulary_size: usize,
    pub width: usize,
    pub ffn_width: usize,
    pub layers: usize,
    pub query_heads: usize,
    pub key_value_heads: usize,
    pub head_width: usize,
    pub sequence_length: usize,
    pub parameter_count: usize,
    pub input_token_ids: Vec<usize>,
    pub target_token_ids: Vec<usize>,
    pub logits_bf16_le_hex: String,
    pub logits_sha256: String,
    pub loss_f32_le_hex: String,
    pub gradient_artifacts: Vec<OracleGradientArtifact>,
    pub gradient_f32_le_hex: String,
    pub gradient_sha256: String,
    pub finite: bool,
    pub causal_mask: &'static str,
    pub gqa_mapping: &'static str,
    pub rope: &'static str,
    pub rms_norm_epsilon_f32_le_hex: String,
    pub loss_normalized_by_valid_targets: usize,
}

pub fn cpu_oracle_fixture() -> CpuOracleResult {
    scalar_oracle_graph()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOracleResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub qualification_status: &'static str,
    pub model: ModelConfig,
    pub parameter_layout_sha256: String,
    pub initialization: InitializationManifest,
    pub optimizer: OptimizerRules,
    pub cpu_oracle: CpuOracleResult,
    pub limitations: Vec<&'static str>,
    pub receipts_written: bool,
}

pub fn parameter_layout_sha256(specs: &[ParameterSpec]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/parameter-layout/v1\0");
    for spec in specs {
        update_length_prefixed(&mut hasher, spec.name.as_bytes());
        hasher.update((spec.shape.len() as u64).to_le_bytes());
        for dimension in &spec.shape {
            hasher.update((*dimension as u64).to_le_bytes());
        }
        hasher.update(spec.elements.to_le_bytes());
        hasher.update([match spec.initialization {
            ParameterInitialization::NormalStddev002 => 0,
            ParameterInitialization::Ones => 1,
        }]);
        hasher.update([match spec.optimizer_group {
            OptimizerGroup::Decay => 0,
            OptimizerGroup::NoDecay => 1,
        }]);
    }
    hex::encode(hasher.finalize())
}

pub fn model_oracle_result() -> Result<ModelOracleResult> {
    let model = model_config(ModelPreset::Gqa135mV1);
    let specs = parameter_specs(&model)?;
    Ok(ModelOracleResult {
        schema: MODEL_ORACLE_RESULT_SCHEMA,
        status: "ORACLE_READY",
        qualification_status: "SKIPPED",
        parameter_layout_sha256: parameter_layout_sha256(&specs),
        initialization: initialization_manifest(ModelPreset::Gqa135mV1)?,
        optimizer: optimizer_rules(),
        cpu_oracle: cpu_oracle_fixture(),
        model,
        limitations: vec![
            "no accelerator parity claim",
            "no training stability claim",
            "no performance or SLA claim",
            "no qualification or publication claim",
        ],
        receipts_written: false,
    })
}

pub fn oracle_result_value() -> Result<serde_json::Value> {
    serde_json::to_value(model_oracle_result()?).map_err(|error| {
        ProductError::internal(
            "MODEL_ORACLE_SERIALIZATION_FAILED",
            format!("model oracle result serialization failed: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_layout_has_exact_names_shapes_count_and_groups() {
        let config = model_config(ModelPreset::Gqa135mV1);
        config.validate().unwrap();
        let specs = parameter_specs(&config).unwrap();
        assert_eq!(specs.len(), 111);
        assert_eq!(
            specs.iter().map(|spec| spec.elements).sum::<u64>(),
            CANONICAL_PARAMETER_COUNT
        );
        assert_eq!(specs[0].name, "tok_embeddings.weight");
        assert_eq!(specs[0].shape, [32_000, 768]);
        assert_eq!(specs[1].name, "blocks.0.attn_norm.weight");
        assert_eq!(specs[3].name, "blocks.0.attn.k.weight");
        assert_eq!(specs[3].shape, [256, 768]);
        assert_eq!(specs[109].name, "final_norm.weight");
        assert_eq!(specs[110].name, "lm_head.weight");
        assert!(
            specs
                .iter()
                .filter(|spec| spec.optimizer_group == OptimizerGroup::NoDecay)
                .all(|spec| spec.name.ends_with("norm.weight"))
        );
        assert_eq!(
            specs
                .iter()
                .filter(|spec| spec.optimizer_group == OptimizerGroup::NoDecay)
                .count(),
            25
        );
    }

    #[test]
    fn reference_layout_remains_noncanonical_and_exact() {
        let config = model_config(ModelPreset::Gqa124mRefV1);
        assert!(!config.canonical);
        assert_eq!(config.ffn_width, 2_048);
        let specs = parameter_specs(&config).unwrap();
        assert_eq!(
            specs.iter().map(|spec| spec.elements).sum::<u64>(),
            REFERENCE_PARAMETER_COUNT
        );
    }

    #[test]
    fn bf16_conversion_is_round_to_nearest_even() {
        assert_eq!(f32_to_bf16_bits(1.0), 0x3f80);
        assert_eq!(bf16_bits_to_f32(0x3f80), 1.0);
        assert_eq!(f32_to_bf16_bits(f32::from_bits(0x3f80_8000)), 0x3f80);
        assert_eq!(f32_to_bf16_bits(f32::from_bits(0x3f81_8000)), 0x3f82);
    }

    #[test]
    fn initialization_seed_and_prefix_are_deterministic() {
        assert_eq!(
            hex::encode(initialization_seed(ModelPreset::Gqa135mV1)),
            "171201c662d86d988d6b086e87494ab31b84631e327ccc156583b30a8304ba32"
        );
        let seed = initialization_seed(ModelPreset::Gqa135mV1);
        let mut first = ChaCha12Rng::from_seed(seed);
        let mut second = ChaCha12Rng::from_seed(seed);
        let first = (0..16)
            .map(|_| {
                let value: f32 = StandardNormal.sample(&mut first);
                f32_to_bf16_bits(value * INITIALIZATION_STDDEV)
            })
            .collect::<Vec<_>>();
        let second = (0..16)
            .map(|_| {
                let value: f32 = StandardNormal.sample(&mut second);
                f32_to_bf16_bits(value * INITIALIZATION_STDDEV)
            })
            .collect::<Vec<_>>();
        assert_eq!(first, second);
        assert_eq!(
            hex::encode(
                first
                    .iter()
                    .flat_map(|bits| bits.to_le_bytes())
                    .collect::<Vec<_>>()
            ),
            "6bbd85bcc5bb5e3c243bdb3a1b3cc33ce9bb083ca5baa23bc6b9f63ba33ab53c"
        );
    }

    #[test]
    fn cpu_oracle_is_deterministic_finite_and_literal() {
        let first = cpu_oracle_fixture();
        let second = cpu_oracle_fixture();
        assert_eq!(first, second);
        assert!(first.finite);
        assert_eq!(first.parameter_count, 140);
        assert_eq!(first.logits_bf16_le_hex, "433ed13e11bee4be6fbe63be0e3f9a3c");
        assert_eq!(first.loss_f32_le_hex, "f52fc23f");
        assert_eq!(
            first.gradient_sha256,
            "60a399908fc3125c6ed07193e14138fb979aed50f0317435ebcf0ea53e0e05ed"
        );
        assert_eq!(
            first.gradient_f32_le_hex,
            "00000000000000000000000000000000bbcf3dbf81e3b13f70bcd13efb983f3ebcec48be54daf5be686dde3ebcbdb43e00000000000000000000000000000000841a863c74c49bbccc2a9d3a6b588bbca5b40ebd4ebc053d952e883c00000000bff79ebbbaf9943b5eb3173b000000004286c93c71dbbcbcd94f40bc00000000427d603be76052bb173ad6ba000000007fde80bcb6086b3b7de0913b0c40a63b70a8fbba5da0b8bb8eab92bafd8e5d3b4885053ef017d9bc778613bd734232bd044c79be8343d1bd90c2d23cb7d5193ecd52593dc7318b3e5170673dda2f8c3e2ea3a3beafd4bcbe9a7f9fbeab80babe00863bbc52d9adbe102998bce8e6afbe25264e3d344023bef3ae343df6d426be318f85bdccdf22bc284ea13b4cbd0abc4f80183db227133d1b763abc5a96a1bc1324db3d3f1e6a3dac78e7bc55543ebd51ead4bd12ab9d3dc016703cd219813cbfd6aa3d462f853d39a6c4bc31f926bd9a562f3d2629273d729955bc63e2b8bc39a8db3da06e653d4201e7bcc69d3dbdbb72adbd1c43893d0ac53c3c42b1423c75e7cd3de822a23de0a2edbc2cf649bd289f0bbd0df8aebd128b43bd0c1e91bd8c813c3d96dd103e3774c7bda2cd013ea6e9c93ca6255c3dc08dac3dbfb82a3da55ba83c3a352f3d87c1a93d2a68043daacc1e3eacae313d7640f2bbbc0a87bb685aabbdfed283be2cb5303cff680c3d378c0cbe82af97be74f3963c69074a3dd412723f95864e3ed6f40cbeccbe74be7a8439bf373fb43e2916de3db3221f3e"
        );
        assert_eq!(first.gradient_artifacts.len(), 12);
        assert_eq!(first.gradient_artifacts[0].name, "tok_embeddings.weight");
        assert_eq!(first.gradient_artifacts[0].shape, [4, 4]);
        assert_eq!(first.gradient_artifacts[11].name, "lm_head.weight");
        let gradient_bytes = hex::decode(&first.gradient_f32_le_hex).unwrap();
        let mut next_offset = 0;
        for artifact in &first.gradient_artifacts {
            assert_eq!(artifact.byte_offset, next_offset);
            let end = artifact.byte_offset + artifact.byte_length;
            assert_eq!(
                artifact.f32_le_sha256,
                sha256_hex(&gradient_bytes[artifact.byte_offset..end])
            );
            next_offset = end;
        }
        assert_eq!(next_offset, gradient_bytes.len());
    }

    #[test]
    fn optimizer_grouping_clipping_and_step_follow_opt_001() {
        assert_eq!(
            gradient_clip_scale(&[3.0, 4.0]).unwrap().to_bits(),
            0.2_f32.to_bits()
        );
        assert_eq!(gradient_clip_scale(&[0.0, 0.0]).unwrap(), 1.0);
        assert_eq!(
            gradient_clip_scale(&[f32::NAN]).unwrap_err().code,
            "MODEL_GRADIENT_NONFINITE"
        );
        let (master, moment, variance, storage) =
            adamw_scalar_step(1.0, 0.5, 0.0, 0.0, 0.001, 1, 0.1).unwrap();
        assert_eq!(moment.to_bits(), 1_028_443_344);
        assert_eq!(variance.to_bits(), 1_011_666_128);
        assert!(master < 1.0);
        assert_eq!(storage, f32_to_bf16_bits(master));
    }

    #[test]
    fn initialization_stream_keeps_norms_at_one_without_consuming_rng() {
        let seed = initialization_seed(ModelPreset::Gqa135mV1);
        let mut rng = ChaCha12Rng::from_seed(seed);
        let matrix = parameter_spec(
            "matrix",
            vec![16],
            ParameterInitialization::NormalStddev002,
            OptimizerGroup::Decay,
        )
        .unwrap();
        let norm = parameter_spec(
            "norm",
            vec![4],
            ParameterInitialization::Ones,
            OptimizerGroup::NoDecay,
        )
        .unwrap();
        let first = fill_initialized_artifact(&mut rng, &matrix).unwrap();
        let mut expected_rng = rng.clone();
        let norm = fill_initialized_artifact(&mut rng, &norm).unwrap();
        let after_norm = fill_initialized_artifact(&mut rng, &matrix).unwrap();
        let expected = fill_initialized_artifact(&mut expected_rng, &matrix).unwrap();
        assert_eq!(first.first_values_bf16_le_hex.len(), 32);
        assert_eq!(norm.first_values_bf16_le_hex, "803f803f803f803f");
        assert_eq!(after_norm.bf16_le_sha256, expected.bf16_le_sha256);
    }

    #[test]
    fn rope_gqa_and_causal_mask_rules_are_exact() {
        assert_eq!(rope_angle(7, 0, 64).unwrap().to_bits(), 7.0_f32.to_bits());
        assert_eq!(
            rope_angle(7, 32, 64).unwrap_err().code,
            "MODEL_ROPE_PAIR_INVALID"
        );
        assert!(causal_attention_allowed(3, 3, false));
        assert!(causal_attention_allowed(3, 0, false));
        assert!(!causal_attention_allowed(3, 4, false));
        assert!(!causal_attention_allowed(3, 0, true));
    }

    #[test]
    fn query_heads_map_to_four_key_value_heads_in_groups_of_three() {
        assert_eq!(
            (0..12).map(query_to_key_value_head).collect::<Vec<_>>(),
            [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3]
        );
    }
}
