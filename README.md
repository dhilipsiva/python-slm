# python-slm

A zero-Python Rust rebuild for a deterministic small-language-model training system.

Normative design and phase order live in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/rebuild-contract-v2.md](docs/rebuild-contract-v2.md), and
[TODO.md](TODO.md). Historical receipts and the original implementation remain evidence,
not an active compatibility surface.

## Active product scaffold

The repository installs one product executable named `python-slm`. Its deterministic,
read-only canonical plan is:

```powershell
cargo run --locked --bin python-slm -- plan
```

It emits one compact `python-slm-plan-result-v1` JSON object. The canonical model is
`gqa-135m-v1` with exactly 135,285,504 parameters. The plan also freezes the
2,000,000,001 stored prefix IDs, 2,000,000,000 valid targets, 30,517 full updates,
37,888 final-update targets, 2,952,790,016-byte compatibility allocation,
25,920-second admission projection, and 28,800-second completion SLA.

The remaining future command names `inspect`, `bench`, and `train` fail
before reading configuration or mutating state with the typed `PHASE_NOT_IMPLEMENTED`
gate until their owning phases land. Configurations are versioned, explicit, and reject
unknown fields; there are no legacy fallbacks or hidden production defaults.

## Governed acquisition and source materialization

Everything upstream of `curate` is hash-pinned or it is not deterministic, so
acquisition is three explicit commands rather than a crawl:

```powershell
cargo run --locked --bin python-slm -- fetch --config <absolute-config-path> --discover
cargo run --locked --bin python-slm -- fetch --config <absolute-config-path>
cargo run --locked --bin python-slm -- materialize-stack-source --config <absolute-config-path>
```

`fetch --discover` transfers an asset and reports its observed SHA-256 and byte
count without publishing anything, which is how a digest is obtained before it can
be pinned. `fetch` then transfers only assets whose digest and length are declared:
HTTPS only, redirects bounded and re-validated at every hop, bounded streaming
rather than download-all, every byte hashed before publication, and credentials read
only from named environment variables. Plain HTTP is admissible solely against a
literal loopback address, which exists so the transport itself is testable.

`materialize-stack-source` is the `SOURCE-001` primary source adapter. The Stack v2
shards record Software Heritage *identifiers* rather than a content column, so the
command reads hash-bound Parquet metadata, projects columns the operator binds by
name, applies the language filter, the licence allowlist and the frozen
1,000,000-byte document ceiling, and resolves each surviving identifier to a blob
through the same transport rules. The archive is content-addressed by `sha1_git`, so
each blob is verified against the identifier that selected it before it is written —
the identifier-to-content step is a link in the hash chain, not a gap in it. Licences
come per row, and a dual-licensed row is admitted only if every term it carries is
allowlisted. Output is a sharded `MaterializedSourceManifestV1` plus content tree.

Three things make a million-blob acquisition survivable rather than merely
correct. Transient failures — a rate limit, a server error, a reset connection —
are retried with a bounded, purely arithmetic backoff, while a `403` or `404` is
not retried at all, because repeating it only hides a configuration error behind
a delay. Retries move wall clock and nothing else: every published byte is still
verified against the digest its metadata declared, so a blob that took three
attempts produces the same artifact as one that took none. The work is split by
`blob_id` prefix, one create-new generation per partition, so a failure costs one
partition instead of the whole run; because the split is a function of the
identifier, partitions cannot duplicate a document between them and their union
is exactly what an unpartitioned run selects. A partition nothing landed in is a
success that publishes nothing (`STACK_PARTITION_EMPTY`), so an operator loop
never has to read a failure code as if it meant success. And because the bulk
mirror and the archive API disagree about body framing, `content_encoding` is
declared rather than sniffed: under `gzip` the body is inflated, bounded by the
declared length, before the length, identifier and digest are checked, so all
three describe the source file rather than its framing.

`materialize-source` does the same job for an already-authorized local tree, where
the operator declares one licence expression over the whole tree instead.

Two boundaries stated plainly: the Parquet codec set is pure Rust, because the data
lane admits no native code beyond the pinned tree-sitter parser, so a shard in an
unsupported codec fails with a typed error naming it; and the authorization record
and dataset terms are the operator's to obtain and declare, never verified here.

## Document source and policy engine

Phase 4 activates bounded materialization of already-authorized local source bytes:

```powershell
cargo run --locked --bin python-slm -- curate --config <absolute-config-path>
```

