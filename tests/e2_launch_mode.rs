//! E2 final-run launch mode contracts.
//!
//! The orchestration is exercised end to end against a stub backend and the
//! resumed final update, which is the same shape the P16 suite uses and the only
//! way to reach the two-billionth target inside a test budget. The corpus-backed
//! batch source is exercised directly, because its job — turning a contiguous span
//! stream into the coordinator's exact micro-batch windows, and landing a resume
//! on a span boundary — is where the accounting can silently go wrong.

use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::error::Result;
use rust_llm_pretrain::model::CANONICAL_MODEL_ID;
use rust_llm_pretrain::storage::{CorpusSplit, SEQUENCE_TARGETS, TokenSequenceEntry};
use rust_llm_pretrain::train::launch::{
    CorpusBatchSource, FinalRunLaunchV1, HostStagedTransfer, LAUNCH_CONFIG_SCHEMA, SlaClock,
    SlaObservationV1, execute_launched_run, parse_launch_configuration, read_validation_set,
};
use rust_llm_pretrain::train::trainer::{
    BackendStateArtifact, BatchGradient, CanonicalTrainingPlan, CompletedEvaluation,
    DeterministicTrainer, EVALUATION_TARGETS, EVENT_INTERVAL_TARGETS, EvaluationResult,
    FULL_UPDATES, TARGETS_PER_FULL_UPDATE, TOTAL_TARGETS, TOTAL_UPDATES, TRAINER_SNAPSHOT_SCHEMA,
    TrainerBackend, TrainerIdentity, TrainerSnapshot, TrainingBatch, canonical_learning_rate,
};
use rust_llm_pretrain::train::{
    AsyncBatchTransfer, FinalBatchSource, LoaderCancellation, PrototypeTrainingDefaultsV1,
    SpanSource,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// A contiguous synthetic split whose tokens are a function of their absolute
/// position, so a batch can be checked against the stream it should have come
/// from rather than against itself.
struct SyntheticCorpus {
    entries: Vec<TokenSequenceEntry>,
}

impl SyntheticCorpus {
    /// `spans` full spans of `SEQUENCE_TARGETS` targets, optionally followed by a
    /// short one, mirroring the canonical stream's ragged tail.
    fn new(train_spans: u64, tail_targets: u64, validation_spans: u64) -> Self {
        let mut entries = Vec::new();
        let mut push = |split: CorpusSplit, sequence: u64, first_id: u64, valid_targets: u64| {
            entries.push(TokenSequenceEntry {
                split,
                sequence,
                first_id,
                logical_ids: valid_targets + 1,
                valid_targets,
            });
        };
        let mut first_id = 0;
        for sequence in 0..train_spans {
            push(CorpusSplit::Train, sequence, first_id, SEQUENCE_TARGETS);
            first_id += SEQUENCE_TARGETS;
        }
        if tail_targets > 0 {
            push(CorpusSplit::Train, train_spans, first_id, tail_targets);
        }
        let mut first_id = 0;
        for sequence in 0..validation_spans {
            push(
                CorpusSplit::Validation,
                sequence,
                first_id,
                SEQUENCE_TARGETS,
            );
            first_id += SEQUENCE_TARGETS;
        }
        Self { entries }
    }

    fn token_for(split: CorpusSplit, absolute_id: u64) -> u16 {
        let salt = match split {
            CorpusSplit::Train => 0,
            CorpusSplit::Validation => 1,
            CorpusSplit::Test => 2,
        };
        ((absolute_id.wrapping_mul(2_654_435_761).wrapping_add(salt)) % 32_000) as u16
    }
}

impl SpanSource for SyntheticCorpus {
    fn sequence_entries(&self) -> &[TokenSequenceEntry] {
        &self.entries
    }

    fn read_sequence(&self, split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.split == split && entry.sequence == sequence)
            .expect("the loader only asks for indexed sequences");
        Ok((0..entry.logical_ids)
            .map(|offset| Self::token_for(split, entry.first_id + offset))
            .collect())
    }
}

#[derive(Clone, Default)]
struct StubBackend {
    state: u64,
}

