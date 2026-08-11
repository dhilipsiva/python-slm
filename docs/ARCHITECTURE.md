# Zero-Python Rust Pre-Training Architecture

Status: target design for the clean rebuild. The existing `src/`, `build.rs`, and
root configuration files are reference evidence only; they are not the implementation
baseline. `docs/rebuild-contract.md` is the normative Phase 0 refinement and `TODO.md`
is the ordered execution specification. A conflict among these documents is a stop
condition. The Phase 0 receipt remains `AWAITING_REVIEW`; technical and data-governance
owner approvals are required before any phase that depends on P0 may start.

## Scope and Success Contract

Build a Windows-native, zero-Python pipeline that curates authorized Python source,
trains a qualified 32K byte-level BPE tokenizer, materializes enough governed token IDs
to supply exactly 2,000,000,000 valid non-padding next-token training targets without
corpus-end wrap, and pre-trains the canonical 135,285,504-parameter decoder on one
RTX 5090.

"Zero Python" means no Python interpreter, Python package, or Python subprocess in
build, curation, tokenization, training, or qualification. Rust owns the control plane.
NVIDIA libraries and a small, feature-gated C/CUDA ABI are permitted. This is not a
literal all-Rust binary: CUDA and Tree-sitter include native code.

After corpus materialization, the eight-hour timer starts immediately before the trainer
opens the frozen artifacts. It includes artifact-integrity verification, startup and model
initialization, any JIT/autotuning incurred by the run, data loading, all
forward/backward/optimizer work, configured evaluation, synchronization, checkpointing,
final-checkpoint durability, and any recovery downtime. Completion requires exactly
2,000,000,000 valid predicted targets and the final durable checkpoint within 28,800
seconds. The canonical training prefix contains 2,000,000,001 stored IDs, exposing exactly
2,000,000,000 consumed real inputs and valid targets with zero boundary exclusions.
Runtime padding inputs and masked targets, stored unused tail, and unmaterialized
documents/bytes are separate counters; a short-run projection only gates entry to the
full run.

## Research Disposition

All 12 files in `docs/research/` were reviewed.

| Classification | Files | Use |
|---|---|---|
| Architecture | `We’ll produce a detailed specificat.txt`, `This is tight but feasible. 2B toke.txt`, `Rust LLM Pre-Training Pipeline.md`, `deep-research-report.md`, `deep-research-report(2).md`, `Conclusion A pure-Rust pipeline is.txt`, `# Pure-Rust LLM Pre-Training Pipeli.txt` | Requirements, candidate designs, and validation gates |
| Prompt chains | `We’ll interpret “Antigenic code too.txt`, `Untitled.txt`, `The ordered prompt chain below driv.txt`, `In Antigravity the workflow is Plan.txt`, `Rust LLM Pipeline Agentic Prompts.md` | Phase and verification patterns |

`deep-research-report(2).md` is the primary synthesis, supplemented by
`deep-research-report.md`. The prompt-card structure from
`The ordered prompt chain below driv.txt` is retained. Code fragments in the research
files are non-normative: several contain concrete shape, iteration, serialization,
label-shift, or accumulation defects and must not be copied.

`Conclusion A pure-Rust pipeline is.txt` is a secondary summary. The remaining four
architecture drafts are rejected as implementation sources: they variously miscount the
model, misidentify the GPU architecture, conflate Stack-v2 identifiers with content,
label pageable memory as pinned, prescribe Hopper-only attention, or assert unmeasured
throughput. Their sound high-level sequencing is retained only where independently
validated or explicitly marked as a qualification gate below.

## Architecture Decisions and Validation Gates

