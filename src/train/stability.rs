//! Bounded deterministic stability trials over the P12 trainer boundary.

use super::checkpoint::{load_checkpoint, publish_checkpoint};
use super::profile::PrototypeTrainingDefaultsV1;
use super::trainer::{
    BackendStateArtifact, BatchGradient, DeterministicTrainer, EVALUATION_TARGETS,
    EvaluationResult, TrainerBackend, TrainerIdentity, TrainerSnapshot, TrainingBatch,
    state_bundle_sha256,
};
use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use crate::model::CANONICAL_MODEL_ID;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const IMPLEMENTATION_PHASE: &str = "P15";
pub const PLAN_SCHEMA: &str = "python-slm-stability-ladder-plan-v1";
pub const RESULT_SCHEMA: &str = "python-slm-stability-ladder-result-v1";
pub const EXECUTION_SURFACE: &str = "provider-neutral-synthetic";
pub const STABILITY_PLAN_BYTES: &[u8] = include_bytes!("prototype-windows-5090-v1.stability.json");

const SMOKE_UPDATES: u64 = 1;
const SHORT_RUN_UPDATES: u64 = 4;
const RESTART_CHECKPOINT_AFTER_UPDATES: u64 = 2;
const RESTART_TOTAL_UPDATES: u64 = 4;
const STABILITY_UPDATES: u64 = 32;
const STABILITY_REPETITIONS: u64 = 3;
const TEMP_PREFIX: &str = "python-slm-p15-";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StabilityPlanV1 {
    pub schema: String,
    pub profile: String,
    pub configuration_sha256: String,
    pub execution_surface: String,
    pub smoke_updates: u64,
    pub short_run_updates: u64,
    pub restart_checkpoint_after_updates: u64,
    pub restart_total_updates: u64,
    pub stability_updates: u64,
    pub stability_repetitions: u64,
}

impl StabilityPlanV1 {
    pub fn canonical(configuration_sha256: &str) -> Self {
        Self {
            schema: PLAN_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            configuration_sha256: configuration_sha256.to_owned(),
            execution_surface: EXECUTION_SURFACE.to_owned(),
            smoke_updates: SMOKE_UPDATES,
            short_run_updates: SHORT_RUN_UPDATES,
            restart_checkpoint_after_updates: RESTART_CHECKPOINT_AFTER_UPDATES,
            restart_total_updates: RESTART_TOTAL_UPDATES,
            stability_updates: STABILITY_UPDATES,
            stability_repetitions: STABILITY_REPETITIONS,
        }
    }

    pub fn validate(&self, configuration_sha256: &str) -> Result<()> {
        if self != &Self::canonical(configuration_sha256) {
            return Err(ProductError::integrity(
                "P15_PLAN_MISMATCH",
                "the stability plan differs from the bounded P15 trial ladder",
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self).map_err(|_| {
            ProductError::internal(
                "P15_PLAN_SERIALIZE_FAILED",
                "could not serialize the closed stability plan",
            )
        })?;
        Ok(sha256(&bytes))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StabilityTrialV1 {
    pub trial_id: String,
    pub trial_kind: String,
    pub repetition: u64,
    pub status: String,
    pub planned_updates: u64,
    pub completed_updates: u64,
    pub completed_targets: u64,
    pub final_state_sha256: String,
    pub configuration_sha256_before: String,
    pub configuration_sha256_after: String,
    pub implementation_sha256_before: String,
    pub implementation_sha256_after: String,
    pub configuration_frozen: bool,
    pub implementation_frozen: bool,
    pub checkpoint_roundtrip_exact: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StabilityLadderResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub hardware_status: String,
    pub long_duration_status: String,
    pub performance_admission_status: String,
    pub completion_sla_status: String,
    pub profile: String,
    pub execution_surface: String,
    pub configuration_sha256: String,
    pub implementation_sha256: String,
    pub plan_sha256: String,
    pub plan: StabilityPlanV1,
    pub trials: Vec<StabilityTrialV1>,
    pub restart_equivalence: bool,
    pub repeated_stability_equivalence: bool,
    pub owned_temporary_root_removed: bool,
    pub limitations: Vec<String>,
}

pub fn parse_stability_plan(bytes: &[u8], configuration_sha256: &str) -> Result<StabilityPlanV1> {
    let plan = serde_json::from_slice::<StabilityPlanV1>(bytes).map_err(|_| {
        ProductError::usage(
            "P15_PLAN_INVALID",
            "the stability input is not a closed P15 ladder plan",
        )
    })?;
    plan.validate(configuration_sha256)?;
    Ok(plan)
}

pub fn stability(config_path: &Path, plan_path: &Path) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = (config_path, plan_path);
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the prototype stability ladder is implemented only for native Windows",
        ));
    }
    #[cfg(windows)]
    {
        let config_bytes =
            super::profile::read_control_file(config_path, "P15_CONFIG_READ_FAILED")?;
        let configuration = super::profile::parse_default_configuration(&config_bytes)?;
        let configuration_sha256 = configuration.sha256()?;
        let plan_bytes = super::profile::read_control_file(plan_path, "P15_PLAN_READ_FAILED")?;
        let plan = parse_stability_plan(&plan_bytes, &configuration_sha256)?;
        serde_json::to_value(build_stability_result(configuration, plan)?).map_err(|_| {
            ProductError::internal(
                "P15_RESULT_SERIALIZE_FAILED",
                "could not serialize the closed stability result",
            )
        })
    }
}

