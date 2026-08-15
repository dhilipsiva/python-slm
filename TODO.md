# Clean-Rebuild Execution Plan

Dependencies, not phase numbers, define implementation order. A dependency is sufficient
for development when its implementation is available or its manual-only gate is explicitly
marked `SKIPPED`; a skipped gate is never a PASS or qualification claim. The
accelerator/backend track (P1B–P2), data track (P3–P9A), and CPU
model track (P3–P9B) may run in parallel where dependencies permit. The existing Rust code is
reference evidence, not code to copy. `AGENTS.md` governs work,
`docs/rebuild-contract.md` records the approved Phase 0 product decisions,
`docs/receipts/P0.md` is their signed approval authority, and `docs/ARCHITECTURE.md`
defines the currently approved target design. Once P0A passes, its selected
`docs/rebuild-contract-v2.md`, prototype-first portable-interface ADR, revised architecture
hash, and decision-ledger hash become the active amendment authority. A conflict is a stop
condition.

This revision starts the `prototype-v2` implementation epoch and fixes the default P1A–P16A
execution profile as `prototype-windows-5090-v1`: native Windows x86_64 on an AMD Ryzen 9
9950X3D, the Rust `x86_64-pc-windows-msvc` target with the default VS 2022 x64
MSVC/Windows SDK toolchain, and one NVIDIA GeForce RTX 5090 at compute capability 12.0/SM120
through CUDA. Portable interfaces, schemas, artifact formats, and typed capability errors
are designed now, but Linux, macOS, GCC/Clang host adapters, ROCm/HIP/AMD accelerators, and
Metal/Apple Silicon accelerators are explicitly `DEFERRED_POST_P16`; they are neither
implemented nor P1A–P16A implementation requirements. The earlier P1A and P1B records remain
immutable historical evidence for their recorded Windows/MSVC/CUDA/SM120/RTX 5090 tuple,
and all existing P2 attempts remain immutable historical attempts. None satisfies a
`prototype-v2` dependency because none binds this amendment, profile, schemas, and complete
hardware identity. No new P1A, P1B, or P2 publication is required. Existing historical schemas, runs,
acceptances, pointers, and seals are never rewritten or reinterpreted.

## Simplified Implementation Mode (Owner Direction)

- Manual qualification runs, sealed phase receipts, human approvals, acceptances, root
  pointers, and checkbox-review commits are `SKIPPED` for P1A and every remaining phase.
  `SKIPPED` is a workflow decision, never `PASS`, `APPROVED`, `tuple_qualified`, or
  `full_run_qualified`.
- Do not create new qualification run directories, seals, receipt chains, approval records,
  acceptances, or phase pointers unless the owner explicitly restores that workflow later.
  Existing historical P0/P0A/P1/P2 and failed P1A evidence remains immutable.
- Continue implementation using the default `prototype-windows-5090-v1` assumptions and
  ordinary automated development checks. Hardware, performance, quality, portability, and
  full-run facts remain assumed or unverified unless an automated test directly establishes
  the narrower fact.
- A later phase may depend on available implementation from an earlier phase or on an
  explicitly skipped manual gate. Missing manual artifacts do not block development.
- In every remaining phase card, any instruction to qualify, seal, publish a receipt, obtain
  approval, create an acceptance or pointer, or wait for human checkbox review is
  non-operative and marked `SKIPPED`. The technical implementation and automated-test
  requirements remain active.

## Operating Rules

- Start every phase by reading `AGENTS.md`, `docs/ARCHITECTURE.md`, this file, and
  `git status`. Treat existing historical authority and evidence as immutable context, but
  do not require new manual artifacts before implementation. Preserve unrelated work.
- Do not run `cargo new`, `git init`, create a nested repository, broadly stage files,
  auto-commit, or tag. Use the existing repository.
- Implement only the active phase. Do not hide failed gates with fallbacks, relaxed
  assertions, ignored errors, or claims based on unexecuted commands.
- Normal tests are offline, deterministic, bounded, and credential-free. No Python
  interpreter, executable, package, module, build backend, code generator, embedded
  runtime, or Python-launched subprocess is part of or invoked by the build, data
  preparation, automated testing, or training pipeline. Python-language source is
  input data only.