impl StubBackend {
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

impl TrainerBackend for StubBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        Ok(BatchGradient {
            loss_sum: batch.valid_targets as f64 * 0.5,
            gradient_sums: vec![batch.valid_targets as f32, 0.25],
            host_rng_state: batch.first_target.to_le_bytes().to_vec(),
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
        hasher.update(b"python-slm/e2-test-evaluation/v1\0");
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

fn completed_evaluations(consumed_targets: u64) -> Vec<CompletedEvaluation> {
    let evaluation = |after_targets: u64, before_first_update: bool| CompletedEvaluation {
        after_targets,
        before_first_update,
        result: EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 1.0,
            result_sha256: "aa".repeat(32),
        },
    };
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
        backend_state: StubBackend { state: 41 }.artifacts(),
    }
}

fn launch_configuration() -> FinalRunLaunchV1 {
    FinalRunLaunchV1 {
        schema: LAUNCH_CONFIG_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        token_generation_root: absolute("tokens"),
        checkpoint_root: absolute("checkpoints"),
        resume_from_generation: None,
        device_ordinal: 0,
        confirm_full_run: true,
        diagnostic_target_budget: None,
    }
}

fn absolute(leaf: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:\\python-slm\\{leaf}"))
    } else {
        PathBuf::from(format!("/python-slm/{leaf}"))
    }
}

/// A stub that answers whatever window it is asked for, so the orchestration can
/// be driven to the two-billionth target without a corpus.
#[derive(Default)]
struct WindowSource {
    requested: Vec<(u64, u64)>,
}