The closed `python-slm-curate-config-v1` configuration names an absolute materialized
source manifest, content root, hash-bound removal manifests, create-new output root, and
explicit document/byte budgets. Generated corpus data belongs under the ignored `data/`
root or another ignored location; it is not a qualification receipt.
An eligible document passes the P4 license, provenance, removal, encoding, and
generated-content policies before reaching the P5 parser and P6 sensitive-data policy.
A successful command emits one compact `python-slm-curate-result-v4` object and installs
an immutable `python-slm-source-generation-v4` generation. The in-process Rust boundary
uses exactly `tree-sitter 0.25.8` and `tree-sitter-python 0.25.0`; its checked-in identity
manifest binds the locked packages, generated parser/scanner sources, runtime sources,
language ABI, frozen compatibility corpus, and canonical bundle hash. Complete Python 3
modules are evaluated with parser-derived comment ranges against the existing
comment-ratio and `generated-v1` rules. No Python executable, generator, subprocess, or
second parser is used.

The hash-bound `sensitive-rules-v1` registry detects confirmed private keys, provider
credentials, credentialed URLs, high-entropy named secrets, personal email addresses,
telephone numbers, government identifiers, payment-card/IBAN identifiers, and postal
addresses. Confirmed findings produce `REJECTED`; lower-confidence labeled secrets,
government identifiers, and postal addresses produce `QUARANTINED`. Policy artifacts
contain only stable rule IDs, counts, source hashes, and the registry binding—never the
matched value. Canonical `.py` bytes are stored only for `POLICY_ACCEPTED` documents.

P6A pins the closed, hash-checked
`tests/fixtures/p6a/adversarial-filter-cases-v1.json` no-code corpus. It exercises encoding
cookies/BOMs/invalid bytes, quoting forms, comments and generated markers, secret and PII
boundaries, portable-path attacks, deterministic repeat publication, restricted-value
non-disclosure, and concurrent write/delete/rename denial. The suite remains a
deterministic conservative regression boundary, not proof that every possible sensitive
value has been recognized. Exact/near deduplication, decontamination, and downstream
corpus acceptance remain later phases. Live Stack-v2 or Software Heritage acquisition
also remains outside this command.

## Tokenizer engine

Phase 7 activates deterministic byte-level BPE training:

```powershell
cargo run --locked --bin python-slm -- train-tokenizer --config <absolute-config-path>
```

The closed `python-slm-tokenizer-train-config-v1` configuration names an absolute,
hash-bound tokenizer sample manifest — either the inline
`python-slm-tokenizer-sample-manifest-v1` or the `-v2` index over hash-bound parts —
its immutable content root, and a create-new tokenizer artifact path. Sample documents bind repository group, source,
curated raw, canonical byte, length, and portable relative-path identities. Whole
documents are ranked by `TOKSAMPLE-001`; the engine enforces the 10,000,000-byte
repository cap and 2,000,000,000-byte global cap, skips non-fitting documents, and never
creates cross-document merge pairs.

The `python-slm-byte-bpe-tokenizer-v1` artifact contains exactly 32,000 contiguous IDs:
`<pad>=0`, `<s>=1`, `</s>=2`, `<unk>=3`, all 256 byte symbols at IDs 4 through 259, and
31,740 deterministic merge rules. Training uses minimum frequency two and resolves equal
frequencies by the lowest `(left_id,right_id)` pair. Source encoding performs no Unicode
normalization, case folding, whitespace stripping, or literal special-token matching;
source encode/decode is byte-exact and never emits IDs 0 through 3. Serialization is
compact and stable, reload validates every constant and merge reference, and publication
is create-new with adjacent temporary cleanup.

Training reports whether the sample falls within the contract's qualified byte range,
but `qualification_status` remains `SKIPPED`; P7 adds no receipt or manual workflow.

## Corpus and token materialization

Phase 8 activates deterministic, create-new corpus tokenization:

```powershell
cargo run --locked --bin python-slm -- tokenize --config <absolute-config-path>
```

The closed `python-slm-token-materialize-config-v1` configuration binds a governed
corpus manifest (`python-slm-governed-corpus-manifest-v1`, `-v2`, or the `-v3` index
over hash-bound parts), content root, P7 tokenizer sample and
tokenizer artifact, output root, and explicit document, byte, token, and shard limits.
The materializer sorts each split by component, repository group, source, and curated
hash identity; encodes each complete document; appends exactly one EOS; and writes
immutable little-endian `u16` shards plus closed document and 2,049-ID sequence indexes.

The installed `python-slm-token-corpus-generation-v1` generation copies the exact
governed manifest, tokenizer sample, and tokenizer artifact and binds every file by
length and SHA-256. The verified reader rejects path escape, reparse entries, malformed
IDs, broken document/EOS boundaries, count drift, and backing-file mutation before
returning a document or sequence. Publication uses a unique adjacent partial generation,
syncs every file, never overwrites, and removes interrupted partial output.

