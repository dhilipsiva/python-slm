use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::error::Result;
use rust_llm_pretrain::model::CANONICAL_MODEL_ID;
use rust_llm_pretrain::storage::{CorpusSplit, SEQUENCE_TARGETS, TokenSequenceEntry};
use rust_llm_pretrain::train::trainer::{
    EVALUATION_TARGETS, TARGETS_PER_FULL_UPDATE, state_bundle_sha256,
};
use rust_llm_pretrain::train::{
    AsyncBatchTransfer, BackendStateArtifact, BatchGradient, DeterministicTrainer,
    EvaluationResult, LoadedSpan, LoaderCancellation, SpanLoader, SpanSource, TrainerBackend,
    TrainerIdentity, TrainingBatch, TransferPipeline, load_checkpoint, publish_checkpoint,
};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

const CPU_WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");
const CUDA_WORKFLOW: &str = include_str!("../.github/workflows/windows-cuda.yml");

struct SyntheticSource {
    entries: Vec<TokenSequenceEntry>,
}

impl SyntheticSource {
    fn two_updates() -> Self {
        let sequence_count = 2 * TARGETS_PER_FULL_UPDATE / SEQUENCE_TARGETS;
        let entries = (0..sequence_count)
            .map(|sequence| TokenSequenceEntry {
                split: CorpusSplit::Train,
                sequence,
                first_id: sequence * SEQUENCE_TARGETS,
                logical_ids: SEQUENCE_TARGETS + 1,
                valid_targets: SEQUENCE_TARGETS,
            })
            .collect();
        Self { entries }
    }
}

impl SpanSource for SyntheticSource {
    fn sequence_entries(&self) -> &[TokenSequenceEntry] {
        &self.entries
    }

    fn read_sequence(&self, _split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        Ok((0..=SEQUENCE_TARGETS)
            .map(|offset| ((sequence * 17 + offset) % u16::MAX as u64) as u16)
            .collect())
    }
}

#[derive(Default)]
struct TransferFacts {
    active: usize,
    maximum_active: usize,
    submitted: u64,
    waited: u64,
    cancelled: u64,
}

struct IdentityTransfer {
    facts: Arc<Mutex<TransferFacts>>,
}

impl AsyncBatchTransfer for IdentityTransfer {
    type Ticket = LoadedSpan;
    type DeviceBatch = LoadedSpan;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let mut facts = self.facts.lock().unwrap();
        facts.active += 1;
        facts.maximum_active = facts.maximum_active.max(facts.active);
        facts.submitted += 1;
        Ok(span)
    }

    fn wait(&mut self, ticket: Self::Ticket) -> Result<Self::DeviceBatch> {
        let mut facts = self.facts.lock().unwrap();
        facts.active -= 1;
        facts.waited += 1;
        Ok(ticket)
    }

    fn cancel(&mut self, _ticket: Self::Ticket) {
        let mut facts = self.facts.lock().unwrap();
        facts.active -= 1;
        facts.cancelled += 1;
    }
}

#[derive(Clone, Default)]
struct SyntheticBackend {
    state: u64,
}

