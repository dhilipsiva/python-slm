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

- [x] P5 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P4.

- Integrate pinned `tree-sitter-python 0.25.0` generated C through the narrow Rust ABI.
- Pin grammar/runtime/scanner identities and deterministic parser outputs.
- Add compatibility fixtures for supported dialects, comments, strings, syntax failures,
  cancellation, and cleanup.

## Phase 6 — Privacy, Secret, and Policy Filtering

- [x] P6 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P5.

- Implement deterministic PII, secret, license, generated-content, and policy filters.
- Preserve restricted values out of logs and public artifacts.
- Add false-positive, false-negative, boundary, cancellation, and mutation regressions.

## Phase 6A — Adversarial Filter Cases

- [x] P6A implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P6.

- Install adversarial no-code cases for encodings, quoting, comments, generated markers,
  secrets, PII, paths, and concurrent mutation.
- Keep outcomes deterministic and automatically testable.

## Phase 7 — Tokenizer Engine

- [x] P7 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P6.

- Implement deterministic byte-level BPE with canonical special IDs.
- Enforce sample byte budgets, repository caps, normalization, tie-breaking, and stable
  serialization.
- Add train/save/reload and exact-token regressions.

## Phase 7A — Governed Source Metadata

- [x] P7A implementation
- Manual approval and publication gate: **SKIPPED**.

Dependencies: P6A and P7.

- Represent source identity, provenance, license, removal, and freshness metadata.
- Use configured defaults where external review is unavailable and label resulting source
  status assumed or unverified.
- Keep sensitive values out of logs and test policy decisions automatically.

## Phase 8 — Corpus and Token Materialization

- [x] P8 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P7A.

- Materialize the governed corpus, tokenizer sample, tokenizer artifact, and token shards in
  deterministic, restart-safe, create-new formats.
- Enforce exact byte/token accounting, capacity limits, mutation detection, and cleanup.
- Add interrupted-write, duplicate-input, and round-trip tests.

## Phase 9A — Deduplication, Decontamination, Splits, and Span Order

- [x] P9A implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P8.

- Implement exact and near deduplication, benchmark decontamination, deterministic splits,
  sample manifests, and `SPAN-001` order.
- Preserve every complete span exactly once and test hash/order stability.

## Phase 9B — Model Initialization and CPU Oracle

- [x] P9B implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P3 and P9A.

- Implement the canonical `135,285,504`-parameter model configuration, parameter naming,
  initialization, optimizer grouping, and BF16/FP32 rules.
- Produce deterministic CPU oracle outputs and literal gradient bytes.
- Add shape, count, initialization, forward, loss, and backpropagation regressions.

## Phase 10 — Accelerator Model Backend

- [x] P10 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P2 and P9B.

- Implement memory-efficient forward, backward, fused loss, and exact-gradient paths behind
  the provider-neutral backend.
- Match canonical CPU gradient bytes and test cancellation, error propagation, cleanup, and
  repeated execution.

## Phase 11 — Data Loader and Transfers

- [x] P11 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P8 and P10.

- Implement deterministic span loading, bounded buffering, pinned staging, asynchronous
  transfer ownership, and end-of-stream behavior.
- Test ordering, backpressure, cancellation, mutation, short reads, and cleanup.

## Phase 12 — Trainer, Checkpoints, and Exact Resume

- [x] P12 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P9B, P10, and P11.

- Implement accumulation, optimizer, scheduler, evaluation, checkpoint retention, and exact
  target accounting with zero overshoot.
- Persist complete deterministic state and require byte-identical continuation after resume.
- Add interruption, corruption, identity-mismatch, and final-partial-update tests.

## Phase 13 — Automated Windows/CUDA CI

- [x] P13 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P12.

- Add ordinary CPU CI plus optional Windows/CUDA jobs when compatible runners are available.
- Keep unavailable hardware from blocking development; report those lanes as unverified.
- Run zero-Python, exact-gradient, resume, cleanup, and synthetic end-to-end tests.

## Phase 14 — Prototype Profiling and Default Configuration

- [x] P14 implementation
- Manual calibration, qualification, and publication gate: **SKIPPED**.

Dependencies: P10–P13.

- Select sensible default batch, accumulation, loader, checkpoint, evaluation, and backend
  settings for the Windows/RTX 5090 profile.
- Collect diagnostics when available without retuning correctness or contract constants.
- Retain fixed `25,920`-second admission and `28,800`-second SLA values as design targets,
  not verified performance claims.

## Phase 15 — Automated Stability Ladder

- [x] P15 implementation
- Manual qualification, approval, and publication gate: **SKIPPED**.

Dependencies: P14.

- Automate bounded smoke, short-run, restart, and stability trials using the default profile.
- Freeze code and configuration within each automated trial.
- Record diagnostics locally; missing long-duration or dedicated-host execution remains
  unverified and does not block implementation.

## Phase 16 — Final Training Run

- [x] P16 implementation
- Manual qualification and publication gate: **SKIPPED**.

Dependencies: P15 implementation.

- Implement and expose the final training command with exact two-billion-valid-target
  accounting, zero overshoot, durable checkpoints, and fresh-process reload.
- A real full run is optional unless explicitly requested. Without one, completion, elapsed
  time, SLA, final-loss, and final-checkpoint claims remain unverified.

## Phase 16A — Automated Quality Evaluation

- [x] P16A implementation
- Manual approval, acceptance, and publication gate: **SKIPPED**.

Dependencies: P16 implementation.

- Implement held-out loss/perplexity evaluation, initialized and unigram baselines, and a
  deterministic prompt/sample replay harness.
- Require finite metrics and deterministic outputs in automated tests.
- Do not require owner approval or pointer publication; any unexecuted final-model evaluation
  remains unverified.

## Phase 17 — Portable Host/Data Adapters

- [x] P17 implementation
- Manual host-matrix qualification and publication gate: **SKIPPED**.

Dependencies: P16A implementation.

- Implement Windows x86_64, Linux x86_64, and macOS arm64 host abstractions without changing
  shared data/artifact semantics.
- Run automated tests on available hosts; unavailable host lanes remain unverified and do
  not block development.

## Phase 18 — Accelerator Provider Adapters

- [x] P18 implementation
- Manual tuple-matrix qualification and publication gate: **SKIPPED**.

Dependencies: P17 and P16A implementation.

- Implement CUDA, ROCm/HIP, and Metal adapters behind the common provider interface.
- Preserve exact-gradient, deterministic-resume, transfer/memory, cleanup, and error
  semantics.
- Test available providers automatically; unavailable tuples remain unverified rather than
  blocking the project.

## Phase 19 — Optional Scale-Up

- [x] P19 implementation (optional)
- Manual amendment, approval, qualification, and publication gate: **SKIPPED**.

Dependencies: P16A implementation. P17 and P18 are independent.

- Start only when explicitly requested.
- Recompute model, memory, schedule, token accounting, evaluation, and SLA defaults for the
  requested larger scope.
- Use automated checks and clearly label unexecuted scale-up results unverified.

## Final-Run Execution Track (owner requested 2026-08-16)