Small synthetic corpora are valid automated diagnostics and report
`training_target_satisfied: false`; only a later governed production manifest can reach
the fixed 2,000,000,001-ID prefix. P8 remains non-qualifying and writes no receipt.

## Corpus policy and span order

Phase 9A adds two deterministic, non-publishing commands:

```powershell
cargo run --locked --bin python-slm -- prepare-corpus --config <absolute-config-path>
cargo run --locked --bin python-slm -- plan-spans --config <absolute-config-path>
```

`prepare-corpus` consumes a hash-bound v4 source generation and a separately
materialized, hash-bound `evalplus-v0.3.1` protection manifest. The benchmark
manifest binds the pinned EvalPlus commit, HumanEval+ `v0.1.10`, MBPP+
`v0.2.0`, and normalized module, fragment, and canonical-JSON records; this
phase does not download assets or execute Python.

The engine applies exact canonical-byte deduplication before the frozen
Tree-sitter lexical-token 5-gram policy, 256 affine MinHash components, 32-by-8
LSH candidate retrieval, and exact Jaccard rejection strictly above `0.85`.
It retains every duplicate-cluster member identity, selects the representative
by complete provenance, comment ratio, lexical-token count, then source ID, and
rejects an entire duplicate cluster if any member matches a protected benchmark
by exact bytes, exact Jaccard, a protected 50-token span, a complete short
sequence, or canonical JSON bytes.

Remaining repository/duplicate connected components receive deterministic
`SPLIT-001` 98/1/1 assignment. A create-new
`python-slm-corpus-policy-generation-v1` contains deduplication,
decontamination, split, tokenizer-sample, and
`python-slm-governed-corpus-manifest-v3` artifacts plus representative source
bytes. The governed corpus and tokenizer sample are each emitted as a hash-bound
index naming parts of at most 50,000 documents, so neither is capped by the
64 MiB control-file bound; each part is verified against the index digest and
rejected if its schema, ordinal, or document count disagrees. P8 accepts the
immutable v1 governed manifest, P9A's inline v2 manifest, and the v3 index, and
the explicit P8 configuration supplies and verifies the later tokenizer artifact
binding, avoiding a circular pre-training hash.

`plan-spans` opens a fully verified P8 token generation, hashes the exact
frozen-decision byte range, and applies `rand_chacha 0.10.0`
`ChaCha12Rng` with the rejection-sampled descending Fisher-Yates algorithm.
Every complete 2,048-target span appears exactly once, token order inside spans
is unchanged, and the partial span remains last. Both commands keep
`qualification_status: "SKIPPED"`, publish no receipts, and refuse overwrite.

## Model initialization and CPU oracle

Phase 9B adds one deterministic, non-publishing diagnostic:

```powershell
cargo run --locked --bin python-slm -- model-oracle
```

The command emits a closed `python-slm-model-oracle-result-v1` object. It
enumerates all 111 stable `PARAM-001` tensors, proves the canonical
135,285,504-parameter count, assigns AdamW decay only to embedding, LM-head,
attention, and FFN matrices, and streams every canonical BF16 artifact through
SHA-256 without retaining the complete model in memory. Initialization uses the
exact `rand_chacha 0.10.0` seed, `rand_distr 0.6.0`
`StandardNormal<f32>` sequence, row-major order, and BF16
round-to-nearest-even conversion frozen by `INIT-001`.

The embedded scalar oracle exercises pre-norm RMSNorm, head-local adjacent-pair
RoPE, 2Q/1KV GQA, inclusive causal attention, residuals, SwiGLU, an untied LM
head, and valid-target-normalized cross-entropy. It emits literal BF16 logits,
an IEEE-754 FP32 loss, and complete FP32 little-endian gradient bytes with
stable name/shape/offset/hash records for every fixture parameter. These bytes
are the provider-independent P10 parity boundary, not a tolerance comparison.

The result keeps `qualification_status: "SKIPPED"`, writes no receipt or model
artifact, and makes no accelerator-parity, training-stability, performance,
SLA, checkpoint, or qualification claim. The full initialization stream is a
developer diagnostic and may take appreciably longer than the small automated
oracle regressions.

## Accelerator model backend

Phase 10 implements the selected `burn-cubecl-cuda` model boundary behind the
provider-neutral result and cancellation types. The CUDA feature contains a
one-layer GQA transformer graph with the same P9B fixture parameters and
semantics: BF16 parameters and activations, explicit FP32 normalization,
attention and loss accumulation, head-local RoPE, causal GQA, SwiGLU,
valid-target-normalized cross-entropy, autodiff, and ordered FP32 gradient
readback.

