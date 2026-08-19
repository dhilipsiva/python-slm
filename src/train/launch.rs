//! E2 final-run launch mode: the explicit execution surface.
//!
//! `train --config <defaults>` remains an inspection boundary that runs nothing,
//! and `--verify-final-checkpoint` remains a reload check. This module adds the
//! third, explicitly requested mode that actually trains: it opens the verified
//! P8 token generation, drives the P11 span loader and transfer ring into
//! `execute_to_completion` on the E1 accelerator backend, and measures the run on
//! a suspend-inclusive monotonic clock so an overnight suspend counts against the
//! completion SLA instead of hiding inside it.
//!
//! Nothing here decides SLA policy. The frozen `25,920`-second admission ceiling
//! and `28,800`-second completion SLA are contract values, so this module reports
//! the measured elapsed time and whether it fell inside the limit, and leaves
//! admission to E5. A run that overruns is reported as overrunning, not stopped
//! partway and not quietly relabelled.

use super::final_run::{
    FinalBatchSource, FinalRunClaimStatusV1, execute_bounded_diagnostic, execute_to_completion,
};
use super::profile::PrototypeTrainingDefaultsV1;
use super::trainer::{
    DeterministicTrainer, TOTAL_TARGETS, TrainerBackend, TrainerIdentity, TrainingBatch,
    state_bundle_sha256,
};
use super::{
    AsyncBatchTransfer, LoadedSpan, LoaderCancellation, SpanLoader, SpanSource, TransferPipeline,
};
use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use crate::platform::{SUSPEND_INCLUSIVE_CLOCK, suspend_inclusive_now_ns};
use crate::storage::CorpusSplit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

pub const IMPLEMENTATION_PHASE: &str = "E2";
pub const LAUNCH_CONFIG_SCHEMA: &str = "python-slm-final-run-launch-v1";
pub const LAUNCH_RESULT_SCHEMA: &str = "python-slm-final-run-launch-result-v1";
pub const EXECUTION_SURFACE: &str = "explicit-final-run-launch-mode";

/// The closed launch configuration.
///
/// The frozen `prototype-windows-5090-v1.defaults.json` is byte-pinned in two
/// places and `validate()` requires byte equality with `canonical()`, so it can
/// never carry a path or a device ordinal. Everything an actual execution needs
/// beyond the frozen defaults lives here instead, in its own versioned closed
/// schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalRunLaunchV1 {
    pub schema: String,
    pub profile: String,
    /// Absolute path to the hash-verified P8 token generation root.
    pub token_generation_root: PathBuf,
    /// Absolute path under which checkpoint generations are published create-new.
    pub checkpoint_root: PathBuf,
    /// Absolute path to a published generation to resume from, or null to begin
    /// a fresh run from INIT-001.
    pub resume_from_generation: Option<PathBuf>,
    pub device_ordinal: u64,
    /// Explicit acknowledgement that this launches the full two-billion-target
    /// run. Requiring it is what keeps the execution mode from being reachable by
    /// a stray flag: the measured projection for this run is tens of hours.
    pub confirm_full_run: bool,
    /// Stop after this many valid targets instead of running the frozen count.
    ///
    /// `null` is the frozen run. A value is a bounded diagnostic: same model, same
    /// corpus, same arithmetic, stopped early, and never described as complete.
    /// It is required rather than defaulted because a launch that silently ran a
    /// fiftieth of the contract would be the worst possible thing to get by
    /// omission, and because this schema has no hidden defaults anywhere else.
    ///
    /// Exactly one of this and `confirm_full_run` may be set: the acknowledgement
    /// is about starting a run of tens of hours, and a bounded run is not that.
    pub diagnostic_target_budget: Option<u64>,
}