pub fn build_stability_result(
    configuration: PrototypeTrainingDefaultsV1,
    plan: StabilityPlanV1,
) -> Result<StabilityLadderResultV1> {
    configuration.validate()?;
    let configuration_sha256 = configuration.sha256()?;
    plan.validate(&configuration_sha256)?;
    let plan_sha256 = plan.sha256()?;
    let implementation_sha256 = implementation_sha256();
    let identity = diagnostic_identity(&implementation_sha256);
    let mut temporary = OwnedTemporaryRoot::create()?;
    let mut trials = Vec::new();

    trials.push(run_fresh_trial(
        "smoke-01",
        "smoke",
        1,
        plan.smoke_updates,
        &configuration,
        &identity,
        &configuration_sha256,
        &implementation_sha256,
    )?);
    trials.push(run_fresh_trial(
        "short-run-01",
        "short-run",
        1,
        plan.short_run_updates,
        &configuration,
        &identity,
        &configuration_sha256,
        &implementation_sha256,
    )?);

    let restart = run_restart_trial(
        temporary.path(),
        &plan,
        &configuration,
        &identity,
        &configuration_sha256,
        &implementation_sha256,
    )?;
    let restart_equivalence = restart.checkpoint_roundtrip_exact == Some(true);
    trials.push(restart);

    let mut repeated_hash: Option<String> = None;
    let mut repeated_stability_equivalence = true;
    for repetition in 1..=plan.stability_repetitions {
        let trial = run_fresh_trial(
            &format!("stability-{repetition:02}"),
            "stability",
            repetition,
            plan.stability_updates,
            &configuration,
            &identity,
            &configuration_sha256,
            &implementation_sha256,
        )?;
        if let Some(expected) = &repeated_hash {
            repeated_stability_equivalence &= expected == &trial.final_state_sha256;
        } else {
            repeated_hash = Some(trial.final_state_sha256.clone());
        }
        trials.push(trial);
    }
    if !restart_equivalence || !repeated_stability_equivalence {
        return Err(ProductError::gate(
            "P15_STABILITY_MISMATCH",
            "restart or repeated bounded trials produced different deterministic state",
        ));
    }
    if trials.iter().any(|trial| {
        !trial.configuration_frozen || !trial.implementation_frozen || trial.status != "PASSED"
    }) {
        return Err(ProductError::gate(
            "P15_TRIAL_FREEZE_FAILED",
            "a bounded trial changed its configuration or implementation identity",
        ));
    }

    temporary.cleanup()?;
    Ok(StabilityLadderResultV1 {
        schema: RESULT_SCHEMA.to_owned(),
        status: "LADDER_OK".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        hardware_status: "UNVERIFIED".to_owned(),
        long_duration_status: "UNVERIFIED".to_owned(),
        performance_admission_status: "UNVERIFIED".to_owned(),
        completion_sla_status: "UNVERIFIED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        execution_surface: EXECUTION_SURFACE.to_owned(),
        configuration_sha256,
        implementation_sha256,
        plan_sha256,
        plan,
        trials,
        restart_equivalence,
        repeated_stability_equivalence,
        owned_temporary_root_removed: true,
        limitations: vec![
            "not-hardware-qualification".to_owned(),
            "not-accelerator-stability".to_owned(),
            "not-long-duration-execution".to_owned(),
            "not-performance-admission".to_owned(),
            "not-sla-verification".to_owned(),
            "not-full-run-evidence".to_owned(),
            "synthetic-backend-does-not-substitute-for-p16-training".to_owned(),
        ],
    })
}

