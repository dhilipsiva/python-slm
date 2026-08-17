//! Deterministic span loading and provider-neutral transfer ownership.

use crate::error::{ProductError, Result};
use crate::storage::{CorpusSplit, SEQUENCE_TARGETS, TokenSequenceEntry, VerifiedTokenCorpus};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const IMPLEMENTATION_PHASE: &str = "P12";

pub mod checkpoint;
pub mod final_run;
pub mod launch;
pub mod profile;
pub mod quality;
pub mod stability;
pub mod trainer;

pub use checkpoint::{
    CheckpointArtifactRef, CheckpointManifestV1, PublishedCheckpoint, load_checkpoint,
    prune_checkpoints, publish_checkpoint,
};
pub use final_run::{
    EXECUTION_SURFACE as FINAL_RUN_EXECUTION_SURFACE, FinalBatchSource,
    FinalCheckpointReloadResultV1, FinalRunCheckpointPolicyV1, FinalRunClaimStatusV1,
    FinalRunExecution, FinalTrainingImplementationResultV1, build_implementation_result,
    execute_to_completion, verify_final_checkpoint,
};
pub use launch::{
    CorpusBatchSource, EXECUTION_SURFACE as LAUNCH_EXECUTION_SURFACE, FinalRunLaunchResultV1,
    FinalRunLaunchV1, HostStagedTransfer, LAUNCH_CONFIG_SCHEMA, LAUNCH_RESULT_SCHEMA, SlaClock,
    SlaObservationV1, execute_launched_run, launch_final_run, launched_identity,
    parse_launch_configuration, read_validation_set, resume_cursor,
};
pub use profile::{
    ACTUAL_ELAPSED_LIMIT_NS, ADMISSION_SECONDS, COMPLETION_SLA_SECONDS, DEFAULT_CONFIG_BYTES,
    DEFAULT_CONFIG_SCHEMA, DIAGNOSTICS_SCHEMA, ProfileDiagnosticsV1, ProfileObservationV1,
    ProfileResultV1, PrototypeTrainingDefaultsV1, RESULT_SCHEMA, build_result,
    parse_default_configuration, parse_diagnostics,
};

pub use quality::{
    AggregateMetricsV1, EVALUATION_RESULT_SCHEMA as QUALITY_EVALUATION_RESULT_SCHEMA,
    EXECUTION_SURFACE as QUALITY_EXECUTION_SURFACE, GenerationSettingsV1,
    IMPLEMENTATION_RESULT_SCHEMA as QUALITY_IMPLEMENTATION_RESULT_SCHEMA, LossChunkV1,
    PromptCaseV1, PromptReplayV1, QualityClaimStatusV1, QualityEvaluationBackend,
    QualityEvaluationImplementationResultV1, QualityEvaluationResultV1, QualityPackV1,
    UnigramBaselineInputV1, aggregate_metrics, build_implementation_result as build_quality_result,
    evaluate_quality, quality_evaluation, unigram_baseline,
};

pub use stability::{
    EXECUTION_SURFACE as STABILITY_EXECUTION_SURFACE, PLAN_SCHEMA as STABILITY_PLAN_SCHEMA,
    RESULT_SCHEMA as STABILITY_RESULT_SCHEMA, STABILITY_PLAN_BYTES, StabilityLadderResultV1,
    StabilityPlanV1, StabilityTrialV1, build_stability_result, parse_stability_plan,
};
pub use trainer::{
    AdamwParameterState, BackendStateArtifact, BatchGradient, CanonicalAdamw,
    CanonicalTrainingPlan, DeterministicTrainer, EvaluationResult, TrainerBackend, TrainerIdentity,
    TrainerSnapshot, TrainingBatch, UpdateEvent, canonical_learning_rate,
    canonical_update_target_count,
};
pub use unified_transfer::{
    UnifiedAllocationProbe, UnifiedDeviceBatch, UnifiedSharedTransfer, UnifiedTicket,
};