impl FinalRunLaunchV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != LAUNCH_CONFIG_SCHEMA || self.profile != PROTOTYPE_PROFILE {
            return Err(ProductError::usage(
                "E2_LAUNCH_CONFIG_INVALID",
                "the launch configuration is not the closed prototype launch schema",
            ));
        }
        match (self.confirm_full_run, self.diagnostic_target_budget) {
            (true, None) | (false, Some(_)) => {}
            (false, None) => {
                return Err(ProductError::usage(
                    "E2_LAUNCH_NOT_CONFIRMED",
                    "the launch configuration must set confirm_full_run to start the canonical \
                     run, or a diagnostic_target_budget to run a bounded diagnostic",
                ));
            }
            (true, Some(_)) => {
                return Err(ProductError::usage(
                    "E2_LAUNCH_SCOPE_AMBIGUOUS",
                    "confirm_full_run acknowledges the frozen two-billion-target run and a \
                     diagnostic_target_budget declines it; setting both states no scope at all",
                ));
            }
        }
        for path in [&self.token_generation_root, &self.checkpoint_root] {
            require_absolute(path)?;
        }
        if let Some(generation) = &self.resume_from_generation {
            require_absolute(generation)?;
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self).map_err(|_| {
            ProductError::internal(
                "E2_LAUNCH_CONFIG_SERIALIZE_FAILED",
                "could not canonicalize the launch configuration",
            )
        })?;
        Ok(hex::encode(Sha256::digest(&bytes)))
    }
}

fn require_absolute(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Ok(());
    }
    Err(ProductError::usage(
        "E2_LAUNCH_PATH_NOT_ABSOLUTE",
        "every launch path must be absolute",
    ))
}

pub fn parse_launch_configuration(bytes: &[u8]) -> Result<FinalRunLaunchV1> {
    let configuration: FinalRunLaunchV1 = serde_json::from_slice(bytes).map_err(|error| {
        ProductError::usage(
            "E2_LAUNCH_CONFIG_INVALID",
            format!("the launch configuration is not the closed schema: {error}"),
        )
    })?;
    configuration.validate()?;
    Ok(configuration)
}

/// Wall-clock measurement for one execution, on a clock that survives suspend.
pub struct SlaClock {
    started_ns: u64,
}

impl SlaClock {
    pub fn start() -> Result<Self> {
        Ok(Self {
            started_ns: suspend_inclusive_now_ns()?,
        })
    }

