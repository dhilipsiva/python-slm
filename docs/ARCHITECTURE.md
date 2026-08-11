# Zero-Python Rust Pre-Training Architecture

Status: target design for the clean rebuild. The existing `src/`, `build.rs`, and
root configuration files are reference evidence only; they are not the implementation
baseline. `TODO.md` is the ordered execution specification.

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
and final-checkpoint durability. Completion requires exactly 2,000,000,000 valid
predicted targets and the final durable checkpoint within 28,800 seconds. Stored corpus
IDs, consumed input IDs, valid predicted targets, padding/boundary exclusions, and
rejected or unused tail are reported separately; a short-run projection only gates entry
to the full run.

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
    E --> F["Repository/cluster split and decontamination"]
    F --> G["Capped deterministic tokenizer sample"]
    G --> H["Qualified 32K byte-level BPE"]
    F --> I["Tokenize immutable train and held-out splits"]
    H --> I
    I --> J["Immutable u16le shards, indexes and manifests"]
    J --> K["mmap bulk-span sampler"]
    K --> L["Reusable contiguous host staging"]
    L --> M["Qualified pageable or pinned H2D"]
    M --> N["BF16 model and fused loss"]
    N --> O["AdamW, telemetry and atomic resumable checkpoint"]
```

Start as one Cargo package with strict module boundaries. Split crates only when an
observed build, ownership, or dependency problem justifies it.

```text
src/
  bin/          curate, train-tokenizer, tokenize, inspect, bench, train
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
feature-gated. CLI stages are independently restartable and consume immutable,
versioned artifacts rather than hidden process state.

Any custom native path uses a small versioned `extern "C"` ABI. Its safe Rust wrapper
validates dtype, device, shape, stride, alignment, lengths, and stream compatibility;
ownership/borrow guards keep allocations and streams alive through asynchronous
completion. Native code returns CUDA status values and never unwinds across the boundary.
Kernel builds emit an SM120 image and a PTX fallback when supported by the qualified
toolchain.

## Data and Artifact Contracts

Each source record carries a stable source ID, optional repository ID, source snapshot,
declared license/provenance including explicit unknown, removal-list version, and declared
encoding. The governed raw artifact binds the original bytes to their raw SHA-256. A
successful deterministic decode creates canonical UTF-8 bytes and a decoded SHA-256; an
unsupported or invalid declared encoding is rejected with a reason code, never silently
replacement-decoded. A permitted policy transformation creates curated bytes, a curated
SHA-256, and a transformation record; when unchanged, the curated hash equals the decoded
hash. The original raw artifact remains immutable.

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
header-scoped generated markers, license/removal rules, and an explicit PII/secret
classify-and-reject, quarantine, or recorded-transform policy. Any permitted
transformation creates a new curated-content hash and reruns syntax filtering and
deduplication; source is never silently rewritten. Repository and duplicate-cluster
boundaries determine train/validation/test splits; benchmark decontamination occurs before
tokenization. The split algorithm, seed, and benchmark-registry version/hash are artifact
inputs.

Deduplication is partitioned or disk-backed at production scale. Its lexical tokenizer,
normalization, MinHash seed, signature width, LSH layout, and representative policy are
versioned artifact inputs. Representative selection is deterministic. The tokenizer
sample is selected after filtering and deduplication with per-repository caps and a
recorded decimal-byte budget. BPE seeds all 256 byte symbols and uses no case folding,
Unicode normalization, or whitespace stripping. Qualification requires exactly 32,000
contiguous IDs, `max_id <= 31,999`, zero unknown IDs for supported source,
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
boundaries using global manifest offsets, but never wrap at corpus end. Padding and
boundary exclusions are masked and excluded from valid-target counts; document
transitions require an explicit EOS token. The training-span sampler has a versioned
algorithm and seed, orders eligible spans without replacement, and hashes its final-tail
policy; it never silently duplicates a span to satisfy the target count.

## Model and Training Contracts

The canonical model is bias-free and pre-normalized. Head width is 64. Query heads
`0..2`, `3..5`, `6..8`, and `9..11` map to KV heads `0..3`. The optimized path addresses
KV heads directly rather than materializing three copies. RoPE uses base 10,000, adjacent
pairs `(2i, 2i+1)`, positions `[0, 2048)`, and resets at each sample start. RMSNorm uses
epsilon `1e-5` with FP32 sum-of-squares accumulation.
SwiGLU is `down(SiLU(gate(x)) * up(x))`. Every optimized operation has a deterministic
reference and dtype-specific parity tolerance.