The path runs the fixture twice on one CUDA device, synchronizes at every
forward/loss/backward/cleanup boundary, releases owned tensors before a final
synchronization, and accepts only literal equality with P9B's logits, loss, and
complete gradient bytes. Cancellation is monotonic and checked between each
resource or execution stage. A mismatch, missing gradient, incomplete stage,
cleanup failure, or repeated-execution drift fails closed.

The CUDA implementation remains isolated from CPU/data builds and can be
compile-checked without launching hardware:

```powershell
cargo check --locked --no-default-features --features cuda --offline
```

P10 is an implementation boundary with `qualification_status: "SKIPPED"`; it
writes no receipt and makes no full-model VRAM, optimizer/resume, throughput,
SLA, hardware-qualification, or cross-provider claim. P11 owns transfers and
P12 owns optimizer state and exact resume.

## Deterministic data loading and transfers

Phase 11 consumes P8's immutable sequence index through `VerifiedTokenCorpus`.
Each read revalidates the contained regular shard, stable file identity, byte
length, and SHA-256 before exposing one ordered autoregressive span. Inputs and
targets are overlapping views over the same `valid_targets + 1` token IDs, so a
complete sequence yields exactly 2,048 targets without duplicating or skipping
the boundary token.

The loader has explicit, nonzero capacities for both host buffering and
in-flight transfers. It rejects reordered or discontinuous indexes, propagates
short-read and backing-file mutation failures, makes cancellation monotonic,
and returns a stable end-of-stream. Transfer tickets are retired in source
order; any submission/wait failure or pipeline drop cancels and releases all
remaining tickets.

Under the Windows `cuda` feature, `CudaPinnedTransfer` loads only System32's
CUDA driver, retains the selected device's primary context, allocates true CUDA
page-locked host staging with `cuMemAllocHost`, and submits a nonblocking
`cuMemcpyHtoDAsync`. The ticket owns the host allocation, stream, context
reference, and device allocation until synchronization. Successful completion
releases staging and the stream while returning an opaque owned device batch;
failure, cancellation, and drop synchronize and release acquired resources in
reverse ownership order. CPU and no-default-feature builds contain no CUDA
loader or discovery path.

P11 is an implementation boundary with `qualification_status: "SKIPPED"`. It
writes no receipt, pointer, acceptance, checkpoint, or persistent loader
artifact and makes no throughput, hardware-qualification, full-training, or
resume claim. P12 owns optimizer state, checkpointing, and exact resume.

## Trainer, checkpoints, and exact resume

Phase 12 adds a provider-neutral deterministic trainer over P11 batches. It owns the
canonical target cursor, valid-target loss/gradient accumulation, one FP32 global-L2 clip
per optimizer update, canonical AdamW FP32 master weights and moments with BF16
round-to-nearest-even storage, and the frozen one-based learning-rate schedule. Full updates
consume exactly 65,536 targets; update 30,518 consumes the remaining 37,888 targets and
terminates at exactly 2,000,000,000 targets with overshoot rejected.

Evaluation runs once before training and after the first completed update that crosses each
100,000,000-target boundary, including completion. The trainer snapshots backend bytes before
and after evaluation and rejects any model, optimizer, runtime, or RNG mutation. Backend
implementations return explicit evolving host/device RNG state for each batch; checkpoint state
also binds the model/backend/device/environment, corpus/tokenizer/span manifests, scheduler,
cursor, counters, evaluation history, and implementation identity.

Checkpoint generations are create-new 20-digit target-count directories under an explicit
absolute checkpoint root. Every model BF16 artifact, FP32 master-weight artifact, both AdamW
moment artifacts, and backend runtime artifact is length/hash bound in a closed manifest and a
complete `SHA256SUMS` seal. Publication is atomic and write-through on Windows, restore
revalidates the complete inventory and exact reconstructed trainer-state digest, and identity
drift fails closed. Mid-update and pre-update checkpoints are rejected. Retention keeps the
latest two generations plus the first generation at or after 500M, 1B, 1.5B, and final 2B
targets. Generated `/checkpoints/` state is ignored by Git.

P12 is an implementation boundary with `qualification_status: "SKIPPED"`. The synthetic
automated suite proves interruption, corruption, identity-mismatch, final-tail, retention, and
byte-identical continuation behavior; it does not claim a completed full-model run, hardware
qualification, throughput, SLA admission, final model quality, or P16/P16A acceptance.

