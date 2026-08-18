# Repository Guidelines

## Project Structure & Module Organization

Read authority in this order: this guide governs work; the signed v1 P0 receipt and
`docs/rebuild-contract.md` remain immutable historical authority; after selected P0A
acceptance, `docs/rebuild-contract-v2.md`, `docs/decision-ledger-v2.md`, and
`docs/adr/0000-prototype-first-portable-interface.md` are the active amendment authority;
`docs/ARCHITECTURE.md` defines the target; and `TODO.md` defines phase dependencies and
literal gates. Before P0A acceptance, treat the v2 documents as the user-directed design
candidate awaiting the required owner decisions, not as a passing dependency.
Conflicts stop work.

The implementation sequence is prototype-first: P1-P16 build only
`prototype-windows-5090-v1` (native Windows x86_64, AMD Ryzen 9 9950X3D, MSVC/Windows
SDK, NVIDIA RTX 5090 SM120, dedicated VRAM, CUDA) behind provider-neutral interfaces;
deferred platform/provider selection fails with `DEFERRED_POST_P16` until P16A acceptance.
P16 is technical completion only. Before P15, freeze the complete P16A quantitative and
qualitative pack, including initialized and unigram aggregate-loss baselines and
deterministic generation settings; a named owner must approve the exact prompt/sample and
rubric artifact hashes before P15. P16A requires finite final aggregate held-out loss
strictly below both baselines, finite aggregate perplexity, frozen qualitative outputs,
and named owner quality approval. P16A alone unlocks P17 host/data portability or optional
P19. P18 depends on both P16A and P17 and requires four tuples: Windows/NVIDIA regression,
Linux/NVIDIA CUDA, Linux/AMD ROCm/HIP, and macOS/Apple Silicon Metal. P19 is
reserved for an optional larger-model or longer-budget amendment, not portability.

The current Rust 2024 crate, root `*.example.json`, `build.rs`, and `kernels/` are
behavioral evidence only; rebuild modules, schemas, formats, tests, and backend
integrations from scratch. `docs/research/` is non-normative background. Keep generated
corpora, tokens, caches, and checkpoints in ignored paths. Never rewrite or reinterpret
historical P0/P1/P2 receipts, schemas, runs, acceptances, pointers, seals, or v1 contract
bytes.
Historical `.ps1` capture artifacts are immutable archival evidence only and are never
executable entry points; active orchestration and verification use Rust `xtask`.

## Build, Test, and Development Commands

- `cargo test --locked -p xtask`: run the active cross-platform verifier tests.
- `cargo run --locked -p xtask --bin xtask -- verify-p0`: validate the immutable P0
  dependency chain through the direct-Rust semantic port.
- `cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --output-root
  docs/receipts/P0A`: prepare a create-new sealed P0A machine-evidence run.
- `cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --finalize
  --output-root docs/receipts/P0A`: after both owner approvals are committed, create the
  acceptance and atomically select it.
- `cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --check-selected
  --output-root docs/receipts/P0A`: validate the selected chain after the checkbox-only
  closure commit.
- `cargo run --locked -- plan`: inspect the legacy reference arithmetic and gate.
- `cargo test --locked --features cpu-reference`: run legacy CPU oracle tests.
- `cargo fmt --all -- --check`: verify formatting without changing files.
- `cargo clippy --locked --all-targets --features cpu-reference -- -D warnings`:
  lint default targets and reject warnings.
- `cargo check --locked --no-default-features --features cuda`: compile-check the
  CUDA graph without invoking the native link probe.

These commands validate only the reference until P3 supplies the stable Rust
`xtask quality-gate`. No platform shell script is a normative entry point. Never treat a
compile check as device launch, correctness, accelerator-memory, deterministic-resume,
performance, quality, portability, or full-run qualification. Use each phase's exact TODO
receipt command and a newly created target directory where required; never delete or reuse
an existing target directory as if it were fresh evidence.

## Coding Style & Naming Conventions

Use Rust 1.96 or newer and preserve `Cargo.lock`. Follow rustfmt and standard Rust naming:
`snake_case` for modules/functions/tests, `PascalCase` for types, and
`SCREAMING_SNAKE_CASE` for constants. Keep CPU/data builds independent of CUDA,
ROCm/HIP, Metal/MPS, and native ML backends. Put isolated `cuda`, `rocm`, and `metal`
features behind one provider-neutral Rust interface even while only the Windows/CUDA
prototype is implemented. Do not weaken `#![deny(unsafe_op_in_unsafe_fn)]`. During
cutover, follow the active phase rather than preserving legacy module boundaries.

## Testing Guidelines

Place focused unit tests beside implementations; put synthetic end-to-end, native-host,
parser-compatibility, backend-parity, and exact-resume suites under `tests/`, and
performance work under `benches/`. Name tests by behavior, add regressions for every
defect, and gate accelerator tests explicitly. CPU/data correctness, host portability,
exact provider-tuple environment, literal gradient bytes, byte-identical resume,
performance admission, P16 technical completion, P16A quality, and P18 matrix acceptance
are separate receipts and claims.

## Commit & Pull Request Guidelines

History remains too sparse to infer a reliable convention. Use short, imperative
subjects, optionally scoped, for example
`data: reject oversized parquet rows`. Pull requests should explain the change
and rationale, identify configuration or data-format effects, link relevant
issues, and list verification commands run. Attach logs or screenshots only when
they clarify CLI output, failures, or performance claims.

## Security & Configuration

Require the contract's HTTPS, redirect, path-containment, backing-file mutation detection,
mandatory hash-chain, provenance, license, removal, and sensitive-data rules. Pass
credentials through named environment variables only. Never commit datasets, caches,
token shards, checkpoints, raw source, PII, or secrets.

Zero Python forbids Python interpreters, executables, packages, modules, wheels, build
backends, code generators, embedded runtimes, and Python-launched subprocesses in build,
data, training, qualification, verification, or receipts. Python-language source is input
data only. Native code is limited to pinned, audited, feature-gated accelerator/native-ML
boundaries, the pinned `tree-sitter-python 0.25.0` generated C parser/runtime used only
through the Rust data-policy boundary, and, under `SCOPE-002`
(`docs/decision-ledger-v4.md`), the Parquet Zstandard decoder used only to decompress
governed metadata shards through the Rust Parquet reader. C/C++ may not own orchestration or unrelated data
transformation.

The canonical model has exactly `135,285,504` parameters. The environment allocation
floor is `align_up(20 * 135,285,504, 256 MiB) = 2,952,790,016` bytes. P16 admission is a
fixed whole-run-equivalent projection `<=25,920` seconds; its actual continuous durable
completion SLA is `<=28,800` seconds on a suspend-inclusive monotonic wall clock. Recovery,
host suspend/system sleep, and resumed-execution downtime count. Do not replace these fixed thresholds with a
device-derived SLA or retune them after measurement.