The P1–P19 implementation phases are complete, but no real training run has occurred:
there is no full-model accelerator training backend, no launch path, no governed
production corpus, and no hardware-execution evidence. This track closes those gaps in
dependency order. Simplified-mode rules continue to apply: ordinary automated checks
only, no receipts or approvals, and every unexecuted or unmeasured fact remains
`UNVERIFIED`. The fixed `25,920`-second admission ceiling and `28,800`-second
completion SLA are never retuned after measurement; a failed projection blocks the
run instead of moving a threshold.

### E1 — Full-Model Accelerator Training Backend

- [x] E1 implementation (core landed and hardware-verified; E1A and E1B both resolved)

Dependencies: P10, P12, and P18 implementation.

Landed, with automated evidence executed on the prototype RTX 5090:

- `src/train/full_state.rs` owns the provider-neutral full-model state: the
  generalized GQA dimensional contract, INIT-001 canonical master-weight
  initialization, AdamW through the frozen P12 `CanonicalAdamw` arithmetic,
  deterministic host and device RNG witness chains, and the closed five-artifact
  checkpoint codec with byte-exact restore.
- `src/model/accelerator/full_model.rs` replaces the fixture-only P10 graph with
  one configuration-parameterized GQA graph shared by every provider adapter. It
  uses straight-through BF16 storage quantization at the frozen points and
  explicit host-FP32 constants for the RoPE tables and the causal mask.
- `src/train/cuda_backend.rs` implements `TrainerBackend` on `burn-cubecl-cuda`.
- Verified on hardware: finite and byte-identical repeated gradients across
  independent backend instances; byte-exact snapshot and restore; byte-identical
  continuation after restore. Verified on CPU: canonical initialization
  reproduces every INIT-001 per-tensor digest for all 111 tensors of the
  135,285,504-parameter model.

A correctness defect found by executing the previously compile-only P2 fixture is
also fixed here: its expected gradient constants had never run on hardware and
were wrong.

### E1A — Exact-Gradient Gate Conflict (resolved by amendment)

- [x] E1A resolution
- Manual amendment, approval, and publication gate: **SKIPPED** (Simplified
  Implementation Mode; owner directed the lightweight route on 2026-08-16).

Resolved by `PRECISION-002` in `docs/decision-ledger-v3.md`, with the rationale and
measurements in `docs/adr/0001-accelerator-numerical-conformance.md`. The forward
remains an exact-byte gate, gradients are bounded by the already-frozen
provider-independent policy values, and determinism and resume are unchanged and
unrelaxed. Measured on the prototype RTX 5090: relative L2 `5.714e-6` against the
`0.03` limit and cosine `0.999999999984` against the `0.999` floor. The gate is no
longer ignored and now runs on the hardware lane.

Dependencies: E1 implementation.

Executing the frozen P10 parity fixture on real hardware for the first time shows
that device gradients do **not** equal the P9B oracle's canonical IEEE-754 bytes,
while the forward BF16 logits and the FP32 loss match it exactly. Observed
relative gradient deviation is roughly `1e-5` to `1e-4`. Replacing
`powf_scalar(2.0)` with explicit multiplication in RMSNorm, so the squaring
derivative no longer travels through a generic power rule, did not close the gap.

The root cause is now measured, not hypothesized. `tests/e1a_numerical_probe.rs`
isolates each candidate mechanism on the RTX 5090:

- **Contraction and reduction order are exonerated.** Elementwise multiplication
  matches the host on 64 of 64 operands, and device summation *and* `matmul`
  reproduce host left-to-right accumulation exactly at widths 2, 4, 8, and 64.
  The earlier suspicion that NVRTC fused multiply-add or tree reduction caused the
  divergence was wrong.
- **Transcendental implementations are the sole cause.** Against Rust's host libm,
  the device differs by one ULP on `exp` (11 of 42 operands), `sin` (5 of 42),
  `cos` (2 of 42), and `ln` (1 of 25). `sqrt` and `recip` are bit-identical, which
  is expected: IEEE-754 requires those to be correctly rounded, and `exp`, `ln`,
  `sin`, and `cos` are not.

Because every other primitive in the graph is bit-exact, the transcendentals are
the cause by elimination. The deviation profile matches that mechanism: worst
relative error tracks how many transcendental operations separate a parameter from
the loss, peaking at `2.418e-4` for `blocks.0.attn.q.weight`, whose gradient passes
back through both RoPE `sin`/`cos` and the softmax `exp`, and falling to `1.083e-7`
for `lm_head.weight` next to the loss. Aggregate deviation is a relative L2 of
`5.714e-6` and a cosine similarity of `0.999999999984`. The forward stays exact
because every frozen BF16 storage point quantizes to eight mantissa bits and
absorbs the difference; gradients are raw FP32 and expose it.

No repository-level setting changes a vendor libm, and Rust's own transcendentals
are not guaranteed bit-stable across platform libm implementations either, so the
cross-host oracle check below is part of the same question. This is irreducible.

The requirement is `PRECISION-001` in `docs/decision-ledger-v2.md`, not `MODEL-001`
as previously recorded here; `MODEL-001` governs only model shape and its reopen
trigger would wrongly route this into P19. Resolution follows `PRECISION-001`'s own
reopen trigger. Do not weaken, tolerance, or delete the gate ahead of that
amendment. `tests/e1_full_model_backend.rs` keeps the gate as an explicitly ignored
test so it remains visible and runnable rather than quietly absent.

### E1B — Batched, Memory-Efficient Attention and Loss

- [x] E1B implementation

Dependencies: E1 implementation.

Measured first, on the prototype RTX 5090 at canonical scale with one sequence per
dispatch (`tests/e1b_canonical_scale_probe.rs`):

| Quantity | Measurement |
|---|---|
| Peak device memory attributable to the process | `16,318` MiB for a single 2,048-target sequence |
| Steady-state time per sequence | `0.7743` s |
| Throughput | `2,645` targets/s against the `69,444` targets/s the SLA requires |
| Projected full-run wall clock | `210` hours against the `8`-hour completion SLA |
| First dispatch, including kernel compilation | `172` s, one-time but charged to the SLA clock |

Both gaps are far larger than the earlier estimates: one sequence already consumes half
the device, and throughput is short by a factor of roughly 26. The graph is also nowhere
near compute-bound — it sustains about `2.1` TFLOP/s, a low single-digit percentage of the
device's FP32 capability — so the gap is dominated by materialization, launch overhead from
the 144 per-head loop iterations, and the straight-through storage helper's extra tensors,
not by arithmetic. That is encouraging for the fix and disqualifying for the current shape.

The current graph is single-sequence and materializes everything:

- it has no batch dimension, so the frozen P14 micro-batch of 16 sequences cannot
  be expressed and every sequence costs a separate dispatch chain. One 1,000,000
  target evaluation at fixture scale needs roughly 470,000 kernel launches and
  does not finish inside a normal test budget;
- it retains the full `[L, V]` logit matrix, about 262 MB per 2,048-target
  sequence at canonical scale;
- it retains full `L x L` attention scores for every head and layer, about 2.4 GB
  per sequence at canonical scale.