## Automated CPU and optional Windows/CUDA CI

Phase 13 adds required hosted CI in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
The required Windows lane runs formatting, warning-free Clippy, the targeted zero-Python xtask
closure, the complete CPU-reference suite, the dependency-minimal product check, and compile-only
P2/product CUDA boundaries, then requires a stable repository. It deliberately excludes P1A's
interactive-host qualification tests: a generic hosted runner is not the qualified 9950X3D host.
The prototype remains Windows-only before P16, so P13 does not fabricate a Linux product lane.
The P13 synthetic test carries ordered P11 spans through bounded transfer, P12 training,
create-new checkpoint publication, restore, and byte-identical continuation.

Actual CUDA execution is isolated in
[`windows-cuda.yml`](.github/workflows/windows-cuda.yml). Required CI reports the hardware lane
as `UNVERIFIED`; the separate diagnostic workflow is manual-only and defaults to a
successful `UNVERIFIED` report, accepts no pull-request trigger, and can run only from `main` on
the fixed `[self-hosted, Windows, X64, cuda, rtx-5090]` labels. Enable it only when that runner is
available:

```powershell
gh workflow run windows-cuda.yml --ref main -f run_hardware=true
# Add -f device_uuid=GPU-... when more than one RTX 5090 is visible.
```

Once enabled, CUDA failures are real failures: the lane has no fallback and no
`continue-on-error`. It runs the CUDA-aware tests, the non-publishing P1B probe, and the P2
backend selector, requires a clean repository afterward, and removes its owned Cargo and target
state. `DIAGNOSTIC_OK` still means only that automated diagnostics ran; absent hardware remains
`UNVERIFIED`, and neither state is host qualification, performance admission, full-run evidence,
or a publication receipt.

## Prototype defaults and optional profiling diagnostics

Phase 14 freezes one explicit `prototype-windows-5090-v1` training configuration in
[`src/train/prototype-windows-5090-v1.defaults.json`](src/train/prototype-windows-5090-v1.defaults.json).
It uses 16 sequences of 2,048 targets per micro-batch and two accumulation steps for the immutable
65,536-target optimizer update. The host loader buffers 32 spans, the CUDA page-locked transfer
ring permits eight in-flight spans, and `burn-cubecl-cuda` is selected explicitly. Evaluation and
completed-boundary checkpoint events remain every 100,000,000 targets; retention keeps the latest
two generations plus the frozen 500M, 1B, 1.5B, and 2B anchors.

Inspect the closed defaults through the non-publishing benchmark boundary:

```powershell
$config = (Resolve-Path src/train/prototype-windows-5090-v1.defaults.json).Path
cargo run --locked --offline --bin python-slm -- bench --config $config
```

The command emits one `python-slm-prototype-profile-result-v1` object and writes no artifact.
An optional `--diagnostics <absolute-path>` accepts sorted, configuration-hash-bound synchronized
observations. Their integer throughput and memory summaries remain `OBSERVED_UNVERIFIED`; they do
not retune the configuration, correctness constants, 25,920-second admission target, or
28,800-second completion SLA. With no observation file, diagnostics are `UNAVAILABLE`. In both
cases qualification is `SKIPPED`, performance is `UNVERIFIED`, and no hardware, admission,
full-run, or SLA claim is made. Non-Windows execution returns `DEFERRED_POST_P16` before reading
configuration bytes.

Phase 15 adds a bounded automated ladder over the same immutable defaults and the P12
checkpoint/reload boundary:

```powershell
$config = (Resolve-Path src/train/prototype-windows-5090-v1.defaults.json).Path
$ladder = (Resolve-Path src/train/prototype-windows-5090-v1.stability.json).Path
cargo run --locked --offline --bin python-slm -- bench --config $config --stability-plan $ladder
```

The ladder runs one smoke trial, one short trial, an uninterrupted-versus-reloaded restart
comparison, and three repeated bounded stability trials. It freezes the configuration and
implementation identities within every trial, uses an owned temporary checkpoint root, removes
that root on success and failure, and emits one closed local JSON result. The execution surface is
explicitly `provider-neutral-synthetic`: `LADDER_OK` proves deterministic automated trainer and
checkpoint behavior only. Hardware stability, long-duration execution, performance admission,
the completion SLA, and a full training run remain `UNVERIFIED`.

## Final training implementation

Phase 16 installs the provider-neutral completion coordinator over the P11 loader and P12
trainer/checkpoint contracts. It requests each micro-batch at the exact trainer cursor, truncates
only the canonical final update, publishes create-new checkpoints at every required event,
applies retention, and requires the durable final checkpoint to reload byte-exactly.

