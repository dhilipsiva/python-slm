use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::error::Result;
use rust_llm_pretrain::model::CANONICAL_MODEL_ID;
use rust_llm_pretrain::train::trainer::{
    BackendStateArtifact, BatchGradient, CanonicalTrainingPlan, CompletedEvaluation,
    DeterministicTrainer, EVALUATION_TARGETS, EVENT_INTERVAL_TARGETS, EvaluationResult,
    FINAL_UPDATE_TARGETS, FULL_UPDATES, TARGETS_PER_FULL_UPDATE, TOTAL_TARGETS, TOTAL_UPDATES,
    TRAINER_SNAPSHOT_SCHEMA, TrainerBackend, TrainerIdentity, TrainerSnapshot, TrainingBatch,
    canonical_learning_rate,
};
use rust_llm_pretrain::train::{
    FinalBatchSource, PrototypeTrainingDefaultsV1, build_implementation_result,
    execute_to_completion, verify_final_checkpoint,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Default)]
struct TestBackend {
    state: u64,
}

impl TestBackend {
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

impl TrainerBackend for TestBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        Ok(BatchGradient {
            loss_sum: batch.valid_targets as f64 * 0.5,
            gradient_sums: vec![
                batch.valid_targets as f32,
                batch.valid_targets as f32 * 0.25,
            ],
            host_rng_state: batch
                .first_target
                .wrapping_add(batch.valid_targets)
                .to_le_bytes()
                .to_vec(),
            device_rng_state: batch.valid_targets.to_le_bytes().to_vec(),
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
        hasher.update(self.state.to_le_bytes());
        hasher.update(one_based_update.to_le_bytes());
        hasher.update(valid_targets.to_le_bytes());
        hasher.update(learning_rate.to_le_bytes());
        for gradient in gradients {
            hasher.update(gradient.to_le_bytes());
        }
        let digest = hasher.finalize();
        self.state = u64::from_le_bytes(digest[..8].try_into().unwrap());
        Ok(hex::encode(digest))
    }

    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/p16-test-evaluation/v1\0");
        hasher.update(validation_span_manifest_sha256.as_bytes());
        hasher.update(self.state.to_le_bytes());
        Ok(EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 1.0,
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
            .unwrap();
        self.state = u64::from_le_bytes(runtime.bytes[1..].try_into().unwrap());
        Ok(())
    }
}

#[derive(Default)]
struct TailSource {
    requested: Vec<(u64, u64)>,
}

impl FinalBatchSource for TailSource {
    fn next_batch(&mut self, first_target: u64, valid_targets: u64) -> Result<TrainingBatch> {
        self.requested.push((first_target, valid_targets));
        Ok(TrainingBatch {
            first_target,
            valid_targets,
            input_ids: vec![7; valid_targets as usize],
            target_ids: vec![8; valid_targets as usize],
            sequence_lengths: vec![valid_targets],
        })
    }
}

fn identity() -> TrainerIdentity {
    TrainerIdentity {
        profile: PROTOTYPE_PROFILE.to_owned(),
        model_identity: CANONICAL_MODEL_ID.to_owned(),
        model_parameter_layout_sha256: "11".repeat(32),
        backend_identity_sha256: "22".repeat(32),
        device_identity_sha256: "33".repeat(32),
        corpus_manifest_sha256: "44".repeat(32),
        training_span_manifest_sha256: "55".repeat(32),
        validation_span_manifest_sha256: "66".repeat(32),
        tokenizer_artifact_sha256: "77".repeat(32),
        environment_identity_sha256: "88".repeat(32),
        implementation_artifact_sha256: "99".repeat(32),
    }
}

fn evaluation(after_targets: u64, before_first_update: bool) -> CompletedEvaluation {
    CompletedEvaluation {
        after_targets,
        before_first_update,
        result: EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 1.0,
            result_sha256: "aa".repeat(32),
        },
    }
}

fn completed_evaluations(consumed_targets: u64) -> Vec<CompletedEvaluation> {
    let mut values = vec![evaluation(0, true)];
    let mut threshold = EVENT_INTERVAL_TARGETS;
    while threshold <= consumed_targets {
        let completed = threshold.div_ceil(TARGETS_PER_FULL_UPDATE) * TARGETS_PER_FULL_UPDATE;
        if completed <= consumed_targets {
            values.push(evaluation(completed, false));
        }
        threshold += EVENT_INTERVAL_TARGETS;
    }
    values
}

fn snapshot_before_final_update() -> TrainerSnapshot {
    let consumed_targets = FULL_UPDATES * TARGETS_PER_FULL_UPDATE;
    TrainerSnapshot {
        schema: TRAINER_SNAPSHOT_SCHEMA.to_owned(),
        identity: identity(),
        plan: CanonicalTrainingPlan::canonical(),
        consumed_targets,
        completed_updates: FULL_UPDATES,
        accumulated_targets: 0,
        next_training_first_target: consumed_targets,
        scheduler_one_based_update: FULL_UPDATES,
        last_learning_rate_f32_le_hex: Some(hex::encode(
            canonical_learning_rate(FULL_UPDATES).unwrap().to_le_bytes(),
        )),
        last_update_state_sha256: Some("bb".repeat(32)),
        host_rng_state: vec![1, 2, 3],
        device_rng_state: vec![4, 5, 6],
        evaluations: completed_evaluations(consumed_targets),
        backend_state: TestBackend { state: 41 }.artifacts(),
    }
}