Together these make the canonical model unable to fit the 32 GB device at the
frozen micro-batch. Implement the batched sequence dimension, the chunked or
fused cross-entropy the contract requires instead of a retained `[B, L, V]`
tensor, and causal GQA that neither materializes complete score matrices nor
repeats K/V. Re-verify determinism, snapshot and restore, and continuation after
the change; the frozen semantics may not move.

**Landed so far.** `burn-autodiff`'s `BalancedCheckpointing` now backs the training
backend and all three provider parity adapters, and the CUDA backend returns pages to
the allocator after each optimizer update. Recomputation reruns the same kernels on the
same inputs, so it is bit-identical: the `PRECISION-002` conformance numbers are
unchanged to the last digit (`5.713872e-6` / `0.999999999984`), and the determinism and
resume gates pass untouched. Re-measured effect: peak memory `16,318` to `13,601` MiB, a
17 percent reduction, for a 7 percent slowdown to `2,453` targets/s. That is the expected
recompute trade and confirms the remaining gap is structural, not a tuning matter.

**Phase 3, batching, has landed.** The graph is now `[B, L, ...]` throughout. Heads fold
into the batch axis so all of them run as one batched matmul instead of twelve per layer,
each K/V head is expanded across its query group rather than re-sliced per query head, and
the causal mask and RoPE tables broadcast instead of being sliced per sequence.
`TrainingBatch` carries `sequence_lengths`, so a micro-batch states its own boundaries;
sequences of equal length are grouped into one dispatch, which keeps the ragged final
update exact rather than padded. The fixture runs through the batched path at one
sequence, so the conformance gate still covers the code production uses.

Measured at one sequence per dispatch: `13,361` MiB and `3,894` targets/s, a 59 percent
throughput gain over the pre-batching `2,453`, bringing the projection from `226.5` to
`142.7` hours. That gain comes from batching the heads, not from batching sequences.

**Batching sequences does not pay until the materializations are bounded, and the
measurement is emphatic.** At four sequences per dispatch the run reaches `30,582` MiB
attributable and a `32,158` MiB peak, essentially the whole 32 GB device, and throughput
*collapses* to `649` targets/s, six times worse than a single sequence, for a `855.9`-hour
projection. Under that memory pressure the allocator thrashes and the arithmetic stops
mattering. The frozen micro-batch of 16 is therefore unreachable by batching alone, and
this settles the order of the remaining work: Phase 4 must land before any batch wider than
one sequence is worth measuring again.

One honest consequence: batched kernels reduce in a different order, so gradient
conformance moved from `5.714e-6` to `3.190342e-4` relative L2 and from `0.999999999984`
to `0.999999949415` cosine. Both remain far inside the frozen `PRECISION-002` bound, and
the forward stays exact byte for byte, but the sensitivity of gradients to kernel shape is
exactly why the amendment used a bound rather than byte equality.

**Phase 4, bounding the materializations, has landed**, and it reverses the Phase 3
finding above. One training step is now split into several backward passes joined by the
exact `sum(value * cotangent)` vector-Jacobian seed: the cross-entropy head is
differentiated `512` flattened positions at a time, so the `[.., 32,000]` logits no longer
scale with sequence length or batch width, and every layer is forwarded once untracked to
record its boundary and recomputed with a tape only while its own gradient is taken, so
retained state falls to the layer boundaries plus one live layer. `burn` 0.21 offers no
trainable flash attention and no downstream custom `Backward`, so this staging is the
available mechanism.

Two defects were found and fixed on the way:

- `FullModelGraph::detached()` never worked. burn's `float_detach` deliberately
  re-applies the `require_grad` flag it finds, so detaching a parameter leaves it rooting
  a tape and evaluation retained one it could never use. `untracked()` clears the flag
  instead, which is what actually prevents a tape forming.
- Gradient readback issued one `try_into_data` per parameter, so every micro-batch paid
  111 device synchronizations. The gradients are now concatenated on device and read back
  once, in the same PARAM-001 order.

Re-measured at canonical scale. The Phase 3 collapse is gone and batching now pays:

| Sequences per dispatch | Peak device memory | Throughput | Projected wall clock |
|---|---|---|---|
| 1 | `9,552` MiB | `5,471` targets/s | `101.5` h |
| 4 | `14,130` MiB | `10,481` targets/s | `53.0` h |
| 8 | `18,734` MiB | `13,731` targets/s | `40.5` h |
| 16 | `31,879` MiB | `16,221` targets/s | `34.2` h |

At one sequence, memory fell from `13,361` to `9,552` MiB while throughput *rose* from
`3,894` to `5,471` targets/s: the extra forward that checkpointing costs was more than
repaid by the readback fix and by the reduced allocator pressure. At four sequences the
comparison against Phase 3 is `14,130` MiB versus `30,582` and `10,481` targets/s versus
`649` — sixteen times the throughput for half the memory. **The frozen P14 micro-batch of
16 sequences now fits and runs**, which was E1B's stated goal, though at `31,879` MiB of
`32,607` it leaves only `728` MiB of headroom; 8 sequences is the safer operating point for
an unattended run and E5 owns that choice.

The staging is numerically transparent, which is the load-bearing claim: the
`PRECISION-002` conformance numbers are unchanged to the last digit
(`3.190342e-4` / `0.999999949415`), and `logits_exact` and `loss_exact` still hold. The
P9B fixture is two positions, so it stays a single head chunk and keeps the oracle's exact
reduction order. `tests/e1_full_model_backend.rs` also gains a fused-versus-sequential
gradient check; its `2.404930e-4` relative L2 is *not* a Phase 4 effect, since running the
identical comparison against the pre-checkpointing graph gives `2.412581e-4`. It is what
FP32 batching costs on gradients that carry this much cancellation.

`evaluation_is_finite_and_leaves_training_state_unchanged` is no longer ignored: the
mandatory `1,000,000`-target held-out evaluation now completes in `58` s.

**Phase 5, true BF16 storage, was implemented, measured, and deliberately not kept.**
Its precondition holds: `MatmulElems::from_globals` promotes the accumulator to FP32
whenever the output is BF16 or FP16 (`cubek-matmul-0.2.0/src/definition/spec.rs:276`), so
`sensitive_accumulation: "fp32"` survives BF16 storage. Isolated BF16 matmul is `2.14x`
to `2.88x` faster than FP32 on this device at the graph's own shapes
(`tests/e1b_canonical_scale_probe.rs::probe_bf16_versus_f32_matmul_throughput`). The
whole-model result did not follow:

| Substrate | Best operating point | Peak memory | Throughput | Projection | Conformance | Batch-shape sensitivity |
|---|---|---|---|---|---|---|
| FP32 straight-through | 16 sequences | `31,879` MiB | `16,221` targets/s | `34.2` h | `3.190342e-4` | `2.405e-4` |
| True BF16 storage | 8 sequences | `17,900` MiB | `16,500` targets/s | `33.7` h | `3.713047e-3` | `3.586e-2` |