| Topic | Decision |
|---|---|
| Model size | Canonical preset is 135,285,504 parameters: vocab 32,000; width 768; `d_ff=2432`; 12 layers; 12 query and 4 KV heads; untied output head; no biases. The `d_ff=2048` preset is explicitly 124,668,672, not 135M. |
| GPU target | RTX 5090 is compute capability 12.0/SM120. Native compilation needs an SM120-capable toolkit; CUDA 12.8 is the minimum 12.x release with SM120 compiler support. Pin the version actually qualified. |
| Backend | Select Burn, Candle, or a lower-level path on the target Windows host using synchronized BF16 primitive, one-layer, and full training-step forward/backward benchmarks reporting numerical parity, p50/p95 latency, throughput, and peak VRAM. No framework is approved by documentation alone. |
| Attention | Build a causal-GQA reference first, then a measured SM120-compatible fused forward/backward path. Upstream FlashAttention-3 is Hopper-specific. FlashAttention-4 names Blackwell but currently uses a Python/CuTeDSL package, so neither is a zero-Python Windows baseline. |
| Precision | BF16 parameters and activations with FP32 accumulation for BF16 GEMMs, RMSNorm, softmax, loss, and gradient reductions; FP32 Adam moments and master weights. Any narrower layout requires forward, gradient, resume, and short loss-curve parity. FP8 remains an isolated later experiment with explicit scaling and layout contracts. |
| Data source | The Stack v2 records are identifiers/provenance, not source content. Use a separate, authorized Software Heritage content adapter. Also support genuinely content-bearing Parquet. |
| GPU input | `mmap` is CPU/page-cache access, not CUDA-pinned memory. Bulk-gather into reusable contiguous host buffers; benchmark pageable transfer against a bounded two/three-slot page-locked ring and adopt pinning only if end-to-end trainer throughput improves. |
| Deduplication | Define similarity over sets of lexical-token 5-grams after a versioned dedup-only normalization. Use 256-component seeded MinHash/LSH for candidate retrieval and exact shingle Jaccard for the `>0.85` decision. LSH recall must be measured; never claim exhaustive removal without evidence. |
| Throughput | The arithmetic floor is 69,444.44 valid predicted targets/s. Entry to the full run requires at least 75K synchronized steady-state targets/s for 30–60 minutes, preferably 80K, plus a whole-run projection below 28,800 seconds including every SLA-clock overhead. Projection alone never passes the SLA. |

Verified facts currently cover model arithmetic, the target GPU's compute capability, and
the CUDA compiler floor. Backend selection, memory-efficient backward, transfer strategy,
authorized corpus access/yield, loss stability, VRAM, sustained throughput, and eight-hour
completion remain unqualified gates.

## System Architecture

```mermaid
flowchart LR
    A["Source metadata and manifests"] --> B["Authorized bounded content fetch"]
    B --> C["Decode plus provenance and hash"]
    C --> D["CST, quality, license, PII and secret policy"]
    D --> E["Exact hash plus MinHash/LSH plus exact Jaccard"]
    E --> F["Pinned benchmark decontamination"]
    F --> G["Repository/duplicate-component 98/1/1 split"]
    G --> H["Capped deterministic tokenizer sample"]
    H --> I["Qualified 32K byte-level BPE"]
    G --> J["Tokenize immutable train and held-out splits"]
    I --> J
    J --> K["Immutable u16le shards, indexes and manifests"]
    K --> L["mmap bulk-span sampler"]
    L --> M["Reusable contiguous host staging"]
    M --> N["Qualified pageable or pinned H2D"]
    N --> O["BF16 model and fused loss"]
    O --> P["AdamW, telemetry and atomic resumable checkpoint"]
```

Start as one Cargo package with strict module boundaries. Split crates only when an
observed build, ownership, or dependency problem justifies it.

```text
src/
  main.rs       one installed python-slm executable
  commands/     plan, curate, train-tokenizer, tokenize, inspect, bench, train
  config/       versioned typed configuration and validation
  data/         source adapters, provenance, filters, dedup, splits
  tokenizer/    sampling, BPE training, artifact validation
  storage/      u16le format, manifests, indexes, mmap loader
  model/        config, reference math, GQA, RoPE, RMSNorm, SwiGLU
  backend/      selected framework plus optional CUDA boundary
  train/        staging, optimizer, schedule, checkpoint, metrics
tests/          synthetic end-to-end and backend parity tests
benches/        ingestion, dedup, transfer, attention, full-step benchmarks
scripts/        environment verification and reproducible qualification
```

CPU builds must neither discover nor link CUDA. Native probes and custom kernels are
feature-gated. CLI subcommands are independently restartable and consume immutable,
versioned artifacts rather than hidden process state. Mutating production subcommands
require explicit versioned JSON configurations whose schemas reject unknown fields.
Handled success and failure follow `CLI-001`, `CONFIG-001`, and `ERROR-001`: one terminal
success JSON object on stdout, typed JSONL errors on stderr, and fixed exit categories.

Any custom native path uses a small versioned `extern "C"` ABI. Its safe Rust wrapper
validates dtype, device, shape, stride, alignment, lengths, and stream compatibility;
ownership/borrow guards keep allocations and streams alive through asynchronous
completion. Native code returns CUDA status values and never unwinds across the boundary.
Kernel builds emit an SM120 image and a PTX fallback when supported by the qualified
toolchain.

