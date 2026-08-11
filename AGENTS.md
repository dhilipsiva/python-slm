# Repository Guidelines

## Project Structure & Module Organization

Read authority in this order: this guide governs work, `docs/rebuild-contract.md`
records Phase 0 product decisions, `docs/ARCHITECTURE.md` defines the target, and
`TODO.md` defines phase dependencies and gates. The signed `PASS` receipt at
`docs/receipts/P0.md` approves the sealed contract; conflicts stop work. The current
Rust 2024 crate, root `*.example.json`, `build.rs`, and `kernels/` are behavioral evidence
only; rebuild modules, schemas, formats, tests, and backend integrations from
scratch. `docs/research/` is non-normative background. Keep generated corpora,
tokens, caches, and checkpoints in ignored paths.

## Build, Test, and Development Commands

- `cargo run --locked -- plan`: inspect the legacy reference arithmetic and gate.
- `cargo test --locked --features cpu-reference`: run legacy CPU oracle tests.
- `cargo fmt --all -- --check`: verify formatting without changing files.
- `cargo clippy --locked --all-targets --features cpu-reference -- -D warnings`:
  lint default targets and reject warnings.
- `cargo check --locked --no-default-features --features cuda`: compile-check the
  CUDA graph without invoking the native link probe.

These commands validate only the reference until P3 supplies
`scripts/quality-gate.ps1`. Never treat a compile check as a device, correctness,
VRAM, or performance qualification. Use each phase's exact TODO receipt commands
and a fresh target directory where required.

## Coding Style & Naming Conventions

Use Rust 1.96 or newer and preserve `Cargo.lock`. Follow rustfmt and standard Rust
naming: `snake_case` for modules/functions/tests, `PascalCase` for types, and
`SCREAMING_SNAKE_CASE` for constants. Keep CPU builds independent of CUDA and
feature-gate native code. Do not weaken `#![deny(unsafe_op_in_unsafe_fn)]`. During
cutover, follow the active phase rather than preserving legacy module boundaries.

## Testing Guidelines

Place focused unit tests beside implementations; put synthetic end-to-end and
backend parity suites under `tests/`, and performance work under `benches/`.
Name tests by behavior, add regressions for every defect, and gate GPU tests
explicitly. CPU correctness, CUDA parity, and performance are separate receipts.

## Commit & Pull Request Guidelines

History remains too sparse to infer a reliable convention. Use short, imperative
subjects, optionally scoped, for example
`data: reject oversized parquet rows`. Pull requests should explain the change
and rationale, identify configuration or data-format effects, link relevant
issues, and list verification commands run. Attach logs or screenshots only when
they clarify CLI output, failures, or performance claims.

## Security & Configuration

Require the contract's HTTPS, redirect, path-containment, mandatory hash-chain,
provenance, license, removal, and sensitive-data rules. Pass credentials through
named environment variables only. Never commit datasets, caches, token shards,
checkpoints, raw source, PII, or secrets.