Equal throughput. The reason BF16 gains so little is structural rather than
incidental: the frozen semantics deliberately place *no* storage point on the attention
scores, their softmax, or the loss, and those are exactly the memory- and
time-dominant tensors, so BF16 can only reach the linear projections while the widening
casts it forces at every boundary give much of that back. Measured against expectation,
peak memory fell only 4 percent at eight sequences and *rose* at sixteen, where
throughput collapsed to `5,571`–`5,981` targets/s across two runs.

Against that, BF16 costs `11.6x` on gradient conformance and `150x` on batch-shape
sensitivity, the latter reaching `3.586e-2` — larger than the `0.03` conformance bound
itself. The forward stayed exact byte for byte, and the frozen gate still passed at
`3.713047e-3`, so this was a trade rather than a failure. **Owner decision on
2026-08-17: revert to the FP32 substrate.** Equal speed does not buy a `11.6x`
determinism regression, and neither substrate changes the SLA verdict. The probe is kept
so the finding stays reproducible.

**The conformance gate now runs at production batch widths.** The fixture executed only
at one sequence, while training runs at eight or sixteen, and batch width demonstrably
moves the gradient — so the gate did not cover the shape that ships.
`run_burn_cubecl_cuda_batched_model_parity` replicates the closed fixture across a batch
and scales the normalizer, which leaves the loss and gradients at their single-sequence
values, making the frozen oracle bytes a gate on the batched path. It also fails closed
if the copies are not bit-identical to each other. Measured: `logits_exact` and
`loss_exact` hold at every width, and gradient deviation is flat at `3.747215e-4`,
`3.747288e-4`, and `3.747141e-4` for 4, 8, and 16 sequences against the `0.03` bound.

**E1B is complete, and the run does not meet the SLA.** The frozen configuration is
runnable, bounded, and gated at its production shape, but `34.2` hours against the
`28,800`-second completion SLA is short by a factor of `4.3`. That constant is frozen and
is not retunable after measurement, so this is a blocking input to E5's admission
projection rather than something E1B can resolve. The remaining levers are outside this
item: the graph sustains roughly `19` TFLOP/s while isolated FP32 matmul on the same
shapes reaches `72`–`79`, so matmul is about a quarter of the wall clock and the rest is
elementwise traffic, launch overhead, and the `541` MB per-micro-batch gradient readback
the `TrainerBackend` contract requires.

### E2 — Final-Run Launch Mode

- [x] E2 implementation

Dependencies: E1, E1A, and E1B.

`train --config <defaults> --launch <launch>` is the explicit execution mode, in
`src/train/launch.rs`. The other two forms are unchanged and verified so: the
no-argument inspection result is byte-identical, including its
`implementation_sha256`, and `--verify-final-checkpoint` still reloads. The two
optional arguments are mutually exclusive at the parser, because one reads a
directory and the other trains for tens of hours.

- **The launch configuration is its own closed schema**,
  `python-slm-final-run-launch-v1`. The frozen
  `prototype-windows-5090-v1.defaults.json` is byte-pinned in two places and
  `validate()` demands byte equality with `canonical()`, so it can never carry a
  path or a device ordinal; everything an execution needs beyond it lives in the
  launch file. It must set `confirm_full_run`, so the execution mode is
  unreachable by a stray flag.
- **A suspend-inclusive monotonic clock** now backs the SLA measurement
  (`platform::suspend_inclusive_now_ns`). `Instant` is `QueryPerformanceCounter`
  on Windows and `CLOCK_MONOTONIC` on Linux, and both stop while the host is
  asleep, so an overnight run that suspended for six hours would report six hours
  short against a wall-clock deadline. Windows `QueryInterruptTime` is the biased
  count and Linux `CLOCK_BOOTTIME` is monotonic plus suspend; other hosts fail
  closed rather than substituting a clock that under-reports. A test compares the
  clock against `Instant` over a short interval, which is what catches the
  100-nanosecond unit conversion being wrong in either direction.
- **`CorpusBatchSource`** turns the P11 span stream into the coordinator's exact
  `(first_target, valid_targets)` windows. Spans are never split: the frozen
  accounting makes every window a whole number of spans, including the ragged
  final update of eighteen full spans plus one 1,024-target span, so a window that
  cannot be filled exactly fails closed instead of being padded. Resume
  fast-forwards to the exact cursor and rejects one that falls inside a span,
  which a checkpoint published at a completed-update boundary never can.
- **The result reports rather than claims.** `completion_sla` carries the measured
  `MET` or `EXCEEDED`, elapsed time and completion are `OBSERVED_UNVERIFIED`, and
  final loss stays `UNVERIFIED`. Nothing here decides admission; E5 owns that.

One honest deviation to carry into E5. The frozen defaults specify
`loader.transfer_memory: "cuda-page-locked"`, and `CudaPinnedTransfer` implements
exactly that, but the P12 `TrainerBackend::accumulate` contract takes a host-side
`TrainingBatch` and the E1 graph uploads the tokens itself when it builds its index
tensors. Routing spans through the pinned ring as well would perform a device copy
that nothing ever reads. The launch path therefore uses a host staging ring that
preserves what the ring genuinely provides on this lane — bounded in-flight
ownership and prefetch that overlaps span I/O with compute — and the result object
carries this as an explicit limitation. The pinned path can only pay once a backend
consumes device pointers directly, which is a P12-contract change and out of scope
here. The cost of the deviation is nil either way: spans are 4 KB, so the whole run
moves about 4 GB of tokens against the 33 TB of gradient readback the same contract
already requires.

### E3 — Governed Production Corpus

- [ ] E3 data materialization

Dependencies: P4–P9A implementation. Independent of E1/E2.

- Acquire authorized, license-clean Python source, plus the hash-bound
  `evalplus-v0.3.1` protection manifest. *(Written before `fetch` and
  `import-benchmark` existed; both now do. See "What is left" at the end of this
  section.)*
- Run `curate`, `prepare-corpus`, `train-tokenizer`, `tokenize`, and `plan-spans`
  on the real inputs until an installed P8 generation reports
  `training_target_satisfied: true` at exactly `2,000,000,001` stored IDs.
- Keep every generated corpus, token, and checkpoint artifact under ignored roots.

**E3 is much larger than "run the pipeline", and the reasons are read from the code
rather than estimated.** Three pieces are missing outright and three more do not
survive production scale.

Missing outright:

- **No acquisition exists.** There is no HTTP client in `src/`. `curate` reads a
  hand-authored `MaterializedSourceManifestV1` plus a `content_root` of plain bytes
  (`src/data.rs:75-114`). This is implementation order, not a boundary: the
  `AGENTS.md` Security section mandates the contract's HTTPS and redirect rules, and
  `docs/rebuild-contract.md:109` marks HTTPS/checksums/credentials `REIMPLEMENT`,
  "mandatory in production".
- **No EvalPlus importer exists and no phase owns one.** `DECONTAM-001`
  (`docs/rebuild-contract.md:159`) specifies the extraction completely and assigns it
  to P6A, but the rebuilt Phase 6A is Adversarial Filter Cases. `README.md:139` states
  the exclusion outright.