Inspect the fixed implementation contract without starting training or writing state:

```powershell
$config = (Resolve-Path src/train/prototype-windows-5090-v1.defaults.json).Path
cargo run --locked --offline --bin python-slm -- train --config $config
```

With no final checkpoint argument, the command emits
`python-slm-final-training-implementation-result-v1` with `IMPLEMENTATION_READY` and every
execution, elapsed-time, SLA, final-loss, and final-checkpoint claim `UNVERIFIED`.

To verify a completed checkpoint from a separate process, run:

```powershell
cargo run --locked --offline --bin python-slm -- train --config $config `
  --verify-final-checkpoint C:\absolute\checkpoints\generations\00000000002000000000
```

That mode revalidates the complete P12 manifest, seal, artifact inventory, backend bytes, exact
2,000,000,000-target cursor, and 30,518-update count. It still does not prove execution
provenance, hardware qualification, elapsed time, the completion SLA, or final model quality.

## Launching the run

E2 adds the third, explicit form of `train`, and it is the only one that executes:

```powershell
cargo run --release --locked --offline --features cuda --bin python-slm -- train `
  --config $config --launch C:\absolute\launch.json
```

The launch file is its own closed schema, `python-slm-final-run-launch-v1`, because the
frozen defaults file is byte-pinned and cannot carry a path or a device ordinal:

```json
{
  "schema": "python-slm-final-run-launch-v1",
  "profile": "prototype-windows-5090-v1",
  "token_generation_root": "C:\\absolute\\tokens\\generations\\0001",
  "checkpoint_root": "C:\\absolute\\checkpoints",
  "resume_from_generation": null,
  "device_ordinal": 0,
  "confirm_full_run": true
}
```

`confirm_full_run` must be set, so the execution mode is unreachable by a stray flag, and
`--launch` and `--verify-final-checkpoint` are mutually exclusive at the parser. The run is
measured on a suspend-inclusive monotonic clock — `QueryInterruptTime` on Windows,
`CLOCK_BOOTTIME` on Linux — so time the host spends asleep counts against the wall-clock
deadline rather than disappearing from it. The result reports the measured elapsed time and
whether it fell inside the frozen limit; it does not decide admission, and it does not claim
hardware qualification or model quality. A real full CUDA run remains optional and has not
been performed.

## Automated quality evaluation

Phase 16A adds provider-neutral held-out evaluation without claiming that a final model has been
run. The reusable evaluator aggregates ordered valid-target negative log likelihood, derives
finite perplexity, computes an exact add-one-smoothed unigram baseline, requires the final loss
to be strictly below both the initialized and unigram baselines, and replays every frozen prompt
twice with exact token equality. Evaluation and generation must leave both backend states
unchanged.

Inspect the implementation boundary without reading a checkpoint or publishing artifacts:

```powershell
$config = (Resolve-Path src/train/prototype-windows-5090-v1.defaults.json).Path
cargo run --locked --offline --bin python-slm -- evaluate-quality --config $config
```

This emits `python-slm-quality-evaluation-implementation-result-v1` with
`IMPLEMENTATION_READY`, `qualification_status: SKIPPED`, and every pack, checkpoint, baseline,
metric, output, and quality claim `UNVERIFIED`. A real final checkpoint, immutable held-out
manifest, and frozen prompt pack were not evaluated. P16A therefore does not claim owner quality
approval, portability unlock, hardware qualification, performance, or SLA evidence.
Phase 7A adds hash-bound governed-source metadata to every curation outcome. The checked-in
default policy labels manifest-declared provenance, license, and removal facts ASSUMED;
freshness and aggregate source status remain UNVERIFIED while external review is unavailable.
The generation records only deterministic identity and policy bindings, not review claims or
sensitive values.

## Portable host and data adapters

Phase 17 runs the CPU/data pipeline through one native host adapter on Windows x86_64 MSVC,
Linux x86_64 GNU, and macOS arm64. Curation, parser, privacy, tokenizer, token-corpus,
deduplication, split, and span-order artifacts retain the same portable path grammar, compact
JSON, byte hashing, ordering, create-new publication, mutation detection, and cleanup semantics.
The adapter uses each host's exclusive same-parent rename primitive and synchronizes publication
metadata; it never broadens accelerator support.

The ordinary CI workflow runs the P4-P9 artifact contracts plus the P17 adapter contract on all
three hosted lanes. These checks are automated implementation evidence only. Manual host-matrix
qualification and publication remain `SKIPPED`, and a lane that cannot run remains `UNVERIFIED`.
Linux CUDA, Linux ROCm/HIP, and macOS Metal are still deferred to P18.

