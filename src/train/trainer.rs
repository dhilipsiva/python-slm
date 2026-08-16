//! Canonical target accounting, optimizer scheduling, evaluation, and resumable trainer state.

use super::LoadedSpan;
use crate::error::{ProductError, Result};
use crate::model::CANONICAL_MODEL_ID;
use crate::model::oracle::{adamw_scalar_step, f32_to_bf16_bits, gradient_clip_scale};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const TRAINER_SNAPSHOT_SCHEMA: &str = "python-slm-trainer-snapshot-v1";
pub const TOTAL_TARGETS: u64 = 2_000_000_000;
pub const TARGETS_PER_FULL_UPDATE: u64 = 65_536;
pub const FULL_UPDATES: u64 = 30_517;
pub const FINAL_UPDATE_TARGETS: u64 = 37_888;
pub const TOTAL_UPDATES: u64 = 30_518;
pub const WARMUP_UPDATES: u64 = 1_000;
pub const EVALUATION_TARGETS: u64 = 1_000_000;
pub const EVENT_INTERVAL_TARGETS: u64 = 100_000_000;
pub const RETENTION_ANCHORS: [u64; 4] = [500_000_000, 1_000_000_000, 1_500_000_000, TOTAL_TARGETS];
pub const PEAK_LEARNING_RATE: f32 = 0.0025;
pub const MINIMUM_LEARNING_RATE: f32 = 0.00025;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTrainingPlan {
    pub total_targets: u64,
    pub targets_per_full_update: u64,
    pub full_updates: u64,
    pub final_update_targets: u64,
    pub total_updates: u64,
    pub warmup_updates: u64,
    pub evaluation_targets: u64,
    pub event_interval_targets: u64,
    pub retention_anchors: Vec<u64>,
    pub optimizer: String,
    pub beta1_f32_le_hex: String,
    pub beta2_f32_le_hex: String,
    pub epsilon_f32_le_hex: String,
    pub decay_f32_le_hex: String,
    pub global_l2_clip_f32_le_hex: String,
    pub peak_learning_rate_f32_le_hex: String,
    pub minimum_learning_rate_f32_le_hex: String,
}