- **The frozen EvalPlus digests are pinned nowhere.** `validate_benchmark_manifest`
  (`src/corpus.rs:602-663`) checks that `asset_sha256` and `decoded_sha256` are
  *shaped* like SHA-256 and never compares them to a known value, and nothing
  cross-checks `records.len()`. A hand-written manifest with plausible hashes and one
  trivial record passes every gate and yields a decontamination manifest that certifies
  almost nothing. This is a live integrity hole independent of E3.

Will not survive scale:

- **Manifest ceilings.** `MAX_CONTROL_FILE_BYTES = 64 MiB`
  (`src/data/source/io.rs:8`) is enforced on every `read_control_file` (`io.rs:206`),
  including when `prepare-corpus` reads the whole P4 generation manifest
  (`src/corpus.rs:382-387`). That manifest is one compact JSON line with one outcome
  per document, and `SourceOutcomeV4` (`src/corpus.rs:146-167`) carries the ~1.1 KB
  `governed_source_metadata` block, so an accepted outcome is roughly `2.4` KB —
  about **28,000 documents per generation**. Reaching `2,000,000,001` train IDs needs
  on the order of 7–8 GB of canonical train text and 1.5–3 million documents, which is
  50–100 times over. Two further reads hit the same cap and are *not* fixed by
  multi-generation input: `governed-corpus-manifest.json` read by `tokenize` (~123k
  documents) and `tokenizer-sample-manifest.json` read twice (~140k).
- **`assign_splits` is quadratic** (`src/corpus.rs:1254-1302`): every duplicate group
  is iterated for every connected component. At ~10^6 of each it does not terminate,
  and it is the most likely place a real run appears to hang.
- **Everything is resident.** `prepare-corpus` holds every document's content, lexical
  tokens, encoded tokens, a `BTreeSet` of 5-gram shingles, and a 256-component MinHash
  signature at once (`src/corpus.rs:169-183`), roughly 15–30 times corpus bytes.
  Tokenizer training is single-threaded and unresumable at ~24–40 GB resident
  (`src/tokenizer.rs:542-685`).

Two traps for whoever runs this: `training_target_satisfied: false` is **not an
error** and exits 0 (`src/storage.rs:401-409`), so the field must be asserted out of
the `tokenize` result rather than inferred from an exit code; and `shard_maximum_ids`
is irreversible once published — `read_range` re-reads and re-hashes the *entire*
backing shard on every sequence read (`src/storage.rs:1110-1111`), so a 67M-ID shard
means hashing 128 MiB per training read.

Also note that an in-repo corpus silently breaks the quality gate for an unrelated
reason: `git status -uall` over ~20-30k untracked files exceeds the 2 MiB
`CAPTURE_LIMIT_BYTES` (`xtask/src/quality_gate.rs:14`) and aborts with the misleading
`QUALITY_CAPTURE_LIMIT_EXCEEDED`. Keep generated corpora under the ignored roots.

**A latent P5 defect that E3 found by executing.** `parse_python` required the
tree-sitter module node to start at byte 0 (`src/parser.rs:281`), but tree-sitter
places leading whitespace *outside* that node as an extra. Any Python file
beginning with a blank line or a space therefore parsed perfectly and was still
reported `PYTHON_SYNTAX_REJECTED`, while a file beginning with a *comment* was
accepted, because comments sit inside the module. The rule was rejecting on layout
rather than on syntax, and P4 curation was silently discarding a large and
arbitrary slice of real Python. It is corrected to the requirement it was plainly
trying to express: everything outside the module node must be insignificant
whitespace. Nothing meaningful can go unparsed, strictly more documents curate and
none fewer, and the whole existing suite including P4 and P5 passes unchanged. It
surfaced because a real EvalPlus prompt starts with two blank lines.

**Two properties of the real assets that the contract's wording does not
anticipate.** `DECONTAM-001` says "strict ... JSONL decoding", but the assets were
produced by Python's `json` module and contain bare `NaN`, `Infinity` and
`-Infinity`, which RFC 8259 forbids. They are recognized explicitly rather than
tolerated loosely, and then refused a canonical form, because RFC 8785 cannot
represent a non-finite value either. Separately, `base_input`/`plus_input` contain
integers far beyond IEEE-754 double precision — one is 55 digits — and RFC 8785
serializes numbers through ECMAScript `Number::toString`, i.e. as doubles. Emitting
the nearest double would silently rewrite `6775685320645824322581483068371419745979053216268760300`
into `6.775685320645824e+54`, which could never match a source document, so those
values are counted and skipped rather than fabricated. `serde_json` cannot support
either decision — it accepts duplicate keys silently and parses oversized integers
straight into `f64`, destroying the evidence — which is why the importer carries its
own strict reader.

**Multi-generation corpus input has landed.** `CorpusPolicyConfigV1` now takes a
non-empty `source_generations` list instead of one manifest and one root, so many
P4 generations compose into a single corpus and each stays independently
verifiable and under the 64 MiB bound rather than the bound being raised. Every
capacity applies to the composed corpus. Identity uniqueness holds across the set,
not merely within one generation, and naming a manifest or root twice is refused
before anything is read. The composed identity is a domain-separated digest over
the per-generation digests in configuration order, which keeps the single value
that `tokenize` binds the sample manifest and tokenizer artifact to by equality.
The load-bearing property is covered by test: a duplicate straddling two
generations collapses to one representative exactly as it would inside one, since
composing would otherwise silently admit the duplicates deduplication exists to
remove.

**This does not finish the ceiling problem.** `tokenize` still reads one
`governed-corpus-manifest.json` and both stages still read one
`tokenizer-sample-manifest.json`, all through the same 64 MiB bound, at roughly
545 and 480 bytes per document — about 123,000 and 140,000 documents. Those are
emitted by `prepare-corpus` rather than supplied, so they need streaming or
splitting rather than a list, and the real per-document cost should be measured in
Phase 5 before either is sized. *(Since fixed by splitting — see below.)*

**The proof run executed the whole chain, and it settles the scale question.**
`fetch` → `import-benchmark` → `materialize-source` → `curate` (three shards) →
`prepare-corpus` → `train-tokenizer` → `tokenize` → `plan-spans` composed on real
inputs for the first time. The measurement corpus is the local CPython 3.14
standard library, 2,239 files, licensed `Python-2.0` which the frozen allowlist
permits. It is a *measurement* corpus and nothing else: every artifact stayed in
an ignored root, no model was produced, and its authorization record is a local
operator assertion.

`materialize-source` is new and was required to get here at all. `curate` consumes
a manifest naming every document with its license, provenance, and expected
SHA-256, which cannot be hand-written even at proof-run scale. It performs the
mechanical half — enumerate, hash, order deterministically, shard — and leaves the
governance half declared by the operator. It shards output because one generation
manifest cannot hold a production corpus.

Measured, replacing the estimates:

| Quantity | Estimated | Measured |
|---|---|---|
| Bytes per source-generation outcome | `~2.4` KB | **`2,319.8`** → `28,929` documents per generation |
| Bytes per governed-corpus document | `~545` | **`561.3`** → `119,550` documents |
| Bytes per tokenizer-sample document | `~480` | **`463.2`** → `144,890` documents |
| `prepare-corpus` residency | `15`–`30x` corpus bytes | **`42.7x`** (`1,250.6` MiB peak for `29.3` MiB accepted) |
| `assign_splits` at proof scale | might not terminate | **completes**; `16` percent above linear at `1,780` documents |

