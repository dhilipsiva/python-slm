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

The remaining future command names `train-tokenizer`, `tokenize`, `inspect`,
`bench`, and `train` fail before reading configuration or mutating state with the
typed `PHASE_NOT_IMPLEMENTED` gate until their owning phases land. Configurations are
versioned, explicit, and reject unknown fields; there are no legacy fallbacks or hidden
production defaults.

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
A successful command emits one compact `python-slm-curate-result-v3` object and installs
an immutable `python-slm-source-generation-v3` generation. The in-process Rust boundary
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

P6 is a deterministic conservative filter, not a proof that every possible sensitive
value has been recognized. P6A owns adversarial expansion; exact/near deduplication,
decontamination, and downstream corpus acceptance remain later phases. Live Stack-v2 or
Software Heritage acquisition also remains outside this command.

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