#[allow(clippy::too_many_arguments)]
fn run_fresh_trial(
    trial_id: &str,
    trial_kind: &str,
    repetition: u64,
    updates: u64,
    configuration: &PrototypeTrainingDefaultsV1,
    identity: &TrainerIdentity,
    configuration_sha256: &str,
    expected_implementation_sha256: &str,
) -> Result<StabilityTrialV1> {
    let before_config = configuration.sha256()?;
    let before_implementation = implementation_sha256();
    let mut trainer = new_trainer(identity)?;
    run_updates(&mut trainer, updates, configuration)?;
    let snapshot = trainer.snapshot()?;
    let final_state_sha256 = state_bundle_sha256(&snapshot)?;
    trial_result(
        trial_id,
        trial_kind,
        repetition,
        updates,
        &snapshot,
        final_state_sha256,
        before_config,
        configuration.sha256()?,
        before_implementation,
        implementation_sha256(),
        configuration_sha256,
        expected_implementation_sha256,
        None,
    )
}

fn run_restart_trial(
    root: &Path,
    plan: &StabilityPlanV1,
    configuration: &PrototypeTrainingDefaultsV1,
    identity: &TrainerIdentity,
    configuration_sha256: &str,
    expected_implementation_sha256: &str,
) -> Result<StabilityTrialV1> {
    let before_config = configuration.sha256()?;
    let before_implementation = implementation_sha256();

    let mut uninterrupted = new_trainer(identity)?;
    run_updates(
        &mut uninterrupted,
        plan.restart_total_updates,
        configuration,
    )?;
    let uninterrupted_snapshot = uninterrupted.snapshot()?;

    let mut interrupted = new_trainer(identity)?;
    run_updates(
        &mut interrupted,
        plan.restart_checkpoint_after_updates,
        configuration,
    )?;
    let checkpoint = interrupted.snapshot()?;
    let published = publish_checkpoint(root, &checkpoint)?;
    let loaded = load_checkpoint(&published.generation_path)?;
    let mut resumed =
        DeterministicTrainer::from_snapshot(loaded, identity, DiagnosticBackend::default())?;
    run_updates(&mut resumed, plan.restart_total_updates, configuration)?;
    let resumed_snapshot = resumed.snapshot()?;
    let roundtrip_exact = resumed_snapshot == uninterrupted_snapshot
        && state_bundle_sha256(&resumed_snapshot)? == state_bundle_sha256(&uninterrupted_snapshot)?;
    if !roundtrip_exact {
        return Err(ProductError::gate(
            "P15_RESTART_MISMATCH",
            "the bounded checkpoint reload differs from uninterrupted execution",
        ));
    }
    trial_result(
        "restart-01",
        "restart",
        1,
        plan.restart_total_updates,
        &resumed_snapshot,
        state_bundle_sha256(&resumed_snapshot)?,
        before_config,
        configuration.sha256()?,
        before_implementation,
        implementation_sha256(),
        configuration_sha256,
        expected_implementation_sha256,
        Some(true),
    )
}

#[allow(clippy::too_many_arguments)]
fn trial_result(
    trial_id: &str,
    trial_kind: &str,
    repetition: u64,
    planned_updates: u64,
    snapshot: &TrainerSnapshot,
    final_state_sha256: String,
    configuration_before: String,
    configuration_after: String,
    implementation_before: String,
    implementation_after: String,
    expected_configuration: &str,
    expected_implementation: &str,
    checkpoint_roundtrip_exact: Option<bool>,
) -> Result<StabilityTrialV1> {
    if snapshot.completed_updates != planned_updates
        || snapshot.consumed_targets
            != planned_updates
                .checked_mul(snapshot.plan.targets_per_full_update)
                .ok_or_else(accounting_overflow)?
    {
        return Err(ProductError::gate(
            "P15_TRIAL_ACCOUNTING_MISMATCH",
            "a bounded trial did not complete its exact optimizer-update target count",
        ));
    }
    let configuration_frozen = configuration_before == configuration_after
        && configuration_after == expected_configuration;
    let implementation_frozen = implementation_before == implementation_after
        && implementation_after == expected_implementation;
    Ok(StabilityTrialV1 {
        trial_id: trial_id.to_owned(),
        trial_kind: trial_kind.to_owned(),
        repetition,
        status: "PASSED".to_owned(),
        planned_updates,
        completed_updates: snapshot.completed_updates,
        completed_targets: snapshot.consumed_targets,
        final_state_sha256,
        configuration_sha256_before: configuration_before,
        configuration_sha256_after: configuration_after,
        implementation_sha256_before: implementation_before,
        implementation_sha256_after: implementation_after,
        configuration_frozen,
        implementation_frozen,
        checkpoint_roundtrip_exact,
    })
}