## Data and Artifact Contracts

Each source record carries a stable source ID, an always-present derived repository-group
ID, source snapshot, declared license/provenance including explicit unknown, removal-list
version, and declared encoding. A provider repository ID may be absent; `SOURCE-002`
defines the conservative fallback grouping and unambiguous identifier serialization. The
governed raw artifact binds the original bytes to their raw SHA-256. A
successful deterministic decode creates canonical UTF-8 bytes and a decoded SHA-256; an
unsupported or invalid declared encoding is rejected with a reason code, never silently
replacement-decoded. In version 1, accepted curated bytes equal decoded bytes and their
hashes match; tokenizer-visible transformation is prohibited. The original raw artifact
remains immutable.

Version 1 accepts only strict UTF-8/ASCII Python 3 under `tree-sitter-python 0.25.0`, with
the BOM/cookie rules in `SOURCE-002`, and canonical decoded size `100..=1,000,000` decimal
bytes. The `permissive-v1` SPDX allowlist is exactly `0BSD`, `Apache-2.0`,
`BSD-2-Clause`, `BSD-3-Clause`, `BSL-1.0`, `ISC`, `MIT`, `MIT-0`, `Python-2.0`, and
`Zlib`; missing, conflicting, exception-bearing, copyleft, or otherwise unapproved
expressions fail according to `GOV-001`.

Credentials come only from named environment variables. Fetching uses HTTPS, timeouts,
retries, bounded concurrency, resumable caches, declared size limits, and checksums.
Redirects are bounded and revalidated; credentials never cross origins; local, loopback,
and link-local targets require an explicit fixture mode. Manifest paths are relative and
cannot escape their artifact root. Public network services are never used by normal tests.

I/O runs on Tokio; CPU-heavy parsing and hashing run in a bounded Rayon pool. Every
channel and batch has a configured memory limit. One Tree-sitter parser belongs to
each worker, and emitted order is canonical rather than completion-order dependent. The
curation policy records reason-coded decisions and covers encoding,
minimum and maximum size, Python dialect, syntax errors, actual comment-node bytes,
generated markers found independently inside each Tree-sitter comment-node intersection
with canonical byte range `[0,8192)`, license/removal rules, and the `sensitive-v1`
PII/secret policy. Confirmed findings reject the whole document, uncertain findings are
quarantined, and v1 never rewrites tokenizer-visible source. Benchmark decontamination
precedes the deterministic repository/duplicate-connected-component 98/1/1 split. The
split algorithm, component identity, and EvalPlus registry/version/hash are artifact
inputs.

Deduplication is partitioned or disk-backed at production scale. Its lexical tokenizer,
normalization, 256 fixed domain-separated affine MinHash components, 32-band by 8-row LSH
layout, exact-Jaccard threshold, and representative policy are defined by `DEDUP-001..003`.
Candidate recall and final precision must pass the sealed 10,000-pair qualification suite.
The tokenizer sample contains only deduplicated, decontaminated training documents. It
uses the `TOKSAMPLE-001` SHA-256 ranking, a 10,000,000-byte repository-group cap, and a
2,000,000,000-byte global cap; a qualified sample contains 1,999,000,000 through
2,000,000,000 complete-document bytes. BPE seeds all 256 byte symbols, uses minimum merge
frequency two, and applies no case folding, Unicode normalization, or whitespace stripping.
Qualification requires exactly 32,000
contiguous IDs, `max_id = 31,999`, zero unknown IDs for supported source,
byte-roundtrip over curated UTF-8 source with special-token injection disabled, and a
repetitive-input overflow regression. Two clean runs over the same ordered input manifest
must produce byte-identical tokenizer artifacts. The canonical special IDs are
`<pad>=0`, `<s>=1`, `</s>=2`, and `<unk>=3`; `</s>` separates documents.