impl CanonicalTrainingPlan {
    pub fn canonical() -> Self {
        Self {
            total_targets: TOTAL_TARGETS,
            targets_per_full_update: TARGETS_PER_FULL_UPDATE,
            full_updates: FULL_UPDATES,
            final_update_targets: FINAL_UPDATE_TARGETS,
            total_updates: TOTAL_UPDATES,
            warmup_updates: WARMUP_UPDATES,
            evaluation_targets: EVALUATION_TARGETS,
            event_interval_targets: EVENT_INTERVAL_TARGETS,
            retention_anchors: RETENTION_ANCHORS.to_vec(),
            optimizer: "adamw-opt-001".to_owned(),
            beta1_f32_le_hex: f32_hex(0.9),
            beta2_f32_le_hex: f32_hex(0.95),
            epsilon_f32_le_hex: f32_hex(1.0e-8),
            decay_f32_le_hex: f32_hex(0.1),
            global_l2_clip_f32_le_hex: f32_hex(1.0),
            peak_learning_rate_f32_le_hex: f32_hex(PEAK_LEARNING_RATE),
            minimum_learning_rate_f32_le_hex: f32_hex(MINIMUM_LEARNING_RATE),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self != &Self::canonical()
            || self
                .full_updates
                .checked_mul(self.targets_per_full_update)
                .and_then(|targets| targets.checked_add(self.final_update_targets))
                != Some(self.total_targets)
            || self.full_updates.checked_add(1) != Some(self.total_updates)
        {
            return Err(ProductError::integrity(
                "P12_TRAINING_PLAN_MISMATCH",
                "the trainer plan differs from the frozen optimizer, schedule, or target arithmetic",
            ));
        }
        Ok(())
    }
}

fn f32_hex(value: f32) -> String {
    hex::encode(value.to_le_bytes())
}

pub fn canonical_update_target_count(one_based_update: u64) -> Result<u64> {
    match one_based_update {
        1..=FULL_UPDATES => Ok(TARGETS_PER_FULL_UPDATE),
        TOTAL_UPDATES => Ok(FINAL_UPDATE_TARGETS),
        _ => Err(ProductError::integrity(
            "P12_UPDATE_INDEX_INVALID",
            "the optimizer update index is outside the canonical 30,518-update run",
        )),
    }
}

pub fn canonical_learning_rate(one_based_update: u64) -> Result<f32> {
    if !(1..=TOTAL_UPDATES).contains(&one_based_update) {
        return Err(ProductError::integrity(
            "P12_SCHEDULE_INDEX_INVALID",
            "the scheduler index is outside the canonical optimizer-update range",
        ));
    }
    if one_based_update <= WARMUP_UPDATES {
        return Ok(PEAK_LEARNING_RATE * one_based_update as f32 / WARMUP_UPDATES as f32);
    }
    let progress =
        (one_based_update - WARMUP_UPDATES) as f64 / (TOTAL_UPDATES - WARMUP_UPDATES) as f64;
    let learning_rate = f64::from(MINIMUM_LEARNING_RATE)
        + 0.5
            * f64::from(PEAK_LEARNING_RATE - MINIMUM_LEARNING_RATE)
            * (1.0 + (std::f64::consts::PI * progress).cos());
    Ok(learning_rate as f32)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdamwParameterState {
    pub name: String,
    pub weight_decay: bool,
    pub master_weights: Vec<f32>,
    pub parameter_bf16: Vec<u16>,
    pub first_moments: Vec<f32>,
    pub second_moments: Vec<f32>,
}

impl AdamwParameterState {
    fn validate(&self) -> Result<()> {
        let elements = self.master_weights.len();
        if self.name.is_empty()
            || elements == 0
            || self.parameter_bf16.len() != elements
            || self.first_moments.len() != elements
            || self.second_moments.len() != elements
            || self
                .master_weights
                .iter()
                .chain(&self.first_moments)
                .chain(&self.second_moments)
                .any(|value| !value.is_finite())
            || self
                .master_weights
                .iter()
                .zip(&self.parameter_bf16)
                .any(|(master, storage)| f32_to_bf16_bits(*master) != *storage)
        {
            return Err(ProductError::integrity(
                "P12_OPTIMIZER_STATE_INVALID",
                "an AdamW parameter has empty, mismatched, or nonfinite state",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalAdamw {
    parameters: Vec<AdamwParameterState>,
}

impl CanonicalAdamw {
    pub fn new(parameters: Vec<AdamwParameterState>) -> Result<Self> {
        let mut names = BTreeSet::new();
        for parameter in &parameters {
            parameter.validate()?;
            if !names.insert(parameter.name.as_str()) {
                return Err(ProductError::integrity(
                    "P12_OPTIMIZER_PARAMETER_DUPLICATE",
                    "the AdamW state contains a duplicate stable parameter name",
                ));
            }
        }
        if parameters.is_empty() {
            return Err(ProductError::integrity(
                "P12_OPTIMIZER_STATE_INVALID",
                "the AdamW state contains no parameters",
            ));
        }
        Ok(Self { parameters })
    }

    pub fn parameters(&self) -> &[AdamwParameterState] {
        &self.parameters
    }

    pub fn apply_normalized_clipped_gradients(
        &mut self,
        gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
    ) -> Result<String> {
        let expected = self
            .parameters
            .iter()
            .try_fold(0_usize, |total, parameter| {
                total
                    .checked_add(parameter.master_weights.len())
                    .ok_or_else(accounting_overflow)
            })?;
        if gradients.len() != expected || gradient_clip_scale(gradients)? < 1.0 {
            return Err(ProductError::integrity(
                "P12_OPTIMIZER_GRADIENT_INVALID",
                "AdamW requires one finite, globally clipped FP32 gradient per parameter",
            ));
        }
        let optimizer_update = u32::try_from(one_based_update).map_err(|_| {
            ProductError::integrity(
                "P12_UPDATE_INDEX_INVALID",
                "the optimizer update index cannot be represented canonically",
            )
        })?;
        let expected_learning_rate = canonical_learning_rate(one_based_update)?;
        if learning_rate.to_bits() != expected_learning_rate.to_bits() {
            return Err(ProductError::integrity(
                "P12_SCHEDULER_STATE_MISMATCH",
                "AdamW received a learning rate that differs from the frozen schedule",
            ));
        }
        let mut offset = 0;
        for parameter in &mut self.parameters {
            let decay = if parameter.weight_decay { 0.1 } else { 0.0 };
            for index in 0..parameter.master_weights.len() {
                let (master, first, second, storage) = adamw_scalar_step(
                    parameter.master_weights[index],
                    gradients[offset],
                    parameter.first_moments[index],
                    parameter.second_moments[index],
                    learning_rate,
                    optimizer_update,
                    decay,
                )?;
                parameter.master_weights[index] = master;
                parameter.first_moments[index] = first;
                parameter.second_moments[index] = second;
                parameter.parameter_bf16[index] = storage;
                offset += 1;
            }
        }
        self.state_sha256()
    }

    pub fn state_sha256(&self) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/adamw-state/v1\0");
        for parameter in &self.parameters {
            parameter.validate()?;
            hasher.update((parameter.name.len() as u64).to_le_bytes());
            hasher.update(parameter.name.as_bytes());
            hasher.update([u8::from(parameter.weight_decay)]);
            hasher.update((parameter.master_weights.len() as u64).to_le_bytes());
            for value in &parameter.master_weights {
                hasher.update(value.to_le_bytes());
            }
            for value in &parameter.parameter_bf16 {
                hasher.update(value.to_le_bytes());
            }
            for value in &parameter.first_moments {
                hasher.update(value.to_le_bytes());
            }
            for value in &parameter.second_moments {
                hasher.update(value.to_le_bytes());
            }
        }
        Ok(hex::encode(hasher.finalize()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerIdentity {
    pub profile: String,
    pub model_identity: String,
    pub model_parameter_layout_sha256: String,
    pub backend_identity_sha256: String,
    pub device_identity_sha256: String,
    pub corpus_manifest_sha256: String,
    pub training_span_manifest_sha256: String,
    pub validation_span_manifest_sha256: String,
    pub tokenizer_artifact_sha256: String,
    pub environment_identity_sha256: String,
    pub implementation_artifact_sha256: String,
}

impl TrainerIdentity {
    pub fn validate(&self) -> Result<()> {
        if !crate::backend::tuples::is_implemented_training_profile(&self.profile)
            || self.model_identity != CANONICAL_MODEL_ID
        {
            return Err(ProductError::integrity(
                "P12_IDENTITY_MISMATCH",
                "the trainer profile is not an implemented tuple or the model is not canonical",
            ));
        }
        for digest in [
            &self.model_parameter_layout_sha256,
            &self.backend_identity_sha256,
            &self.device_identity_sha256,
            &self.corpus_manifest_sha256,
            &self.training_span_manifest_sha256,
            &self.validation_span_manifest_sha256,
            &self.tokenizer_artifact_sha256,
            &self.environment_identity_sha256,
            &self.implementation_artifact_sha256,
        ] {
            if !is_sha256(digest) {
                return Err(ProductError::integrity(
                    "P12_IDENTITY_HASH_INVALID",
                    "a trainer identity is not lowercase SHA-256",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendStateArtifact {
    pub role: String,
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchGradient {
    pub loss_sum: f64,
    pub gradient_sums: Vec<f32>,
    pub host_rng_state: Vec<u8>,
    pub device_rng_state: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingBatch {
    pub first_target: u64,
    pub valid_targets: u64,
    pub input_ids: Vec<u16>,
    pub target_ids: Vec<u16>,
}

impl TrainingBatch {
    pub fn from_loaded_span(span: &LoadedSpan) -> Self {
        Self {
            first_target: span.first_id,
            valid_targets: span.valid_targets,
            input_ids: span.input_ids().to_vec(),
            target_ids: span.target_ids().to_vec(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.valid_targets == 0
            || self.input_ids.len() as u64 != self.valid_targets
            || self.target_ids.len() as u64 != self.valid_targets
        {
            return Err(ProductError::integrity(
                "P12_BATCH_SHAPE_INVALID",
                "a training batch does not contain one input and target per valid target",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationResult {
    pub evaluated_targets: u64,
    pub aggregate_loss: f64,
    pub result_sha256: String,
}

impl EvaluationResult {
    fn validate(&self) -> Result<()> {
        if self.evaluated_targets != EVALUATION_TARGETS
            || !self.aggregate_loss.is_finite()
            || !is_sha256(&self.result_sha256)
        {
            return Err(ProductError::gate(
                "P12_EVALUATION_INVALID",
                "the backend returned incomplete, nonfinite, or unbound evaluation facts",
            ));
        }
        Ok(())
    }
}

pub trait TrainerBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient>;
    fn apply_update(
        &mut self,
        normalized_clipped_gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
        valid_targets: u64,
    ) -> Result<String>;
    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult>;
    fn snapshot(&self) -> Result<Vec<BackendStateArtifact>>;
    fn restore(&mut self, artifacts: &[BackendStateArtifact]) -> Result<()>;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedEvaluation {
    pub after_targets: u64,
    pub before_first_update: bool,
    pub result: EvaluationResult,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerSnapshot {
    pub schema: String,
    pub identity: TrainerIdentity,
    pub plan: CanonicalTrainingPlan,
    pub consumed_targets: u64,
    pub completed_updates: u64,
    pub accumulated_targets: u64,
    pub next_training_first_target: u64,
    pub scheduler_one_based_update: u64,
    pub last_learning_rate_f32_le_hex: Option<String>,
    pub last_update_state_sha256: Option<String>,
    pub host_rng_state: Vec<u8>,
    pub device_rng_state: Vec<u8>,
    pub evaluations: Vec<CompletedEvaluation>,
    pub backend_state: Vec<BackendStateArtifact>,
}

impl TrainerSnapshot {
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        self.plan.validate()?;
        if self.schema != TRAINER_SNAPSHOT_SCHEMA
            || self.accumulated_targets != 0
            || self.consumed_targets != self.next_training_first_target
            || self.completed_updates > TOTAL_UPDATES
            || self.scheduler_one_based_update != self.completed_updates
            || expected_consumed_targets(self.completed_updates)? != self.consumed_targets
            || (self.completed_updates == 0) != self.last_learning_rate_f32_le_hex.is_none()
            || (self.completed_updates == 0) != self.last_update_state_sha256.is_none()
            || self
                .last_update_state_sha256
                .as_ref()
                .is_some_and(|value| !is_sha256(value))
            || self.host_rng_state.is_empty()
            || self.device_rng_state.is_empty()
        {
            return Err(ProductError::integrity(
                "P12_SNAPSHOT_STATE_INVALID",
                "the trainer snapshot is not a complete post-update deterministic boundary",
            ));
        }
        if let Some(encoded) = &self.last_learning_rate_f32_le_hex
            && encoded != &f32_hex(canonical_learning_rate(self.completed_updates)?)
        {
            return Err(ProductError::integrity(
                "P12_SCHEDULER_STATE_MISMATCH",
                "the stored scheduler state differs from the canonical update schedule",
            ));
        }
        validate_evaluations(&self.evaluations, self.consumed_targets)?;
        if canonical_backend_state(self.backend_state.clone())? != self.backend_state {
            return Err(ProductError::integrity(
                "P12_BACKEND_STATE_ORDER_INVALID",
                "backend checkpoint artifacts are not in canonical role/path order",
            ));
        }
        Ok(())
    }
}

fn expected_consumed_targets(completed_updates: u64) -> Result<u64> {
    match completed_updates {
        0..=FULL_UPDATES => completed_updates
            .checked_mul(TARGETS_PER_FULL_UPDATE)
            .ok_or_else(accounting_overflow),
        TOTAL_UPDATES => Ok(TOTAL_TARGETS),
        _ => Err(ProductError::integrity(
            "P12_UPDATE_INDEX_INVALID",
            "the completed update count exceeds the canonical run",
        )),
    }
}

pub(crate) fn validate_backend_state(artifacts: &[BackendStateArtifact]) -> Result<()> {
    let required = [
        "parameters_bf16",
        "master_parameters_fp32",
        "adamw_first_moments_fp32",
        "adamw_second_moments_fp32",
        "backend_runtime_state",
    ];
    let mut roles = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        if artifact.role.is_empty()
            || !roles.insert(artifact.role.as_str())
            || artifact.bytes.is_empty()
            || !portable_relative_path(&artifact.relative_path)
            || !paths.insert(artifact.relative_path.as_str())
        {
            return Err(ProductError::integrity(
                "P12_BACKEND_STATE_INVALID",
                "backend checkpoint artifacts are empty, duplicated, or not portable relative paths",
            ));
        }
    }
    if required.iter().any(|role| !roles.contains(role)) {
        return Err(ProductError::integrity(
            "P12_BACKEND_STATE_INCOMPLETE",
            "the checkpoint omits model, master-weight, moment, or runtime state",
        ));
    }
    Ok(())
}

pub(crate) fn canonical_backend_state(
    mut artifacts: Vec<BackendStateArtifact>,
) -> Result<Vec<BackendStateArtifact>> {
    validate_backend_state(&artifacts)?;
    artifacts.sort_by(|left, right| {
        left.role
            .cmp(&right.role)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    Ok(artifacts)
}

pub(crate) fn portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.starts_with('/')
        && path.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.contains(':')
        })
}

fn validate_evaluations(evaluations: &[CompletedEvaluation], consumed_targets: u64) -> Result<()> {
    let mut expected_targets = vec![0_u64];
    let mut threshold = EVENT_INTERVAL_TARGETS;
    while threshold <= TOTAL_TARGETS {
        let update = threshold
            .div_ceil(TARGETS_PER_FULL_UPDATE)
            .min(TOTAL_UPDATES);
        let position = expected_consumed_targets(update)?;
        if position <= consumed_targets && expected_targets.last().copied() != Some(position) {
            expected_targets.push(position);
        }
        threshold = threshold
            .checked_add(EVENT_INTERVAL_TARGETS)
            .ok_or_else(accounting_overflow)?;
    }
    if evaluations.len() != expected_targets.len() {
        return Err(ProductError::integrity(
            "P12_EVALUATION_SET_INCOMPLETE",
            "the trainer state does not contain every mandatory evaluation boundary",
        ));
    }
    for (index, (evaluation, expected)) in evaluations.iter().zip(expected_targets).enumerate() {
        evaluation.result.validate()?;
        if evaluation.after_targets != expected || evaluation.before_first_update != (index == 0) {
            return Err(ProductError::integrity(
                "P12_EVALUATION_ORDER_INVALID",
                "an evaluation record is not at its exact required post-update boundary",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateEvent {
    pub one_based_update: u64,
    pub consumed_targets: u64,
    pub valid_targets: u64,
    pub normalized_loss_f64_le_hex: String,
    pub learning_rate_f32_le_hex: String,
    pub update_state_sha256: String,
    pub evaluation: Option<EvaluationResult>,
    pub checkpoint_due: bool,
    pub training_complete: bool,
}

pub struct DeterministicTrainer<B: TrainerBackend> {
    identity: TrainerIdentity,
    backend: B,
    consumed_targets: u64,
    completed_updates: u64,
    accumulated_targets: u64,
    accumulated_loss: f64,
    accumulated_gradients: Vec<f32>,
    last_learning_rate: Option<f32>,
    last_update_state_sha256: Option<String>,
    host_rng_state: Vec<u8>,
    device_rng_state: Vec<u8>,
    evaluations: Vec<CompletedEvaluation>,
}

impl<B: TrainerBackend> DeterministicTrainer<B> {
    pub fn new(
        identity: TrainerIdentity,
        mut backend: B,
        host_rng_state: Vec<u8>,
        device_rng_state: Vec<u8>,
    ) -> Result<Self> {
        identity.validate()?;
        if host_rng_state.is_empty() || device_rng_state.is_empty() {
            return Err(ProductError::integrity(
                "P12_RNG_STATE_MISSING",
                "host and device RNG state must be explicit before training",
            ));
        }
        let before = canonical_backend_state(backend.snapshot()?)?;
        validate_backend_state(&before)?;
        let evaluation = backend.evaluate(&identity.validation_span_manifest_sha256)?;
        evaluation.validate()?;
        if before != canonical_backend_state(backend.snapshot()?)? {
            return Err(ProductError::gate(
                "P12_EVALUATION_MUTATED_STATE",
                "evaluation changed backend training state before the first update",
            ));
        }
        Ok(Self {
            identity,
            backend,
            consumed_targets: 0,
            completed_updates: 0,
            accumulated_targets: 0,
            accumulated_loss: 0.0,
            accumulated_gradients: Vec::new(),
            last_learning_rate: None,
            last_update_state_sha256: None,
            host_rng_state,
            device_rng_state,
            evaluations: vec![CompletedEvaluation {
                after_targets: 0,
                before_first_update: true,
                result: evaluation,
            }],
        })
    }

    pub fn from_snapshot(
        snapshot: TrainerSnapshot,
        expected_identity: &TrainerIdentity,
        mut backend: B,
    ) -> Result<Self> {
        snapshot.validate()?;
        expected_identity.validate()?;
        if &snapshot.identity != expected_identity {
            return Err(ProductError::integrity(
                "P12_RESUME_IDENTITY_MISMATCH",
                "checkpoint identities differ from the requested training run",
            ));
        }
        backend.restore(&snapshot.backend_state)?;
        if canonical_backend_state(backend.snapshot()?)? != snapshot.backend_state {
            return Err(ProductError::integrity(
                "P12_RESUME_STATE_MISMATCH",
                "the backend did not restore the checkpoint bytes exactly",
            ));
        }
        Ok(Self {
            identity: snapshot.identity,
            backend,
            consumed_targets: snapshot.consumed_targets,
            completed_updates: snapshot.completed_updates,
            accumulated_targets: 0,
            accumulated_loss: 0.0,
            accumulated_gradients: Vec::new(),
            last_learning_rate: snapshot
                .last_learning_rate_f32_le_hex
                .as_deref()
                .map(parse_f32_hex)
                .transpose()?,
            last_update_state_sha256: snapshot.last_update_state_sha256,
            host_rng_state: snapshot.host_rng_state,
            device_rng_state: snapshot.device_rng_state,
            evaluations: snapshot.evaluations,
        })
    }

    pub fn process_batch(&mut self, batch: &TrainingBatch) -> Result<Option<UpdateEvent>> {
        batch.validate()?;
        if self.consumed_targets == TOTAL_TARGETS {
            return Err(ProductError::gate(
                "P12_TARGET_OVERSHOOT",
                "training received another batch after exactly two billion targets",
            ));
        }
        let first = self
            .consumed_targets
            .checked_add(self.accumulated_targets)
            .ok_or_else(accounting_overflow)?;
        if batch.first_target != first {
            return Err(ProductError::integrity(
                "P12_TRAINING_CURSOR_MISMATCH",
                "the batch does not begin at the exact deterministic training cursor",
            ));
        }
        let one_based_update = self.completed_updates + 1;
        let required = canonical_update_target_count(one_based_update)?;
        if self
            .accumulated_targets
            .checked_add(batch.valid_targets)
            .ok_or_else(accounting_overflow)?
            > required
        {
            return Err(ProductError::gate(
                "P12_UPDATE_TARGET_OVERSHOOT",
                "a batch would cross the exact optimizer-update target boundary",
            ));
        }
        let contribution = self.backend.accumulate(batch)?;
        if !contribution.loss_sum.is_finite()
            || contribution.gradient_sums.is_empty()
            || contribution
                .gradient_sums
                .iter()
                .any(|value| !value.is_finite())
            || contribution.host_rng_state.is_empty()
            || contribution.device_rng_state.is_empty()
        {
            return Err(ProductError::gate(
                "P12_GRADIENT_NONFINITE",
                "a batch returned empty or nonfinite loss or gradients",
            ));
        }
        self.host_rng_state = contribution.host_rng_state.clone();
        self.device_rng_state = contribution.device_rng_state.clone();
        if self.accumulated_gradients.is_empty() {
            self.accumulated_gradients = vec![0.0; contribution.gradient_sums.len()];
        }
        if self.accumulated_gradients.len() != contribution.gradient_sums.len() {
            return Err(ProductError::integrity(
                "P12_GRADIENT_LAYOUT_MISMATCH",
                "gradient layout changed within an optimizer update",
            ));
        }
        for (total, value) in self
            .accumulated_gradients
            .iter_mut()
            .zip(contribution.gradient_sums)
        {
            *total += value;
            if !total.is_finite() {
                return Err(ProductError::gate(
                    "P12_GRADIENT_NONFINITE",
                    "gradient accumulation became nonfinite",
                ));
            }
        }
        self.accumulated_loss += contribution.loss_sum;
        self.accumulated_targets += batch.valid_targets;
        if self.accumulated_targets != required {
            return Ok(None);
        }

        let divisor = required as f32;
        let normalized_loss = self.accumulated_loss / required as f64;
        if !normalized_loss.is_finite() {
            return Err(ProductError::gate(
                "P12_LOSS_NONFINITE",
                "valid-target-normalized training loss became nonfinite",
            ));
        }
        let mut normalized = self
            .accumulated_gradients
            .iter()
            .map(|value| *value / divisor)
            .collect::<Vec<_>>();
        let clip = gradient_clip_scale(&normalized)?;
        for value in &mut normalized {
            *value *= clip;
        }
        let learning_rate = canonical_learning_rate(one_based_update)?;
        let update_state_sha256 =
            self.backend
                .apply_update(&normalized, learning_rate, one_based_update, required)?;
        if !is_sha256(&update_state_sha256) {
            return Err(ProductError::gate(
                "P12_UPDATE_STATE_INVALID",
                "the backend update did not return a lowercase SHA-256 state identity",
            ));
        }
        let previous_targets = self.consumed_targets;
        self.consumed_targets = self
            .consumed_targets
            .checked_add(required)
            .ok_or_else(accounting_overflow)?;
        self.completed_updates = one_based_update;
        self.accumulated_targets = 0;
        self.accumulated_loss = 0.0;
        self.accumulated_gradients.clear();
        self.last_learning_rate = Some(learning_rate);
        self.last_update_state_sha256 = Some(update_state_sha256.clone());

        let evaluation_due = event_boundary_crossed(previous_targets, self.consumed_targets);
        let evaluation = if evaluation_due {
            let before = canonical_backend_state(self.backend.snapshot()?)?;
            let result = self
                .backend
                .evaluate(&self.identity.validation_span_manifest_sha256)?;
            result.validate()?;
            if before != canonical_backend_state(self.backend.snapshot()?)? {
                return Err(ProductError::gate(
                    "P12_EVALUATION_MUTATED_STATE",
                    "evaluation changed optimizer, RNG, or model state",
                ));
            }
            self.evaluations.push(CompletedEvaluation {
                after_targets: self.consumed_targets,
                before_first_update: false,
                result: result.clone(),
            });
            Some(result)
        } else {
            None
        };
        Ok(Some(UpdateEvent {
            one_based_update,
            consumed_targets: self.consumed_targets,
            valid_targets: required,
            normalized_loss_f64_le_hex: hex::encode(normalized_loss.to_le_bytes()),
            learning_rate_f32_le_hex: f32_hex(learning_rate),
            update_state_sha256,
            evaluation,
            checkpoint_due: evaluation_due,
            training_complete: self.consumed_targets == TOTAL_TARGETS,
        }))
    }

    pub fn snapshot(&self) -> Result<TrainerSnapshot> {
        if self.accumulated_targets != 0 {
            return Err(ProductError::gate(
                "P12_MID_UPDATE_CHECKPOINT_REJECTED",
                "checkpoints are permitted only after a complete optimizer update",
            ));
        }
        let snapshot = TrainerSnapshot {
            schema: TRAINER_SNAPSHOT_SCHEMA.to_owned(),
            identity: self.identity.clone(),
            plan: CanonicalTrainingPlan::canonical(),
            consumed_targets: self.consumed_targets,
            completed_updates: self.completed_updates,
            accumulated_targets: 0,
            next_training_first_target: self.consumed_targets,
            scheduler_one_based_update: self.completed_updates,
            last_learning_rate_f32_le_hex: self.last_learning_rate.map(f32_hex),
            last_update_state_sha256: self.last_update_state_sha256.clone(),
            host_rng_state: self.host_rng_state.clone(),
            device_rng_state: self.device_rng_state.clone(),
            evaluations: self.evaluations.clone(),
            backend_state: canonical_backend_state(self.backend.snapshot()?)?,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn consumed_targets(&self) -> u64 {
        self.consumed_targets + self.accumulated_targets
    }

    pub fn completed_updates(&self) -> u64 {
        self.completed_updates
    }
}

fn parse_f32_hex(value: &str) -> Result<f32> {
    let bytes = hex::decode(value).map_err(|_| {
        ProductError::integrity(
            "P12_FLOAT_ENCODING_INVALID",
            "a checkpoint float is not hexadecimal",
        )
    })?;
    let array: [u8; 4] = bytes.try_into().map_err(|_| {
        ProductError::integrity(
            "P12_FLOAT_ENCODING_INVALID",
            "a checkpoint float is not four bytes",
        )
    })?;
    Ok(f32::from_le_bytes(array))
}

fn event_boundary_crossed(previous: u64, current: u64) -> bool {
    current == TOTAL_TARGETS || previous / EVENT_INTERVAL_TARGETS < current / EVENT_INTERVAL_TARGETS
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P12_TARGET_ACCOUNTING_OVERFLOW",
        "trainer target accounting overflowed",
    )
}

pub fn state_bundle_sha256(snapshot: &TrainerSnapshot) -> Result<String> {
    snapshot.validate()?;
    let bytes = serde_json::to_vec(snapshot).map_err(|_| {
        ProductError::internal(
            "P12_SNAPSHOT_SERIALIZE_FAILED",
            "could not serialize trainer state",
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