## Accelerator provider adapters

Phase 18 implements the CUDA, ROCm/HIP, and Metal accelerator adapters behind the
provider-neutral backend interface for the four mandatory tuple lanes: the
Windows/NVIDIA CUDA regression, Linux/NVIDIA CUDA, Linux/AMD ROCm/HIP, and macOS
arm64/Apple Silicon Metal. The closed `python-slm-provider-adapter-matrix-v1`
enumerates exactly those lanes; every unlisted host/provider combination still fails
before discovery or mutation with `DEFERRED_POST_P16`, and the prototype training
profile remains CUDA-only.

Each provider executes the same generic one-layer parity graph over the P9B fixture
parameters. Under `PRECISION-002` it must reproduce the CPU oracle's literal BF16 logits
and FP32 loss exactly, and its FP32 gradients must fall inside the frozen
provider-independent bound; the create-new `python-slm-provider-parity-result-v2` accepts
only byte-identical repeated executions with complete stage order, explicit
synchronization, and cleanup. The deterministic trainer, checkpoints, and
byte-identical resume run unchanged behind the provider interface for every lane
identity. Discrete CUDA and ROCm lanes use true page-locked staging with
asynchronous host-to-device rings (`nvcuda.dll` from System32 on Windows;
`libcuda.so.1` or `libamdhip64.so` via `dlopen` on Linux). The Apple lane uses
unified shared-memory access with explicit synchronization and source-order
retirement; it is never reported as an H2D copy.

The isolated feature boundaries compile only on their matching hosts:

```powershell
cargo check --locked --no-default-features --features cuda --offline   # Windows or Linux
cargo check --locked --no-default-features --features rocm --offline   # Linux
cargo check --locked --no-default-features --features metal --offline  # macOS
```

Ordinary CI runs the provider-neutral P18 contracts on all three hosted lanes and
compile-checks each provider surface on its matching host without launching
hardware. Manual tuple-matrix qualification and publication remain `SKIPPED`; every
lane's execution status is `UNVERIFIED` until its exact device tuple actually runs.
P18 makes no performance-equivalence, cross-provider checkpoint-migration,
hardware-qualification, or AMD/Apple two-billion-target-run claim.

## Full-model accelerator training backend

The post-phase execution track in [TODO.md](TODO.md) covers what stands between the
implemented phases and an actual training run. Its first item, E1, adds the concrete
full-model backend:

- [`src/train/full_state.rs`](src/train/full_state.rs) holds the provider-neutral
  training state: the generalized GQA dimensional contract, INIT-001 canonical
  master-weight initialization, AdamW through the frozen P12 arithmetic,
  deterministic host and device RNG witness chains, and the closed five-artifact
  checkpoint codec with byte-exact restore.
- [`src/model/accelerator/full_model.rs`](src/model/accelerator/full_model.rs)
  replaces the fixture-only P10 graph with one configuration-parameterized GQA
  graph shared by every provider adapter, using straight-through BF16 storage
  quantization and explicit host-FP32 RoPE and causal-mask constants.
- [`src/train/cuda_backend.rs`](src/train/cuda_backend.rs) implements the
  `TrainerBackend` contract on `burn-cubecl-cuda`.

Executed on the prototype RTX 5090, the backend produces finite, byte-identical
repeated gradients across independent instances, snapshots and restores byte
exactly, and continues byte-identically after a restore. Canonical initialization
reproduces every INIT-001 per-tensor digest for all 111 tensors of the
135,285,504-parameter model.

Running the P10 parity fixture on hardware for the first time showed device gradients
that do not equal the P9B oracle's canonical bytes even though the forward matched
exactly. `tests/e1a_numerical_probe.rs` isolated the cause: contraction and reduction
order reproduce the oracle exactly, while the device `exp`, `ln`, `sin`, and `cos`
differ from Rust's host libm by one ULP — IEEE-754 requires correctly rounded square
root and division but not those. `PRECISION-002` (`docs/decision-ledger-v3.md`, ADR
0001) resolves this: the forward stays an exact-byte gate, gradients are bounded by
the frozen provider-independent policy, and determinism is untouched. Measured on the
RTX 5090: relative L2 `5.714e-6` against a `0.03` limit and cosine `0.999999999984`
against a `0.999` floor.