Token artifacts are immutable raw little-endian `u16` shards plus a versioned JSON
manifest and document/sequence indexes. The manifest records tokenizer, source,
configuration, split, and shard hashes; global offsets; token counts; endianness; special
IDs; and document-boundary policy. Writers create unique same-volume partial files,
sync, and atomically finalize without overwriting existing artifacts. Readers verify
versions, IDs, lengths, and hashes before mapping and keep files read-only;
post-verification external mutation is a prohibited precondition, not something claimed
detectable in real time. Readers interpret IDs with explicit little-endian decoding; a
zero-copy cast is permitted only after compile-time little-endian and runtime
alignment/even-length checks. A 2,048-token causal sample consumes a 2,049-token logical
span: inputs are `t[0..2048]`, targets are `t[1..2049]`. Samples may cross physical shard
boundaries using global manifest offsets, but never wrap at corpus end. Exactly one EOS is
stored after every document; EOS-to-next-document transitions are ordinary valid targets
and positions do not reset at EOS. Materialization stops after the first complete document
whose EOS reaches at least 2,000,000,001 IDs. The prefix is contracted training data and
only that document's remainder is stored unused tail; later documents are counted but not
materialized. The prefix yields 976,562 full 2,048-target spans and one final 1,024-target
span. The final span adds 1,024 runtime PAD inputs and masked targets. The fixed
`SPAN-001` shuffle orders only full spans without replacement and keeps the partial span
last.

## Model and Training Contracts

The canonical model is bias-free and pre-normalized. Head width is 64. Query head `q`
maps to KV head `floor(q/3)`, so groups `0..2`, `3..5`, `6..8`, and `9..11` map to KV
heads `0..3`. The optimized path addresses
KV heads directly rather than materializing three copies. RoPE uses base 10,000, adjacent
pairs `(2i, 2i+1)`, positions `[0, 2048)`, and resets at each sample start. RMSNorm uses
epsilon `1e-5` with FP32 sum-of-squares accumulation.
SwiGLU is `down(SiLU(gate(x)) * up(x))`. The mask is inclusive lower-triangular: query
`i` may attend valid key `j` iff `j <= i`; EOS does not reset attention. Every optimized
operation has a deterministic reference and dtype-specific parity tolerance.

The RoPE pairing/layout convention is explicit in configuration and parity fixtures,
never inherited from a backend default. Canonical dropout is zero. Matrix and embedding
weights initialize from `Normal(0, 0.02)` and norm scales initialize to one; there is no
residual-specific scaling. `INIT-001` and `PARAM-001` fix parameter names, initialization
order, per-preset ChaCha12/StandardNormal seed, rounding, and initialized-artifact hashes.

Do not materialize avoidable `[B, L, V]` logits: at batch 16, length 2,048, vocabulary
32K, BF16 logits alone occupy about 1.95 GiB. Use chunked or fused cross-entropy.
Likewise, the production attention backward must be memory-efficient; the quadratic
reference graph is only a correctness oracle.

The canonical recipe uses AdamW `beta1=0.9`, `beta2=0.95`, `eps=1e-8`, decoupled weight
decay `0.1`, and FP32 global-L2 gradient clipping at `1.0`. Embedding, LM-head, attention,
and FFN matrices decay; norm scales do not. `OPT-001` fixes the bias correction, epsilon
placement, decay equation, clipping order, and BF16 master-weight update. A full optimizer
update contains 65,536 valid targets. The run has 30,517 full updates and one final
37,888-target update, for 30,518 total. The first 1,000 updates warm linearly to `2.5e-3`;
cosine decay thereafter makes update 30,518 exactly `2.5e-4`. Scheduler steps advance on
optimizer updates, never microsteps.

Autotuning chooses microbatch/accumulation pairs while preserving 65,536 valid targets per
full update. Loss is normalized by the actual accumulated target count, gradients are
zeroed once per update, and clipping occurs after accumulation and before AdamW. The final
partial update is normalized by 37,888 targets and never duplicates or overshoots. Full
spans use the deterministic `SPAN-001` order; the partial span stays last.

At a completed optimizer boundary, a restart checkpoint atomically captures model
parameters, optimizer/master/moment state, scheduler and optimizer step, loss-scaling
state, RNG state, dataloader order and cursor, exact valid-predicted-target count, and all
relevant artifact/configuration hashes. An interrupted run must match an uninterrupted run
within a predeclared BF16 tolerance. Checkpoints occur after the first completed update
crossing each 100M-target threshold and at completion. Retention keeps the latest two plus
the first generations at or after 500M, 1B, 1.5B, and final 2B targets.

A fixed, immutable 1,000,000-target validation sample reports mean next-token loss and
perplexity before update one and after the first completed update crossing each 100M-target
threshold, including completion. Its selection and order are fixed by `EVAL-001`. Held-out
targets never increment the training valid-target count.
Evaluation and checkpointing are mandatory in the final run and their overhead is always
inside the SLA clock; no arbitrary loss target is invented after seeing results.

## Verification and Qualification

