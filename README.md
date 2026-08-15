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
hash-bound `python-slm-tokenizer-sample-manifest-v1`, its immutable content root, and a
create-new tokenizer artifact path. Sample documents bind repository group, source,
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
`python-slm-governed-corpus-manifest-v1`, content root, P7 tokenizer sample and
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
`python-slm-governed-corpus-manifest-v2` artifacts plus representative source
bytes. P8 accepts both its immutable v1 governed manifest and P9A's v2 manifest;
the explicit P8 configuration supplies and verifies the later tokenizer
artifact binding, avoiding a circular pre-training hash.

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

Phase 7A adds hash-bound governed-source metadata to every curation outcome. The checked-in
default policy labels manifest-declared provenance, license, and removal facts ASSUMED;
freshness and aggregate source status remain UNVERIFIED while external review is unavailable.
The generation records only deterministic identity and policy bindings, not review claims or
sensitive values.

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