- Rust owns CLI orchestration, data preparation, tokenization, storage, model/training
  control, checkpointing, and automated testing. C and C++ are permitted
  only inside pinned, audited, feature-gated hardware-accelerated kernels, standard native
  ML libraries, and native accelerator or graphics API bridges such as CUDA, HIP/ROCm,
  and Metal, plus the pinned Tree-sitter generated C parser/runtime used solely for the
  frozen Python CST, comment, and lexical-token semantics. No native component may own
  orchestration or general data transformation. Accelerator components expose a narrow
  ABI, propagate native errors, retain resources through asynchronous completion, and are
  absent from CPU-only builds; the parser exception is independently pinned and audited.
- Historical `.ps1` capture artifacts remain byte-for-byte archival evidence only. They
  are never invoked as entry points, copied into active orchestration, or rewritten; every
  active automated project command is a Rust or Cargo interface.
- Host-specific process, filesystem, dynamic-library, compiler, and accelerator mechanisms
  exist only behind internal Rust abstractions. Public argv, schemas, exit categories,
  redaction, process-tree containment, cleanup, and public artifact semantics
  are frozen as portable contracts. Through P16A, only their Windows x86_64/MSVC/CUDA
  implementations and automated checks are required. Rust invokes platform tools directly; a
  platform shell script is never a normative entry point. Selecting a deferred OS or
  provider must fail before discovery or mutation with a stable `DEFERRED_POST_P16`
  capability error; it must never silently fall back or report stub success.
- Manual qualification and publication workflows are disabled by the simplified
  implementation mode. Do not generate new phase receipts, seals, approvals, acceptances,
  pointers, or review-only checkbox commits.
- Use ordinary automated checks as development evidence. A passing check supports only the
  behavior it exercised and never upgrades an assumed environment into a qualified tuple.
- A suggested implementation commit may be created after automated checks pass; no separate
  human-review commit is required.

Until Phase 3 replaces the command contract, the CPU quality gate is:

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
cargo test --locked --features cpu-reference
```

From P3 onward, every phase runs `cargo run --locked -p xtask --bin xtask -- quality-gate` plus a
named targeted test or benchmark command for that phase. Neither command publishes a receipt.

The active development checks are ordinary, non-publishing commands. Phase implementations
may add focused automated tests, but they must not require manual qualification or receipt
publication.

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- verify-p0
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
cargo test --locked --features cpu-reference
```

## Phase 0 — Freeze the Rebuild Contract

- [x] P0 complete

Status: `PASS`. Sealed machine evidence passed; both top-level approval summaries are
`APPROVED`, and both owner sign-off decisions are `APPROVE`. Under `AGENTS.md`, the signed
receipt is the approval authority and activates Change Control. The sealed contract and
machine-evidence snapshots retain their historical `AWAITING_REVIEW` fields unchanged.

Dependencies: none.

Prompt:

> Inspect the current repository without changing production code. Treat the Rust
> implementation as a behavioral sample only. Write `docs/rebuild-contract.md` with a
> keep/reimplement/drop matrix for CLI commands, configuration, artifacts, tests, and
> failure behavior. Freeze these decisions: zero-Python scope; prepared-corpus training
> SLA; canonical 135,285,504-parameter preset (`d_ff=2432`, untied head); separately
> named 124,668,672 reference preset; token-count semantics—exactly two billion valid
> predicted targets, with stored IDs, consumed inputs, padding/boundary exclusions, and
> unused tail reported separately and zero overshoot; canonical special IDs (`<pad>=0`,
> `<s>=1`, `</s>=2`, `<unk>=3`) and injection/boundary policy; tokenizer-sample
> decimal-byte budget and per-repository caps; corpus encoding, accepted Python dialect,
> min/max bytes, generated-marker scope, license allow/deny/unknown rules, and PII/secret
> reject/quarantine/no-redaction policy; train/validation/test split and
> benchmark-decontamination policy; dedup-only normalization, lexical-token
> 5-gram semantics, MinHash coefficient construction/signature width, LSH layout,
> labeled-suite corpus/version,
> and minimum recall/precision; RoPE pairing/position and causal-mask
> conventions; initialization and parameter naming; optimizer decay groups; cosine
> floor/end behavior; global valid targets per optimizer update; final partial-update
> policy; training-span order/seed; evaluation cadence; checkpoint cadence/retention; and
> provenance/removal requirements. Recovery downtime always counts inside the SLA if
> recovery occurs. Run
> `cargo test --locked --features cpu-reference`, `cargo run --locked -- plan`, and
> `cargo run --locked -- plan --gqa-135m`; capture stdout/stderr and exit codes, and
> serialize the evidence summary as JSON. Do not edit or delete `src/`, `Cargo.toml`,
> `Cargo.lock`, or `build.rs`.