impl SyntheticBackend {
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

impl TrainerBackend for SyntheticBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        let input_sum = batch
            .input_ids
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>();
        Ok(BatchGradient {
            loss_sum: batch.valid_targets as f64 * 0.25,
            gradient_sums: vec![batch.valid_targets as f32, input_sum as f32],
            host_rng_state: batch
                .first_target
                .wrapping_add(batch.valid_targets)
                .to_le_bytes()
                .to_vec(),
            device_rng_state: input_sum.to_le_bytes().to_vec(),
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
            .expect("the validated checkpoint has runtime state");
        self.state = u64::from_le_bytes(
            runtime.bytes[1..]
                .try_into()
                .expect("the validated runtime state has eight bytes"),
        );
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

#[test]
fn synthetic_loader_transfer_checkpoint_and_resume_are_end_to_end_exact() {
    let source = SyntheticSource::two_updates();
    let loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        4,
        LoaderCancellation::default(),
    )
    .unwrap();
    let facts = Arc::new(Mutex::new(TransferFacts::default()));
    let transfer = IdentityTransfer {
        facts: facts.clone(),
    };
    let mut pipeline = TransferPipeline::new(loader, transfer, 2).unwrap();
    let mut batches = Vec::new();
    while let Some(span) = pipeline.next_device_batch().unwrap() {
        batches.push(TrainingBatch::from_loaded_span(&span));
    }
    drop(pipeline);

    let facts = facts.lock().unwrap();
    assert_eq!(facts.active, 0);
    assert_eq!(facts.maximum_active, 2);
    assert_eq!(facts.submitted, 64);
    assert_eq!(facts.waited, 64);
    assert_eq!(facts.cancelled, 0);
    drop(facts);

    let mut uninterrupted =
        DeterministicTrainer::new(identity(), SyntheticBackend::default(), vec![1], vec![2])
            .unwrap();
    for batch in &batches[..32] {
        uninterrupted.process_batch(batch).unwrap();
    }
    assert_eq!(uninterrupted.completed_updates(), 1);
    let checkpoint = uninterrupted.snapshot().unwrap();
    let checkpoint_root = tempfile::tempdir().unwrap();
    let published = publish_checkpoint(checkpoint_root.path(), &checkpoint).unwrap();
    let loaded = load_checkpoint(&published.generation_path).unwrap();

    for batch in &batches[32..] {
        uninterrupted.process_batch(batch).unwrap();
    }
    let uninterrupted_state = uninterrupted.snapshot().unwrap();

    let mut resumed =
        DeterministicTrainer::from_snapshot(loaded, &identity(), SyntheticBackend::default())
            .unwrap();
    for batch in &batches[32..] {
        resumed.process_batch(batch).unwrap();
    }
    let resumed_state = resumed.snapshot().unwrap();
    assert_eq!(resumed.completed_updates(), 2);
    assert_eq!(resumed.consumed_targets(), 2 * TARGETS_PER_FULL_UPDATE);
    assert_eq!(uninterrupted_state, resumed_state);
    assert_eq!(
        state_bundle_sha256(&uninterrupted_state).unwrap(),
        state_bundle_sha256(&resumed_state).unwrap()
    );
}

#[test]
fn required_ci_is_read_only_pinned_and_runs_the_phase_contracts() {
    for expected in [
        "permissions:\n  contents: read",
        "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683",
        "rustup toolchain install 1.96.0",
        "fetch-depth: 0",
        "runs-on: windows-latest",
        "cargo fmt --all -- --check",
        "cargo clippy --locked --workspace --all-targets --offline -- -D warnings",
        "live_xtask_runtime_and_build_closure_is_zero_python_and_native_link_free",
        "cargo test --locked --features cpu-reference --offline",
        "cargo check --locked --no-default-features --offline",
        "cargo test --locked -p xtask --features p2-cuda --no-run --offline",
        "cargo check --locked --no-default-features --features cuda --offline",
        "git status --porcelain=v1 --untracked-files=all",
        "required-ci-does-not-use-self-hosted-hardware",
    ] {
        assert!(
            CPU_WORKFLOW.contains(expected),
            "missing CI contract: {expected}"
        );
    }
    assert!(!CPU_WORKFLOW.contains("pull_request_target"));
    assert!(!CPU_WORKFLOW.contains("continue-on-error"));
    assert!(!CPU_WORKFLOW.contains("docs/receipts"));
}

#[test]
fn cuda_lane_defaults_to_unverified_and_cannot_run_untrusted_pr_code() {
    for expected in [
        "workflow_dispatch:",
        "default: false",
        "status\":\"UNVERIFIED",
        "inputs.run_hardware == true",
        "github.ref == 'refs/heads/main'",
        "runs-on: [self-hosted, Windows, X64, cuda, rtx-5090]",
        "persist-credentials: false",
        "RUNNER_TEMP",
        "probe-cuda",
        "select-backend",
        "qualification_status\":\"SKIPPED",
        "if: always()",
    ] {
        assert!(
            CUDA_WORKFLOW.contains(expected),
            "missing CUDA CI contract: {expected}"
        );
    }
    assert!(!CUDA_WORKFLOW.contains("pull_request:"));
    assert!(!CUDA_WORKFLOW.contains("pull_request_target"));
    assert!(!CUDA_WORKFLOW.contains("continue-on-error"));
    assert!(!CUDA_WORKFLOW.contains("docs/receipts"));
    assert!(!CUDA_WORKFLOW.contains("secrets."));
    assert!(!CUDA_WORKFLOW.contains("${{ runner.temp }}"));
}
