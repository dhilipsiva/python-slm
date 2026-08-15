use rust_llm_pretrain::error::{ProductError, Result};
use rust_llm_pretrain::storage::{CorpusSplit, TokenSequenceEntry};
use rust_llm_pretrain::train::{
    AsyncBatchTransfer, LoadedSpan, LoaderCancellation, SpanLoader, SpanSource, TransferPipeline,
};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

struct SyntheticSource {
    entries: Vec<TokenSequenceEntry>,
    spans: Vec<Vec<u16>>,
    fail_at: Option<(u64, &'static str)>,
    reads: Cell<usize>,
}

impl SyntheticSource {
    fn ordered(count: u64) -> Self {
        let mut entries = Vec::new();
        let mut spans = Vec::new();
        let mut first_id = 0;
        for sequence in 0..count {
            let valid_targets = if sequence + 1 == count { 3 } else { 4 };
            entries.push(TokenSequenceEntry {
                split: CorpusSplit::Train,
                sequence,
                first_id,
                logical_ids: valid_targets + 1,
                valid_targets,
            });
            spans.push(
                (0..=valid_targets)
                    .map(|offset| (first_id + offset + 10) as u16)
                    .collect(),
            );
            first_id += valid_targets;
        }
        Self {
            entries,
            spans,
            fail_at: None,
            reads: Cell::new(0),
        }
    }
}

impl SpanSource for SyntheticSource {
    fn sequence_entries(&self) -> &[TokenSequenceEntry] {
        &self.entries
    }

    fn read_sequence(&self, _split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        self.reads.set(self.reads.get() + 1);
        if let Some((failed, code)) = self.fail_at
            && sequence == failed
        {
            return Err(ProductError::integrity(
                code,
                "synthetic backing-file failure",
            ));
        }
        Ok(self.spans[sequence as usize].clone())
    }
}

#[test]
fn loader_is_ordered_bounded_and_has_stable_end_of_stream() {
    let source = SyntheticSource::ordered(4);
    let mut loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        2,
        LoaderCancellation::default(),
    )
    .unwrap();
    assert_eq!(loader.capacity(), 2);

    let first = loader.next_span().unwrap().unwrap();
    assert_eq!(first.sequence, 0);
    assert_eq!(first.input_ids(), &[10, 11, 12, 13]);
    assert_eq!(first.target_ids(), &[11, 12, 13, 14]);
    assert_eq!(loader.buffered(), 1);
    assert_eq!(source.reads.get(), 2);

    let rest = std::iter::from_fn(|| loader.next_span().transpose())
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        rest.iter().map(|span| span.sequence).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(loader.is_terminal());
    assert!(loader.next_span().unwrap().is_none());
    assert!(loader.next_span().unwrap().is_none());
}

#[test]
fn loader_rejects_invalid_order_short_read_mutation_and_cancellation() {
    let mut invalid = SyntheticSource::ordered(2);
    invalid.entries[1].first_id += 1;
    assert_eq!(
        SpanLoader::new(
            &invalid,
            CorpusSplit::Train,
            1,
            LoaderCancellation::default()
        )
        .err()
        .unwrap()
        .code,
        "P11_SEQUENCE_ORDER_INVALID"
    );

    let mut short = SyntheticSource::ordered(1);
    short.spans[0].pop();
    let mut loader =
        SpanLoader::new(&short, CorpusSplit::Train, 1, LoaderCancellation::default()).unwrap();
    assert_eq!(
        loader.next_span().unwrap_err().code,
        "P11_SPAN_LENGTH_MISMATCH"
    );

    let mut mutated = SyntheticSource::ordered(2);
    mutated.fail_at = Some((1, "TOKEN_ARTIFACT_IDENTITY_MISMATCH"));
    let mut loader = SpanLoader::new(
        &mutated,
        CorpusSplit::Train,
        1,
        LoaderCancellation::default(),
    )
    .unwrap();
    loader.next_span().unwrap();
    assert_eq!(
        loader.next_span().unwrap_err().code,
        "TOKEN_ARTIFACT_IDENTITY_MISMATCH"
    );
    assert!(loader.next_span().unwrap().is_none());

    let source = SyntheticSource::ordered(2);
    let cancellation = LoaderCancellation::default();
    let mut loader = SpanLoader::new(&source, CorpusSplit::Train, 1, cancellation.clone()).unwrap();
    cancellation.cancel();
    assert_eq!(loader.next_span().unwrap_err().code, "P11_LOAD_CANCELLED");
    assert!(loader.is_terminal());
    assert!(loader.next_span().unwrap().is_none());
}

