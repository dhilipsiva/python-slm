//! Provider-neutral unified-memory transfer ownership for the Apple Silicon lane.
//!
//! Apple unified memory has no host-to-device staging copy: a span's canonical
//! token bytes become one shared device-visible allocation whose ownership moves
//! from submission ticket to device batch at an explicit synchronization
//! boundary. The Metal backend maps the shared allocation directly; this path is
//! never reported as an H2D copy. The ownership, source-order retirement, and
//! cleanup semantics are testable on every host without a Metal device.

use super::{AsyncBatchTransfer, LoadedSpan};
use crate::backend::tuples::UNIFIED_MEMORY_PATH;
use crate::error::{ProductError, Result};
use crate::storage::CorpusSplit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct AllocationGuard(Arc<AtomicUsize>);

impl AllocationGuard {
    fn new(ledger: &Arc<AtomicUsize>) -> Self {
        ledger.fetch_add(1, Ordering::AcqRel);
        Self(ledger.clone())
    }
}

impl Drop for AllocationGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub struct UnifiedTicket {
    shared: Arc<Vec<u16>>,
    order: u64,
    split: CorpusSplit,
    sequence: u64,
    first_id: u64,
    valid_targets: u64,
    bytes: usize,
    guard: AllocationGuard,
}

#[derive(Debug)]
pub struct UnifiedDeviceBatch {
    shared: Arc<Vec<u16>>,
    pub split: CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub valid_targets: u64,
    pub bytes: usize,
    pub synchronized: bool,
    #[allow(
        dead_code,
        reason = "holding the guard keeps the allocation ledger exact"
    )]
    guard: AllocationGuard,
}

impl UnifiedDeviceBatch {
    pub fn shared_token_ids(&self) -> &[u16] {
        &self.shared
    }

    pub fn shared_allocation(&self) -> &Arc<Vec<u16>> {
        &self.shared
    }
}

#[derive(Clone)]
pub struct UnifiedAllocationProbe(Arc<AtomicUsize>);

impl UnifiedAllocationProbe {
    pub fn live(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub struct UnifiedSharedTransfer {
    live_allocations: Arc<AtomicUsize>,
    submitted: u64,
    retired: u64,
}

impl UnifiedSharedTransfer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn memory_path(&self) -> &'static str {
        UNIFIED_MEMORY_PATH
    }

    pub fn live_shared_allocations(&self) -> usize {
        self.live_allocations.load(Ordering::Acquire)
    }

    pub fn allocation_probe(&self) -> UnifiedAllocationProbe {
        UnifiedAllocationProbe(self.live_allocations.clone())
    }
}

impl AsyncBatchTransfer for UnifiedSharedTransfer {
    type Ticket = UnifiedTicket;
    type DeviceBatch = UnifiedDeviceBatch;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let order = self.submitted;
        self.submitted = self.submitted.checked_add(1).ok_or_else(|| {
            ProductError::integrity(
                "P18_UNIFIED_SUBMISSION_OVERFLOW",
                "the unified transfer submission counter overflowed",
            )
        })?;
        let guard = AllocationGuard::new(&self.live_allocations);
        let bytes = span.bytes();
        Ok(UnifiedTicket {
            split: span.split,
            sequence: span.sequence,
            first_id: span.first_id,
            valid_targets: span.valid_targets,
            bytes,
            // Moving the span's canonical buffer into the shared allocation is the whole
            // transfer: unified memory forbids a staging copy.
            shared: Arc::new(span.token_ids),
            order,
            guard,
        })
    }

    fn wait(&mut self, ticket: Self::Ticket) -> Result<Self::DeviceBatch> {
        if ticket.order != self.retired {
            return Err(ProductError::integrity(
                "P18_UNIFIED_RETIREMENT_ORDER_INVALID",
                "unified transfer tickets must retire in exact submission order",
            ));
        }
        self.retired += 1;
        Ok(UnifiedDeviceBatch {
            shared: ticket.shared,
            split: ticket.split,
            sequence: ticket.sequence,
            first_id: ticket.first_id,
            valid_targets: ticket.valid_targets,
            bytes: ticket.bytes,
            synchronized: true,
            guard: ticket.guard,
        })
    }

    fn cancel(&mut self, ticket: Self::Ticket) {
        self.retired = self.retired.max(ticket.order + 1);
    }
}