impl FinalBatchSource for WindowSource {
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

#[test]
fn the_launch_configuration_is_closed_and_must_be_confirmed() {
    let json = serde_json::to_string(&launch_configuration()).unwrap();
    let parsed = parse_launch_configuration(json.as_bytes()).unwrap();
    assert_eq!(parsed, launch_configuration());

    // Unknown fields are rejected rather than ignored.
    let widened = json.replace("{", "{\"extra\":1,");
    assert_eq!(
        parse_launch_configuration(widened.as_bytes())
            .unwrap_err()
            .code,
        "E2_LAUNCH_CONFIG_INVALID"
    );

    let mut foreign = launch_configuration();
    foreign.schema = "python-slm-final-run-launch-v2".to_owned();
    assert_eq!(
        foreign.validate().unwrap_err().code,
        "E2_LAUNCH_CONFIG_INVALID"
    );

    let mut other_profile = launch_configuration();
    other_profile.profile = "linux-mi300x-v1".to_owned();
    assert_eq!(
        other_profile.validate().unwrap_err().code,
        "E2_LAUNCH_CONFIG_INVALID"
    );

    // A tens-of-hours execution must never start from a default-shaped file.
    let mut unconfirmed = launch_configuration();
    unconfirmed.confirm_full_run = false;
    assert_eq!(
        unconfirmed.validate().unwrap_err().code,
        "E2_LAUNCH_NOT_CONFIRMED"
    );

    let mut relative = launch_configuration();
    relative.checkpoint_root = PathBuf::from("checkpoints");
    assert_eq!(
        relative.validate().unwrap_err().code,
        "E2_LAUNCH_PATH_NOT_ABSOLUTE"
    );
}

#[test]
fn the_batch_source_fills_the_exact_requested_window_from_contiguous_spans() {
    let corpus = SyntheticCorpus::new(48, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();

    let micro_batch = configuration.batch.micro_batch_targets;
    assert_eq!(micro_batch, SEQUENCE_TARGETS * 16);

    for index in 0..3 {
        let first_target = index * micro_batch;
        let batch = source.next_batch(first_target, micro_batch).unwrap();
        assert_eq!(batch.first_target, first_target);
        assert_eq!(batch.valid_targets, micro_batch);
        assert_eq!(batch.input_ids.len() as u64, micro_batch);
        // Sixteen whole spans, never one fused or padded sequence: the backend
        // dispatches on these boundaries.
        assert_eq!(batch.sequence_lengths, vec![SEQUENCE_TARGETS; 16]);
        assert_eq!(batch.sequences().len(), 16);

        // The tokens are the canonical stream at this cursor, not merely
        // self-consistent.
        for offset in [0_u64, 1, micro_batch - 1] {
            let absolute = first_target + offset;
            assert_eq!(
                batch.input_ids[offset as usize],
                SyntheticCorpus::token_for(CorpusSplit::Train, absolute)
            );
            assert_eq!(
                batch.target_ids[offset as usize],
                SyntheticCorpus::token_for(CorpusSplit::Train, absolute + 1)
            );
        }
    }
    assert_eq!(source.next_first_target(), 3 * micro_batch);
}

/// The final update is 37,888 targets: one full micro-batch and then 5,120, which
/// is two full spans plus the canonical 1,024-target tail. Padding it would
/// corrupt valid-target accounting, so the source must return it exactly.
#[test]
fn the_ragged_final_window_stays_exact_rather_than_padded() {
    let corpus = SyntheticCorpus::new(18, 1_024, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();

    let full = source.next_batch(0, 32_768).unwrap();
    assert_eq!(full.sequence_lengths, vec![SEQUENCE_TARGETS; 16]);

    let tail = source.next_batch(32_768, 5_120).unwrap();
    assert_eq!(tail.valid_targets, 5_120);
    assert_eq!(
        tail.sequence_lengths,
        vec![SEQUENCE_TARGETS, SEQUENCE_TARGETS, 1_024]
    );
    assert_eq!(tail.input_ids.len(), 5_120);
    assert_eq!(32_768 + 5_120, 37_888);
}

#[test]
fn a_window_that_is_not_a_whole_number_of_spans_fails_closed() {
    let corpus = SyntheticCorpus::new(8, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();
    assert_eq!(
        source.next_batch(0, 100).unwrap_err().code,
        "E2_MICRO_BATCH_NOT_SPAN_ALIGNED"
    );
}

#[test]
fn the_source_rejects_a_window_it_is_not_positioned_at() {
    let corpus = SyntheticCorpus::new(8, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();
    assert_eq!(
        source
            .next_batch(SEQUENCE_TARGETS, SEQUENCE_TARGETS)
            .unwrap_err()
            .code,
        "E2_BATCH_CURSOR_MISMATCH"
    );
}

#[test]
fn resume_fast_forwards_to_the_exact_cursor_and_continues_the_stream() {
    let corpus = SyntheticCorpus::new(48, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();

    let resume_at = SEQUENCE_TARGETS * 32;
    assert_eq!(source.fast_forward_to(resume_at).unwrap(), 32);
    assert_eq!(source.next_first_target(), resume_at);

    // Continuation after the skip is the same stream an uninterrupted run would
    // have produced at this cursor.
    let batch = source.next_batch(resume_at, SEQUENCE_TARGETS * 16).unwrap();
    assert_eq!(batch.first_target, resume_at);
    assert_eq!(
        batch.input_ids[0],
        SyntheticCorpus::token_for(CorpusSplit::Train, resume_at)
    );

    let mut uninterrupted =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();
    for index in 0..2 {
        uninterrupted
            .next_batch(index * SEQUENCE_TARGETS * 16, SEQUENCE_TARGETS * 16)
            .unwrap();
    }
    let straight_through = uninterrupted
        .next_batch(resume_at, SEQUENCE_TARGETS * 16)
        .unwrap();
    assert_eq!(batch, straight_through);
}

#[test]
fn a_resume_cursor_inside_a_span_fails_closed() {
    let corpus = SyntheticCorpus::new(8, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();
    assert_eq!(
        source
            .fast_forward_to(SEQUENCE_TARGETS + 1)
            .unwrap_err()
            .code,
        "E2_RESUME_CURSOR_NOT_ON_SPAN_BOUNDARY"
    );
}

#[test]
fn a_training_stream_shorter_than_the_plan_fails_closed() {
    let corpus = SyntheticCorpus::new(2, 0, 0);
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default()).unwrap();
    assert_eq!(
        source.next_batch(0, 32_768).unwrap_err().code,
        "E2_TRAINING_STREAM_EXHAUSTED"
    );
}

#[test]
fn staged_spans_retire_in_submission_order() {
    let corpus = SyntheticCorpus::new(4, 0, 0);
    let mut loader = rust_llm_pretrain::train::SpanLoader::new(
        &corpus,
        CorpusSplit::Train,
        4,
        LoaderCancellation::default(),
    )
    .unwrap();
    let mut transfer = HostStagedTransfer::new();
    let first = transfer
        .submit(loader.next_span().unwrap().unwrap())
        .unwrap();
    let second = transfer
        .submit(loader.next_span().unwrap().unwrap())
        .unwrap();

    // Out-of-order retirement would silently reorder the target stream.
    assert_eq!(
        transfer.wait(second).unwrap_err().code,
        "E2_STAGING_RETIREMENT_ORDER_INVALID"
    );
    assert_eq!(transfer.wait(first).unwrap().first_id, 0);
}

#[test]
fn the_validation_set_covers_the_mandatory_targets_exactly() {
    // 1,000,000 targets is not a whole number of 2,048-target spans, so the last
    // one has to be truncated rather than the set rounded up.
    let spans = EVALUATION_TARGETS.div_ceil(SEQUENCE_TARGETS);
    let corpus = SyntheticCorpus::new(1, 0, spans);
    let dimensions = rust_llm_pretrain::train::full_state::GqaDimensions::canonical();
    let set = read_validation_set(&corpus, &dimensions, LoaderCancellation::default()).unwrap();
    let covered = set
        .spans()
        .iter()
        .map(|span| span.input_ids.len() as u64)
        .sum::<u64>();
    assert_eq!(covered, EVALUATION_TARGETS);
    assert_eq!(set.spans().len() as u64, spans);
    assert_eq!(
        set.spans().last().unwrap().input_ids.len() as u64,
        EVALUATION_TARGETS - (spans - 1) * SEQUENCE_TARGETS
    );

    let short = SyntheticCorpus::new(1, 0, 2);
    assert_eq!(
        read_validation_set(&short, &dimensions, LoaderCancellation::default())
            .unwrap_err()
            .code,
        "E2_VALIDATION_STREAM_EXHAUSTED"
    );
}

#[test]
fn the_sla_clock_is_monotonic_and_classifies_against_the_frozen_limit() {
    let clock = SlaClock::start().unwrap();
    let first = clock.elapsed_ns().unwrap();
    let second = clock.elapsed_ns().unwrap();
    assert!(second >= first, "the SLA clock moved backwards");

    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let limit = configuration.sla.actual_elapsed_limit_ns;
    assert_eq!(limit, 28_800_000_000_000);
    assert_eq!(
        SlaObservationV1::measured(limit, &configuration).completion_sla_status,
        "MET"
    );
    assert_eq!(
        SlaObservationV1::measured(limit + 1, &configuration).completion_sla_status,
        "EXCEEDED"
    );
    assert_eq!(
        SlaObservationV1::measured(0, &configuration).clock,
        "suspend-inclusive-monotonic-v1"
    );
}

/// The clock must tick in nanoseconds at wall-clock rate.
///
/// Windows reports interrupt time in 100-nanosecond units, so a missing or extra
/// factor of one hundred would misreport an eight-hour SLA by two orders of
/// magnitude while still looking perfectly monotonic. Comparing against `Instant`
/// over a short interval catches exactly that: the two clocks disagree only about
/// suspend, and nothing suspends inside this test.
#[test]
fn the_sla_clock_advances_at_wall_clock_rate() {
    use rust_llm_pretrain::platform::suspend_inclusive_now_ns;

    let instant = std::time::Instant::now();
    let started = suspend_inclusive_now_ns().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    let suspend_inclusive = suspend_inclusive_now_ns().unwrap() - started;
    let reference = instant.elapsed().as_nanos() as u64;

    // Generous bounds: this only has to catch a unit error, and the interrupt-time
    // counter has coarse (roughly 15 ms) resolution on Windows.
    let ratio = suspend_inclusive as f64 / reference as f64;
    assert!(
        (0.5..2.0).contains(&ratio),
        "suspend-inclusive clock ran at {ratio:.4}x wall clock \
         ({suspend_inclusive} ns against {reference} ns)"
    );
}

#[test]
fn a_launched_run_completes_the_plan_and_reports_a_measured_sla() {
    let mut trainer = DeterministicTrainer::from_snapshot(
        snapshot_before_final_update(),
        &identity(),
        StubBackend::default(),
    )
    .unwrap();
    let mut source = WindowSource::default();
    let checkpoint_root = tempfile::tempdir().unwrap();
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let launch = launch_configuration();

    let result = execute_launched_run(
        &mut trainer,
        &mut source,
        checkpoint_root.path(),
        &configuration,
        &launch,
    )
    .unwrap();

    assert_eq!(result.schema, "python-slm-final-run-launch-result-v1");
    assert_eq!(result.status, "TRAINING_COMPLETE");
    assert_eq!(result.execution_status, "EXECUTED");
    assert_eq!(result.qualification_status, "SKIPPED");
    assert_eq!(result.consumed_targets, TOTAL_TARGETS);
    assert_eq!(result.completed_updates, TOTAL_UPDATES);
    assert_eq!(
        result.resumed_from_consumed_targets,
        FULL_UPDATES * TARGETS_PER_FULL_UPDATE
    );
    assert_eq!(result.published_checkpoints, 1);
    assert_eq!(result.retained_checkpoint_targets, [TOTAL_TARGETS]);
    assert!(result.same_process_reload_exact);
    assert_eq!(result.final_checkpoint_generation, "00000000002000000000");
    assert_eq!(
        source.requested,
        [(1_999_962_112, 32_768), (1_999_994_880, 5_120)]
    );

    // The tail took milliseconds, so the measured elapsed time is inside the
    // frozen limit; that is a measurement of this execution, not a claim that the
    // canonical run meets the SLA.
    assert_eq!(result.sla.clock, "suspend-inclusive-monotonic-v1");
    assert_eq!(result.sla.completion_sla_seconds, 28_800);
    assert_eq!(result.sla.admission_seconds, 25_920);
    assert_eq!(result.sla.completion_sla_status, "MET");
    assert_eq!(result.claims.completion_sla, "MET");

    // Execution never upgrades the claims it cannot establish.
    assert_eq!(result.claims.final_loss, "UNVERIFIED");
    assert_eq!(result.claims.full_run_completion, "OBSERVED_UNVERIFIED");
    assert!(
        result
            .limitations
            .contains(&"not-hardware-qualification".to_owned())
    );
    assert!(
        result
            .limitations
            .contains(&"not-final-model-quality".to_owned())
    );

    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("receipts"));
    assert!(!json.contains("pointer"));
}

/// The provider wiring itself, which the unit tests above cannot reach because
/// they supply their own trainer and source.
///
/// An unconfirmed launch configuration fails before any device, corpus, or
/// checkpoint is touched, so this covers that `launch_final_run` is reachable and
/// validates in the right order without needing a corpus or a GPU.
#[cfg(feature = "cuda")]
#[test]
fn the_launch_entry_point_validates_before_touching_a_device() {
    use std::io::Write;

    let defaults = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/train/prototype-windows-5090-v1.defaults.json")
        .canonicalize()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let launch_path = directory.path().join("launch.json");
    let mut unconfirmed = launch_configuration();
    unconfirmed.confirm_full_run = false;
    let mut file = std::fs::File::create(&launch_path).unwrap();
    file.write_all(&serde_json::to_vec(&unconfirmed).unwrap())
        .unwrap();
    drop(file);

    assert_eq!(
        rust_llm_pretrain::train::launch::launch_final_run(&defaults, &launch_path)
            .unwrap_err()
            .code,
        "E2_LAUNCH_NOT_CONFIRMED"
    );
}

#[test]
fn an_unconfirmed_launch_never_reaches_execution() {
    let mut trainer = DeterministicTrainer::from_snapshot(
        snapshot_before_final_update(),
        &identity(),
        StubBackend::default(),
    )
    .unwrap();
    let mut source = WindowSource::default();
    let checkpoint_root = tempfile::tempdir().unwrap();
    let mut launch = launch_configuration();
    launch.confirm_full_run = false;

    assert_eq!(
        execute_launched_run(
            &mut trainer,
            &mut source,
            checkpoint_root.path(),
            &PrototypeTrainingDefaultsV1::canonical(),
            &launch,
        )
        .unwrap_err()
        .code,
        "E2_LAUNCH_NOT_CONFIRMED"
    );
    assert!(source.requested.is_empty());
}

/// The launch scope must be stated, never inferred.
///
/// A bounded run trains a fiftieth of the frozen target count, so the two ways of
/// getting one by accident — omitting both fields, or setting both — are the two
/// this checks. Neither is a default; both are typed rejections.
#[test]
fn launch_scope_must_be_stated_exactly_once() {
    let full = launch_configuration();
    assert!(full.validate().is_ok(), "the frozen run stays valid");

    let mut neither = launch_configuration();
    neither.confirm_full_run = false;
    assert_eq!(
        neither.validate().unwrap_err().code,
        "E2_LAUNCH_NOT_CONFIRMED",
        "a configuration that states no scope is refused"
    );

    let mut both = launch_configuration();
    both.diagnostic_target_budget = Some(56_000_000);
    assert_eq!(
        both.validate().unwrap_err().code,
        "E2_LAUNCH_SCOPE_AMBIGUOUS",
        "acknowledging the frozen run while declining it is refused"
    );

    let mut bounded = launch_configuration();
    bounded.confirm_full_run = false;
    bounded.diagnostic_target_budget = Some(56_000_000);
    assert!(
        bounded.validate().is_ok(),
        "a bounded run that declines the frozen acknowledgement is valid"
    );
}
