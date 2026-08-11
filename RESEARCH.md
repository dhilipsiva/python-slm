# Feasibility and implementation boundary

> **Historical, non-normative analysis.** This file explains why the legacy reference is
> insufficient; it does not select the rebuild architecture or freeze policy. Use
> [`docs/rebuild-contract.md`](docs/rebuild-contract.md) for product decisions,
> [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the target design, and
> [`TODO.md`](TODO.md) for ordered qualification gates. Where this analysis prescribes a
> fixed microbatch, pinned transfer, adapter, index, flag, or backend, the normative
> documents control.

## Conclusion

A zero-Python Windows/MSVC data and reference-training pipeline is feasible. The
specific promise—two billion training tokens on one RTX 5090 in less than eight
hours—is **not established by existing Rust framework kernels** and is not a
credible acceptance claim for this repository.

The repository is an executable reference implementation, not a completed
production run: it still needs an authorized Stack v2 blob materializer, a
partitioned disk-backed deduplication index, and an SM120 fused training backend.
Those are acceptance blockers, not optional optimizations.

Two billion tokens in 28,800 seconds requires 69,444.44 token/s with no overhead.
At 70,000 token/s, only 229 seconds remain for startup, data stalls, evaluation,
logging, and checkpoints. The engineering acceptance threshold should be at least
75,000 sustained end-to-end training token/s, measured after warm-up over a long
run and including the real input path.

## Verified constraints

1. The RTX 5090 is a 32 GB GDDR7 Blackwell GPU and uses compute capability 12.0
   (SM120). CUDA 12.8 introduced SM120 compiler support; use CUDA 12.9 Update 2 for
   a reproducible late-12.x Windows target.
2. NVIDIA's architecture table gives the RTX 5090 a dense peak of 209.5 BF16
   Tensor TFLOP/s with FP32 accumulation (the second number shown is sparse).
3. The requested 124.67M model needs about 714M to 827M matmul-dominant training
   FLOPs/token, depending on triangular versus full-square attention accounting.
   At 70k token/s this is roughly 50.0–57.9 TFLOP/s. The arithmetic roofline does
   not rule out the target; kernel availability, utilization, memory traffic, and
   framework overhead determine whether it is achievable.
4. Burn 0.21's CubeCL backend has a fused attention forward dispatch, but the
   autodiff implementation explicitly invokes `attention_fallback`. Therefore the
   fused forward kernel is not used by the supplied training graph.
5. Candle 0.11's FlashAttention v3 wrapper supplies CUDA forward methods through
   `CustomOp3` but no backward implementation. Its build targets Hopper `sm90a`,
   not GeForce Blackwell `sm120`. It is not a drop-in training solution here.
6. Neither stock Burn nor stock Candle constitutes a validated native FP8 Llama
   training stack with scaling/amax tracking, FP32 master state, and SM120 fused
   backward kernels. BF16 is the correctness baseline.
7. The Stack v2 Hugging Face rows provide Software Heritage IDs and metadata.
   Bulk source blobs live behind a separate Software Heritage S3 access agreement
   and credentials. A generic Hugging Face Parquet GET cannot yield source text.

## Why micro-batch 32 is not a safe starting assertion

For `B=32`, `H=12`, and `L=2048`, one BF16 attention-score tensor is:

`32 * 12 * 2048 * 2048 * 2 bytes = 3 GiB`.

A conventional backward graph keeps multiple such tensors. The BF16 logits alone
are `32 * 2048 * 32000 * 2 = 3.90625 GiB`. These values exclude Q/K/V, residuals,
SwiGLU activations, gradients, allocator workspace, and optimizer state. Fused
attention backward, activation recomputation, and chunked/fused cross-entropy are
mandatory for the proposed batch scale.

Micro-batch 16 with accumulation 2 preserves 65,536 tokens per optimizer update
while halving the largest batch-shaped tensors. It is only an initial autotuning
candidate after fused kernels exist, not a promise that 28 GB will suffice.

## Kernel work required to open the production gate

The optimized implementation should remain behind a separate feature and must
provide all of the following before the eight-hour run is allowed:

1. An SM120 BF16 causal attention forward **and backward** kernel supporting the
   requested 3:1 Q:KV ratio without materializing repeated K/V or `[B,H,L,L]`.
2. Block-level activation checkpointing/recomputation.
3. Chunked or fused linear-plus-cross-entropy so `[B,L,32000]` logits are not
   retained in full precision across backward.
4. A fused AdamW path with FP32 moments and preferably FP32 master weights;
   gradient unscaling, overflow/non-finite detection, and norm clipping.
5. A bounded, overlapped loader with read-only mmap shards, a reusable CUDA-pinned
   staging ring, asynchronous H2D copies, and double buffering.
6. An OOM-aware autotuner that starts below the target micro-batch, observes peak
   allocated/reserved VRAM, and increases only while preserving a safety margin.
7. An end-to-end benchmark at the full 2,048 context and real model. It must report
   warm-up-excluded tokens/s, step latency percentiles, GPU utilization, memory,
   data-wait time, loss, and non-finite counts for at least several hundred steps.
8. Periodic, atomic restart checkpoints containing model and optimizer state,
   scheduler/data position, and any mixed-precision scaling state. The reference
   CLI's optional final `.mpk` file contains model weights only.

Until those conditions are met, `allow_reference_attention=false` is a deliberate
fail-closed default. Changing the flag proves only that the differentiable graph
runs at a small test size.

## Data-quality boundaries

Syntax validity and near-deduplication do not by themselves produce a safe or
legally clean training set. A production corpus also needs license-policy
enforcement, opt-out/version tracking, malware filtering, secret and PII scanning,
benchmark decontamination, repository-level train/eval splitting, and an immutable
provenance manifest. Those are policy inputs, not facts the parser can infer.

Comment percentage here is CST comment-node bytes divided by source bytes.
Docstrings remain executable string literals and are not counted as comments.
Generated banners are inspected only in the first 8 KiB to avoid rejecting an
ordinary string literal later in a program.

The implemented Parquet adapter deliberately begins after source materialization.
It does not use Software Heritage/AWS credentials, fetch gzip blobs by content ID,
or transcode the Stack v2 `src_encoding` field. Its input contract is a direct
content-bearing Parquet shard with an optional string/binary ID column. Likewise,
the exact MinHash index is in-memory; despite eliminating per-shingle temporary
byte-vector allocations, retaining every signature, shingle set, and LSH posting
does not scale to the requested corpus. A credential-aware materializer and
partitioned external-memory index must precede a claimed two-billion-token build.

## Pinned-memory correction

Network and Parquet libraries own their intermediate allocations. `reqwest` yields
its own byte buffers; Parquet creates Arrow buffers; `memmap2` maps pageable host
pages. "Decompress directly into pinned memory" is therefore not a property of
this stack. Pinning the entire curated corpus would also waste a scarce OS/GPU
resource. Use normal CPU memory during curation and pin only a small final staging
ring for transfer.

## Primary sources checked

- [The Stack v2 dataset card](https://huggingface.co/datasets/bigcode/the-stack-v2)
- [CUDA 12.8 release notes](https://docs.nvidia.com/cuda/archive/12.8.0/cuda-toolkit-release-notes/index.html)
- [NVIDIA CUDA GPU compute-capability table](https://developer.nvidia.com/cuda-gpus)
- [GeForce RTX 5090 specifications](https://www.nvidia.com/en-us/geforce/graphics-cards/50-series/rtx-5090/)
- [NVIDIA RTX Blackwell architecture whitepaper](https://images.nvidia.com/aem-dam/Solutions/geforce/blackwell/nvidia-rtx-blackwell-gpu-architecture.pdf)
- [Burn 0.21 autodiff attention implementation](https://raw.githubusercontent.com/tracel-ai/burn/v0.21.0/crates/burn-autodiff/src/ops/module.rs)
- [Burn 0.21 CubeCL attention dispatch](https://raw.githubusercontent.com/tracel-ai/burn/v0.21.0/crates/burn-cubecl/src/ops/module.rs)
- [Candle 0.11 FlashAttention v3 Rust wrapper](https://raw.githubusercontent.com/huggingface/candle/0.11.0/candle-flash-attn-v3/src/lib.rs)
- [Candle 0.11 FlashAttention v3 build script](https://raw.githubusercontent.com/huggingface/candle/0.11.0/candle-flash-attn-v3/build.rs)
- [tree-sitter-python API](https://docs.rs/tree-sitter-python/latest/tree_sitter_python/)
- [tokenizers 0.23.1 API](https://docs.rs/tokenizers/0.23.1/tokenizers/)
- [memmap2 API and safety contract](https://docs.rs/memmap2/latest/memmap2/)