1. CPU unit/reference tests cover configuration, filters, dedup determinism, formats,
   parameter counts, model math, optimizer updates, and schedule boundaries.
2. CUDA tests compare every custom/fused forward and backward operation with the
   reference implementation.
3. A synthetic corpus exercises the full no-Python pipeline through checkpoint resume.
4. Performance suites are isolated from correctness CI and emit machine-readable JSON.
5. Runs advance through 1M and 100M valid predicted targets, then 30 minutes and 60
   minutes, before 2B.

Every rung records synchronized throughput, p50/p95 latency, loader stalls, checkpoint
time, host RAM, peak allocated/reserved VRAM, loss, gradient norm, and non-finite values.
Frozen-code qualification reports loader, kernel-only, steady-state trainer, and
whole-run-equivalent rates separately.

The final `PROV-001` receipt records exactly 2,000,000,000 valid predicted targets, zero
overshoot, total elapsed wall clock below 28,800 seconds, and final-checkpoint
synchronization and durability inside that clock. It includes approved contract and ledger
hashes; frozen source-tree hash; Git commit and dirty status; `Cargo.lock`; toolchain,
hardware, backend/kernel, configuration, tokenizer, corpus, split, source/removal approval,
telemetry, benchmark, log, and final-checkpoint hashes; peak VRAM; loss/evaluation
diagnostics; all non-overlapping stored/prefix/tail/unmaterialized/input/padding/target/
boundary/update counters; and checkpoint/evaluation counts.

## Explicit Non-Goals

- Counting corpus acquisition or curation inside the eight-hour training SLA.
- Treating current reference tests as GPU-performance evidence.
- Making FA3, FP8, CUTLASS, a fixed microbatch, or 28 GiB occupancy a prerequisite.
- Calling mmap, Arrow buffers, or `Bytes` CUDA-pinned without registration and proof.
- All-pairs deduplication, estimated-MinHash threshold decisions, or unbounded in-memory
  indexes.
- Treating Tree-sitter syntax acceptance as evidence that code is safe, useful, licensed,
  or free of sensitive material.
- Copying research pseudocode or inventing commands for an unverified agent product.

## Validation Record — 2026-08-11

The authoritative Phase 0 machine-evidence run is
`20260811T074740Z-d5008e94`. Its 68-file seal is
`184dc926bb9e5e2963a61182398580f7dedbf5aa5992f062dacfc6db6f1430f5`;
the contract and decision-ledger hashes are respectively
`fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4`
and `8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d`.
All 30 captured commands passed, but Phase 0 acceptance remains pending both owner
approvals. These results validate only the existing reference checkout and local host;
they do not qualify the rebuild, GPU training path, or eight-hour objective.
This architecture and `TODO.md` reconciliation is a separate change set from the capture.
The seal still authenticates its frozen contract bytes and reference observations, but it
does not attest to the reconciled documentation tree; technical approval must review the
resulting commit as well.

- `cargo test --locked --features cpu-reference`: 22 passed.
- `cargo run --locked -- plan`: confirmed 124,668,672 parameters for `d_ff=2048`, the
  69,444.44 valid-predicted-targets/s floor, 1.95 GiB BF16 logits, and a fail-closed
  production gate.
- `cargo run --locked -- plan --gqa-135m`: confirmed 135,285,504 parameters.

Earlier unsealed observations found that the locked CUDA feature graph compile-checked and
that the host exposed an RTX 5090 and CUDA toolkit. They are research context only: the
authoritative P0 capture did not qualify a device launch, native MSVC/CUDA link, backend,
VRAM use, or throughput.

Primary-source checks: [NVIDIA GPU compute capabilities](https://developer.nvidia.com/cuda/gpus),
[CUDA 12.8 SM120 release notes](https://docs.nvidia.com/cuda/archive/12.8.0/cuda-toolkit-release-notes/index.html#new-features),
[NVIDIA pinned-memory guidance](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html#pinned-memory),
[The Stack v2 dataset card](https://huggingface.co/datasets/bigcode/the-stack-v2/blob/main/README.md),
[Burn releases](https://github.com/Tracel-AI/burn/releases),
[Candle README](https://github.com/huggingface/candle/blob/main/README.md),
[FlashAttention repository](https://github.com/Dao-AILab/flash-attention),
[Parquet 59.1 async reader](https://docs.rs/parquet/59.1.0/parquet/arrow/async_reader/), and
[Tokenizers 0.23.1 Rust BPE API](https://docs.rs/tokenizers/0.23.1/tokenizers/models/bpe/).