    pub fn elapsed_ns(&self) -> Result<u64> {
        let now = suspend_inclusive_now_ns()?;
        now.checked_sub(self.started_ns).ok_or_else(|| {
            ProductError::internal(
                "SLA_CLOCK_NOT_MONOTONIC",
                "the suspend-inclusive clock moved backwards during the run",
            )
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlaObservationV1 {
    pub clock: String,
    pub elapsed_ns: u64,
    pub admission_seconds: u64,
    pub completion_sla_seconds: u64,
    pub actual_elapsed_limit_ns: u64,
    pub completion_sla_status: String,
}

impl SlaObservationV1 {
    pub fn measured(elapsed_ns: u64, configuration: &PrototypeTrainingDefaultsV1) -> Self {
        let limit = configuration.sla.actual_elapsed_limit_ns;
        Self {
            clock: SUSPEND_INCLUSIVE_CLOCK.to_owned(),
            elapsed_ns,
            admission_seconds: configuration.sla.admission_seconds,
            completion_sla_seconds: configuration.sla.completion_seconds,
            actual_elapsed_limit_ns: limit,
            completion_sla_status: if elapsed_ns <= limit {
                "MET".to_owned()
            } else {
                "EXCEEDED".to_owned()
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalRunLaunchResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub execution_status: String,
    pub execution_surface: String,
    pub configuration_sha256: String,
    pub launch_sha256: String,
    pub implementation_sha256: String,
    pub model_identity: String,
    pub resumed_from_consumed_targets: u64,
    pub consumed_targets: u64,
    pub completed_updates: u64,
    pub published_checkpoints: u64,
    pub retained_checkpoint_targets: Vec<u64>,
    pub final_checkpoint_generation: String,
    pub trainer_state_sha256: String,
    pub same_process_reload_exact: bool,
    pub sla: SlaObservationV1,
    pub claims: FinalRunClaimStatusV1,
    /// Held-out loss for the state a bounded run produced, as little-endian f64
    /// hex. Absent for the full run, whose evaluation schedule already covers
    /// every boundary it defines. Hex rather than a float because this schema is
    /// compared for equality, which `f64` does not support, and because it is how
    /// every other loss in these results is already carried.
    pub final_evaluation_loss_f64_le_hex: Option<String>,
    pub final_evaluation_targets: Option<u64>,
    pub limitations: Vec<String>,
}

/// Host-side staging for the P11 ring.
///
/// The ring is generic over how a span reaches the device, and on this lane the
/// answer is that it does not reach it here: `TrainerBackend::accumulate` takes a
/// host-side `TrainingBatch`, and the E1 graph uploads the tokens itself when it
/// builds the index tensors. Submitting the same 4 KB span through
/// `CudaPinnedTransfer` as well would perform a device copy that nothing ever
/// reads. What the ring genuinely provides on this lane is bounded in-flight
/// ownership and prefetch that overlaps span I/O with compute, and that is what
/// this staging preserves. Retirement stays in exact submission order, because
/// `first_target` is a single monotone cursor over the canonical target stream.
#[derive(Default)]
pub struct HostStagedTransfer {
    submitted: u64,
    retired: u64,
}

impl HostStagedTransfer {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct StagedTicket {
    order: u64,
    span: LoadedSpan,
}

impl AsyncBatchTransfer for HostStagedTransfer {
    type Ticket = StagedTicket;
    type DeviceBatch = LoadedSpan;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let order = self.submitted;
        self.submitted = self.submitted.checked_add(1).ok_or_else(|| {
            ProductError::integrity(
                "E2_STAGING_SUBMISSION_OVERFLOW",
                "the staging submission counter overflowed",
            )
        })?;
        Ok(StagedTicket { order, span })
    }

    fn wait(&mut self, ticket: Self::Ticket) -> Result<Self::DeviceBatch> {
        if ticket.order != self.retired {
            return Err(ProductError::integrity(
                "E2_STAGING_RETIREMENT_ORDER_INVALID",
                "staged spans must retire in exact submission order",
            ));
        }
        self.retired += 1;
        Ok(ticket.span)
    }

    fn cancel(&mut self, ticket: Self::Ticket) {
        self.retired = self.retired.max(ticket.order + 1);
    }
}

/// The canonical training stream, presented as exact micro-batches.
///
/// The coordinator asks for a precise `(first_target, valid_targets)` window and
/// this fuses whole spans to fill it. Spans are never split: the frozen
/// accounting makes every window a whole number of spans, including the ragged
/// final update of eighteen full spans plus one 1,024-target span, so a window
/// that cannot be filled exactly is an accounting defect and fails closed rather
/// than being padded.
pub struct CorpusBatchSource<'a, S: SpanSource + ?Sized> {
    pipeline: TransferPipeline<'a, S, HostStagedTransfer>,
    pending: VecDeque<LoadedSpan>,
    next_first_target: u64,
}

impl<'a, S: SpanSource + ?Sized> CorpusBatchSource<'a, S> {
    pub fn new(
        source: &'a S,
        configuration: &PrototypeTrainingDefaultsV1,
        cancellation: LoaderCancellation,
    ) -> Result<Self> {
        let loader = SpanLoader::new(
            source,
            CorpusSplit::Train,
            usize::try_from(configuration.loader.host_buffer_spans)
                .map_err(|_| capacity_error())?,
            cancellation,
        )?;
        let pipeline = TransferPipeline::new(
            loader,
            HostStagedTransfer::new(),
            usize::try_from(configuration.loader.maximum_in_flight_transfers)
                .map_err(|_| capacity_error())?,
        )?;
        Ok(Self {
            pipeline,
            pending: VecDeque::new(),
            next_first_target: 0,
        })
    }

    pub fn next_first_target(&self) -> u64 {
        self.next_first_target
    }

    fn pull_span(&mut self) -> Result<LoadedSpan> {
        if let Some(span) = self.pending.pop_front() {
            return Ok(span);
        }
        self.pipeline.next_device_batch()?.ok_or_else(|| {
            ProductError::integrity(
                "E2_TRAINING_STREAM_EXHAUSTED",
                "the training split ended before the canonical target count was consumed",
            )
        })
    }

    /// Advance the stream to an exact resume cursor, discarding consumed spans.
    ///
    /// Checkpoints are published only at completed optimizer-update boundaries and
    /// an update is a whole number of spans, so a resume cursor always lands on a
    /// span boundary. One that does not is a corrupt checkpoint rather than a
    /// stream to be re-aligned, so it fails closed.
    pub fn fast_forward_to(&mut self, target: u64) -> Result<u64> {
        let mut skipped = 0_u64;
        while self.next_first_target < target {
            let span = self.pull_span()?;
            if span.first_id != self.next_first_target {
                return Err(ProductError::integrity(
                    "E2_RESUME_STREAM_MISALIGNED",
                    "the training stream is not contiguous at the resume cursor",
                ));
            }
            let next = self
                .next_first_target
                .checked_add(span.valid_targets)
                .ok_or_else(accounting_overflow)?;
            if next > target {
                return Err(ProductError::integrity(
                    "E2_RESUME_CURSOR_NOT_ON_SPAN_BOUNDARY",
                    "the resume cursor falls inside a span rather than on its boundary",
                ));
            }
            self.next_first_target = next;
            skipped = skipped.checked_add(1).ok_or_else(accounting_overflow)?;
        }
        Ok(skipped)
    }
}

impl<S: SpanSource + ?Sized> FinalBatchSource for CorpusBatchSource<'_, S> {
    fn next_batch(&mut self, first_target: u64, valid_targets: u64) -> Result<TrainingBatch> {
        if first_target != self.next_first_target {
            return Err(ProductError::integrity(
                "E2_BATCH_CURSOR_MISMATCH",
                "the coordinator requested a window the training stream is not positioned at",
            ));
        }
        let mut spans = Vec::new();
        let mut accumulated = 0_u64;
        while accumulated < valid_targets {
            let span = self.pull_span()?;
            if span.first_id != first_target + accumulated {
                return Err(ProductError::integrity(
                    "E2_TRAINING_STREAM_DISCONTIGUOUS",
                    "the training stream skipped or repeated a span",
                ));
            }
            accumulated = accumulated
                .checked_add(span.valid_targets)
                .ok_or_else(accounting_overflow)?;
            spans.push(span);
        }
        if accumulated != valid_targets {
            return Err(ProductError::integrity(
                "E2_MICRO_BATCH_NOT_SPAN_ALIGNED",
                "the requested micro-batch is not a whole number of canonical spans",
            ));
        }
        self.next_first_target = first_target
            .checked_add(valid_targets)
            .ok_or_else(accounting_overflow)?;
        TrainingBatch::from_loaded_spans(&spans)
    }
}

/// Execute the canonical run to completion and report the measured result.
///
/// The trainer, backend, and batch source are supplied by the caller so the whole
/// orchestration is testable without an accelerator: the provider wiring lives in
/// [`launch_final_run`], and everything that decides correctness lives here.
pub fn execute_launched_run<B: TrainerBackend, S: FinalBatchSource>(
    trainer: &mut DeterministicTrainer<B>,
    source: &mut S,
    checkpoint_root: &Path,
    configuration: &PrototypeTrainingDefaultsV1,
    launch: &FinalRunLaunchV1,
) -> Result<FinalRunLaunchResultV1> {
    configuration.validate()?;
    launch.validate()?;
    let resumed_from_consumed_targets = trainer.consumed_targets();

    let clock = SlaClock::start()?;
    let execution = match launch.diagnostic_target_budget {
        Some(budget) => {
            execute_bounded_diagnostic(trainer, source, checkpoint_root, configuration, budget)?
        }
        None => execute_to_completion(trainer, source, checkpoint_root, configuration)?,
    };
    let sla = SlaObservationV1::measured(clock.elapsed_ns()?, configuration);

    let generation = execution
        .final_checkpoint
        .generation_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ProductError::internal(
                "E2_FINAL_GENERATION_NAME_INVALID",
                "the published final checkpoint generation has no readable name",
            )
        })?
        .to_owned();

    Ok(FinalRunLaunchResultV1 {
        schema: LAUNCH_RESULT_SCHEMA.to_owned(),
        // A bounded run says so in both fields. Reporting TRAINING_COMPLETE for a
        // fiftieth of the frozen target count would be the single most misleading
        // thing this result could contain.
        status: if launch.diagnostic_target_budget.is_some() {
            "DIAGNOSTIC_BUDGET_REACHED".to_owned()
        } else {
            "TRAINING_COMPLETE".to_owned()
        },
        qualification_status: "SKIPPED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        execution_status: if launch.diagnostic_target_budget.is_some() {
            "EXECUTED_PARTIAL".to_owned()
        } else {
            "EXECUTED".to_owned()
        },
        execution_surface: EXECUTION_SURFACE.to_owned(),
        configuration_sha256: configuration.sha256()?,
        launch_sha256: launch.sha256()?,
        implementation_sha256: implementation_sha256(),
        model_identity: execution.final_snapshot.identity.model_identity.clone(),
        resumed_from_consumed_targets,
        consumed_targets: execution.final_snapshot.consumed_targets,
        completed_updates: execution.final_snapshot.completed_updates,
        published_checkpoints: execution.published_checkpoints,
        retained_checkpoint_targets: execution.retained_checkpoint_targets,
        final_checkpoint_generation: generation,
        trainer_state_sha256: state_bundle_sha256(&execution.final_snapshot)?,
        same_process_reload_exact: execution.same_process_reload_exact,
        // Execution establishes target accounting, checkpoint durability, and
        // elapsed time. It does not establish hardware qualification, admission,
        // or model quality, and the SLA verdict is reported rather than claimed.
        final_evaluation_loss_f64_le_hex: execution
            .final_evaluation
            .as_ref()
            .map(|e| hex::encode(e.aggregate_loss.to_le_bytes())),
        final_evaluation_targets: execution
            .final_evaluation
            .as_ref()
            .map(|e| e.evaluated_targets),
        claims: FinalRunClaimStatusV1 {
            full_run_completion: "OBSERVED_UNVERIFIED".to_owned(),
            elapsed_time: "OBSERVED_UNVERIFIED".to_owned(),
            completion_sla: sla.completion_sla_status.clone(),
            final_loss: "UNVERIFIED".to_owned(),
            final_checkpoint: "OBSERVED_UNVERIFIED".to_owned(),
        },
        sla,
        limitations: vec![
            "not-hardware-qualification".to_owned(),
            "not-performance-admission".to_owned(),
            "not-final-model-quality".to_owned(),
            "elapsed-time-is-observed-not-independently-witnessed".to_owned(),
            "device-transfer-ring-stages-on-host-because-the-batch-contract-is-host-side"
                .to_owned(),
        ],
    })
}

fn capacity_error() -> ProductError {
    ProductError::usage(
        "E2_LOADER_CAPACITY_INVALID",
        "a configured loader capacity does not fit this host's address width",
    )
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "E2_ACCOUNTING_OVERFLOW",
        "launch-mode target accounting overflowed",
    )
}

fn implementation_sha256() -> String {
    hex::encode(Sha256::digest(include_bytes!("launch.rs")))
}

/// Resolve the trainer's starting state: a fresh canonical run, or an exact
/// resume from a published generation.
pub fn resume_cursor(resume_from_generation: Option<&Path>) -> Result<u64> {
    let Some(generation) = resume_from_generation else {
        return Ok(0);
    };
    let snapshot = super::checkpoint::load_checkpoint(generation)?;
    if snapshot.consumed_targets >= TOTAL_TARGETS {
        return Err(ProductError::gate(
            "E2_RESUME_ALREADY_COMPLETE",
            "the resume checkpoint is already the completed run; verify it instead",
        ));
    }
    Ok(snapshot.consumed_targets)
}

/// Read exactly the mandatory held-out evaluation targets from the validation
/// split.
///
/// `ValidationSet::new` requires the total to be `EVALUATION_TARGETS` exactly, and
/// that count is not a whole number of canonical spans, so the last span is
/// truncated to land on it rather than the set being rounded to a span boundary.
pub fn read_validation_set<S: SpanSource + ?Sized>(
    source: &S,
    dimensions: &crate::train::full_state::GqaDimensions,
    cancellation: LoaderCancellation,
) -> Result<crate::train::full_state::ValidationSet> {
    use crate::train::full_state::{ValidationSet, ValidationSpan};
    use crate::train::trainer::EVALUATION_TARGETS;

    let mut loader = SpanLoader::new(source, CorpusSplit::Validation, 8, cancellation)?;
    let mut spans = Vec::new();
    let mut covered = 0_u64;
    while covered < EVALUATION_TARGETS {
        let span = loader.next_span()?.ok_or_else(|| {
            ProductError::integrity(
                "E2_VALIDATION_STREAM_EXHAUSTED",
                "the validation split is smaller than the mandatory evaluation target count",
            )
        })?;
        let remaining = EVALUATION_TARGETS - covered;
        let take = remaining.min(span.valid_targets) as usize;
        spans.push(ValidationSpan {
            input_ids: span.input_ids()[..take].to_vec(),
            target_ids: span.target_ids()[..take].to_vec(),
        });
        covered += take as u64;
    }
    ValidationSet::new(spans, dimensions)
}

/// The explicit execution mode, wired to the prototype CUDA lane.
#[cfg(feature = "cuda")]
pub fn launch_final_run(config_path: &Path, launch_path: &Path) -> Result<serde_json::Value> {
    use crate::train::cuda_backend::{CUDA_FULL_MODEL_BACKEND, CudaTrainerBackend};
    use crate::train::full_state::GqaDimensions;

    crate::platform::require_prototype_tuple(
        crate::platform::current_host_data_adapter()?.host,
        crate::platform::AcceleratorProvider::Cuda,
    )?;

    let configuration = super::profile::parse_default_configuration(
        &super::profile::read_control_file(config_path, "P16_CONFIG_READ_FAILED")?,
    )?;
    let launch = parse_launch_configuration(&super::profile::read_control_file(
        launch_path,
        "E2_LAUNCH_CONFIG_READ_FAILED",
    )?)?;

    let corpus = crate::storage::VerifiedTokenCorpus::open(&launch.token_generation_root)?;
    let dimensions = GqaDimensions::canonical();
    let validation = read_validation_set(&corpus, &dimensions, LoaderCancellation::default())?;
    let device_ordinal = usize::try_from(launch.device_ordinal).map_err(|_| {
        ProductError::usage(
            "E2_DEVICE_ORDINAL_INVALID",
            "the launch device ordinal does not fit this host's address width",
        )
    })?;

    let identity = launched_identity(
        &corpus,
        CUDA_FULL_MODEL_BACKEND,
        launch.device_ordinal,
        crate::model::accelerator_execution_plan()?.parameter_layout_sha256,
    )?;

    let backend = CudaTrainerBackend::canonical(device_ordinal, validation)?;
    let mut trainer = match &launch.resume_from_generation {
        Some(generation) => DeterministicTrainer::from_snapshot(
            super::checkpoint::load_checkpoint(generation)?,
            &identity,
            backend,
        )?,
        None => {
            let (host_rng_state, device_rng_state) = backend.rng_states();
            DeterministicTrainer::new(identity, backend, host_rng_state, device_rng_state)?
        }
    };

    let mut source =
        CorpusBatchSource::new(&corpus, &configuration, LoaderCancellation::default())?;
    source.fast_forward_to(trainer.consumed_targets())?;

    let result = execute_launched_run(
        &mut trainer,
        &mut source,
        &launch.checkpoint_root,
        &configuration,
        &launch,
    )?;
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "E2_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed launch result",
        )
    })
}