PASS:

- Every listed decision is explicit and consistent with `docs/ARCHITECTURE.md`.
- Reference commands and outputs are captured; unknowns are labeled, not guessed.
- The authoritative capture ran from its recorded frozen source commit and changed only
  the contract/receipt evidence paths allowed by that capture. Later documentation
  reconciliation is a separate reviewed commit and does not alter sealed bytes.
- Both top-level approval summaries say `APPROVED`, both named owner decisions explicitly
  say `APPROVE`, and the authoritative receipt is `PASS`. The signed receipt resolves the
  status transition without rewriting the sealed contract or machine-evidence snapshot.

VERIFY:

P0A has one narrow pre-dependency bootstrap exception: before P0 validation, it may add
only the minimal developer-only `xtask` package, the root workspace-manifest membership and
resulting `Cargo.lock` entry needed to invoke it with `-p xtask`, the `verify-p0` command,
focused tests, and this bootstrap wording. It may not change the
product package, draft the contract amendment, change any product/data/receipt byte, or
begin another phase. The command
must preserve every pinned baseline, receipt commit/hash, sealed path/run/SHA256SUMS,
ancestry, diff/dirty, unique status, approval-summary, owner-decision,
signature/reference, and timestamp check from the signed historical P0 verifier, but
implement them directly in Rust without invoking a platform shell or historical script.
The immutable verifier oracle is
`804cdcec24996895c469f54593a5aa79a4cd706a:TODO.md`: the normalized `VERIFY` block
beginning at line 138 through its closing fence (line 179 in that Git blob; the payload was
previously cited as lines 138-178) is exactly 3,388 UTF-8/LF bytes with SHA-256
`ffaa3b309e31782c95bfed1d4c37eb4b56427bc878c3d796f99d8663de53fe46`.
The hash and fence boundaries, not a mutable working-tree line number, are authoritative.
Once this command passes, P0 is validated and the remainder of P0A may begin:

```text
cargo run --locked -p xtask --bin xtask -- verify-p0
```

The sealed capture remains authoritative for its frozen contract bytes and reference
observations. The signed receipt records technical and data-governance approval of the
separate architecture/TODO reconciliation commit without rewriting any sealed byte.

STOP/loop: a revoked or contradictory approval, failed P0 verifier, or changed sealed byte
blocks P0A. A frozen-decision change is handled only by the create-new P0A amendment chain;
it never rewrites P0. Suggested commit:
`docs: approve phase 0 rebuild contract`.

## Phase 0A — Approve the Prototype-First Portable-Interface Amendment

- [x] P0A complete

Dependencies: P0.

Prompt:

