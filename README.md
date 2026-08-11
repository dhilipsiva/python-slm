# Zero-Python Rust Llama pre-training reference

> **Rebuild notice:** this file documents the legacy behavioral oracle, not the target
> implementation. Normative rebuild decisions live in
> [`docs/rebuild-contract.md`](docs/rebuild-contract.md), the target design is
> [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), and ordered gates are in
> [`TODO.md`](TODO.md). Phase 0 is approved through its signed
> [`PASS` receipt](docs/receipts/P0.md). Commands, flags, schemas, artifacts, backend
> choices, and defaults below are historical unless a normative document explicitly
> retains them.

This repository implements the correctness side of a Windows/MSVC pipeline:

- bounded asynchronous HTTPS downloads of content-bearing Parquet shards;
- Arrow/Parquet decoding, per-document strict UTF-8 and tree-sitter Python validation;
- generated-file/comment filters and parallel 128-permutation MinHash deduplication;
- 32,000-entry byte-level BPE training on a decimal 2 GB subset;
- immutable, hashed, little-endian `u16` token shards with read-only `memmap2` access;
- a decoder-only Llama implementation with 12 query heads, 4 KV heads, RoPE,
  pre-RMSNorm, and SwiGLU;
- a differentiable AdamW reference loop with gradient accumulation, cosine decay,
  synchronized throughput reporting, and a fail-closed performance gate.

It does **not** claim that the supplied framework kernels train two billion tokens
in eight hours. Burn 0.21's CUDA backend has a fused attention *forward* path, but
its autodiff wrapper sends attention through the conventional quadratic graph.
The gate therefore rejects a production run unless the operator explicitly asks
for the correctness path. See [RESEARCH.md](RESEARCH.md) for historical feasibility
analysis; use the architecture and contract for target requirements.

## Exact model size

The legacy default shape has exactly **124,668,672** parameters with an untied output
projection; tied embeddings produce 100,092,672. The rebuild's canonical
`gqa-135m-v1` preset keeps GQA and changes `d_ff` from 2,048 to 2,432, producing exactly
**135,285,504** parameters. The 124M shape remains reference-only.
Using 12 KV heads (ordinary MHA) with the original widths would instead produce
134,105,856, so the stated 135M budget likely predates the switch to four KV heads.

| Component | Requested shape |
|---|---:|
| Token embedding | 24,576,000 |
| One decoder block | 6,292,992 |
| 12 blocks | 75,515,904 |
| Final RMSNorm | 768 |
| Untied LM head | 24,576,000 |
| Total | 124,668,672 |

Use `--gqa-135m` to inspect the corrected 135M variant.

## Windows build

Use an **x64 Developer PowerShell for Visual Studio 2022**, Rust's MSVC target,
and CUDA 12.8 or 12.9. CUDA 12.9 Update 2 is the recommended reproducible line.
The RTX 5090 is SM120; older CUDA 12.x toolkits cannot compile for it.

```powershell
rustup default stable-x86_64-pc-windows-msvc
$env:CUDA_PATH = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.9'
$env:CUDNN_PATH = 'C:\Program Files\NVIDIA\CUDNN\v9.21'
$env:CUDA_COMPUTE_CAP = '120'

cargo test --locked
cargo build --locked --release --no-default-features --features cuda-msvc-link
```

The optional `cuda-msvc-link` build script performs two ABI probes, invokes
`cl.exe` and `nvcc`, and verifies/links `cudart.lib`, `cublas.lib`, `curand.lib`,
and `cudnn.lib`. The normal CPU build deliberately does none of that. cuDNN is
linked to satisfy the requested ABI surface; the Llama graph itself does not call
cuDNN Transformer operations.

This is a zero-Python host workflow, not a literal all-Rust binary: NVIDIA's CUDA
libraries, the small C++/CUDA probes, and tree-sitter-python's generated C parser
are native code. No local Python interpreter or Python package is used.

## Pipeline

### 1. Inspect the arithmetic and gate

```powershell
cargo run --locked -- plan
cargo run --locked -- plan --gqa-135m
```

The report includes parameter count, training FLOPs per token, optimizer steps,
large activation sizes, and the exact 69,444.44 token/s arithmetic floor for an
eight-hour run. The configured acceptance target is 75,000 token/s to leave some
room for data, logging, evaluation, and checkpoints.

### 2. Curate content-bearing Parquet

Copy [remote-manifest.example.json](remote-manifest.example.json), replace its URL
and checksum with authorized content-bearing shards, then run:

```powershell
cargo run --locked --release -- curate `
  --manifest remote-manifest.json `
  --work-dir work\curation `
  --output data\python.corpus `
  --config curate.example.json
```

The downloader requires HTTPS and SHA-256 by default, caps response size, accepts credentials
only through the named environment variable, validates the declared hashes,
and never embeds a bearer token in the manifest. Files are ordered by manifest
position before filtering, so parallel filtering still keeps deterministic
"first document wins" deduplication.

