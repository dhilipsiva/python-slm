# Historical Reference Validation

Validation date: 2026-08-11.

This document records the legacy implementation only. It is not clean-rebuild, GPU, data,
or performance qualification. The sealed Phase 0 machine evidence and signed `PASS`
approval are recorded in [`docs/receipts/P0.md`](docs/receipts/P0.md); target gates are
defined by [`docs/rebuild-contract.md`](docs/rebuild-contract.md),
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), and [`TODO.md`](TODO.md).
The sealed capture predates the architecture/TODO reconciliation and authenticates only
its frozen contract bytes and reference observations; technical review must include the
reconciliation change set and its resulting commit.

## Passed in this workspace

```text
cargo test --locked --features cpu-reference
22 passed; 0 failed

cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
passed

cargo check --locked --no-default-features --features cuda
passed

cargo clippy --locked --all-targets --no-default-features --features cuda -- -D warnings
passed

cargo fmt --all -- --check
passed
```

The tests execute:

- corpus round-trip, required completion footer, atomic finalization, and overwrite refusal;
- projected Parquet reads, per-row invalid UTF-8 preservation, and hard clone-budget refusal;
- strict Python CST accept/reject and generated-banner behavior;
- MinHash/exact-Jaccard deduplication;
- deterministic quality-proxy selection, BPE training, serialization, reload,
  tokenization, hashing, and mmap reading;
- tied/untied Llama shape and parameter-count agreement;
- causal-prefix invariance, contiguous GQA KV expansion, and attention backprop;
- two micro-steps of gradient accumulation, one AdamW update, and an atomically
  finalized model checkpoint;
- default 2,048-context reference-attention VRAM refusal.

The `plan` command returned 124,668,672 parameters for the requested GQA shape,
135,285,504 for `--gqa-135m`, and 69,444.444 token/s as the zero-overhead
eight-hour floor.

## Not validated here

The optional `cuda-msvc-link` feature was not executed on the requested native
toolchain. This host exposes CUDA 13.1 and Visual Studio 18/2026, has no cuDNN
development library installed, and does not have `cl.exe` in the current shell.
The build script intentionally expects CUDA 12.8/12.9 and Visual Studio 2022 for
the reproducible target. Its Rust build-script logic type-checked, and the normal
Burn CUDA feature compiled, but the native `cl.exe`/`nvcc`/library ABI probe still
must be run in the target x64 VS 2022 Developer PowerShell.

No RTX 5090 training benchmark was run. No authorized The Stack v2 source blobs
were available. Consequently there is no evidence in this receipt for the
75,000-token/s acceptance target or the two-billion-token corpus result.