#[test]
fn implementation_result_is_exact_and_claim_limited() {
    let result = build_implementation_result(PrototypeTrainingDefaultsV1::canonical()).unwrap();
    assert_eq!(
        result.schema,
        "python-slm-final-training-implementation-result-v1"
    );
    assert_eq!(result.status, "IMPLEMENTATION_READY");
    assert_eq!(result.qualification_status, "SKIPPED");
    assert_eq!(result.execution_status, "NOT_RUN");
    assert_eq!(result.training_plan.total_targets, TOTAL_TARGETS);
    assert_eq!(result.training_plan.total_updates, TOTAL_UPDATES);
    assert_eq!(
        result.training_plan.final_update_targets,
        FINAL_UPDATE_TARGETS
    );
    assert!(result.checkpoint_policy.final_checkpoint_required);
    assert!(result.checkpoint_policy.fresh_process_reload_required);
    assert_eq!(result.claims.full_run_completion, "UNVERIFIED");
    assert_eq!(result.claims.completion_sla, "UNVERIFIED");
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains(":\\"));
    assert!(!json.contains("receipts"));
    assert!(!json.contains("pointer"));
}

#[test]
fn resumed_tail_stops_exactly_and_publishes_reloadable_final_checkpoint() {
    let snapshot = snapshot_before_final_update();
    snapshot.validate().unwrap();
    let mut trainer =
        DeterministicTrainer::from_snapshot(snapshot, &identity(), TestBackend::default()).unwrap();
    let mut source = TailSource::default();
    let checkpoint_root = tempfile::tempdir().unwrap();
    let execution = execute_to_completion(
        &mut trainer,
        &mut source,
        checkpoint_root.path(),
        &PrototypeTrainingDefaultsV1::canonical(),
    )
    .unwrap();

    assert_eq!(source.requested.len(), 2);
    assert_eq!(source.requested[0].1, 32_768);
    assert_eq!(source.requested[1].1, 5_120);
    assert_eq!(execution.final_snapshot.consumed_targets, TOTAL_TARGETS);
    assert_eq!(execution.final_snapshot.completed_updates, TOTAL_UPDATES);
    assert_eq!(execution.final_checkpoint.consumed_targets, TOTAL_TARGETS);
    assert_eq!(execution.published_checkpoints, 1);
    assert_eq!(execution.retained_checkpoint_targets, [TOTAL_TARGETS]);
    assert!(execution.same_process_reload_exact);

    let verified =
        verify_final_checkpoint(&execution.final_checkpoint.generation_path, false).unwrap();
    assert_eq!(verified.status, "FINAL_CHECKPOINT_RELOADED");
    assert_eq!(
        verified.target_accounting_status,
        "VERIFIED_FROM_CHECKPOINT"
    );
    assert!(!verified.fresh_process_boundary);
    assert_eq!(verified.full_run_status, "UNVERIFIED");
}

#[cfg(windows)]
#[test]
fn product_child_reloads_final_checkpoint_in_a_fresh_process() {
    let snapshot = snapshot_before_final_update();
    let mut trainer =
        DeterministicTrainer::from_snapshot(snapshot, &identity(), TestBackend::default()).unwrap();
    let mut source = TailSource::default();
    let checkpoint_root = tempfile::tempdir().unwrap();
    let execution = execute_to_completion(
        &mut trainer,
        &mut source,
        checkpoint_root.path(),
        &PrototypeTrainingDefaultsV1::canonical(),
    )
    .unwrap();
    let defaults = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/train/prototype-windows-5090-v1.defaults.json")
        .canonicalize()
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .arg("train")
        .arg("--config")
        .arg(defaults)
        .arg("--verify-final-checkpoint")
        .arg(&execution.final_checkpoint.generation_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["schema"],
        "python-slm-final-checkpoint-reload-result-v1"
    );
    assert_eq!(value["status"], "FINAL_CHECKPOINT_RELOADED");
    assert_eq!(value["fresh_process_boundary"], true);
    assert_eq!(value["consumed_targets"], TOTAL_TARGETS);
    assert_eq!(value["full_run_status"], "UNVERIFIED");
    assert!(!String::from_utf8(output.stdout).unwrap().contains(":\\"));
}

#[cfg(not(windows))]
#[test]
fn product_boundary_defers_before_reading_configuration() {
    let error = rust_llm_pretrain::commands::run([
        "python-slm".into(),
        "train".into(),
        "--config".into(),
        "missing.json".into(),
    ])
    .unwrap_err();
    assert_eq!(error.code, "DEFERRED_POST_P16");
}
