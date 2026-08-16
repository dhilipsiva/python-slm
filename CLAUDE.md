# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

Despite the name, this is a **zero-Python** pure-Rust rebuild of a deterministic small-language-model training pipeline. Python-language source files are *input data only* (curated as a training corpus). No Python interpreter, package, code generator, or Python-launched subprocess may appear anywhere in build, data prep, training, verification, or receipts. The only permitted native code is the pinned `tree-sitter-python 0.25.0` generated C parser (behind the Rust data-policy boundary) and feature-gated accelerator backends.

`AGENTS.md` is the governing repository guide — read it before making non-trivial changes. Document authority order: `AGENTS.md` → immutable historical `docs/rebuild-contract.md` + P0 receipt → active `docs/rebuild-contract-v2.md`, `docs/decision-ledger-v2.md`, `docs/adr/0000-prototype-first-portable-interface.md` → `docs/ARCHITECTURE.md` (target design) → `TODO.md` (phase order and literal gates). Conflicts between these are a stop condition, not something to resolve ad hoc.

## Commands

```powershell
# Full automated quality gate (fmt, clippy, tests, feature-matrix compiles; Windows only)
cargo run --locked -p xtask --bin xtask -- quality-gate

# Test suites
cargo test --locked --features cpu-reference          # main CPU-reference suite
cargo test --locked -p xtask                          # xtask verifier tests
cargo test --locked --test scaffold_contract          # CLI/scaffold contract only

# Single test file / single test
cargo test --locked --features cpu-reference --test p12_trainer
cargo test --locked --features cpu-reference --test p12_trainer -- <test_name>

# Lint / format / feature-matrix checks
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --offline -- -D warnings
cargo check --locked --no-default-features --offline
cargo check --locked --no-default-features --features cuda --offline   # CUDA compile surface, no hardware needed

# Product CLI (single binary `python-slm`)
cargo run --locked --bin python-slm -- plan
cargo run --locked --bin python-slm -- curate --config <absolute-path>
cargo run --locked --bin python-slm -- train-tokenizer --config <absolute-path>
cargo run --locked --bin python-slm -- tokenize --config <absolute-path>
cargo run --locked --bin python-slm -- prepare-corpus | plan-spans | model-oracle | bench | train | evaluate-quality
```

Config paths passed to the CLI must be absolute. Configs are versioned, closed schemas that reject unknown fields; there are no hidden defaults or legacy fallbacks.

## Architecture

**Workspace**: root crate `rust-llm-pretrain` (produces the single `python-slm` binary from `src/main.rs`) plus `xtask/` (orchestration, phase-receipt verification, quality gate — the *only* normative automation entry point; shell scripts never are).

**Phase-driven structure**: the rebuild proceeds through numbered phases (P0–P19) defined in `TODO.md`. Each phase's integration suite lives at `tests/pN_*.rs` (e.g. `tests/p12_trainer.rs`), and commits are scoped like `train: implement checkpoints and exact resume`. Data pipeline: P4 curation (`src/data/`, license/provenance/removal policy) → P5 tree-sitter parsing (`src/parser/`) → P6/P6A privacy + adversarial filters → P7 byte-BPE tokenizer (`src/tokenizer.rs`) → P8 token materialization (`src/corpus.rs`, `src/storage.rs`) → P9A dedup/decontamination/split → P9B CPU model oracle (`src/model/`) → P10 CUDA backend (`src/backend.rs`) → P11 loading/transfers → P12 trainer/checkpoints (`src/train/`) → P16/P16A final run + quality evaluation → P17 host portability (`src/platform.rs`) → P18 accelerator provider adapters (`src/backend/`, `src/model/accelerator/`).

**Prototype-first gating**: P1–P16 target only `prototype-windows-5090-v1` (Windows x86_64 MSVC + RTX 5090/CUDA) behind provider-neutral interfaces. Any deferred platform/provider selection must fail with the typed code `DEFERRED_POST_P16` *before* reading config or mutating state — never fall back or fake success. Unimplemented subcommands fail with `PHASE_NOT_IMPLEMENTED`.

**Feature flags**: `cpu-reference` (default, intentionally dependency-light) plus isolated accelerator boundaries `cuda` (Windows/Linux), `rocm` (Linux), and `metal` (macOS), all burn 0.21 + cubecl. CPU/data builds must never discover, link, or load any accelerator SDK. Keep this separation strict when adding dependencies.

**Determinism is the product**: every artifact (source generations, tokenizer, token shards, checkpoints, results) is hash-bound (SHA-256 + length), published create-new (never overwrite), and verified on read including backing-file-mutation detection. RNG is pinned (`rand_chacha 0.10.0` ChaCha12, `rand_distr 0.6.0`); gradient/checkpoint comparisons are literal byte equality, never tolerance-based. Commands emit exactly one compact versioned JSON result object.

## Hard constraints

- **Never modify historical receipts**: `docs/receipts/**`, historical schemas, runs, acceptances, pointers, seals, and v1 contract bytes are immutable evidence. Historical `.ps1` files are archival, never executable entry points.
- **Frozen constants — do not retune**: 135,285,504 canonical parameters (`gqa-135m-v1`), 2,000,000,000 training targets, 2,952,790,016-byte allocation floor, 25,920 s admission projection, 28,800 s completion SLA, 32,000-token vocabulary. These are contract values, not measurements.
- Preserve `Cargo.lock` (all commands use `--locked`); Rust 2024 edition, 1.96+. Exact-pinned deps (`tree-sitter = =0.25.8`, `rand_chacha = =0.10.0`, etc.) stay pinned.
- Do not weaken `#![deny(unsafe_op_in_unsafe_fn)]`.
- A compile check is never evidence of device launch, correctness, performance, or qualification — keep claims in code, docs, and result objects labeled honestly (`SKIPPED`, `UNVERIFIED`, `OBSERVED_UNVERIFIED`).
- Generated corpora, token shards, caches, and `/checkpoints/` belong only in Git-ignored paths. Never commit datasets, PII, or secrets; credentials pass through named environment variables only.
- Tests: unit tests beside implementations, synthetic end-to-end/parity/resume suites under `tests/`, accelerator tests explicitly gated.
- Commits: short imperative subjects with optional scope, e.g. `data: reject oversized parquet rows`.
