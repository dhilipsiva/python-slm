# Repository Guidelines

## Project Structure & Module Organization

This is a single Rust 2024 crate. `src/main.rs` defines the Clap CLI, while
`src/lib.rs` exposes reusable modules. Data ingestion, filtering, deduplication,
tokenizer training, and token shards live under `src/data/`. Model code is in
`src/model.rs`, training logic in `src/train.rs`, and model/training arithmetic in
`src/config.rs`. `build.rs` and `kernels/` contain the optional MSVC/CUDA ABI probes.
Root `*.example.json` files are configuration templates; `docs/research/` is
background material. Keep generated data in ignored paths such as `data/` and
`target/`.

## Build, Test, and Development Commands

- `cargo run --locked -- plan`: fast CLI smoke test; prints parameter, memory,
  FLOP, and throughput-gate calculations.
- `cargo test --locked --features cpu-reference`: run the CPU correctness suite.
- `cargo fmt --all -- --check`: verify formatting without changing files.
- `cargo clippy --locked --all-targets --features cpu-reference -- -D warnings`:
  lint all default targets and reject warnings.
- `cargo check --locked --no-default-features --features cuda`: compile-check the
  CUDA graph without invoking the native link probe.

The `cuda-msvc-link` release build requires an x64 VS 2022 Developer PowerShell
and the documented CUDA/cuDNN environment. Consult `README.md` and `VALIDATION.md`
before treating it as verified.

## Coding Style & Naming Conventions

Use Rust 1.96 or newer and preserve `Cargo.lock`. Rustfmt uses four-space
indentation, a 100-column limit, and field-init shorthand. Follow standard Rust
naming: `snake_case` for modules, functions, and tests; `PascalCase` for types;
`SCREAMING_SNAKE_CASE` for constants. Keep feature-specific code behind its
existing `cpu-reference`, `cuda`, or `cuda-msvc-link` gate. Do not weaken
`#![deny(unsafe_op_in_unsafe_fn)]`.

## Testing Guidelines

Place unit tests beside the implementation in `#[cfg(test)] mod tests` blocks.
Use descriptive names such as `corpus_round_trip_and_overwrite_refusal`. Add a
regression test for behavior changes and gate backend-dependent tests explicitly.
No coverage threshold is configured; passing tests, rustfmt, and warning-free
Clippy are the required baseline.

## Commit & Pull Request Guidelines

History currently contains only `initial commit`, so no established convention
can be inferred. Use short, imperative subjects, optionally scoped, for example
`data: reject oversized parquet rows`. Pull requests should explain the change
and rationale, identify configuration or data-format effects, link relevant
issues, and list verification commands run. Attach logs or screenshots only when
they clarify CLI output, failures, or performance claims.

## Security & Configuration

Require HTTPS and SHA-256 for remote shards. Pass credentials through named
environment variables, never in manifests or source control. Do not commit
datasets, caches, token shards, checkpoints, or secrets.