Per-stage wall clock at this scale: `materialize-source` `54.8` s for 2,206 files,
`curate` `4.8`–`5.7` s per 800-document shard, `prepare-corpus` `57.7` s for 1,780
accepted, `train-tokenizer` `6.2` s at `318` MiB peak, `tokenize` `2.9` s,
`plan-spans` `0.5` s.

Pipeline yield, which is new information and transfers better than any of the
byte figures: **`80.7` percent** of enumerated documents survive `curate`, and
**`64.6` percent** of those become dedup representatives, so about **`52` percent**
of an enumerated tree reaches the token corpus. Scaling a corpus needs that factor
of roughly two on top of everything else.

The observed `7.655` bytes per stored ID does **not** transfer and should not be
used for sizing: the tokenizer was trained on `8.78` MB of the same `9.0` MB it
then encoded, so its compression is overfit to this corpus. A diverse corpus
compresses less well, which means *fewer* raw bytes per token, not more.

**E3 cannot reach `training_target_satisfied: true` on this host, and the gap is
now quantified rather than estimated.** `tokenize` reported
`training_prefix_ids: 1,136,321` against the required `2,000,000,001` and exited
`0`, which is exactly why the field has to be asserted rather than inferred. Three
independent blockers, in the order they bite:

1. **`prepare-corpus` residency.** At `42.7x` measured, a production corpus needs
   on the order of a terabyte of RAM against this host's `93.6` GB. Composing
   generations does not help — the union is what is held resident.
   *(Since fixed structurally — see below.)*
2. **The two downstream manifest ceilings that Phase 4 did not fix.** A production
   corpus implies roughly `917,000` representatives against `119,550` and
   `144,890`, so both are six to eight times over. They are emitted rather than
   supplied, so they need streaming or splitting. *(Since fixed — see below.)*
3. **`assign_splits`.** It completes here, but a two-term fit over 580, 1,253 and
   1,780 documents attributes the `16` percent superlinearity to a quadratic
   coefficient which, extrapolated to `1.5` million documents, dominates by orders
   of magnitude. That is an extrapolation across a thousandfold range and should
   be treated as a direction, not a number — but it agrees with the code, which is
   `O(components x duplicate_groups)`.
   *(Since fixed, and the "orders of magnitude" was too strong — see below.)*

Roughly `61` source generations would be needed for a production corpus, which
Phase 4 now supports. The other two blockers are unaddressed and each is a real
piece of work. *(That count assumed the `1.5`–`3` million documents estimated
before the corpus was sized; the settled figure is in "What is left" below.)*

**`prepare-corpus` residency is down from `42.7x` to `13.8x`**, peak `1,250.6` to
`402.9` MiB on the same corpus, for about five percent more wall clock. Both
changes are losslessly semantics-preserving and the evidence is that the published
generation digest is byte-identical across all three runs
(`b507dcad7f164d958e9e3577b3ef6c1a97a9bbf54ea942081615838af69cad60`).

- **The lexical tokens were retained for a number.** `PreparedDocument` held a
  `Vec<LexicalToken>` — a heap `String` kind and a heap `Vec<u8>` text per token,
  across millions of tokens — and every later use needed only `.len()`, which the
  one-to-one `encoded_tokens` already carries. Deleting the field alone took
  `42.7x` to `30x`.
- **The shingle set is derived, so it is no longer stored.** A shingle
  concatenates five encoded tokens, so the set costs roughly five times the token
  bytes and was the single largest structure, yet it is wholly a function of
  `encoded_tokens` and is consulted only for the exact Jaccard of an LSH candidate
  pair and once per document in decontamination. It is rebuilt for those cases
  instead. The candidate set is ordered, so pairs sharing a left document are
  contiguous and that side is built once. That took `30x` to `13.8x`.

Deliberately not done: hashing shingles to fixed-width keys would cut the rebuild
cost further but replaces `DEDUP-001`'s exact Jaccard with a probabilistic one,
and flattening `encoded_tokens` into a buffer plus offsets is lossless but rewrites
frozen span-matching code for perhaps a further `1.5x`. Neither changes the verdict
below, so neither is worth its risk yet.

**It is a real improvement and it still does not reach production.** The remaining
dominant term is `encoded_tokens` at roughly half the residue. At `13.8x` a
production corpus of about `24.5` GB accepted canonical text implies `338` GB
against this host's `93.6` GB, where before it implied over a terabyte. Put the
other way, the tractable corpus at `64` GB of working memory moved from about
`1.5` GB of accepted text to about `4.6` GB — call it three times more corpus, not
the sixty times the frozen target needs. Closing that needs the structural change
rather than more constant factors: not holding the whole corpus resident at once,
which means spilling shingles and signatures and running deduplication in blocked
passes.

**The two downstream manifest ceilings are gone.** `prepare-corpus` emits
`governed-corpus-manifest.json` and `tokenizer-sample-manifest.json` as an index
plus hash-bound parts of `50,000` documents each, instead of one line holding
every document. At the measured `561.3` and `463.2` bytes per document that puts a
part near `28` and `23` MB against the `64` MiB control-file bound, and the
document count is no longer bounded at all. The ceilings were `119,550` and
`144,890` against a production corpus of roughly `917,000` representatives.

The filenames, the two configuration shapes, and all five `tokenize` binding
equalities are unchanged, because the index keeps the header — the three digests
that bind a corpus to its source generations, its splits, and its sample — and is
itself the hash-bound file the configuration names. What moved is only where the
documents live. A part is refused rather than published if it would not read back
(`MAXIMUM_PART_BYTES`, `32` MiB), so a future change to the shard size cannot
silently emit an unreadable artifact. On read, each part is verified against the
index digest, and its schema, its ordinal, and its document count are checked, so
a reordered, mislabelled, truncated, substituted, or absent part is rejected
rather than accepted as a smaller corpus. Part paths are portable relative paths
resolved against the index's own directory and cannot escape it.

Verified by re-running the real chain over the sharded form: `train-tokenizer`
reported the identical `selected_bytes: 8,780,188` across `1,148` documents and
`tokenize` the identical `stored_ids: 1,177,713` and
`training_prefix_ids: 1,136,321` across `1,149` documents, and the two tokenizer
artifacts agree on all `31,740` merges. Only the embedded binding digests differ,
which they must, since the index is a different artifact from the file it
replaced.

**`prepare-corpus` no longer holds the corpus, and residency no longer scales
with corpus size at all.** Peak working set on the proof corpus falls from
`403.7` MiB to `91.6` MiB — a further `4.4x` on top of the `42.7x` to `13.8x`
already taken — for `3.2` percent more wall clock (`59.1` s to `61.0` s). Both
runs were measured back to back on this host with the same inputs, and the
evidence that the change is lossless is as strong as it gets: **the two runs
publish the same generation digest**,
`6efbf0bc077c8ffc31a75c34cf125852142c996760cc41c5cc9cdbcd66aaaf4b`, and all
`1,157` published files agree by path and SHA-256.