> Implement the developer-only Rust `xtask` workspace package and use it to verify the complete
> immutable P0 chain. Write and obtain named owner approval for
> `docs/rebuild-contract-v2.md` and
> a prototype-first portable-interface ADR that together amend `SCOPE-001`, `SLA-001`,
> `PROV-001`, the deferred qualification table, and `docs/ARCHITECTURE.md`. Seal the new contract, decision-ledger,
> architecture, a create-new preapproval TODO snapshot, schema-bundle, and source identities
> in a create-new amendment receipt. Preserve every P0/P1/P2 historical byte and acceptance.
> Keep the live P0A checkbox unchecked in the sealed snapshot and preapproval commit. Obtain
> separate named technical and data-governance approvals with identities,
> signatures/references, decisions, UTC timestamps, and the exact sealed amendment hashes;
> one person may fill both roles only when the amendment records their explicit authority
> and two distinct role decisions. Create an approval commit containing those decisions but
> not the checkbox transition. The create-new acceptance binds the sealed unchecked TODO,
> preapproval commit, approval commit, and their exact ancestry/diffs; it never claims a
> future checkbox commit hash. Atomically publish the acceptance/root-pointer commit, then
> create a final checkbox-only commit. `xtask verify-phase --phase P0A --check-selected`
> validates that final commit as exactly the one-line `[ ]` to `[x]` P0A change over the
> pointer commit parent, with no other byte changed. Later phase checkbox commits do not
> retroactively invalidate the already validated P0A lifecycle.
> Approval attempts and acceptances have independent create-new counters: a failed or
> superseded approval attempt remains immutable without consuming an acceptance number, and
> every acceptance binds the exact same approval-attempt-sequence role pair by path, hash,
> and commit.
>
> Freeze `profile_id = "prototype-windows-5090-v1"` as the only supported and required
> P1A–P16A execution profile: native Windows x86_64; AMD Ryzen 9 9950X3D; Rust
> `x86_64-pc-windows-msvc`; VS 2022 x64 MSVC and Windows SDK; one NVIDIA GeForce RTX 5090;
> compute capability 12.0/SM120; dedicated VRAM; and CUDA. Every receipt proves its exact
> OS, host target, CPU, compiler, SDK/runtime/driver, device, architecture, memory model,
> and backend tuple and binds this profile ID. Freeze portable Rust interfaces and closed
> schemas for host processes, filesystems, toolchains, accelerators, transfers, and receipt
> publication, but explicitly classify Linux/GCC-or-Clang, macOS/AppleClang,
> ROCm/HIP/AMD, and Metal/Apple Silicon implementations and qualifications as
> `DEFERRED_POST_P16`. Deferred profiles are not supported, discovered, compiled, tested,
> or allowed to block P1A–P16A; their stable selection failure is part of the current API.
>
> Restore the fixed prepared-corpus training SLA: `sla_seconds = 28_800` and
> `actual_elapsed_limit_ns = 28_800_000_000_000` for the continuous P16 clock from
> immediately before the trainer verifies and opens frozen artifacts until the
> final checkpoint is durable. Recovery downtime counts. P15 admission requires a measured
> whole-run projection, including every frozen overhead, of no more than
> `admission_seconds = 25_920`,
> reserving exactly `2,880` seconds (10%) for ordinary runtime variance. P14 may profile and
> autotune the prototype but may not replace, scale, or reinterpret either fixed threshold.
>
> Freeze the narrow parser exception: `tree-sitter-python 0.25.0` and its generated C
> parser/runtime may be used only through the Rust data-policy boundary for the exact
> `SOURCE-002`, `SOURCE-003`, `DEDUP-001`, and `DECONTAM-001` CST/comment/lexical semantics.
> Pin and hash the grammar, generated C, runtime, build flags, and compatibility corpus;
> forbid Python code generation at build or runtime and forbid C/C++ orchestration or
> unrelated data transformation. Preserve the deterministic custom BPE, exact dedup
> threshold, 256-component MinHash,
> 32-by-8 LSH layout, exhaustive Jaccard check, split rules, cryptographic hash chains,
> and role-ledger arithmetic unchanged. Any parser-driven artifact identity changes and
> all affected downstream seeds must be explicit in the new decision ledger.