Important: The Stack v2 Hugging Face Parquet files contain Software Heritage IDs
and metadata, not source bytes. Bulk source access requires the applicable
Software Heritage/INRIA agreement and AWS credentials. This program refuses to
pretend an ID column is code. An authorized operator must expose direct
content-bearing Parquet shards or presigned HTTPS materializations to this source
adapter. The raw files can also contain personal data and secrets; add a reviewed
PII/secret scrubber before production training.

The supplied adapter is therefore **not** a complete Stack v2 blob resolver. It
accepts only content-bearing Parquet that an authorized materialization stage has
already produced. Implementing the access-agreement-specific S3/gzip/encoding
stage is still required for an actual Stack v2 run.

The implementation streams HTTP bodies to bounded temporary files and lets
Parquet/Arrow allocate ordinary host buffers. `reqwest`, Arrow, and `memmap2` do
not decode directly into CUDA-pinned memory. A production loader should use a
small reusable pinned staging ring only at the final host-to-device boundary.
The default decoder batch is 16 rows and source files are capped at 16 MiB before
the pipeline clones, parses, or hashes them, limiting its cloned batch to 256 MiB.
Malformed binary UTF-8 is rejected and counted per row. Arrow may still allocate
an entire encoded page/row group internally, so this is not a hard bound on the
Parquet decoder itself; production shards must also use bounded page/row-group
sizes.

### 3. Train BPE

```powershell
cargo run --locked --release -- train-tokenizer `
  --corpus data\python.corpus `
  --subset work\tokenizer\subset.txt `
  --output data\tokenizer.json
```

The default is a byte-level, no-Unicode-normalization BPE with 32,000 entries and
`<pad>`, `<s>`, `</s>`, and `<unk>` special tokens. The subset limit is exactly
2,000,000,000 bytes, not 2 GiB. A two-pass deterministic selector scores every
scrubbed document for non-whitespace density, non-repeated lines, and useful file
size, then writes the highest-scoring set in stable corpus order. This is a code
quality proxy, not a license/provenance or benchmark-decontamination classifier.

### 4. Emit two billion tokens

```powershell
cargo run --locked --release -- tokenize `
  --corpus data\python.corpus `
  --tokenizer data\tokenizer.json `
  --output-dir data\tokens
```

Output shards are headerless little-endian `u16` values plus
`tokens.manifest.json`, which records lengths and SHA-256 hashes. Mappings are
read-only CPU mappings. They avoid an extra disk-to-heap copy, but are neither
CUDA-pinned nor GPU zero-copy.

### 5. Reference training smoke path

```powershell
cargo run --locked --release --no-default-features --features cuda -- train `
  --tokens data\tokens\tokens.manifest.json `
  --model-config model-smoke.example.json `
  --train-config train-smoke.example.json `
  --verify-hashes `
  --checkpoint checkpoints\final-model `
  --allow-reference-attention
```

Do not run the full defaults through this path: at micro-batch 16 and context
2,048, a single BF16 score matrix is 1.5 GiB per layer, and retaining scores plus
probabilities across 12 layers alone is about 36 GiB. The command is an executable
correctness oracle for reduced smoke-test configurations, not the optimized
trainer. Without `--allow-reference-attention`, it stops before allocating the
model.

If `--checkpoint` is supplied, the final model weights are synced to a temporary
Burn Named-MsgPack file and atomically renamed to `<base>.mpk`. This is a final
model-only artifact. Periodic checkpoints with AdamW state, data position, and
loss-scaler state are still required before a production run can be resumed.

## Determinism and restart policy

- Corpus, subset, tokenizer, and token writers use create-new semantics.
- Corpus records are first written to `*.part`; a completion footer is synced and
  the file is atomically renamed only after every shard succeeds. Readers require
  the footer and verify its document/byte counts.
- Final token manifests are written only after the exact target is reached.
- Download order and accepted-document order are deterministic.
- Token shards and cached Parquet files can be hash-verified on restart.
- Partial files are retained for inspection; the program never silently treats
  them as complete.

MinHash LSH is probabilistic candidate generation. With the default 16 bands of
8 rows it has about 99.4% candidate probability at 0.85 similarity, and candidates receive exact
shingle-Jaccard verification. Retaining all exact shingles is memory intensive;
the supplied in-memory index is not a two-billion-token production design. Before
a full run, partition it and persist signatures, exact shingles, and LSH buckets
in a disk-backed store instead of merely increasing RAM.

## Source layout

- `build.rs` and `kernels/`: MSVC/CUDA ABI and linkage probe.
- `src/data/`: ingestion, syntax filtering, deduplication, BPE, and mmap shards.
- `src/model.rs`: explicit GQA, RoPE, RMSNorm, and SwiGLU decoder.
- `src/train.rs`: optimizer, accumulation, throughput logging, and acceptance gate.
- `src/config.rs`: exact counts, schedule, FLOP range, and memory arithmetic.