Three changes, none of which touches a frozen decision:

- **The encoded tokens are one buffer plus offsets** rather than a `Vec<u8>` per
  token. `DEDUP-001`'s encoding is self-delimiting — a `u64` kind length, the
  kind, a `u64` text length, the text — so two token sequences are equal exactly
  when their concatenations are, and a shingle is the concatenation of five
  *consecutive* tokens with no separator, which in this layout is a contiguous
  slice. The shingle set and the protected-span hash therefore read the bytes in
  place instead of rebuilding them, and a covering test asserts the flat shingles
  equal the per-token concatenation they replaced.
- **Decontamination matching moved into the load pass.** Its per-document half
  depends only on the protected set, never on other documents, so deciding it
  while a document's tokens exist removes the only reason to produce every
  document's tokens twice. What remains of the old pass is the cluster rule,
  which needs one flag per document rather than any content.
- **Neither the bytes nor the tokens are retained.** The bytes are re-read from
  the hash-bound generation and re-verified on every read, so a file that changes
  between passes fails the run instead of contributing stale bytes — strictly
  more tamper-evident than pinning them in memory. The tokens are re-derived by
  the same pinned parser for the only thing that still needs them, the exact
  Jaccard of an LSH candidate pair, which the configuration already bounds.

The result is that residency scales with the document *count*, not the corpus
size. Three points confirm it is linear in that count: `581`, `1,262` and `1,780`
documents at `59.4`, `77.8` and `91.6` MiB, whose successive slopes agree at
`27.67` and `27.28` KiB per document over a fixed `43.8` MiB. A production corpus
of roughly `1.45` million documents therefore implies about **`38` GiB against
this host's `93.6` GB**, where the same corpus previously implied `338` GB. This
host can hold it. Note that `27.5` KiB per document is peak working set, not live
data — the structures account for roughly `5` KiB (a `2` KiB MinHash signature,
about `2.9` KiB of LSH band index, and the identities) and the rest is retained
heap arena, which is the conservative figure to plan against because it is what
the host must actually supply.

**The canonical-JSON check no longer costs anything measurable.** It had been
`44.8` s of `prepare-corpus`'s `61.0` s — `73` percent — searching every document
for each of `1,053` protected records with `content.windows(record).any(...)`, so
it cost corpus bytes times record count with no prefilter. One anchored
Aho-Corasick pass replaces it: **`61.0` s to `16.2` s** on the same corpus, which
is exactly the `16.2` s measured with those records removed entirely. The check
has gone from the dominant term to a free one, and the generation digest is
unchanged.

The records run from `2` bytes to `197,766`, so no fixed window or prefix hash
separates them, but an automaton over them whole is not the answer either — built
that way it measured `411.1` MiB peak against `91.6`, hundreds of megabytes of
states bought for nothing. The automaton is built over a `64`-byte anchor of each
record and every hit is then confirmed in full at its offset. That keeps peak at
`91.6` MiB exactly, because `64` bytes separates `1,026` of the `1,053` records
outright and the verification settles the rest. `Standard` match kind with an
overlapping search is what preserves the old semantics — a record nested inside
another's match still counts, which the leftmost kinds would drop — and a covering
test checks the automaton against the record-at-a-time scan on the cases that
could diverge: a shared anchor completed by only one record, a shared anchor
completed by neither, a record nested in another's span, and a record shorter
than the anchor.

`aho-corasick 1.1` joins `flate2` and `ureq` as a data-lane dependency. It was
already resolved in `Cargo.lock` through the accelerator tree, so naming it
directly adds one edge and no version churn, and it is pure Rust behind the same
boundary.

Extrapolated linearly, a `24.5` GB corpus is now about `3.9` hours of
`prepare-corpus` rather than `14.6`. The stage is single-threaded throughout,
which is where any further time would come from.

**`assign_splits` is linear, and the earlier characterization of it was too
strong.** It asked, for each component, which duplicate groups touch it, which
costs components times groups. It now asks the reverse: a group's members are all
unioned with each other, so a group sits in exactly one component and its
representative is filed there in the same pass that builds the components. The
generation digest is unchanged and all `1,157` files still agree.

The measurement corrects the record. At the worst case for the old form — every
document its own repository and its own cluster, so components and groups are
both the document count — it ran in `0.88` s at `20,000` documents and `3.46` s
at `40,000`. That ratio of `3.93` confirms the term really was quadratic, but
`0.19` s for the same input now is an `18.2x` improvement, not the orders of
magnitude the estimate above claimed. Extrapolating the worst case to `1.42`
million documents gives roughly `1.2` hours, and the proof corpus is nowhere near
that shape: repository grouping collapsed its `1,757` clusters into `91`
components, so its actual product was `5.0` percent of the worst case, or a few
minutes at production scale. **`assign_splits` was never going to hang.** It was
a real quadratic worth removing — cheaply, and provably without changing output —
but the estimate that named it a top-three blocker was wrong, and the reason it
was wrong is that it reasoned from the code without measuring the shape of the
data the code runs on.

**Landed so far.** `.gitignore` now matches the artifacts the pipeline actually
writes. Its extension rules previously matched none of them: token shards are
`shards/<split>-<seq>.u16le` (`src/storage.rs:820`) and checkpoint tensors are
`model/parameters.bf16` and `optimizer/*.f32` (`src/train/full_state.rs:28-36`), so
neither `*.bin` nor `*.safetensors` covered them and protection rested entirely on the
root-anchored `/data/` and `/checkpoints/` directory rules. Interrupted-stage partial
directories are ignored too. `CLAUDE.md` now records that acquisition is
contract-required rather than forbidden, which nothing in it previously said.

#### What is left

Every finding the exploration above opened is now closed, and each fix is
verified the same way: the proof corpus republishes generation digest
`6efbf0bc077c8ffc31a75c34cf125852142c996760cc41c5cc9cdbcd66aaaf4b` with all
`1,157` files agreeing by path and SHA-256.

| Finding | Status |
|---|---|
| No acquisition exists | Closed — `fetch`, governed HTTPS with bounded redirects and streaming |
| No EvalPlus importer | Closed — `import-benchmark`, `DECONTAM-001` transcribed |
| EvalPlus digests unpinned | Closed — four frozen digests compared, `BENCHMARK_ASSET_DIGEST_MISMATCH` |
| Source-generation manifest ceiling | Closed — multi-generation input |
| Governed and sample manifest ceilings | Closed — index plus hash-bound parts |
| `prepare-corpus` residency | Closed — `403.7` → `91.6` MiB, linear in document count |
| Canonical-JSON scan | Closed — `61.0` → `16.2` s, anchored Aho-Corasick |
| `assign_splits` quadratic | Closed — linear; and it was never going to hang |

Two more things had to be fixed that the exploration never predicted, and both
were found by executing rather than by reading: `materialize-source` had to be
written before an authorized tree could be enumerated at all, and a latent P5
defect was silently discarding every Python file that began with whitespace.