The RoPE pairing/layout convention is explicit in configuration and parity fixtures,
never inherited from a backend default. Canonical dropout is zero. Matrix and embedding
weights initialize from `Normal(0, 0.02)` and norm scales initialize to one; any residual
projection scaling is explicit and artifact-hashed.

Do not materialize avoidable `[B, L, V]` logits: at batch 16, length 2,048, vocabulary
32K, BF16 logits alone occupy about 1.95 GiB. Use chunked or fused cross-entropy.
Likewise, the production attention backward must be memory-efficient; the quadratic
reference graph is only a correctness oracle.

AdamW defaults are `beta1=0.9`, `beta2=0.95`, `eps=1e-8`, weight decay `0.1`, and global
gradient clipping at `1.0`, with norm scales excluded by an explicit parameter policy. A
1,000-optimizer-step warmup to `2.5e-3` followed by cosine decay is a configurable
experiment preset, not a hidden framework default. Scheduler steps advance on optimizer
updates, never microsteps.
Autotuning chooses microbatch/accumulation pairs while preserving the configured valid
predicted targets per optimizer update; changing the global valid-target batch is a
separate, artifact-hashed experiment. Loss is normalized by the actual accumulated target
count, gradients are zeroed once per update, and clipping occurs after accumulation and
before AdamW. The final partial update consumes exactly the remaining targets without
duplication or overshoot.

At a completed optimizer boundary, a restart checkpoint atomically captures model
parameters, optimizer/master/moment state, scheduler and optimizer step, loss-scaling
state, RNG state, dataloader order and cursor, exact valid-predicted-target count, and all
relevant artifact/configuration hashes. An interrupted run must match an uninterrupted run
within a predeclared BF16 tolerance.

A fixed, immutable held-out split reports mean next-token loss and perplexity at a
configured cadence. Held-out targets never increment the training valid-target count.
Qualification includes evaluation and checkpoint overhead when those operations are
enabled for the final run; no arbitrary loss target is invented after seeing results.

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

The final receipt records exactly 2,000,000,000 valid predicted targets, zero overshoot,
total elapsed wall clock below 28,800 seconds, and final-checkpoint synchronization and
durability inside that clock. It hashes the frozen source tree, `Cargo.lock`, qualified
toolchain/hardware environment, backend and kernel build, configurations, tokenizer,
corpus and split manifests, and final checkpoint.

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

These results validate only the existing reference checkout and local host. They do not
qualify the clean rebuild, GPU training path, or eight-hour objective.

- `cargo test --locked --features cpu-reference`: 22 passed.
- `cargo run --locked -- plan`: confirmed 124,668,672 parameters for `d_ff=2048`, the
  69,444.44 valid-predicted-targets/s floor, 1.95 GiB BF16 logits, and a fail-closed
  production gate.
- `cargo run --locked -- plan --gqa-135m`: confirmed 135,285,504 parameters.
- `cargo check --locked --no-default-features --features cuda`: passed the locked CUDA
  feature-graph compile check; it did not launch a GPU kernel or prove fused backward.
- Host inspection: Rust 1.96/MSVC target, RTX 5090 compute capability 12.0, driver
  610.88, and CUDA 13.1. `cl.exe` was absent from the current shell, so no native
  MSVC/CUDA link or training benchmark was validated.

Primary-source checks: [NVIDIA GPU compute capabilities](https://developer.nvidia.com/cuda/gpus),
[CUDA 12.8 SM120 release notes](https://docs.nvidia.com/cuda/archive/12.8.0/cuda-toolkit-release-notes/index.html#new-features),
[NVIDIA pinned-memory guidance](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html#pinned-memory),
[The Stack v2 dataset card](https://huggingface.co/datasets/bigcode/the-stack-v2/blob/main/README.md),
[Burn releases](https://github.com/Tracel-AI/burn/releases),
[Candle README](https://github.com/huggingface/candle/blob/main/README.md),
[FlashAttention repository](https://github.com/Dao-AILab/flash-attention),
[Parquet 59.1 async reader](https://docs.rs/parquet/59.1.0/parquet/arrow/async_reader/), and
[Tokenizers 0.23.1 Rust BPE API](https://docs.rs/tokenizers/0.23.1/tokenizers/models/bpe/).
