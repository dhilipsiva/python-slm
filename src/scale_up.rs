//! P19 optional scale-up amendment planning.
//!
//! The engine recomputes every derived model, memory, schedule, token-accounting,
//! evaluation, and SLA identity for one explicitly requested larger scope. It is
//! deterministic, non-publishing, and never changes a frozen canonical constant:
//! the emitted plan is an unapproved amendment candidate, and every affected
//! qualification or training phase must rerun before any claim exists.

use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use crate::model::{
    CANONICAL_MODEL_ID, CANONICAL_PARAMETER_COUNT, COMPLETION_SLA_SECONDS, VALID_TRAINING_TARGETS,
};
use crate::train::trainer::{
    EVENT_INTERVAL_TARGETS, MINIMUM_LEARNING_RATE, PEAK_LEARNING_RATE, TARGETS_PER_FULL_UPDATE,
    WARMUP_UPDATES,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

pub const SCALE_UP_CONFIG_SCHEMA: &str = "python-slm-scale-up-config-v1";
pub const SCALE_UP_RESULT_SCHEMA: &str = "python-slm-scale-up-plan-result-v1";
pub const ALIGNMENT_QUANTUM_BYTES: u64 = 268_435_456;
pub const TRAINING_STATE_FLOOR_FACTOR: u64 = 20;
pub const MINIMUM_VOCABULARY: u64 = 260;
pub const MAXIMUM_U16_VOCABULARY: u64 = 65_536;

const CANONICAL_SCOPE: RequestedModelScopeV1 = RequestedModelScopeV1 {
    vocabulary: 32_000,
    width: 768,
    ffn_width: 2_432,
    layers: 12,
    query_heads: 12,
    kv_heads: 4,
    head_width: 64,
    context_length: 2_048,
    untied_lm_head: true,
};

const REBUILD_CHAIN: [&str; 13] = [
    "P7", "P7A", "P8", "P9A", "P9B", "P10", "P11", "P12", "P13", "P14", "P15", "P16", "P16A",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedModelScopeV1 {
    pub vocabulary: u64,
    pub width: u64,
    pub ffn_width: u64,
    pub layers: u64,
    pub query_heads: u64,
    pub kv_heads: u64,
    pub head_width: u64,
    pub context_length: u64,
    pub untied_lm_head: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleUpConfigV1 {
    pub schema: String,
    pub profile: String,
    pub requested_model: RequestedModelScopeV1,
    pub requested_total_valid_targets: u64,
    pub requested_completion_sla_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleUpBaselineV1 {
    pub model_identity: &'static str,
    pub parameters: u64,
    pub valid_training_targets: u64,
    pub completion_sla_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateModelPlanV1 {
    pub parameters: u64,
    pub embedding_parameters: u64,
    pub lm_head_parameters: u64,
    pub per_layer_attention_parameters: u64,
    pub per_layer_ffn_parameters: u64,
    pub per_layer_norm_parameters: u64,
    pub decoder_parameters: u64,
    pub final_norm_parameters: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateMemoryPlanV1 {
    pub raw_training_state_floor_bytes: u64,
    pub minimum_accelerator_bytes: u64,
    pub alignment_quantum_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTokenPlanV1 {
    pub stored_prefix_ids: u64,
    pub valid_training_targets: u64,
    pub targets_per_full_update: u64,
    pub full_updates: u64,
    pub final_update_targets: u64,
    pub total_updates: u64,
    pub context_length: u64,
    pub complete_spans: u64,
    pub final_span_targets: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSchedulePlanV1 {
    pub optimizer: &'static str,
    pub warmup_updates: u64,
    pub peak_learning_rate_f32_le_hex: String,
    pub minimum_learning_rate_f32_le_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateEvaluationPlanV1 {
    pub event_interval_targets: u64,
    pub evaluation_event_count: u64,
    pub includes_initial_baseline: bool,
    pub includes_completion: bool,
    pub retention_anchor_targets: [u64; 4],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSlaPlanV1 {
    pub admission_seconds: u64,
    pub completion_sla_seconds: u64,
    pub actual_elapsed_limit_ns: u64,
    pub admission_fraction: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleUpPlanResultV1 {
    pub schema: &'static str,
    pub status: &'static str,
    pub qualification_status: &'static str,
    pub amendment_status: &'static str,
    pub profile: String,
    pub candidate_identity: String,
    pub scope_sha256: String,
    pub baseline: ScaleUpBaselineV1,
    pub requested_model: RequestedModelScopeV1,
    pub increased_axes: Vec<&'static str>,
    pub model: CandidateModelPlanV1,
    pub memory: CandidateMemoryPlanV1,
    pub tokens: CandidateTokenPlanV1,
    pub schedule: CandidateSchedulePlanV1,
    pub evaluation: CandidateEvaluationPlanV1,
    pub sla: CandidateSlaPlanV1,
    pub required_rerun_phases: Vec<&'static str>,
    pub receipts_written: bool,
    pub limitations: Vec<&'static str>,
}

fn overflow() -> ProductError {
    ProductError::integrity(
        "P19_ARITHMETIC_OVERFLOW",
        "the requested scope overflows exact 64-bit arithmetic",
    )
}

fn mul(left: u64, right: u64) -> Result<u64> {
    left.checked_mul(right).ok_or_else(overflow)
}

fn add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right).ok_or_else(overflow)
}

fn validate_scope(scope: &RequestedModelScopeV1) -> Result<()> {
    if !scope.untied_lm_head
        || scope.width == 0
        || scope.ffn_width == 0
        || scope.layers == 0
        || scope.query_heads == 0
        || scope.kv_heads == 0
        || scope.head_width == 0
        || scope.context_length == 0
        || !scope.head_width.is_multiple_of(2)
        || scope
            .query_heads
            .checked_mul(scope.head_width)
            .is_none_or(|projected| projected != scope.width)
        || !scope.query_heads.is_multiple_of(scope.kv_heads)
    {
        return Err(ProductError::usage(
            "P19_MODEL_SHAPE_INVALID",
            "the requested model is not a valid untied pre-norm GQA shape: \
             query_heads*head_width must equal width, kv_heads must divide query_heads, \
             head_width must be even, and every dimension must be nonzero",
        ));
    }
    if scope.vocabulary < MINIMUM_VOCABULARY || scope.vocabulary > MAXIMUM_U16_VOCABULARY {
        return Err(ProductError::usage(
            "P19_VOCABULARY_INVALID",
            format!(
                "the vocabulary must cover the {MINIMUM_VOCABULARY} fixed byte and special IDs \
                 and fit the immutable u16 shard format ({MAXIMUM_U16_VOCABULARY} maximum)"
            ),
        ));
    }
    Ok(())
}

fn model_plan(scope: &RequestedModelScopeV1) -> Result<CandidateModelPlanV1> {
    let embedding = mul(scope.vocabulary, scope.width)?;
    let lm_head = embedding;
    let query_width = mul(scope.query_heads, scope.head_width)?;
    let kv_width = mul(scope.kv_heads, scope.head_width)?;
    let attention = add(
        add(
            mul(scope.width, query_width)?,
            mul(2, mul(scope.width, kv_width)?)?,
        )?,
        mul(query_width, scope.width)?,
    )?;
    let ffn = mul(3, mul(scope.width, scope.ffn_width)?)?;
    let norms = mul(2, scope.width)?;
    let per_layer = add(add(attention, ffn)?, norms)?;
    let decoder = mul(scope.layers, per_layer)?;
    let final_norm = scope.width;
    let parameters = add(add(add(embedding, lm_head)?, decoder)?, final_norm)?;
    Ok(CandidateModelPlanV1 {
        parameters,
        embedding_parameters: embedding,
        lm_head_parameters: lm_head,
        per_layer_attention_parameters: attention,
        per_layer_ffn_parameters: ffn,
        per_layer_norm_parameters: norms,
        decoder_parameters: decoder,
        final_norm_parameters: final_norm,
    })
}

fn memory_plan(parameters: u64) -> Result<CandidateMemoryPlanV1> {
    let raw = mul(TRAINING_STATE_FLOOR_FACTOR, parameters)?;
    let aligned_quanta = add(raw, ALIGNMENT_QUANTUM_BYTES - 1)? / ALIGNMENT_QUANTUM_BYTES;
    Ok(CandidateMemoryPlanV1 {
        raw_training_state_floor_bytes: raw,
        minimum_accelerator_bytes: mul(aligned_quanta, ALIGNMENT_QUANTUM_BYTES)?,
        alignment_quantum_bytes: ALIGNMENT_QUANTUM_BYTES,
    })
}

fn token_plan(targets: u64, context_length: u64) -> Result<CandidateTokenPlanV1> {
    if targets == 0 || !targets.is_multiple_of(4) {
        return Err(ProductError::usage(
            "P19_TARGETS_INVALID",
            "the requested valid-target count must be a nonzero multiple of four so the \
             quarter retention anchors stay exact",
        ));
    }
    let full_updates = targets / TARGETS_PER_FULL_UPDATE;
    let final_update_targets = targets % TARGETS_PER_FULL_UPDATE;
    let total_updates = if final_update_targets == 0 {
        full_updates
    } else {
        add(full_updates, 1)?
    };
    if total_updates <= WARMUP_UPDATES {
        return Err(ProductError::usage(
            "P19_SCHEDULE_INVALID",
            "the requested run has fewer optimizer updates than the frozen warmup schedule",
        ));
    }
    let plan = CandidateTokenPlanV1 {
        stored_prefix_ids: add(targets, 1)?,
        valid_training_targets: targets,
        targets_per_full_update: TARGETS_PER_FULL_UPDATE,
        full_updates,
        final_update_targets,
        total_updates,
        context_length,
        complete_spans: targets / context_length,
        final_span_targets: targets % context_length,
    };
    let accounted = add(
        mul(plan.full_updates, plan.targets_per_full_update)?,
        plan.final_update_targets,
    )?;
    if accounted != targets {
        return Err(ProductError::internal(
            "P19_ACCOUNTING_INVALID",
            "scale-up update accounting failed the zero-overshoot identity",
        ));
    }
    Ok(plan)
}

fn evaluation_plan(tokens: &CandidateTokenPlanV1) -> Result<CandidateEvaluationPlanV1> {
    let targets = tokens.valid_training_targets;
    let consumed_at = |one_based_update: u64| -> u64 {
        if one_based_update >= tokens.total_updates {
            targets
        } else {
            one_based_update * tokens.targets_per_full_update
        }
    };
    let mut positions = BTreeSet::from([0_u64, targets]);
    let mut threshold = EVENT_INTERVAL_TARGETS;
    while threshold <= targets {
        let update = threshold
            .div_ceil(tokens.targets_per_full_update)
            .min(tokens.total_updates);
        positions.insert(consumed_at(update));
        threshold = add(threshold, EVENT_INTERVAL_TARGETS)?;
    }
    let quarter = targets / 4;
    Ok(CandidateEvaluationPlanV1 {
        event_interval_targets: EVENT_INTERVAL_TARGETS,
        evaluation_event_count: positions.len() as u64,
        includes_initial_baseline: true,
        includes_completion: true,
        retention_anchor_targets: [quarter, quarter * 2, quarter * 3, targets],
    })
}

fn sla_plan(completion_sla_seconds: u64) -> Result<CandidateSlaPlanV1> {
    if completion_sla_seconds == 0 || !completion_sla_seconds.is_multiple_of(10) {
        return Err(ProductError::usage(
            "P19_SLA_INVALID",
            "the completion SLA must be a nonzero multiple of ten seconds so the fixed \
             90 percent admission ceiling stays exact",
        ));
    }
    Ok(CandidateSlaPlanV1 {
        admission_seconds: completion_sla_seconds / 10 * 9,
        completion_sla_seconds,
        actual_elapsed_limit_ns: completion_sla_seconds
            .checked_mul(1_000_000_000)
            .ok_or_else(|| {
                ProductError::usage(
                    "P19_SLA_INVALID",
                    "the completion SLA cannot be represented in nanoseconds",
                )
            })?,
        admission_fraction: "9/10",
    })
}

fn increased_axes(
    parameters: u64,
    targets: u64,
    completion_sla_seconds: u64,
) -> Result<Vec<&'static str>> {
    if parameters < CANONICAL_PARAMETER_COUNT
        || targets < VALID_TRAINING_TARGETS
        || completion_sla_seconds < COMPLETION_SLA_SECONDS
    {
        return Err(ProductError::usage(
            "P19_SCOPE_DECREASED",
            "a scale-up amendment may not shrink the canonical model, target count, or \
             time budget",
        ));
    }
    let mut axes = Vec::new();
    if parameters > CANONICAL_PARAMETER_COUNT {
        axes.push("model_parameters");
    }
    if targets > VALID_TRAINING_TARGETS {
        axes.push("valid_training_targets");
    }
    if completion_sla_seconds > COMPLETION_SLA_SECONDS {
        axes.push("completion_sla_seconds");
    }
    if axes.is_empty() {
        return Err(ProductError::usage(
            "P19_SCOPE_NOT_INCREASED",
            "the requested scope does not increase model size, target count, or time budget",
        ));
    }
    Ok(axes)
}

fn required_rerun_phases(
    scope: &RequestedModelScopeV1,
    targets: u64,
    axes: &[&'static str],
) -> Vec<&'static str> {
    let earliest = if scope.vocabulary != CANONICAL_SCOPE.vocabulary {
        "P7"
    } else if scope.context_length != CANONICAL_SCOPE.context_length
        || targets != VALID_TRAINING_TARGETS
    {
        "P8"
    } else if scope != &CANONICAL_SCOPE || axes.contains(&"model_parameters") {
        "P9B"
    } else {
        "P14"
    };
    let start = REBUILD_CHAIN
        .iter()
        .position(|phase| *phase == earliest)
        .expect("the rebuild chain names every earliest phase");
    REBUILD_CHAIN[start..].to_vec()
}

fn scope_digest(config: &ScaleUpConfigV1) -> Result<(String, String)> {
    #[derive(Serialize)]
    struct CandidateScope<'a> {
        requested_model: &'a RequestedModelScopeV1,
        requested_total_valid_targets: u64,
        requested_completion_sla_seconds: u64,
    }
    let bytes = serde_json::to_vec(&CandidateScope {
        requested_model: &config.requested_model,
        requested_total_valid_targets: config.requested_total_valid_targets,
        requested_completion_sla_seconds: config.requested_completion_sla_seconds,
    })
    .map_err(|error| {
        ProductError::internal(
            "P19_SCOPE_SERIALIZATION_FAILED",
            format!("could not serialize the requested scope: {error}"),
        )
    })?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let identity = format!("scale-up-candidate-{}", &digest[..16]);
    Ok((digest, identity))
}

pub fn compute_scale_up_plan(config: &ScaleUpConfigV1) -> Result<ScaleUpPlanResultV1> {
    if config.schema != SCALE_UP_CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "SCALE_UP_CONFIG_SCHEMA_UNSUPPORTED",
            format!("configuration schema {} is not supported", config.schema),
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            format!(
                "scale-up amendments target the prototype product; profile {} is deferred",
                config.profile
            ),
        ));
    }
    validate_scope(&config.requested_model)?;
    let model = model_plan(&config.requested_model)?;
    let memory = memory_plan(model.parameters)?;
    let tokens = token_plan(
        config.requested_total_valid_targets,
        config.requested_model.context_length,
    )?;
    let sla = sla_plan(config.requested_completion_sla_seconds)?;
    let axes = increased_axes(
        model.parameters,
        tokens.valid_training_targets,
        sla.completion_sla_seconds,
    )?;
    let evaluation = evaluation_plan(&tokens)?;
    let rerun = required_rerun_phases(
        &config.requested_model,
        tokens.valid_training_targets,
        &axes,
    );
    let (scope_sha256, candidate_identity) = scope_digest(config)?;
    Ok(ScaleUpPlanResultV1 {
        schema: SCALE_UP_RESULT_SCHEMA,
        status: "CANDIDATE_PLANNED",
        qualification_status: "SKIPPED",
        amendment_status: "UNAPPROVED_CANDIDATE",
        profile: config.profile.clone(),
        candidate_identity,
        scope_sha256,
        baseline: ScaleUpBaselineV1 {
            model_identity: CANONICAL_MODEL_ID,
            parameters: CANONICAL_PARAMETER_COUNT,
            valid_training_targets: VALID_TRAINING_TARGETS,
            completion_sla_seconds: COMPLETION_SLA_SECONDS,
        },
        requested_model: config.requested_model,
        increased_axes: axes,
        model,
        memory,
        tokens,
        schedule: CandidateSchedulePlanV1 {
            optimizer: "adamw-opt-001",
            warmup_updates: WARMUP_UPDATES,
            peak_learning_rate_f32_le_hex: hex::encode(PEAK_LEARNING_RATE.to_le_bytes()),
            minimum_learning_rate_f32_le_hex: hex::encode(MINIMUM_LEARNING_RATE.to_le_bytes()),
        },
        evaluation,
        sla,
        required_rerun_phases: rerun,
        receipts_written: false,
        limitations: vec![
            "no_owner_approval_claim",
            "no_contract_amendment_claim",
            "no_canonical_constant_change",
            "no_execution_claim",
            "no_hardware_qualification_claim",
            "no_sla_or_admission_claim",
            "no_quality_claim",
        ],
    })
}

pub fn plan_scale_up(config_path: &Path) -> Result<Value> {
    let bytes = std::fs::read(config_path).map_err(|error| {
        ProductError::environment(
            "SCALE_UP_CONFIG_READ_FAILED",
            format!(
                "could not read the scale-up configuration {}: {error}",
                config_path.display()
            ),
        )
    })?;
    let config = serde_json::from_slice::<ScaleUpConfigV1>(&bytes).map_err(|error| {
        ProductError::usage(
            "SCALE_UP_CONFIG_INVALID",
            format!("configuration is not a closed {SCALE_UP_CONFIG_SCHEMA} object: {error}"),
        )
    })?;
    let result = compute_scale_up_plan(&config)?;
    serde_json::to_value(&result).map_err(|error| {
        ProductError::internal(
            "RESULT_SERIALIZATION_FAILED",
            format!("scale-up plan serialization failed: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_config() -> ScaleUpConfigV1 {
        ScaleUpConfigV1 {
            schema: SCALE_UP_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            requested_model: CANONICAL_SCOPE,
            requested_total_valid_targets: VALID_TRAINING_TARGETS,
            requested_completion_sla_seconds: COMPLETION_SLA_SECONDS,
        }
    }

    #[test]
    fn the_engine_reproduces_every_canonical_derived_constant() {
        let mut config = canonical_config();
        config.requested_completion_sla_seconds = 36_000;
        let plan = compute_scale_up_plan(&config).unwrap();
        assert_eq!(plan.model.parameters, 135_285_504);
        assert_eq!(plan.model.per_layer_attention_parameters, 1_572_864);
        assert_eq!(plan.model.per_layer_ffn_parameters, 5_603_328);
        assert_eq!(plan.model.decoder_parameters, 86_132_736);
        assert_eq!(plan.memory.raw_training_state_floor_bytes, 2_705_710_080);
        assert_eq!(plan.memory.minimum_accelerator_bytes, 2_952_790_016);
        assert_eq!(plan.tokens.stored_prefix_ids, 2_000_000_001);
        assert_eq!(plan.tokens.full_updates, 30_517);
        assert_eq!(plan.tokens.final_update_targets, 37_888);
        assert_eq!(plan.tokens.total_updates, 30_518);
        assert_eq!(plan.tokens.complete_spans, 976_562);
        assert_eq!(plan.tokens.final_span_targets, 1_024);
        assert_eq!(plan.evaluation.evaluation_event_count, 21);
        assert_eq!(
            plan.evaluation.retention_anchor_targets,
            [500_000_000, 1_000_000_000, 1_500_000_000, 2_000_000_000]
        );
        assert_eq!(plan.increased_axes, ["completion_sla_seconds"]);
        assert_eq!(plan.sla.admission_seconds, 32_400);
        assert_eq!(plan.required_rerun_phases, ["P14", "P15", "P16", "P16A"]);
    }

    #[test]
    fn unchanged_and_shrunken_scopes_are_typed_rejections() {
        assert_eq!(
            compute_scale_up_plan(&canonical_config()).unwrap_err().code,
            "P19_SCOPE_NOT_INCREASED"
        );
        let mut smaller = canonical_config();
        smaller.requested_model.layers = 8;
        assert_eq!(
            compute_scale_up_plan(&smaller).unwrap_err().code,
            "P19_SCOPE_DECREASED"
        );
    }
}
