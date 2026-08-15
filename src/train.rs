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
pub mod trainer;

pub use checkpoint::{
    CheckpointArtifactRef, CheckpointManifestV1, PublishedCheckpoint, load_checkpoint,
    prune_checkpoints, publish_checkpoint,
};
pub use trainer::{
    AdamwParameterState, BackendStateArtifact, BatchGradient, CanonicalAdamw,
    CanonicalTrainingPlan, DeterministicTrainer, EvaluationResult, TrainerBackend, TrainerIdentity,
    TrainerSnapshot, TrainingBatch, UpdateEvent, canonical_learning_rate,
    canonical_update_target_count,
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

#[cfg(all(feature = "cuda", windows))]
pub mod cuda_transfer;
