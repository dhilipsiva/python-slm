use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::model::CANONICAL_MODEL_ID;
use rust_llm_pretrain::train::checkpoint::retained_generation_targets;
use rust_llm_pretrain::train::trainer::{
    AdamwParameterState, BackendStateArtifact, BatchGradient, CanonicalAdamw,
    CanonicalTrainingPlan, CompletedEvaluation, DeterministicTrainer, EVALUATION_TARGETS,
    EVENT_INTERVAL_TARGETS, EvaluationResult, FINAL_UPDATE_TARGETS, FULL_UPDATES,
    TARGETS_PER_FULL_UPDATE, TOTAL_TARGETS, TOTAL_UPDATES, TRAINER_SNAPSHOT_SCHEMA, TrainerBackend,
    TrainerIdentity, TrainerSnapshot, TrainingBatch, canonical_learning_rate,
    canonical_update_target_count, state_bundle_sha256,
};
use rust_llm_pretrain::train::{load_checkpoint, publish_checkpoint};
use rust_llm_pretrain::{error::Result, train};
use sha2::{Digest, Sha256};

#[derive(Clone, Default)]
struct ScalarBackend {
    state: u64,
}

impl ScalarBackend {
    fn artifacts(&self) -> Vec<BackendStateArtifact> {
        let mut artifacts = [
            ("parameters_bf16", "state/parameters.bf16", 1_u8),
            ("master_parameters_fp32", "state/master.f32", 2),
            ("adamw_first_moments_fp32", "state/moment1.f32", 3),
            ("adamw_second_moments_fp32", "state/moment2.f32", 4),
            ("backend_runtime_state", "state/runtime.bin", 5),
        ]
        .into_iter()
        .map(|(role, path, tag)| {
            let mut bytes = vec![tag];
            bytes.extend_from_slice(&self.state.to_le_bytes());
            BackendStateArtifact {
                role: role.to_owned(),
                relative_path: path.to_owned(),
                bytes,
            }
        })
        .collect::<Vec<_>>();
        artifacts.sort_by(|left, right| left.role.cmp(&right.role));
        artifacts
    }
}