E1B has since batched the graph and bounded both materializations, so the canonical
model runs at the frozen micro-batch of 16 sequences: one training step is split into
several backward passes joined by an exact vector-Jacobian seed, differentiating the
cross-entropy head in position chunks and recomputing each layer only while its own
gradient is taken. Measured on the RTX 5090 at 16 sequences per dispatch: `31,879` MiB
peak and `16,221` targets/s. The conformance numbers are unchanged to the last digit, so
the staging is numerically transparent, and the gate now also runs at the production
batch widths rather than only at one sequence. True BF16 storage was implemented and
measured but not kept: it delivered equal throughput for `11.6x` worse gradient
conformance, because the frozen semantics place no storage point on the attention scores
or their softmax, which are the dominant tensors.

The projected wall clock is `34.2` hours against the frozen `28,800`-second completion
SLA, short by a factor of `4.3`. That constant is not retunable after measurement, so it
is a blocking input to the E5 admission projection. No training-run, performance, SLA,
or quality claim is made.

## Optional scale-up planning

Phase 19 installs the deterministic, non-publishing amendment planner behind the
explicitly requested optional scale-up boundary:

```powershell
cargo run --locked --offline --bin python-slm -- plan-scale-up --config <absolute-config-path>
```

The closed `python-slm-scale-up-config-v1` names one requested scope: the complete
untied GQA shape, the total valid-target count, and the completion SLA. The engine
recomputes every derived identity with exact checked integer arithmetic — the
component-wise parameter count, the `align_up(20 * P, 256 MiB)` accelerator floor,
the 65,536-target update schedule with zero overshoot, stored-prefix/span
accounting, the 100,000,000-target evaluation boundaries, quarter retention
anchors, and the exact 90 percent admission ceiling — and emits one compact
`python-slm-scale-up-plan-result-v1` object.

A valid amendment candidate may not shrink the canonical model, target count, or
time budget, and must strictly increase at least one of them; vocabulary stays
within the fixed 260-ID floor and the immutable `u16` shard bound. The result
names the earliest affected phase and the complete downstream rerun chain, binds a
deterministic `scale-up-candidate-<digest>` identity, and remains
`UNAPPROVED_CANDIDATE` with `qualification_status: "SKIPPED"`. It changes no
canonical constant, writes no artifact or receipt, and makes no approval,
execution, hardware, SLA, or quality claim: P19 owner approval, the create-new
contract/ledger/ADR amendment, and every affected phase rerun remain outstanding
before any scaled run exists.

## Automated quality gate

Run the non-publishing Phase 3 gate from native Windows:

```powershell
cargo run --locked -p xtask --bin xtask -- quality-gate
```

The gate uses fixed direct Cargo commands, offline dependency resolution, a fresh
temporary target directory, bounded output capture, timeouts, and a kill-on-close Windows
Job Object. It verifies formatting, Clippy, CPU-reference tests, xtask tests,
no-default-feature compilation, the P2 CUDA compile surface, and the product CUDA compile
surface. It compares repository status before and after execution and removes its
temporary target on success and failure.

Success writes one closed `python-slm-quality-gate-result-v1` JSON object to stdout with
`qualification_status: "SKIPPED"`. The command writes no qualification receipt,
approval, acceptance, pointer, or repository artifact. Non-Windows execution returns
`DEFERRED_POST_P16` before spawning tools.

## Non-publishing RTX 5090/CUDA probe

Phase 1B remains available as an optional diagnostic:

```powershell
cargo run --locked -p xtask --bin xtask -- probe-cuda
```

It discovers the prototype toolchain, builds and inspects SM120 plus PTX fallback
artifacts, exercises the 2,952,790,016-byte allocation, emits one local JSON result, and
removes its temporary state. A live invocation is not an implementation gate.

## Non-publishing backend selection

Phase 2 retains one production candidate, `burn-cubecl-cuda`, behind provider-neutral
Rust types:

```powershell
cargo run --locked -p xtask --features p2-cuda --bin xtask -- select-backend
```

The command reuses the P1B diagnostic and exercises exact forward, gradient, allocation,
synchronization, and cleanup checks in a contained child. ROCm and Metal remain
`DEFERRED_POST_P16`. This is primitive backend correctness, not hardware qualification,
performance, model/checkpoint parity, or a full-run claim.

## Development checks

```powershell
cargo test --locked -p xtask
cargo test --locked --features cpu-reference
cargo test --locked --test scaffold_contract
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --offline -- -D warnings
cargo check --locked --no-default-features --offline
cargo test --locked -p xtask --features p2-cuda --no-run --offline
cargo check --locked --no-default-features --features cuda --offline
```

CPU and no-default-feature builds do not discover or link accelerator components.
Historical P0/P0A/P1/P2 receipts, schemas, runs, acceptances, pointers, and seals are
immutable.