#[cfg(not(feature = "cuda"))]
pub fn launch_final_run(config_path: &Path, launch_path: &Path) -> Result<serde_json::Value> {
    let _ = (config_path, launch_path);
    Err(ProductError::gate(
        "E2_ACCELERATOR_BACKEND_ABSENT",
        "the execution mode requires an accelerator backend feature; this build has none",
    ))
}

/// Build the trainer identity from the artifacts actually being trained on.
///
/// Every digest binds a real input rather than a placeholder, so a checkpoint can
/// only resume against the same corpus, tokenizer, layout, and backend it was
/// produced from; `DeterministicTrainer::from_snapshot` rejects any mismatch.
pub fn launched_identity(
    corpus: &crate::storage::VerifiedTokenCorpus,
    backend_identity: &str,
    device_ordinal: u64,
    parameter_layout_sha256: String,
) -> Result<TrainerIdentity> {
    let host = crate::platform::current_host_data_adapter()?;
    let span_manifest = |split: CorpusSplit| -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/e2-span-manifest/v1\0");
        for entry in corpus
            .sequences
            .entries
            .iter()
            .filter(|entry| entry.split == split)
        {
            hasher.update(entry.sequence.to_le_bytes());
            hasher.update(entry.first_id.to_le_bytes());
            hasher.update(entry.logical_ids.to_le_bytes());
            hasher.update(entry.valid_targets.to_le_bytes());
        }
        hex::encode(hasher.finalize())
    };
    Ok(TrainerIdentity {
        profile: PROTOTYPE_PROFILE.to_owned(),
        model_identity: crate::model::CANONICAL_MODEL_ID.to_owned(),
        model_parameter_layout_sha256: parameter_layout_sha256,
        backend_identity_sha256: hex::encode(Sha256::digest(backend_identity.as_bytes())),
        device_identity_sha256: hex::encode(Sha256::digest(
            format!("{}/{}", host.target_triple, device_ordinal).as_bytes(),
        )),
        corpus_manifest_sha256: corpus.manifest.corpus_manifest.sha256.clone(),
        training_span_manifest_sha256: span_manifest(CorpusSplit::Train),
        validation_span_manifest_sha256: span_manifest(CorpusSplit::Validation),
        tokenizer_artifact_sha256: corpus.manifest.tokenizer_artifact.sha256.clone(),
        environment_identity_sha256: hex::encode(Sha256::digest(host.target_triple.as_bytes())),
        implementation_artifact_sha256: implementation_sha256(),
    })
}