impl TrainerBackend for ScalarBackend {
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
        for value in gradients {
            hasher.update(value.to_le_bytes());
        }
        let digest = hasher.finalize();
        self.state = u64::from_le_bytes(digest[..8].try_into().unwrap());
        Ok(hex::encode(digest))
    }

    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/p12-evaluation/v1\0");
        hasher.update(validation_span_manifest_sha256.as_bytes());
        hasher.update(self.state.to_le_bytes());
        Ok(EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 12.5,
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

fn batch(first_target: u64, valid_targets: u64) -> TrainingBatch {
    TrainingBatch {
        first_target,
        valid_targets,
        input_ids: vec![7; valid_targets as usize],
        target_ids: vec![8; valid_targets as usize],
        sequence_lengths: vec![valid_targets],
    }
}

fn evaluation(after_targets: u64, baseline: bool) -> CompletedEvaluation {
    CompletedEvaluation {
        after_targets,
        before_first_update: baseline,
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
        backend_state: ScalarBackend { state: 41 }.artifacts(),
    }
}

#[test]
fn canonical_target_and_schedule_arithmetic_is_exact() {
    CanonicalTrainingPlan::canonical().validate().unwrap();
    assert_eq!(
        FULL_UPDATES * TARGETS_PER_FULL_UPDATE + FINAL_UPDATE_TARGETS,
        TOTAL_TARGETS
    );
    assert_eq!(
        canonical_update_target_count(1).unwrap(),
        TARGETS_PER_FULL_UPDATE
    );
    assert_eq!(
        canonical_update_target_count(TOTAL_UPDATES).unwrap(),
        FINAL_UPDATE_TARGETS
    );
    assert_eq!(
        canonical_learning_rate(1).unwrap().to_bits(),
        0.0000025_f32.to_bits()
    );
    assert_eq!(
        canonical_learning_rate(1_000).unwrap().to_bits(),
        0.0025_f32.to_bits()
    );
    assert_eq!(
        canonical_learning_rate(TOTAL_UPDATES).unwrap().to_bits(),
        0.00025_f32.to_bits()
    );
}

#[test]
fn interrupted_update_cannot_publish_a_checkpoint() {
    let mut trainer =
        DeterministicTrainer::new(identity(), ScalarBackend::default(), vec![1], vec![2]).unwrap();
    assert!(trainer.process_batch(&batch(0, 2_048)).unwrap().is_none());
    assert_eq!(
        trainer.snapshot().unwrap_err().code,
        "P12_MID_UPDATE_CHECKPOINT_REJECTED"
    );
}

#[test]
fn checkpoint_roundtrip_and_resume_are_byte_identical() {
    let mut uninterrupted =
        DeterministicTrainer::new(identity(), ScalarBackend::default(), vec![1, 2], vec![3, 4])
            .unwrap();
    let first = batch(0, TARGETS_PER_FULL_UPDATE);
    let first_event = uninterrupted.process_batch(&first).unwrap().unwrap();
    assert_eq!(first_event.one_based_update, 1);
    assert!(!first_event.checkpoint_due);
    let checkpoint = uninterrupted.snapshot().unwrap();

    let directory = tempfile::tempdir().unwrap();
    let published = publish_checkpoint(directory.path(), &checkpoint).unwrap();
    let loaded = load_checkpoint(&published.generation_path).unwrap();
    assert_eq!(checkpoint, loaded);

    let second = batch(TARGETS_PER_FULL_UPDATE, TARGETS_PER_FULL_UPDATE);
    uninterrupted.process_batch(&second).unwrap().unwrap();
    let uninterrupted_state = uninterrupted.snapshot().unwrap();

    let mut resumed =
        DeterministicTrainer::from_snapshot(loaded, &identity(), ScalarBackend::default()).unwrap();
    resumed.process_batch(&second).unwrap().unwrap();
    let resumed_state = resumed.snapshot().unwrap();
    assert_eq!(uninterrupted_state, resumed_state);
    assert_eq!(
        state_bundle_sha256(&uninterrupted_state).unwrap(),
        state_bundle_sha256(&resumed_state).unwrap()
    );
}

#[test]
fn corruption_and_identity_drift_fail_closed() {
    let mut trainer =
        DeterministicTrainer::new(identity(), ScalarBackend::default(), vec![1], vec![2]).unwrap();
    trainer
        .process_batch(&batch(0, TARGETS_PER_FULL_UPDATE))
        .unwrap();
    let snapshot = trainer.snapshot().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let published = publish_checkpoint(directory.path(), &snapshot).unwrap();

    let mut mismatched = identity();
    mismatched.device_identity_sha256 = "ab".repeat(32);
    let loaded = load_checkpoint(&published.generation_path).unwrap();
    let error = DeterministicTrainer::from_snapshot(loaded, &mismatched, ScalarBackend::default())
        .err()
        .unwrap();
    assert_eq!(error.code, "P12_RESUME_IDENTITY_MISMATCH");

    std::fs::write(
        published.generation_path.join("state/runtime.bin"),
        b"corrupt",
    )
    .unwrap();
    assert_eq!(
        load_checkpoint(&published.generation_path)
            .unwrap_err()
            .code,
        "P12_CHECKPOINT_ARTIFACT_MISMATCH"
    );
}

#[test]
fn final_partial_update_reaches_exact_target_and_rejects_overshoot() {
    let snapshot = snapshot_before_final_update();
    snapshot.validate().unwrap();
    let mut trainer =
        DeterministicTrainer::from_snapshot(snapshot, &identity(), ScalarBackend::default())
            .unwrap();
    let first_target = FULL_UPDATES * TARGETS_PER_FULL_UPDATE;
    let event = trainer
        .process_batch(&batch(first_target, FINAL_UPDATE_TARGETS))
        .unwrap()
        .unwrap();
    assert_eq!(event.one_based_update, TOTAL_UPDATES);
    assert_eq!(event.consumed_targets, TOTAL_TARGETS);
    assert_eq!(event.valid_targets, FINAL_UPDATE_TARGETS);
    assert!(event.evaluation.is_some());
    assert!(event.checkpoint_due);
    assert!(event.training_complete);
    assert_eq!(trainer.snapshot().unwrap().consumed_targets, TOTAL_TARGETS);
    assert_eq!(
        trainer
            .process_batch(&batch(TOTAL_TARGETS, 1))
            .unwrap_err()
            .code,
        "P12_TARGET_OVERSHOOT"
    );
}

#[test]
fn retention_keeps_latest_two_and_first_generation_at_each_anchor() {
    let generations = (1..=20)
        .map(|index| index * EVENT_INTERVAL_TARGETS)
        .collect::<Vec<_>>();
    assert_eq!(
        retained_generation_targets(&generations),
        [
            500_000_000,
            1_000_000_000,
            1_500_000_000,
            1_900_000_000,
            2_000_000_000,
        ]
        .into_iter()
        .collect()
    );
}
#[test]
fn canonical_adamw_updates_fp32_masters_moments_and_bf16_storage() {
    let mut optimizer = CanonicalAdamw::new(vec![
        AdamwParameterState {
            name: "matrix.weight".to_owned(),
            weight_decay: true,
            master_weights: vec![1.0],
            parameter_bf16: vec![0x3f80],
            first_moments: vec![0.0],
            second_moments: vec![0.0],
        },
        AdamwParameterState {
            name: "norm.weight".to_owned(),
            weight_decay: false,
            master_weights: vec![1.0],
            parameter_bf16: vec![0x3f80],
            first_moments: vec![0.0],
            second_moments: vec![0.0],
        },
    ])
    .unwrap();
    let before = optimizer.state_sha256().unwrap();
    let after = optimizer
        .apply_normalized_clipped_gradients(&[0.5, 0.5], canonical_learning_rate(1).unwrap(), 1)
        .unwrap();
    assert_ne!(before, after);
    assert!(
        optimizer.parameters()[0].master_weights[0] < optimizer.parameters()[1].master_weights[0]
    );
    assert_ne!(optimizer.parameters()[0].first_moments[0], 0.0);
    assert_ne!(optimizer.parameters()[0].second_moments[0], 0.0);
}

#[test]
fn snapshots_require_every_evaluation_and_canonical_bf16_projection() {
    let mut incomplete = snapshot_before_final_update();
    incomplete.evaluations.pop();
    assert_eq!(
        incomplete.validate().unwrap_err().code,
        "P12_EVALUATION_SET_INCOMPLETE"
    );

    let error = CanonicalAdamw::new(vec![AdamwParameterState {
        name: "weight".to_owned(),
        weight_decay: true,
        master_weights: vec![1.0],
        parameter_bf16: vec![0],
        first_moments: vec![0.0],
        second_moments: vec![0.0],
    }])
    .unwrap_err();
    assert_eq!(error.code, "P12_OPTIMIZER_STATE_INVALID");
}

#[test]
fn p12_adds_no_receipt_or_manual_qualification_surface() {
    assert_eq!(train::IMPLEMENTATION_PHASE, "P12");
    assert_eq!(CanonicalTrainingPlan::canonical().total_updates, 30_518);
}

/// Clipping is not idempotent in f32, and the check that verifies it must survive
/// its own rounding.
///
/// The trainer clips by multiplying every gradient by `1/norm`; the state then
/// re-derives that scale from the result and rejects anything below one. Summing
/// `135,285,504` squares in f32 accumulates enough error that the re-derived
/// scale lands just under it, so the strict form rejected the first update whose
/// gradients actually needed clipping — which is what killed the first real
/// training run after five minutes, and would have killed the full one the same
/// way. Every prior test kept the norm under the bound, where the scale is
/// exactly one and the gradients pass through untouched, which is why nothing
/// caught it.
///
/// This pins the property that matters: a vector large enough to show the
/// rounding still verifies after being clipped once.
#[test]
fn a_clipped_gradient_still_verifies_at_production_width() {
    use rust_llm_pretrain::model::oracle::gradient_clip_scale;

    for count in [1_000_usize, 1_000_000, 10_000_000] {
        let mut gradients: Vec<f32> = (0..count)
            .map(|index| ((index % 977) as f32 - 488.0) * 1e-3)
            .collect();
        let scale = gradient_clip_scale(&gradients).expect("finite gradients");
        assert!(
            scale < 1.0,
            "the probe vector must exceed the clip bound, or it tests nothing"
        );
        for value in &mut gradients {
            *value *= scale;
        }
        let verified = gradient_clip_scale(&gradients).expect("finite after clipping");
        assert!(
            verified >= 1.0 - 0.01,
            "a clipped {count}-element gradient re-checked at {verified}, outside the tolerance \
             the state accepts"
        );
    }
}