**`train-tokenizer` at the sample cap is measured, and it fits.** It had one
point — `6.2` s at `318` MiB on the proof run's `8.78` MB sample — against a
`2,000,000,000`-byte cap, and the exploration's `24`–`40` GB guess had never been
checked. A six-point sweep over local Python from `8` MB to `184.6` MB, a `23x`
range, settles it:

| Sample | Peak | Wall clock |
|---|---|---|
| `8.0` MB | `332.4` MiB | `5.0` s |
| `16.0` MB | `515.0` MiB | `10.1` s |
| `32.0` MB | `993.9` MiB | `19.7` s |
| `64.0` MB | `1,610.7` MiB | `39.1` s |
| `128.0` MB | `3,085.4` MiB | `78.3` s |
| `184.6` MB | `4,351.3` MiB | `117.2` s |

Both are strictly linear: `peak_MiB = 184.1 + 22.62 x sample_MB` at `R^2 =
0.99921`, and `seconds = 0.63 x sample_MB` at `R^2 = 0.99935` — a flat `1.55`
MB/s across the whole range. **At the `2,000,000,000`-byte cap that is `42.3` GiB
and `20.0` minutes.** The memory lands at the top of the old estimate and inside
this host's `93.6` GB, and it never coincides with `prepare-corpus`'s `38` GiB
because the stages are separate processes run in sequence. Twenty minutes also
makes "single-threaded and unresumable" a much smaller property than it sounded:
losing a run costs twenty minutes, not an evening.

Linearity is what the code says too, which is why the fit is believable rather
than a coincidence of this corpus. `train_rules` (`src/tokenizer.rs:628`) builds
`tokens`, `previous` and `next` as one `u32` per sample byte, and an `occurrences`
map holding one `u32` per adjacent position; every structure is `O(sample bytes)`
and none is `O(bytes^2)` or `O(bytes x vocabulary)`. There is a hard ceiling well
above the cap: a sample over `u32::MAX - 1` bytes is refused outright with
`TOKENIZER_SAMPLE_TOO_LARGE`.

The probe drove the real `train-tokenizer` command over a hand-built sample
manifest rather than the whole pipeline, because the manifest carries identities
and byte counts and no license field — so measuring against arbitrary local
Python asserts nothing governed, and no artifact left the ignored scratch root.
Each run trained a full `32,000`-token vocabulary, and the `8` MB point
reproduces the pipeline's known one (`332.4` MiB against `318` MiB at `8.78` MB),
which is what says the harness is measuring the same thing.

**What actually blocks E3 is not code any more: it is the corpus.** The frozen
target needs roughly `2,000,000,001` stored train IDs. `SOURCE-001`
(`docs/rebuild-contract.md:152`) makes that Stack v2 metadata plus authorized
Software Heritage content, which means real credentials. Those are the operator's
to supply and pass only through named environment variables, never a config file
or a log. The tooling for it is built and tested; what it lacks is an
authorization to use.

Working projections for that run. Each basis says what it rests on, because they
are not equally solid: the two corpus-size rows inherit an estimate, the rest
extrapolate measurements taken on this host. Nothing here is now unmeasured.

| Quantity | Projection | Basis |
|---|---|---|
| Accepted canonical text | `~24.5` GB | *estimate* — rests on a bytes-per-ID figure the proof run itself says is overfit; the first real corpus re-measures it, and everything below inherits it |
| Documents | `~1.45` million | the row above at a `~16.9` KiB mean accepted document |
| Enumerated documents needed | `~2.8` million | measured yield: `52` percent of an enumerated tree reaches the token corpus |
| Source generations | `~50`, more in practice | `1.45` million accepted at a measured `28,929` per generation, before the rejected outcomes each manifest also carries |
| `prepare-corpus` peak memory | `~38` GiB | measured `43.8` MiB fixed plus `27.5` KiB per document, linear over three points |
| `prepare-corpus` wall clock | `~3.9` hours | measured `16.0` s for `29.3` MiB, single-threaded |
| `train-tokenizer` peak memory | `~42.3` GiB | measured `184.1` MiB fixed plus `22.62` MiB per sample MB, linear over six points to `184.6` MB |
| `train-tokenizer` wall clock | `~20` minutes | measured flat `1.55` MB/s over the same range |
| Token shards on disk | `~4` GB | exact: `2,000,000,001` IDs at `2` bytes |

Three traps that still apply to whoever runs it. `training_target_satisfied:
false` is **not an error** and exits `0` (`src/storage.rs:401-409`), so assert the
field out of the `tokenize` result rather than inferring it from an exit code.
`shard_maximum_ids` is irreversible once published, and `read_range` re-reads and
re-hashes the *entire* backing shard on every sequence read
(`src/storage.rs:1110-1111`), so a 67M-ID shard means hashing `128` MiB per
training read — pick it small. And keep the corpus out of the repository: `git
status -uall` over tens of thousands of untracked files exceeds the `2` MiB
`CAPTURE_LIMIT_BYTES` (`xtask/src/quality_gate.rs:14`) and aborts the quality gate
with the misleading `QUALITY_CAPTURE_LIMIT_EXCEEDED`.

**The honest position on the checkbox.** Nothing known now says the frozen target
is unreachable on this host. The memory blocker that did say so is gone, every
projection above is now measured rather than estimated, and the two heaviest
stages peak at `38` and `42.3` GiB in sequence against `93.6` GB. But E3 stays
unchecked until an installed P8 generation actually reports
`training_target_satisfied: true` at exactly `2,000,000,001` stored IDs, and
reaching that needs a governed corpus this repository cannot acquire on its own.

### E4 — Hardware Diagnostics on the Qualified Tuple

- [ ] E4 diagnostics

Dependencies: P18 implementation and the physical RTX 5090 host. Independent of E3.

- Run `xtask probe-cuda`, `xtask select-backend` (`p2-cuda`), and the CUDA-aware
  suites (`p10`/`p11`/`p12`/`p13`/`p18` with `--features cuda`) on the RTX 5090,
  or dispatch `windows-cuda.yml` from `main` with `run_hardware=true`.
- CUDA failures are real failures; absent or skipped hardware remains `UNVERIFIED`.

### E5 — Performance Calibration and Admission

- [ ] E5 admission projection

Dependencies: E1–E4.

- Run the P14 profile and P15 stability ladder on hardware with the real corpus;
  collect exactly five fresh-process samples per overhead class and compute the
  frozen `O_bound`/`R_qual` projection.
- Admission requires the whole-run-equivalent projection at or below `25,920`
  seconds. Roughly `69,445` targets per second sustained is needed; if the
  measured projection misses, the run is blocked and the thresholds stay fixed.

### E6 — The 2,000,000,000-Target Run and Quality Evaluation

- [ ] E6 execution

Dependencies: E5.

- Execute the run to a durable final checkpoint with actual continuous elapsed
  time at or below `28,800` seconds on the suspend-inclusive clock, then
  revalidate it with `train --verify-final-checkpoint` from a fresh process.
- Freeze the held-out validation manifest, unigram baseline artifact, and prompt
  pack, then run `evaluate-quality` against the final checkpoint.
- Manual approvals, receipts, and pointers remain `SKIPPED`; any claim not backed
  by the executed run remains `UNVERIFIED`.