#[derive(Default)]
struct TransferState {
    active: usize,
    maximum_active: usize,
    submitted: Vec<u64>,
    waited: Vec<u64>,
    cancelled: Vec<u64>,
}

struct MockTransfer {
    state: Arc<Mutex<TransferState>>,
    fail_wait: Option<u64>,
}

impl AsyncBatchTransfer for MockTransfer {
    type Ticket = LoadedSpan;
    type DeviceBatch = u64;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let mut state = self.state.lock().unwrap();
        state.active += 1;
        state.maximum_active = state.maximum_active.max(state.active);
        state.submitted.push(span.sequence);
        Ok(span)
    }

    fn wait(&mut self, ticket: Self::Ticket) -> Result<Self::DeviceBatch> {
        let mut state = self.state.lock().unwrap();
        state.active -= 1;
        state.waited.push(ticket.sequence);
        if self.fail_wait == Some(ticket.sequence) {
            return Err(ProductError::environment(
                "P11_TRANSFER_WAIT_FAILED",
                "synthetic wait failure",
            ));
        }
        Ok(ticket.sequence)
    }

    fn cancel(&mut self, ticket: Self::Ticket) {
        let mut state = self.state.lock().unwrap();
        state.active -= 1;
        state.cancelled.push(ticket.sequence);
    }
}

#[test]
fn transfer_pipeline_applies_backpressure_and_retires_in_order() {
    let source = SyntheticSource::ordered(5);
    let loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        3,
        LoaderCancellation::default(),
    )
    .unwrap();
    let state = Arc::new(Mutex::new(TransferState::default()));
    let transfer = MockTransfer {
        state: state.clone(),
        fail_wait: None,
    };
    let mut pipeline = TransferPipeline::new(loader, transfer, 2).unwrap();
    let mut completed = Vec::new();
    while let Some(sequence) = pipeline.next_device_batch().unwrap() {
        completed.push(sequence);
    }
    assert_eq!(completed, [0, 1, 2, 3, 4]);
    let state = state.lock().unwrap();
    assert_eq!(state.maximum_active, 2);
    assert_eq!(state.submitted, [0, 1, 2, 3, 4]);
    assert_eq!(state.waited, [0, 1, 2, 3, 4]);
    assert!(state.cancelled.is_empty());
    assert_eq!(state.active, 0);
}

#[test]
fn transfer_failure_and_drop_clean_every_in_flight_ticket() {
    let source = SyntheticSource::ordered(4);
    let state = Arc::new(Mutex::new(TransferState::default()));
    {
        let loader = SpanLoader::new(
            &source,
            CorpusSplit::Train,
            2,
            LoaderCancellation::default(),
        )
        .unwrap();
        let transfer = MockTransfer {
            state: state.clone(),
            fail_wait: Some(0),
        };
        let mut pipeline = TransferPipeline::new(loader, transfer, 3).unwrap();
        for _ in 0..2 {
            assert_eq!(
                pipeline.next_device_batch().unwrap_err().code,
                "P11_TRANSFER_WAIT_FAILED"
            );
        }
    }
    let state = state.lock().unwrap();
    assert_eq!(state.active, 0);
    assert_eq!(state.cancelled, [1, 2]);
}