fn new_trainer(identity: &TrainerIdentity) -> Result<DeterministicTrainer<DiagnosticBackend>> {
    DeterministicTrainer::new(
        identity.clone(),
        DiagnosticBackend::default(),
        b"python-slm/p15/host-rng/v1".to_vec(),
        b"python-slm/p15/device-rng/v1".to_vec(),
    )
}

fn run_updates(
    trainer: &mut DeterministicTrainer<DiagnosticBackend>,
    target_completed_updates: u64,
    configuration: &PrototypeTrainingDefaultsV1,
) -> Result<()> {
    if target_completed_updates < trainer.completed_updates() {
        return Err(ProductError::internal(
            "P15_UPDATE_TARGET_INVALID",
            "a stability trial attempted to run backward",
        ));
    }
    while trainer.completed_updates() < target_completed_updates {
        for step in 0..configuration.batch.gradient_accumulation_steps {
            let first_target = trainer.consumed_targets();
            let batch = diagnostic_batch(first_target, configuration.batch.micro_batch_targets);
            let event = trainer.process_batch(&batch)?;
            if (step + 1 == configuration.batch.gradient_accumulation_steps) != event.is_some() {
                return Err(ProductError::gate(
                    "P15_ACCUMULATION_BOUNDARY_MISMATCH",
                    "the trainer update boundary differs from the P14 accumulation defaults",
                ));
            }
        }
    }
    Ok(())
}

fn diagnostic_batch(first_target: u64, valid_targets: u64) -> TrainingBatch {
    let input = ((first_target / valid_targets) % 251) as u16;
    let target = input.wrapping_add(1);
    TrainingBatch {
        first_target,
        valid_targets,
        input_ids: vec![input; valid_targets as usize],
        target_ids: vec![target; valid_targets as usize],
    }
}

#[derive(Clone, Default)]
struct DiagnosticBackend {
    state: u64,
}

impl DiagnosticBackend {
    fn artifacts(&self) -> Vec<BackendStateArtifact> {
        let mut artifacts = [
            ("parameters_bf16", "state/parameters.bf16", 1_u8),
            ("master_parameters_fp32", "state/master.f32", 2),
            ("adamw_first_moments_fp32", "state/moment1.f32", 3),
            ("adamw_second_moments_fp32", "state/moment2.f32", 4),
            ("backend_runtime_state", "state/runtime.bin", 5),
        ]
        .into_iter()
        .map(|(role, relative_path, tag)| {
            let mut bytes = vec![tag];
            bytes.extend_from_slice(&self.state.to_le_bytes());
            BackendStateArtifact {
                role: role.to_owned(),
                relative_path: relative_path.to_owned(),
                bytes,
            }
        })
        .collect::<Vec<_>>();
        artifacts.sort_by(|left, right| left.role.cmp(&right.role));
        artifacts
    }
}