#[derive(Clone, Debug, Default)]
pub struct LoaderCancellation(Arc<AtomicBool>);

impl LoaderCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub trait SpanSource {
    fn sequence_entries(&self) -> &[TokenSequenceEntry];
    fn read_sequence(&self, split: CorpusSplit, sequence: u64) -> Result<Vec<u16>>;
}

impl SpanSource for VerifiedTokenCorpus {
    fn sequence_entries(&self) -> &[TokenSequenceEntry] {
        &self.sequences.entries
    }
    fn read_sequence(&self, split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        VerifiedTokenCorpus::read_sequence(self, split, sequence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedSpan {
    pub split: CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub valid_targets: u64,
    token_ids: Vec<u16>,
}

impl LoadedSpan {
    pub fn token_ids(&self) -> &[u16] {
        &self.token_ids
    }
    pub fn input_ids(&self) -> &[u16] {
        &self.token_ids[..self.valid_targets as usize]
    }
    pub fn target_ids(&self) -> &[u16] {
        &self.token_ids[1..=self.valid_targets as usize]
    }
    pub fn bytes(&self) -> usize {
        self.token_ids.len() * size_of::<u16>()
    }
}

pub struct SpanLoader<'a, S: SpanSource + ?Sized> {
    source: &'a S,
    split: CorpusSplit,
    entries: Vec<TokenSequenceEntry>,
    next_entry: usize,
    buffer: VecDeque<LoadedSpan>,
    capacity: usize,
    cancellation: LoaderCancellation,
    terminal: bool,
}

impl<'a, S: SpanSource + ?Sized> SpanLoader<'a, S> {
    pub fn new(
        source: &'a S,
        split: CorpusSplit,
        capacity: usize,
        cancellation: LoaderCancellation,
    ) -> Result<Self> {
        if capacity == 0 {
            return Err(ProductError::usage(
                "P11_BUFFER_CAPACITY_INVALID",
                "the span buffer capacity must be at least one",
            ));
        }
        let entries = source
            .sequence_entries()
            .iter()
            .filter(|entry| entry.split == split)
            .cloned()
            .collect::<Vec<_>>();
        let (mut expected_sequence, mut expected_first_id) = (0_u64, 0_u64);
        for entry in &entries {
            if entry.sequence != expected_sequence
                || entry.first_id != expected_first_id
                || entry.valid_targets == 0
                || entry.valid_targets > SEQUENCE_TARGETS
                || entry.logical_ids != entry.valid_targets + 1
            {
                return Err(ProductError::integrity(
                    "P11_SEQUENCE_ORDER_INVALID",
                    "the sequence index is not contiguous, ordered, or target-aligned",
                ));
            }
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or_else(accounting_overflow)?;
            expected_first_id = expected_first_id
                .checked_add(entry.valid_targets)
                .ok_or_else(accounting_overflow)?;
        }
        Ok(Self {
            source,
            split,
            entries,
            next_entry: 0,
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            cancellation,
            terminal: false,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    fn require_active(&mut self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            self.buffer.clear();
            self.terminal = true;
            return Err(ProductError::gate(
                "P11_LOAD_CANCELLED",
                "span loading was cancelled before end-of-stream",
            ));
        }
        Ok(())
    }

    fn fill(&mut self) -> Result<()> {
        self.require_active()?;
        while self.buffer.len() < self.capacity && self.next_entry < self.entries.len() {
            self.require_active()?;
            let entry = &self.entries[self.next_entry];
            let token_ids = match self.source.read_sequence(self.split, entry.sequence) {
                Ok(token_ids) => token_ids,
                Err(error) => {
                    self.buffer.clear();
                    self.terminal = true;
                    return Err(error);
                }
            };
            if token_ids.len() as u64 != entry.logical_ids {
                self.buffer.clear();
                self.terminal = true;
                return Err(ProductError::integrity(
                    "P11_SPAN_LENGTH_MISMATCH",
                    "a loaded span differs from its immutable sequence-index length",
                ));
            }
            self.buffer.push_back(LoadedSpan {
                split: self.split,
                sequence: entry.sequence,
                first_id: entry.first_id,
                valid_targets: entry.valid_targets,
                token_ids,
            });
            self.next_entry += 1;
        }
        Ok(())
    }

    pub fn next_span(&mut self) -> Result<Option<LoadedSpan>> {
        if self.terminal {
            return Ok(None);
        }
        self.fill()?;
        let span = self.buffer.pop_front();
        if span.is_none() && self.next_entry == self.entries.len() {
            self.terminal = true;
        }
        Ok(span)
    }
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P11_SEQUENCE_ACCOUNTING_OVERFLOW",
        "sequence accounting overflowed",
    )
}

pub trait AsyncBatchTransfer {
    type Ticket;
    type DeviceBatch;
    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket>;
    fn wait(&mut self, ticket: Self::Ticket) -> Result<Self::DeviceBatch>;
    fn cancel(&mut self, ticket: Self::Ticket);
}

pub struct TransferPipeline<'a, S: SpanSource + ?Sized, T: AsyncBatchTransfer> {
    loader: SpanLoader<'a, S>,
    transfer: T,
    in_flight: VecDeque<T::Ticket>,
    maximum_in_flight: usize,
    source_exhausted: bool,
    terminal_error: Option<ProductError>,
}

impl<'a, S: SpanSource + ?Sized, T: AsyncBatchTransfer> TransferPipeline<'a, S, T> {
    pub fn new(loader: SpanLoader<'a, S>, transfer: T, maximum_in_flight: usize) -> Result<Self> {
        if maximum_in_flight == 0 {
            return Err(ProductError::usage(
                "P11_TRANSFER_CAPACITY_INVALID",
                "the asynchronous transfer capacity must be at least one",
            ));
        }
        Ok(Self {
            loader,
            transfer,
            in_flight: VecDeque::with_capacity(maximum_in_flight),
            maximum_in_flight,
            source_exhausted: false,
            terminal_error: None,
        })
    }
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
    pub fn transfer(&self) -> &T {
        &self.transfer
    }
    fn cancel_pending(&mut self) {
        while let Some(ticket) = self.in_flight.pop_front() {
            self.transfer.cancel(ticket);
        }
    }
    fn fill(&mut self) -> Result<()> {
        while self.in_flight.len() < self.maximum_in_flight && !self.source_exhausted {
            match self.loader.next_span() {
                Ok(Some(span)) => match self.transfer.submit(span) {
                    Ok(ticket) => self.in_flight.push_back(ticket),
                    Err(error) => {
                        self.cancel_pending();
                        self.terminal_error = Some(error.clone());
                        return Err(error);
                    }
                },
                Ok(None) => self.source_exhausted = true,
                Err(error) => {
                    self.cancel_pending();
                    self.terminal_error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    pub fn next_device_batch(&mut self) -> Result<Option<T::DeviceBatch>> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        self.fill()?;
        let Some(ticket) = self.in_flight.pop_front() else {
            return Ok(None);
        };
        match self.transfer.wait(ticket) {
            Ok(batch) => Ok(Some(batch)),
            Err(error) => {
                self.cancel_pending();
                self.terminal_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl<S: SpanSource + ?Sized, T: AsyncBatchTransfer> Drop for TransferPipeline<'_, S, T> {
    fn drop(&mut self) {
        self.cancel_pending();
    }
}

#[cfg(all(feature = "cuda", any(windows, target_os = "linux")))]
pub mod cuda_transfer;

#[cfg(all(feature = "rocm", target_os = "linux"))]
pub mod rocm_transfer;

#[cfg(feature = "cuda")]
pub mod cuda_backend;

pub mod full_state;
pub mod unified_transfer;
