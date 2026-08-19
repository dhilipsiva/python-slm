//! Final-run orchestration and final-checkpoint reload verification.

use super::checkpoint::{
    PublishedCheckpoint, load_checkpoint, prune_checkpoints, publish_checkpoint,
};
use super::profile::PrototypeTrainingDefaultsV1;
use super::trainer::{
    CanonicalTrainingPlan, DeterministicTrainer, EvaluationResult, TOTAL_TARGETS, TrainerBackend,
    TrainerSnapshot, TrainingBatch, canonical_update_target_count, state_bundle_sha256,
};
use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use crate::model::CANONICAL_MODEL_ID;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const IMPLEMENTATION_PHASE: &str = "P16";
pub const IMPLEMENTATION_RESULT_SCHEMA: &str = "python-slm-final-training-implementation-result-v1";
pub const RELOAD_RESULT_SCHEMA: &str = "python-slm-final-checkpoint-reload-result-v1";
pub const EXECUTION_SURFACE: &str = "provider-neutral-final-run-coordinator";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalRunCheckpointPolicyV1 {
    pub create_new_generations: bool,
    pub completed_update_boundary_only: bool,
    pub interval_targets: u64,
    pub retain_latest: u64,
    pub retention_anchors: Vec<u64>,
    pub final_checkpoint_required: bool,
    pub fresh_process_reload_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalRunClaimStatusV1 {
    pub full_run_completion: String,
    pub elapsed_time: String,
    pub completion_sla: String,
    pub final_loss: String,
    pub final_checkpoint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalTrainingImplementationResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub execution_status: String,
    pub execution_surface: String,
    pub configuration_sha256: String,
    pub implementation_sha256: String,
    pub training_plan: CanonicalTrainingPlan,
    pub checkpoint_policy: FinalRunCheckpointPolicyV1,
    pub claims: FinalRunClaimStatusV1,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalCheckpointReloadResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub model_identity: String,
    pub target_accounting_status: String,
    pub consumed_targets: u64,
    pub completed_updates: u64,
    pub trainer_state_sha256: String,
    pub backend_state_artifacts: u64,
    pub fresh_process_boundary: bool,
    pub full_run_status: String,
    pub elapsed_time_status: String,
    pub completion_sla_status: String,
    pub final_loss_status: String,
    pub limitations: Vec<String>,
}

#[derive(Debug)]
pub struct FinalRunExecution {
    pub final_checkpoint: PublishedCheckpoint,
    pub final_snapshot: TrainerSnapshot,
    pub published_checkpoints: u64,
    pub retained_checkpoint_targets: Vec<u64>,
    pub same_process_reload_exact: bool,
    /// Held-out loss for the state this run produced, when it stopped somewhere
    /// the frozen evaluation schedule does not cover. `None` for the full run,
    /// whose schedule already evaluates at every boundary it defines.
    pub final_evaluation: Option<EvaluationResult>,
}

pub trait FinalBatchSource {
    fn next_batch(&mut self, first_target: u64, valid_targets: u64) -> Result<TrainingBatch>;
}

pub fn build_implementation_result(
    configuration: PrototypeTrainingDefaultsV1,
) -> Result<FinalTrainingImplementationResultV1> {
    configuration.validate()?;
    Ok(FinalTrainingImplementationResultV1 {
        schema: IMPLEMENTATION_RESULT_SCHEMA.to_owned(),
        status: "IMPLEMENTATION_READY".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        execution_status: "NOT_RUN".to_owned(),
        execution_surface: EXECUTION_SURFACE.to_owned(),
        configuration_sha256: configuration.sha256()?,
        implementation_sha256: implementation_sha256(),
        training_plan: CanonicalTrainingPlan::canonical(),
        checkpoint_policy: FinalRunCheckpointPolicyV1 {
            create_new_generations: true,
            completed_update_boundary_only: configuration.checkpoint.completed_update_boundary_only,
            interval_targets: configuration.checkpoint.interval_targets,
            retain_latest: configuration.checkpoint.retain_latest,
            retention_anchors: configuration.checkpoint.retention_anchors,
            final_checkpoint_required: true,
            fresh_process_reload_required: true,
        },
        claims: FinalRunClaimStatusV1 {
            full_run_completion: "UNVERIFIED".to_owned(),
            elapsed_time: "UNVERIFIED".to_owned(),
            completion_sla: "UNVERIFIED".to_owned(),
            final_loss: "UNVERIFIED".to_owned(),
            final_checkpoint: "UNVERIFIED".to_owned(),
        },
        limitations: vec![
            "implementation-readiness-only".to_owned(),
            "not-a-two-billion-target-execution".to_owned(),
            "not-hardware-qualification".to_owned(),
            "not-performance-admission".to_owned(),
            "not-sla-verification".to_owned(),
            "not-final-model-quality".to_owned(),
        ],
    })
}

pub fn execute_to_completion<B: TrainerBackend, S: FinalBatchSource>(
    trainer: &mut DeterministicTrainer<B>,
    source: &mut S,
    checkpoint_root: &Path,
    configuration: &PrototypeTrainingDefaultsV1,
) -> Result<FinalRunExecution> {
    configuration.validate()?;
    if trainer.consumed_targets() == TOTAL_TARGETS {
        return Err(ProductError::gate(
            "P16_ALREADY_COMPLETE",
            "the supplied trainer is already complete; verify its final checkpoint instead",
        ));
    }
    let mut published_checkpoints = 0_u64;
    let mut final_checkpoint: Option<PublishedCheckpoint> = None;
    while trainer.consumed_targets() < TOTAL_TARGETS {
        let update = trainer
            .completed_updates()
            .checked_add(1)
            .ok_or_else(accounting_overflow)?;
        let required = canonical_update_target_count(update)?;
        let mut submitted = 0_u64;
        while submitted < required {
            let valid_targets = configuration
                .batch
                .micro_batch_targets
                .min(required - submitted);
            let first_target = trainer.consumed_targets();
            let batch = source.next_batch(first_target, valid_targets)?;
            if batch.first_target != first_target
                || batch.valid_targets != valid_targets
                || batch.input_ids.len() as u64 != valid_targets
                || batch.target_ids.len() as u64 != valid_targets
            {
                return Err(ProductError::integrity(
                    "P16_BATCH_SOURCE_MISMATCH",
                    "the final-run source returned a batch outside the exact requested cursor or shape",
                ));
            }
            let event = trainer.process_batch(&batch)?;
            submitted = submitted
                .checked_add(valid_targets)
                .ok_or_else(accounting_overflow)?;
            if (submitted == required) != event.is_some() {
                return Err(ProductError::integrity(
                    "P16_UPDATE_BOUNDARY_MISMATCH",
                    "the final-run coordinator and trainer disagree on an optimizer-update boundary",
                ));
            }
            if let Some(event) = event {
                if event.valid_targets != required
                    || event.one_based_update != update
                    || event.consumed_targets != trainer.consumed_targets()
                {
                    return Err(ProductError::integrity(
                        "P16_UPDATE_EVENT_MISMATCH",
                        "the completed optimizer update differs from the canonical final-run plan",
                    ));
                }
                if event.checkpoint_due {
                    let snapshot = trainer.snapshot()?;
                    let published = publish_checkpoint(checkpoint_root, &snapshot)?;
                    published_checkpoints = published_checkpoints
                        .checked_add(1)
                        .ok_or_else(accounting_overflow)?;
                    let _ = prune_checkpoints(checkpoint_root)?;
                    if event.training_complete {
                        final_checkpoint = Some(published);
                    }
                }
            }
        }
    }
    let final_snapshot = trainer.snapshot()?;
    if final_snapshot.consumed_targets != TOTAL_TARGETS
        || final_snapshot.completed_updates != final_snapshot.plan.total_updates
    {
        return Err(ProductError::gate(
            "P16_TARGET_ACCOUNTING_INCOMPLETE",
            "the final-run coordinator did not stop at exactly two billion valid targets",
        ));
    }
    let final_checkpoint = final_checkpoint.ok_or_else(|| {
        ProductError::gate(
            "P16_FINAL_CHECKPOINT_MISSING",
            "training completed without publishing the mandatory durable final checkpoint",
        )
    })?;
    let reloaded = load_checkpoint(&final_checkpoint.generation_path)?;
    let same_process_reload_exact = reloaded == final_snapshot
        && state_bundle_sha256(&reloaded)? == state_bundle_sha256(&final_snapshot)?;
    if !same_process_reload_exact {
        return Err(ProductError::integrity(
            "P16_FINAL_CHECKPOINT_RELOAD_MISMATCH",
            "the durable final checkpoint differs from the in-memory completed trainer state",
        ));
    }
    let retained_checkpoint_targets = prune_checkpoints(checkpoint_root)?;
    Ok(FinalRunExecution {
        final_checkpoint,
        final_snapshot,
        published_checkpoints,
        retained_checkpoint_targets,
        same_process_reload_exact,
        final_evaluation: None,
    })
}

/// Train the canonical model to a bounded target budget and stop.
///
/// This is **not** the frozen run and never claims to be. `execute_to_completion`
/// asserts `consumed_targets == TOTAL_TARGETS` and that assertion is the whole
/// point of it, so bounding it there would have turned the contract's own
/// completion check into something conditional. This is a sibling with its own,
/// weaker post-conditions instead: it stops on an optimizer-update boundary at or
/// past the budget, publishes a checkpoint there because the interval policy
/// would otherwise publish none inside a short run, and reports the targets it
/// actually consumed.
///
/// The model, corpus, initialization, arithmetic, and checkpoint codec are
/// untouched — this changes only how long the loop runs, which is why it needs no
/// amendment to any frozen constant. What it produces is a partial checkpoint of
/// `gqa-135m-v1`, and every result that carries it says so.
pub fn execute_bounded_diagnostic<B: TrainerBackend, S: FinalBatchSource>(
    trainer: &mut DeterministicTrainer<B>,
    source: &mut S,
    checkpoint_root: &Path,
    configuration: &PrototypeTrainingDefaultsV1,
    target_budget: u64,
) -> Result<FinalRunExecution> {
    configuration.validate()?;
    if target_budget == 0 || target_budget >= TOTAL_TARGETS {
        return Err(ProductError::usage(
            "E7_DIAGNOSTIC_BUDGET_INVALID",
            "a diagnostic target budget must be positive and below the frozen target count; \
             use the full launch path to run the frozen count",
        ));
    }
    if trainer.consumed_targets() >= target_budget {
        return Err(ProductError::gate(
            "E7_DIAGNOSTIC_BUDGET_ALREADY_MET",
            "the supplied trainer has already consumed the diagnostic budget",
        ));
    }
    let mut published_checkpoints = 0_u64;
    let mut final_checkpoint: Option<PublishedCheckpoint> = None;
    while trainer.consumed_targets() < target_budget {
        let update = trainer
            .completed_updates()
            .checked_add(1)
            .ok_or_else(accounting_overflow)?;
        let required = canonical_update_target_count(update)?;
        let mut submitted = 0_u64;
        while submitted < required {
            let valid_targets = configuration
                .batch
                .micro_batch_targets
                .min(required - submitted);
            let first_target = trainer.consumed_targets();
            let batch = source.next_batch(first_target, valid_targets)?;
            if batch.first_target != first_target
                || batch.valid_targets != valid_targets
                || batch.input_ids.len() as u64 != valid_targets
                || batch.target_ids.len() as u64 != valid_targets
            {
                return Err(ProductError::integrity(
                    "P16_BATCH_SOURCE_MISMATCH",
                    "the final-run source returned a batch outside the exact requested cursor or shape",
                ));
            }
            let event = trainer.process_batch(&batch)?;
            submitted = submitted
                .checked_add(valid_targets)
                .ok_or_else(accounting_overflow)?;
            if (submitted == required) != event.is_some() {
                return Err(ProductError::integrity(
                    "P16_UPDATE_BOUNDARY_MISMATCH",
                    "the final-run coordinator and trainer disagree on an optimizer-update boundary",
                ));
            }
            if let Some(event) = event
                && event.checkpoint_due
            {
                let snapshot = trainer.snapshot()?;
                let published = publish_checkpoint(checkpoint_root, &snapshot)?;
                published_checkpoints = published_checkpoints
                    .checked_add(1)
                    .ok_or_else(accounting_overflow)?;
                let _ = prune_checkpoints(checkpoint_root)?;
                final_checkpoint = Some(published);
            }
        }
    }
    // Score what the run produced. A bounded diagnostic that reports no loss says
    // nothing about the state it just spent an hour making, and the frozen
    // schedule will not have fired if the budget is shorter than its interval.
    // This is deliberately not recorded in the snapshot: `validate_evaluations`
    // requires the recorded set to sit on exact schedule boundaries.
    let final_evaluation = Some(trainer.evaluate_now()?);

    // The interval policy publishes every hundred million targets, so a budget
    // shorter than that would otherwise end with nothing durable. Publishing here
    // is unconditional rather than interval-driven, and lands on the update
    // boundary the loop just finished, which is what the checkpoint policy's
    // `completed_update_boundary_only` requires.
    let final_snapshot = trainer.snapshot()?;
    let final_checkpoint = match final_checkpoint {
        Some(published) if published.consumed_targets == final_snapshot.consumed_targets => {
            published
        }
        _ => {
            let published = publish_checkpoint(checkpoint_root, &final_snapshot)?;
            published_checkpoints = published_checkpoints
                .checked_add(1)
                .ok_or_else(accounting_overflow)?;
            published
        }
    };
    let reloaded = load_checkpoint(&final_checkpoint.generation_path)?;
    let same_process_reload_exact = reloaded == final_snapshot
        && state_bundle_sha256(&reloaded)? == state_bundle_sha256(&final_snapshot)?;
    if !same_process_reload_exact {
        return Err(ProductError::integrity(
            "P16_FINAL_CHECKPOINT_RELOAD_MISMATCH",
            "the durable checkpoint differs from the in-memory trainer state",
        ));
    }
    let retained_checkpoint_targets = prune_checkpoints(checkpoint_root)?;
    Ok(FinalRunExecution {
        final_checkpoint,
        final_snapshot,
        published_checkpoints,
        retained_checkpoint_targets,
        same_process_reload_exact,
        final_evaluation,
    })
}

pub fn verify_final_checkpoint(
    generation: &Path,
    fresh_process_boundary: bool,
) -> Result<FinalCheckpointReloadResultV1> {
    let snapshot = load_checkpoint(generation)?;
    if snapshot.identity.profile != PROTOTYPE_PROFILE
        || snapshot.identity.model_identity != CANONICAL_MODEL_ID
        || snapshot.consumed_targets != TOTAL_TARGETS
        || snapshot.completed_updates != snapshot.plan.total_updates
        || snapshot.next_training_first_target != TOTAL_TARGETS
    {
        return Err(ProductError::integrity(
            "P16_FINAL_CHECKPOINT_INCOMPLETE",
            "the checkpoint is not the exact completed prototype training state",
        ));
    }
    Ok(FinalCheckpointReloadResultV1 {
        schema: RELOAD_RESULT_SCHEMA.to_owned(),
        status: "FINAL_CHECKPOINT_RELOADED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: snapshot.identity.profile.clone(),
        model_identity: snapshot.identity.model_identity.clone(),
        target_accounting_status: "VERIFIED_FROM_CHECKPOINT".to_owned(),
        consumed_targets: snapshot.consumed_targets,
        completed_updates: snapshot.completed_updates,
        trainer_state_sha256: state_bundle_sha256(&snapshot)?,
        backend_state_artifacts: snapshot.backend_state.len() as u64,
        fresh_process_boundary,
        full_run_status: "UNVERIFIED".to_owned(),
        elapsed_time_status: "UNVERIFIED".to_owned(),
        completion_sla_status: "UNVERIFIED".to_owned(),
        final_loss_status: "UNVERIFIED".to_owned(),
        limitations: vec![
            "checkpoint-integrity-is-not-execution-provenance".to_owned(),
            "not-hardware-qualification".to_owned(),
            "not-performance-admission".to_owned(),
            "not-sla-verification".to_owned(),
            "not-final-model-quality".to_owned(),
        ],
    })
}

pub fn final_training(config_path: &Path, verify_generation: Option<&Path>) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = (config_path, verify_generation);
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the prototype final-training boundary is implemented only for native Windows",
        ));
    }
    #[cfg(windows)]
    {
        let config_bytes =
            super::profile::read_control_file(config_path, "P16_CONFIG_READ_FAILED")?;
        let configuration = super::profile::parse_default_configuration(&config_bytes)?;
        let value = match verify_generation {
            Some(generation) => serde_json::to_value(verify_final_checkpoint(generation, true)?),
            None => serde_json::to_value(build_implementation_result(configuration)?),
        };
        value.map_err(|_| {
            ProductError::internal(
                "P16_RESULT_SERIALIZE_FAILED",
                "could not serialize the closed final-training result",
            )
        })
    }
}

fn implementation_sha256() -> String {
    hex::encode(Sha256::digest(include_bytes!("final_run.rs")))
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P16_ACCOUNTING_OVERFLOW",
        "final-run target or checkpoint accounting overflowed",
    )
}