impl TrainerBackend for DiagnosticBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        let input_sum = batch
            .input_ids
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>();
        let target_sum = batch
            .target_ids
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>();
        let mut rng = Sha256::new();
        rng.update(b"python-slm/p15/diagnostic-rng/v1\0");
        rng.update(batch.first_target.to_le_bytes());
        rng.update(batch.valid_targets.to_le_bytes());
        rng.update(input_sum.to_le_bytes());
        rng.update(target_sum.to_le_bytes());
        let digest = rng.finalize();
        Ok(BatchGradient {
            loss_sum: batch.valid_targets as f64 * 0.25,
            gradient_sums: vec![
                batch.valid_targets as f32,
                input_sum as f32,
                target_sum as f32,
            ],
            host_rng_state: digest[..16].to_vec(),
            device_rng_state: digest[16..].to_vec(),
        })
    }

    fn apply_update(
        &mut self,
        gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
        valid_targets: u64,
    ) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/p15/diagnostic-update/v1\0");
        hasher.update(self.state.to_le_bytes());
        hasher.update(one_based_update.to_le_bytes());
        hasher.update(valid_targets.to_le_bytes());
        hasher.update(learning_rate.to_le_bytes());
        for gradient in gradients {
            hasher.update(gradient.to_le_bytes());
        }
        let digest = hasher.finalize();
        self.state = u64::from_le_bytes(digest[..8].try_into().map_err(|_| {
            ProductError::internal(
                "P15_BACKEND_STATE_INVALID",
                "could not derive diagnostic backend state",
            )
        })?);
        Ok(hex::encode(digest))
    }

    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/p15/diagnostic-evaluation/v1\0");
        hasher.update(validation_span_manifest_sha256.as_bytes());
        hasher.update(self.state.to_le_bytes());
        Ok(EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 0.25,
            result_sha256: hex::encode(hasher.finalize()),
        })
    }

    fn snapshot(&self) -> Result<Vec<BackendStateArtifact>> {
        Ok(self.artifacts())
    }

    fn restore(&mut self, artifacts: &[BackendStateArtifact]) -> Result<()> {
        let runtime = artifacts
            .iter()
            .find(|artifact| artifact.role == "backend_runtime_state")
            .ok_or_else(|| {
                ProductError::integrity(
                    "P15_BACKEND_STATE_INCOMPLETE",
                    "the diagnostic checkpoint omits runtime state",
                )
            })?;
        let state_bytes: [u8; 8] = runtime
            .bytes
            .get(1..)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| {
                ProductError::integrity(
                    "P15_BACKEND_STATE_INVALID",
                    "the diagnostic runtime state has the wrong byte length",
                )
            })?;
        let restored = Self {
            state: u64::from_le_bytes(state_bytes),
        };
        if restored.artifacts() != artifacts {
            return Err(ProductError::integrity(
                "P15_BACKEND_STATE_MISMATCH",
                "the diagnostic checkpoint contains inconsistent backend artifacts",
            ));
        }
        *self = restored;
        Ok(())
    }
}

fn diagnostic_identity(implementation_sha256: &str) -> TrainerIdentity {
    TrainerIdentity {
        profile: PROTOTYPE_PROFILE.to_owned(),
        model_identity: CANONICAL_MODEL_ID.to_owned(),
        model_parameter_layout_sha256: domain_sha256("model-parameter-layout"),
        backend_identity_sha256: domain_sha256(EXECUTION_SURFACE),
        device_identity_sha256: domain_sha256("synthetic-device"),
        corpus_manifest_sha256: domain_sha256("synthetic-corpus"),
        training_span_manifest_sha256: domain_sha256("synthetic-training-spans"),
        validation_span_manifest_sha256: domain_sha256("synthetic-validation-spans"),
        tokenizer_artifact_sha256: domain_sha256("synthetic-tokenizer"),
        environment_identity_sha256: domain_sha256("bounded-local-environment"),
        implementation_artifact_sha256: implementation_sha256.to_owned(),
    }
}

fn implementation_sha256() -> String {
    let mut hasher = Sha256::new();
    hash_component(&mut hasher, b"python-slm/p15/implementation/v1");
    hash_component(&mut hasher, include_bytes!("stability.rs"));
    hash_component(&mut hasher, include_bytes!("trainer.rs"));
    hash_component(&mut hasher, include_bytes!("checkpoint.rs"));
    hash_component(&mut hasher, include_bytes!("profile.rs"));
    hex::encode(hasher.finalize())
}

fn hash_component(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn domain_sha256(label: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/p15/diagnostic-identity/v1\0");
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label.as_bytes());
    hex::encode(hasher.finalize())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P15_TARGET_ACCOUNTING_OVERFLOW",
        "bounded stability target accounting overflowed",
    )
}

struct OwnedTemporaryRoot {
    path: PathBuf,
    active: bool,
}