VERIFY:

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- verify-p0
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --output-root docs/receipts/P0A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --finalize --output-root docs/receipts/P0A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --check-selected --output-root docs/receipts/P0A
```

PASS:

- The historical P0 chain validates without byte changes, and the amendment has a separate
  create-new seal and acceptance, an atomically published pointer, and named technical plus
  data-governance approvals.
  Its acceptance binds only already-existing unchecked preapproval and approval commits;
  the pointer commit is followed by a separately validated one-line checkbox-only commit.
- `xtask verify-p0` is regression-tested against the historical verifier and enforces every
  original pinned hash, ancestry, cleanliness, approval-uniqueness, and sign-off condition.
- The amendment records the exact portable argv/schema/exit/publication contract, the sole
  P1A–P16A prototype profile, the deferred implementation matrix, the native-code boundary
  and narrow Tree-sitter exception, and the fixed `28,800`/`25,920`-second SLA gates.
- New phase schemas and receipt namespaces cannot collide with or reinterpret historical
  P1A/P1B/P2 evidence.

STOP/loop: until P0A has owner approval, prototype P1A and every downstream active phase are
blocked. Suggested commit: `docs: approve prototype-first rebuild contract`.

## Phase 1A — Host Toolchain Defaults

- **SKIPPED — P1A manual qualification and publication gate.**

Status: `SKIPPED`, not complete and not PASS. The existing Windows host verifier remains
available as optional diagnostic tooling. Development assumes native Windows x86_64, AMD
Ryzen 9 9950X3D, Rust 1.96 or newer, VS 2022 x64 MSVC, and Windows SDK/UCRT.

No host run, receipt, approval, acceptance, pointer, or review commit is required.

## Phase 1B — Prototype RTX 5090/CUDA Implementation

- [x] P1B implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P1A defaults.

- Implement native CUDA discovery for the default RTX 5090/SM120 configuration.
- Keep ROCm/HIP and Metal adapters deferred without blocking the Windows prototype.
- Compile and inspect SM120 native code plus compute_120 PTX fallback.
- Exercise allocation, synchronization, sentinel round-trip, release, and cleanup using the
  fixed minimum persistent allocation of `2,952,790,016` bytes.
- Add automated tests for missing tools, incompatible runtime/driver combinations, multiple
  devices, malformed artifacts, and cleanup failures.

## Phase 2 — Backend Selection Implementation

- [x] P2 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P1B implementation.

- Implement the provider-neutral backend interface and Windows/CUDA candidates.
- Compare available candidates with automated correctness, gradient, memory, launch, and
  cleanup tests; use a sensible default when only one viable candidate is installed.
- Keep backend choice configurable and record runtime diagnostics without treating them as
  qualification evidence.

## Phase 3 — Clean Rust Scaffold and Quality Gate

- [x] P3 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P1A defaults. P2 may proceed in parallel.

- Establish the clean Rust 2024 module layout and provider-neutral interfaces.
- Implement `xtask quality-gate` as a non-publishing automated command.
- Preserve exact model, token-accounting, zero-Python, and native-boundary constants.
- Add focused unit and synthetic end-to-end tests.

## Phase 4 — Document Source and Policy Engine

- [x] P4 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P3.

- Implement bounded document ingestion with stable identities and deterministic ordering.
- Enforce encoding, declared Python dialect, size, license, provenance, removal,
  path-containment, and mutation rules. Define the exact comment/generated-marker policy
  over parser facts; P5 supplies those facts and activates the final decision.
- Return typed per-document outcomes and test malformed, oversized, mutated, and duplicate
  inputs automatically.

## Phase 5 — Pinned Python Parser Boundary

- [ ] P5 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P4.

- Integrate pinned `tree-sitter-python 0.25.0` generated C through the narrow Rust ABI.
- Pin grammar/runtime/scanner identities and deterministic parser outputs.
- Add compatibility fixtures for supported dialects, comments, strings, syntax failures,
  cancellation, and cleanup.

## Phase 6 — Privacy, Secret, and Policy Filtering

- [ ] P6 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P5.

- Implement deterministic PII, secret, license, generated-content, and policy filters.
- Preserve restricted values out of logs and public artifacts.
- Add false-positive, false-negative, boundary, cancellation, and mutation regressions.

## Phase 6A — Adversarial Filter Cases

- [ ] P6A implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P6.

- Install adversarial no-code cases for encodings, quoting, comments, generated markers,
  secrets, PII, paths, and concurrent mutation.
- Keep outcomes deterministic and automatically testable.

## Phase 7 — Tokenizer Engine

- [ ] P7 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P6.

- Implement deterministic byte-level BPE with canonical special IDs.
- Enforce sample byte budgets, repository caps, normalization, tie-breaking, and stable
  serialization.
- Add train/save/reload and exact-token regressions.

## Phase 7A — Governed Source Metadata

- [ ] P7A implementation
- Manual approval and publication gate: **SKIPPED**.

Dependencies: P6A and P7.

- Represent source identity, provenance, license, removal, and freshness metadata.
- Use configured defaults where external review is unavailable and label resulting source
  status assumed or unverified.
- Keep sensitive values out of logs and test policy decisions automatically.

## Phase 8 — Corpus and Token Materialization

- [ ] P8 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P7A.

- Materialize the governed corpus, tokenizer sample, tokenizer artifact, and token shards in
  deterministic, restart-safe, create-new formats.
- Enforce exact byte/token accounting, capacity limits, mutation detection, and cleanup.
- Add interrupted-write, duplicate-input, and round-trip tests.

## Phase 9A — Deduplication, Decontamination, Splits, and Span Order

- [ ] P9A implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P8.

- Implement exact and near deduplication, benchmark decontamination, deterministic splits,
  sample manifests, and `SPAN-001` order.
- Preserve every complete span exactly once and test hash/order stability.

## Phase 9B — Model Initialization and CPU Oracle

- [ ] P9B implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P3 and P9A.

- Implement the canonical `135,285,504`-parameter model configuration, parameter naming,
  initialization, optimizer grouping, and BF16/FP32 rules.
- Produce deterministic CPU oracle outputs and literal gradient bytes.
- Add shape, count, initialization, forward, loss, and backpropagation regressions.

## Phase 10 — Accelerator Model Backend

- [ ] P10 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P2 and P9B.

- Implement memory-efficient forward, backward, fused loss, and exact-gradient paths behind
  the provider-neutral backend.
- Match canonical CPU gradient bytes and test cancellation, error propagation, cleanup, and
  repeated execution.

## Phase 11 — Data Loader and Transfers

- [ ] P11 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P8 and P10.

- Implement deterministic span loading, bounded buffering, pinned staging, asynchronous
  transfer ownership, and end-of-stream behavior.
- Test ordering, backpressure, cancellation, mutation, short reads, and cleanup.

## Phase 12 — Trainer, Checkpoints, and Exact Resume

- [ ] P12 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P9B, P10, and P11.

- Implement accumulation, optimizer, scheduler, evaluation, checkpoint retention, and exact
  target accounting with zero overshoot.
- Persist complete deterministic state and require byte-identical continuation after resume.
- Add interruption, corruption, identity-mismatch, and final-partial-update tests.

## Phase 13 — Automated Windows/CUDA CI

- [ ] P13 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P12.

- Add ordinary CPU CI plus optional Windows/CUDA jobs when compatible runners are available.
- Keep unavailable hardware from blocking development; report those lanes as unverified.
- Run zero-Python, exact-gradient, resume, cleanup, and synthetic end-to-end tests.

## Phase 14 — Prototype Profiling and Default Configuration

- [ ] P14 implementation
- Manual calibration, qualification, and publication gate: **SKIPPED**.

Dependencies: P10–P13.

- Select sensible default batch, accumulation, loader, checkpoint, evaluation, and backend
  settings for the Windows/RTX 5090 profile.
- Collect diagnostics when available without retuning correctness or contract constants.
- Retain fixed `25,920`-second admission and `28,800`-second SLA values as design targets,
  not verified performance claims.

## Phase 15 — Automated Stability Ladder

- [ ] P15 implementation
- Manual qualification, approval, and publication gate: **SKIPPED**.

Dependencies: P14.

- Automate bounded smoke, short-run, restart, and stability trials using the default profile.
- Freeze code and configuration within each automated trial.
- Record diagnostics locally; missing long-duration or dedicated-host execution remains
  unverified and does not block implementation.

## Phase 16 — Final Training Run

- [ ] P16 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P15 implementation.

- Implement and expose the final training command with exact two-billion-valid-target
  accounting, zero overshoot, durable checkpoints, and fresh-process reload.
- A real full run is optional unless explicitly requested. Without one, completion, elapsed
  time, SLA, final-loss, and final-checkpoint claims remain unverified.

## Phase 16A — Automated Quality Evaluation

- [ ] P16A implementation
- Manual approval, acceptance, and publication gate: **SKIPPED**.

Dependencies: P16 implementation.

- Implement held-out loss/perplexity evaluation, initialized and unigram baselines, and a
  deterministic prompt/sample replay harness.
- Require finite metrics and deterministic outputs in automated tests.
- Do not require owner approval or pointer publication; any unexecuted final-model evaluation
  remains unverified.

## Phase 17 — Portable Host/Data Adapters

- [ ] P17 implementation
- Manual host-matrix qualification and publication gate: **SKIPPED**.

Dependencies: P16A implementation.

- Implement Windows x86_64, Linux x86_64, and macOS arm64 host abstractions without changing
  shared data/artifact semantics.
- Run automated tests on available hosts; unavailable host lanes remain unverified and do
  not block development.

## Phase 18 — Accelerator Provider Adapters

- [ ] P18 implementation
- Manual tuple-matrix qualification and publication gate: **SKIPPED**.

Dependencies: P17 and P16A implementation.

- Implement CUDA, ROCm/HIP, and Metal adapters behind the common provider interface.
- Preserve exact-gradient, deterministic-resume, transfer/memory, cleanup, and error
  semantics.
- Test available providers automatically; unavailable tuples remain unverified rather than
  blocking the project.

## Phase 19 — Optional Scale-Up

- [ ] P19 implementation (optional)
- Manual amendment, approval, qualification, and publication gate: **SKIPPED**.

Dependencies: P16A implementation. P17 and P18 are independent.

- Start only when explicitly requested.
- Recompute model, memory, schedule, token accounting, evaluation, and SLA defaults for the
  requested larger scope.
- Use automated checks and clearly label unexecuted scale-up results unverified.