impl OwnedTemporaryRoot {
    fn create() -> Result<Self> {
        let parent = std::env::temp_dir().canonicalize().map_err(|_| {
            ProductError::environment(
                "P15_TEMP_ROOT_INVALID",
                "could not resolve the operating-system temporary directory",
            )
        })?;
        for _ in 0..64 {
            let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let leaf = format!("{TEMP_PREFIX}{}-{nonce:016x}", std::process::id());
            let candidate = parent.join(leaf);
            match fs::create_dir(&candidate) {
                Ok(()) => {
                    let canonical = match candidate.canonicalize() {
                        Ok(path) => path,
                        Err(_) => {
                            let _ = fs::remove_dir(&candidate);
                            return Err(ProductError::environment(
                                "P15_TEMP_ROOT_INVALID",
                                "could not bind the owned temporary directory",
                            ));
                        }
                    };
                    if canonical.parent() != Some(parent.as_path())
                        || !canonical
                            .file_name()
                            .and_then(|value| value.to_str())
                            .is_some_and(|value| value.starts_with(TEMP_PREFIX))
                    {
                        let _ = fs::remove_dir(&candidate);
                        return Err(ProductError::integrity(
                            "P15_TEMP_ROOT_ESCAPED",
                            "the owned temporary directory escaped its canonical parent",
                        ));
                    }
                    return Ok(Self {
                        path: canonical,
                        active: true,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(ProductError::environment(
                        "P15_TEMP_ROOT_CREATE_FAILED",
                        "could not create the owned temporary directory",
                    ));
                }
            }
        }
        Err(ProductError::environment(
            "P15_TEMP_ROOT_CREATE_FAILED",
            "could not allocate a unique owned temporary directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        require_owned_temp_root(&self.path)?;
        fs::remove_dir_all(&self.path).map_err(|_| {
            ProductError::environment(
                "P15_TEMP_ROOT_REMOVE_FAILED",
                "could not remove the owned temporary directory",
            )
        })?;
        self.active = false;
        Ok(())
    }
}

impl Drop for OwnedTemporaryRoot {
    fn drop(&mut self) {
        if self.active && require_owned_temp_root(&self.path).is_ok() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn require_owned_temp_root(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        ProductError::environment(
            "P15_TEMP_ROOT_INVALID",
            "could not inspect the owned temporary directory",
        )
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse(&metadata)
        || !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with(TEMP_PREFIX))
    {
        return Err(ProductError::integrity(
            "P15_TEMP_ROOT_INVALID",
            "the cleanup target is not the exact owned temporary directory",
        ));
    }
    let canonical = path.canonicalize().map_err(|_| {
        ProductError::environment(
            "P15_TEMP_ROOT_INVALID",
            "could not resolve the owned temporary directory",
        )
    })?;
    let parent = std::env::temp_dir().canonicalize().map_err(|_| {
        ProductError::environment(
            "P15_TEMP_ROOT_INVALID",
            "could not resolve the operating-system temporary directory",
        )
    })?;
    if canonical != path || canonical.parent() != Some(parent.as_path()) {
        return Err(ProductError::integrity(
            "P15_TEMP_ROOT_ESCAPED",
            "the cleanup target escaped its canonical temporary parent",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_temporary_root_is_removed_on_drop() {
        let temporary = OwnedTemporaryRoot::create().unwrap();
        let path = temporary.path().to_path_buf();
        assert!(path.is_dir());
        drop(temporary);
        assert!(!path.exists());
    }

    #[test]
    fn plan_rejects_unknown_or_retuned_fields() {
        let configuration = super::super::profile::parse_default_configuration(
            super::super::profile::DEFAULT_CONFIG_BYTES,
        )
        .unwrap();
        let digest = configuration.sha256().unwrap();
        let plan = parse_stability_plan(STABILITY_PLAN_BYTES, &digest).unwrap();
        assert_eq!(plan, StabilityPlanV1::canonical(&digest));

        let mut value = serde_json::to_value(&plan).unwrap();
        value["stability_updates"] = 31.into();
        assert_eq!(
            parse_stability_plan(&serde_json::to_vec(&value).unwrap(), &digest)
                .unwrap_err()
                .code,
            "P15_PLAN_MISMATCH"
        );
        value["stability_updates"] = 32.into();
        value["unknown"] = true.into();
        assert_eq!(
            parse_stability_plan(&serde_json::to_vec(&value).unwrap(), &digest)
                .unwrap_err()
                .code,
            "P15_PLAN_INVALID"
        );
    }
}
