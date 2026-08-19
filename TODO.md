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

The P1–P19 implementation phases are complete, but no real training run has occurred.
This track closes that gap in dependency order. Simplified-mode rules continue to
apply: ordinary automated checks only, no receipts or approvals, and every unexecuted
or unmeasured fact remains `UNVERIFIED`. The fixed `25,920`-second admission ceiling
and `28,800`-second completion SLA are never retuned after measurement; a failed
projection blocks the run instead of moving a threshold.

**Where this stands.** Every piece of code the run needs is built, measured and
gated, and the corpus it needs is installed. What is left is one thing this
repository cannot do for itself:

| | State | Blocked on |
|---|---|---|
| E1, E1A, E1B, E2 | Complete, hardware-verified | — |
| E3 corpus | **Complete**: `2,000,000,001` training IDs installed and verified | — |
| E4 diagnostics | Not run | Physical RTX 5090 session, or a `windows-cuda.yml` dispatch |
| **E5** admission | Not run | E4 — and it starts from a measured `4.3x` SLA miss |
| E6 execution | Unreachable | E5 admitting the run |

One blocking fact to carry, measured rather than estimated. **The run does not
meet the SLA on current evidence**: `34.2` hours projected against `28,800`
seconds, which E5 must either close or record as a refusal. The other blocker
that stood here — that E3 could not finish without an authorization no amount of
engineering supplies — is gone: the authorization was granted, the source it
pointed at proved unobtainable, and `SOURCE-002` reached the same code through a
dataset that carries it directly.

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

**Why the bound exists, so it is never mistaken for a tolerance added to make a
test pass.** Device gradients do not equal the P9B oracle's canonical IEEE-754
bytes, while the forward BF16 logits and FP32 loss match exactly.
`tests/e1a_numerical_probe.rs` isolated the cause on hardware rather than
hypothesizing it: contraction and reduction order are *exonerated* — device
summation and `matmul` reproduce host left-to-right accumulation exactly — and
vendor transcendentals are the sole cause, differing by one ULP on `exp`, `sin`,
`cos` and `ln` while `sqrt` and `recip` stay bit-identical, which is what IEEE-754
requires of each. The deviation profile matches: error tracks how many
transcendentals separate a parameter from the loss.

No repository setting changes a vendor libm, and Rust's own transcendentals are
not bit-stable across platform libm implementations either, so **this is
irreducible** and byte equality on gradients was never achievable. The forward
stays exact because every frozen BF16 storage point absorbs the difference;
gradients are raw FP32 and expose it.

Do not weaken, widen, or delete the gate. It is frozen policy, and the current
margin against it is reported per operating shape in E1B below.

### E1B — Batched, Memory-Efficient Attention and Loss

- [x] E1B implementation

Dependencies: E1 implementation.

**The operating table E5 chooses from.** Measured on the prototype RTX 5090 at
canonical scale (`tests/e1b_canonical_scale_probe.rs`):

| Sequences per dispatch | Peak device memory | Throughput | Projected wall clock |
|---|---|---|---|
| 1 | `9,552` MiB | `5,471` targets/s | `101.5` h |
| 4 | `14,130` MiB | `10,481` targets/s | `53.0` h |
| 8 | `18,734` MiB | `13,731` targets/s | `40.5` h |
| 16 | `31,879` MiB | `16,221` targets/s | `34.2` h |

The frozen P14 micro-batch of 16 fits and runs, which was E1B's goal, but at
`31,879` MiB of `32,607` it leaves `728` MiB of headroom. **8 sequences is the
safer operating point for an unattended run, and E5 owns that choice.**

**E1B is complete and the run does not meet the SLA.** `34.2` hours against the
`28,800`-second completion SLA is short by a factor of `4.3`. That constant is
frozen and not retunable after measurement, so this is a blocking input to E5's
admission projection rather than something E1B could resolve. Where the time
goes, for whoever attacks it: the graph sustains roughly `19` TFLOP/s while
isolated FP32 matmul on the same shapes reaches `72`–`79`, so matmul is about a
quarter of the wall clock and the rest is elementwise traffic, launch overhead,
and the `541` MB per-micro-batch gradient readback the `TrainerBackend` contract
requires.

**The conformance gate runs at production batch widths**, not just at the
single-sequence fixture, because batch width demonstrably moves the gradient.
`logits_exact` and `loss_exact` hold at every width, and gradient deviation is
flat at `3.747215e-4`, `3.747288e-4` and `3.747141e-4` for 4, 8 and 16 sequences
against the `0.03` bound.

**True BF16 storage was measured and deliberately rejected — do not retry it
without new information.** Its precondition holds and isolated BF16 matmul is
`2.14x`–`2.88x` faster, but the whole-model result was equal throughput
(`16,500` vs `16,221` targets/s) for `11.6x` worse gradient conformance and
`150x` worse batch-shape sensitivity, the latter reaching `3.586e-2` — larger
than the `0.03` bound itself. The reason it gains so little is structural: the
frozen semantics place *no* storage point on the attention scores, their softmax,
or the loss, which are exactly the memory- and time-dominant tensors, so BF16
reaches only the linear projections while its boundary casts give much of that
back. Owner decision 2026-08-17: keep the FP32 substrate. The probe is retained so
the finding stays reproducible.

<details>
<summary>Superseded measurements from the pre-batching and pre-checkpointing graph</summary>

Kept because they make the final numbers legible as a result rather than an
assertion, and because they record a dead end worth not repeating.

| Graph, at one sequence per dispatch | Peak memory | Throughput | Projection |
|---|---|---|---|
| Original, unbatched and fully materializing | `16,318` MiB | `2,645` targets/s | `210` h |
| Activation checkpointing only | `13,601` MiB | `2,453` targets/s | `226.5` h |
| Heads batched, sequences not | `13,361` MiB | `3,894` targets/s | `142.7` h |

**Batching sequences did not pay until the materializations were bounded**, and
the measurement was emphatic: at four sequences the pre-checkpointing graph
reached a `32,158` MiB peak and throughput *collapsed* to `649` targets/s — six
times worse than a single sequence, because under that memory pressure the
allocator thrashes and the arithmetic stops mattering. That is why bounding the
materializations had to land before any batch wider than one sequence was worth
measuring again, and why the final table shows `10,481` targets/s at that same
width. First dispatch also costs `172` s of kernel compilation, one-time but
charged to the SLA clock.

</details>

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

- [x] E3 data materialization

Dependencies: P4–P9A implementation. Independent of E1/E2.

- Acquire authorized, license-clean Python source, plus the hash-bound
  `evalplus-v0.3.1` protection manifest.
- Run `curate`, `prepare-corpus`, `train-tokenizer`, `tokenize`, and `plan-spans`
  on the real inputs until an installed P8 generation reports
  `training_target_satisfied: true` at exactly `2,000,000,001` stored IDs.
- Keep every generated corpus, token, and checkpoint artifact under ignored roots.

E3 turned out to be much larger than "run the pipeline": exploration found three
pieces missing outright and three more that would not survive production scale,
and executing the chain found three further defects that reading it had not — the
last of them only after `4.8` million documents of real code had gone through it.
All nine are closed, and the chain has run to completion: an installed token
generation reports `training_target_satisfied: true` at exactly `2,000,000,001`
training IDs. What follows is the record of how, kept because most of it was
expensive to learn and none of it is visible from the code alone.

#### The pipeline is built and measured

Every stage now runs, and every fix was verified the same way: the proof corpus
republishes generation digest
`6efbf0bc077c8ffc31a75c34cf125852142c996760cc41c5cc9cdbcd66aaaf4b` with all
`1,157` files agreeing by path and SHA-256. That digest is the regression anchor —
if a future change to `curate`, `prepare-corpus` or the adapters moves it, the
change is not semantics-preserving.

Two behaviours worth knowing before touching these stages, because both were
expensive to find and neither is visible from the code alone. `prepare-corpus`
residency scales with document *count*, not corpus size, at `43.8` MiB fixed plus
a per-document slope — so holding anything per-document is the expensive mistake
there. The slope itself is a property of the corpus, not of the code: `27.5` KiB
per document on `16.9` KiB documents, `16.45` KiB on the `3,517`-byte documents
Stack v1 actually delivers. Re-measure it against the corpus in hand rather than
carrying a number over from another one.

And `train-tokenizer` is strictly linear in sample bytes at
`peak_MiB = 184.1 + 22.62 x sample_MB` (`R^2 = 0.99921`) and a flat `1.55` MB/s,
measured over a `23x` range, which is why the `2,000,000,000`-byte cap projects to
`42.3` GiB and `20.0` minutes rather than the `24`–`40` GB and unknown time
originally feared.

**What blocked E3 was authorization, and the alternate route removed it.** The frozen
target needs roughly `2,000,000,001` stored train IDs, and `SOURCE-001`
(`docs/rebuild-contract.md:152`) makes the source Stack v2 metadata plus
authorized Software Heritage content.

`materialize-stack-source` now implements exactly that. It takes hash-bound
Parquet metadata shards — acquired by `fetch`, so what it reads is pinned rather
than crawled — projects the columns the operator binds by name, applies the
language filter, the licence allowlist and the frozen `1,000,000`-byte document
ceiling, resolves each surviving identifier to a Software Heritage blob through
the same transport rules as any other acquisition, and publishes the sharded
`MaterializedSourceManifestV1` plus content tree that `curate` already consumes.

Three properties are worth stating because they are what make it governed rather
than merely functional. The identifier-to-content step is a link in the hash
chain, not a gap in it: the archive addresses content by `sha1_git`, so the blob
is verified against the identifier that selected it before it is written, and a
substituted body of the same length fails the run. Licences come per row from the
shard rather than one blanket declaration over a tree, and a dual-licensed row is
admitted only if *every* term it carries is allowlisted. And each rejection is
counted by rule in the result — language, licence, oversize, incomplete,
duplicate — so a run that admits far fewer documents than expected says which
rule did it instead of leaving one number to guess from.

`parquet` and `arrow` are the first data-lane dependencies that genuinely widen
the tree: 30 new packages. The codec set was pure Rust when it landed, on the
reasoning that `CLAUDE.md` permitted exactly one piece of native code on the data
lane and Parquet's `zstd` feature would bring `zstd-sys` with it. The real shards
then turned out to be Zstandard, and `SCOPE-002` admitted that decoder as a third
named boundary — so the constraint below is now history rather than current
policy, and it is kept because the reasoning still applies to every codec nobody
has approved. `zstd-sys` was already in `Cargo.lock` through `zip` in the
accelerator tree, but that tree is feature-gated and never compiles into a CPU or
data build, so enabling it was a real widening of the default build rather than a
bookkeeping change. Everything else stayed out: no `zip`, `libz`, `lz4-sys`,
`snappy-sys`, `brotli-sys` or `openssl` is reachable from the default build. A
shard in a codec outside that set still fails with a typed error naming it rather
than being silently unreadable, which is what turned the Zstandard discovery into
a decision instead of a mystery.

**What remained was the operator's, and it has since been supplied.** Accepting
the HuggingFace dataset terms and providing a token through a named environment
variable was the whole of it. Software Heritage bulk access — the other half of
this sentence when it was written — turned out to be unobtainable on any useful
timescale, and `SOURCE-002` replaced it with a content-bearing Parquet source
that carries the code itself. The adapter is still verified against Parquet
fixtures and a loopback origin rather than against the dataset, which is why the
column mapping is declared in configuration rather than assumed: a schema
revision is an operator change instead of a code change, and that is exactly what
made the switch to a different dataset a config-and-adapter job rather than a
redesign.

#### Acquisition hardening, and what the first real contact established

The adapter was correct and operationally unusable before this: no retry, and a
create-new tree is discarded on any error, so one `429` at blob N threw away N-1
completed transfers. Over roughly `1.45` million fetches that is not a run that
finishes, and `docs/rebuild-contract.md:113` had asked for retries and resumable
generations all along. Three changes close it, all fixture-tested.

Transient failures — transport errors, `429`, `5xx` — retry on a bounded, purely
arithmetic backoff; every other status does not retry at all, because repeating a
`403` only hides a configuration error behind a delay. Retrying cannot affect
determinism: every published byte is still verified against the `sha1_git` its
metadata declared, so a blob that needed three attempts yields the same artifact
as one that needed none, and attempt counts are an input to no hash.

Resumability is **partitioning, not checkpointing**, because a resumable partial
tree is precisely the half-populated generation that create-new publication
exists to prevent. The work splits by `blob_id` prefix, one generation per
partition, and a failed partition is rerun. Since the split is a function of the
identifier, partitions cannot duplicate a document and their union is exactly
what an unpartitioned run selects — both asserted against a real sixteen-way
split. A partition nothing landed in succeeds without publishing
(`STACK_PARTITION_EMPTY`), so an operator loop never reads a failure code as
success; an *unpartitioned* run selecting nothing still fails, because there the
filters really are wrong.

`content_encoding` is declared, never sniffed, because the bulk mirror and the
archive API disagree about framing and guessing from a magic number would make
the corpus depend on what a server happened to send.

**First real contact with HuggingFace, and what it settled.** The governed path
works against the live API: `discover` reports a digest without publishing, and
`fetch` transfers only against that pinned digest.

| Quantity | Measured |
|---|---|
| `bigcode/the-stack-v2` Python metadata | `9` shards, `13.0` GB total, about `1.55` GB each |
| `bigcode/the-stack-v2-dedup` Python metadata | `6` shards, `8.02` GB |
| Gating | `gated: auto` with a terms prompt |

**Source choice: the plain `bigcode/the-stack-v2`.** The tempting argument for
the deduplicated variant is that `8.02` GB beats `13.0` GB, and it is a red
herring: shards are fetched one at a time until enough documents accumulate, and
one shard holds far more rows than the roughly `1.45` million needed, so the
difference is in shards that are never transferred. With that gone, the plain
dataset is the literal `SOURCE-001` source, and upstream deduplication would hand
over *its* representative per cluster instead of letting `DEDUP-001` choose by
provenance and comment ratio. Small, but control given up for a benefit that does
not exist.

**The `403` was terms acceptance, not token scope.** The token authenticates and
the metadata API answers while file downloads return `403`, which looks like a
token-permission problem and is not one: accepting the dataset terms cleared it
with the token's scopes byte-identical to before, still fine-grained and still
scoped to its own user. Recorded because the first diagnosis here was the wrong
one — the symptom points at scope and the cause was acceptance — and because the
adapter's refusal to retry a `403` is what kept that diagnosis cheap instead of
burying it under five identical failures.

**The metadata is Zstandard, and reading it required an amendment.** The real
shard fails to decode as shipped: `Disabled feature at compile time: zstd`.
HuggingFace's auto-converted branch is byte-identical to the main branch, so no
supported-codec copy of the same data exists, and `parquet-rs` wires only the
C-backed `zstd` crate with no pure-Rust backend in any published version. The
owner approved `SCOPE-002` (`docs/decision-ledger-v4.md`) on 2026-08-18, which
supersedes `SCOPE-001` and admits the Zstandard decoder as a third named native
boundary — decode-only, reached solely through the Parquet reader, with every
byte it produces still subject to the same digest verification as any other
input. The data lane already compiled C for the Tree-sitter parser, so this is a
second pinned C dependency rather than a new class of requirement. The span seed
is unaffected: it reads the frozen-decision range of the immutable
`docs/rebuild-contract.md`, not a ledger.

**What one real shard contains.** `train-00000-of-00009.parquet`, `1.55` GB,
fetched at `4.9` MB/s and read in `5.9` s at `37` MiB peak — the batched reader
never holds the shard.

| Quantity | Measured |
|---|---|
| Metadata rows | `8,550,924` |
| Rejected by licence | `6,421,647` (`75.1` percent) |
| Rejected as oversize | `521` |
| Rejected by language or incomplete identity | `0` |
| **Eligible before deduplication** | **`2,128,756`** (`24.9` percent) |

At the measured downstream yield — `80.7` percent surviving `curate` and `64.6`
percent becoming representatives — one shard is worth roughly `1.11` million
representatives against the `1.45` million the target needs, so **two shards
comfortably exceed it** and the remaining seven are never transferred. That is
the fact that settles the source question: preferring the deduplicated variant to
save `5` GB would have been optimizing a download that does not happen.

**The licence allowlist was already frozen, and the adapter now enforces that.**
`PERMISSIVE_LICENSES` (`src/data/policy.rs:19`) fixes the permitted set to
`0BSD`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `BSL-1.0`, `ISC`, `MIT`,
`MIT-0`, `Python-2.0`, `Zlib`, and the shard carries canonical SPDX casing, so a
lowercase allowlist matches nothing — the first probe rejected all `8,550,924`
rows for exactly that reason. Configuring an allowlist the frozen policy refuses
is now `STACK_LICENSE_NOT_PERMITTED` at validation time, because the cost of that
mistake is not a wrong answer but a wasted acquisition: the blob is transferred,
verified and written before `curate` rejects its licence.

Two diagnostics were added for the same reason, both after guessing proved
expensive. A missing column now names the columns the shard *does* carry with
their Arrow types, which turned a schema unknown into one run instead of one
guess per attempt. And a rejected licence is now reported by value, bounded to
`32` distinct examples, which is what turned "everything was rejected" into a
correct allowlist immediately.

**The source moved to content-bearing Parquet, and the chain got shorter.**
`SOURCE-002` (`docs/decision-ledger-v5.md`, owner-approved 2026-08-18) adopts the
Stack v1 deduplicated Python subset under `SOURCE-001`'s own trigger, both halves
of which are satisfied: the Stack v2 content path needs a bulk agreement with
Software Heritage and INRIA, an AWS account for a requester-pays bucket, and
SigV4 signing the client does not implement — established by reading the
dataset's own terms, not assumed — and the owner named the alternate. The Stack
v2 adapter is retained rather than removed, because that access may yet exist.

`materialize-stack-content` is the adapter, and it is *simpler* than the one it
supplements: no retry, no backoff, no partitioning, no content encoding, because
none of that is needed when the text is already in the row. Everything governed
is unchanged — shards hash-bound before reading, the frozen allowlist, the frozen
ceiling, create-new publication.

Measured on `data-00000-of-00144.parquet`, `205` MB, read in `77.7` s at `202`
MiB peak:

| Quantity | Measured |
|---|---|
| Content rows | `90,016` |
| **Admitted documents** | **`84,634`** (`94.0` percent) |
| Rejected by licence | `5,367` |
| Rejected as oversize | `1` |
| Skipped for an unusable provenance path | `14` |
| Rows whose content did not reproduce its declared identity | `22` (`0.026` percent) |
| Python source extracted | `429` MB |

Then through `curate`: `20,000` documents in, `17,654` parser-accepted, **`16,384`
policy-accepted** (`81.9` percent), `83` quarantined. That end-to-end run is the
requalification evidence `SOURCE-002` asks for, and it is what establishes the
alternate ends at the same contract the primary route does.

At roughly `69,300` curated documents per shard, the `1.45` million target needs
about `21` shards of the `144` available — some `4.3` GB of download against the
`13.0` GB the metadata-only route would have needed before a single blob was
fetched.

**Three findings the end-to-end check produced that fixtures had not.**

The identity cross-check is a property of the shard, not an invariant. A
content-bearing row carries *decoded* text, so reproducing the original blob's
`sha1_git` depends on that decoding round-tripping; it does for `99.974` percent
of rows here. `blob_identity_verified` is therefore an explicit operator
declaration rather than an assumption, and the disagreement count is reported
either way so the claim is never taken on trust. The chain does not rest on it:
the shard's own digest is what binds its rows.

Real repository paths are not all portable relative paths — `14` of `90,016`
carry separators or segments that `curate` refuses as provenance. Rewriting one
would falsify the provenance the manifest records, so the row is skipped and
counted.

**Both adapters emitted a manifest `curate` refuses**, and only running `curate`
found it. `adapter_namespace` and `authorization.scheme` are pinned contract
values (`stack-v2-swh-materialized-v1` and `materialized-source-authorization-v1`),
and both adapters — including the Stack v2 one already committed and
fixture-tested — used their own labels instead. A full acquisition would have
completed and then been rejected at the first `curate` invocation. Both now emit
the pinned namespace and validate the scheme before reading anything, and the
fixtures assert the published manifest carries values `curate` accepts, because
a test that only checks the adapter against itself cannot catch this class of
error.

#### The pilot run, and the last estimate becomes a measurement

One shard driven through the whole chain — `materialize-stack-content` →
`curate` → `prepare-corpus` → `train-tokenizer` → `tokenize` — on real
permissively-licensed code with a tokenizer trained on that code.

| Stage | Result | Wall clock | Peak |
|---|---|---|---|
| Content adapter | `90,016` rows → `84,633` documents, `429` MB | `78` s | `202` MiB |
| `curate` (5 generations) | `68,963` policy-accepted (`81.5` percent) | `~400` s | — |
| `prepare-corpus` | `52,890` representatives, `16,065` excluded | `213.5` s | `1,152` MiB |
| `train-tokenizer` | `182.5` MB sample, `51,813` documents, `32,000` merges | `125.6` s | `4,802` MiB |
| `tokenize` | `35,358,525` stored IDs, `34,696,920` train | `36.2` s | — |

**`5.26` bytes per stored ID, `5.36` per training ID.** That is the figure every
size projection rested on, and until now it was inherited from a corpus whose
tokenizer had been trained on the same `9` MB it then encoded. The proof run's
`7.655` is superseded, and the direction is the one predicted when it was
flagged: a diverse corpus compresses *better* per byte, so a byte buys more
tokens and the corpus needed is smaller.

Re-derived from measurement rather than estimate:

| Quantity | Estimated | **Measured** |
|---|---|---|
| Representative text for `2,000,000,001` train IDs | `~24.5` GB | **`10.0` GB** |
| Representative documents | `~1.45` million | **`3.05` million** |
| Mean representative document | `~16.9` KiB | **`3,517` bytes** |
| Shards to fetch, of `144` | — | **`~58`**, about `11.8` GB |

The document count rose while the byte count fell, because Stack v1 documents are
five times smaller than the CPython standard library's. Both numbers moved, and
only measuring them together would have caught that.

`prepare-corpus` residency measured `16.45` KiB per input document here against
the `27.5` KiB the earlier model gave — the model was built on `16.9` KiB
documents and this corpus's are `3.5` KiB, so residency tracks document count
*for a given size distribution* rather than count alone. At the full corpus that
is roughly `62` GiB against this host's `93.6` GB: it fits, with less headroom
than the earlier figure implied. The `train-tokenizer` model needed no such
correction — it predicted `4,312` MiB and `115` s against `4,802` MiB and `125.6`
s measured, within `11` and `9` percent on data it had never seen.

`training_target_satisfied: false` at `34,696,920` of `2,000,000,001`, which is
`1.7` percent and exactly what one shard of fifty-eight should give.

**Windows Defender deletes corpus files after they are written.** Two documents
of `84,633` — from `TAO/Firewall/EXPLOITS` and `Empire/persistence/osx` — were
quarantined between the adapter writing them and `curate` reading them, and the
detection log names them by full path. It also raced the `.acquire-partial`
tree mid-write. The rate is `0.0024` percent, which sounds negligible and is not:
each loss fails the entire `curate` invocation for its generation of `20,000`
documents, so two lost documents cost two of five generations. Extrapolated, the
full corpus would lose roughly `34` files spread unpredictably across `58`
shards. An exclusion for the corpus root fixes it, and the pipeline behaved
correctly throughout — a file present at write and absent at read is precisely
what the verification exists to catch.

Two more real-data findings, both now skipped and counted rather than fatal:
`14` rows per shard carry repository paths that are not portable relative paths,
and `1` carries a repository identity the governed rule refuses. Rewriting either
would falsify what the manifest records.

#### The full acquisition, and the defect that only real code found

`66` of the `144` `the-stack-dedup` Python shards, fetched over the governed
HTTPS path one shard per invocation and curated one generation at a time.

| Stage | Result |
|---|---|
| Fetched | `66` shards, `12.2` GB, no fetch or materialize failure |
| Materialized | `5,589,328` documents |
| `curate` | `330` generations, `4,546,453` policy-accepted (`81.3` percent) |
| `prepare-corpus` | `3,500,856` representatives, `1,033,536` excluded, `11.3` GB |
| Wall clock | `~7.2` h acquisition and curation, `5.85` h `prepare-corpus` |
| `prepare-corpus` peak | `42.6` GiB of `93.6` GiB |

**Nesting depth aborted the process instead of being rejected.** The governed
limit `MAXIMUM_CST_DEPTH` is `4,096`, but the pinned grammar recurses in C, and a
Windows main thread's one mebibyte runs out at roughly `1,900` levels. Every
document nested between those two figures is one the policy *admits*, and the
process died on it; documents past the limit died rather than being rejected by
the limit that exists to reject them. Reading the code could not have found this
— `traverse` is explicitly stack-based, so the recursion is entirely on the C
side — and neither could the proof corpus. It took `4.8` million documents of
real code, at about one crashing document per `200,000`, to produce `24` aborts.
Each one destroyed a whole generation of `20,000` documents, `327,000` in total,
and an abort leaves no result object, no typed code, and no partial-tree cleanup.

The fix sizes the stack from the frozen document ceiling rather than tuning it: a
document is at most `1,000,000` bytes, every level of nesting costs at least two
of them, so nothing admissible nests deeper than `500,000`, and the costliest
shape measures about `1,090` stack bytes per level. One gibibyte covers the worst
case with margin and costs nothing — Windows commits stack pages on demand, and
four threads holding that reservation moved neither working set nor private
bytes. `platform::run_on_command_stack` is the single boundary that provides it.
All `24` generations re-ran cleanly afterwards, and `8` further shards acquired
after the fix curated `550,487` documents out of `677,497` with no aborts at all.

**The residency model was measuring the wrong thing.** `16.45` KiB per accepted
document reproduced the pilot's peak exactly and still projected `71` GiB against
an actual `42.6`, because it folded two unrelated costs into one slope. The real
shape is visible in the trace: `38.6` GiB loading `330` generation manifests
(`6.6` million outcome records at about `5.95` KiB each), then a drop to `5.6`
GiB once those manifests are released and the documents are projected, then a
climb to the `42.6` GiB peak through shingling and dedup. Residency is dominated
by *manifest outcomes*, which scale with documents **enumerated**, not documents
accepted. The drop from `38.6` to `5.6` is the residency fix doing its job;
without it that `38.6` would have stayed resident underneath everything after it.

Full-scale dedup behaved as the single-shard pilot predicted, which is worth
recording because it was not obvious that it would: `490,047` candidate pairs and
`86,823` near-duplicate edges over `4.5` million documents, and **zero** exact
duplicates, because the source is already deduplicated by content hash.
Exclusion is almost entirely decontamination — `1,033,387` of `1,033,536` —
against `2,843` protected `evalplus-v0.3.1` records.

#### The token corpus, and E3 closing

Three commands against the published corpus, and the frozen target is met
exactly.

| Stage | Result | Wall clock | Peak |
|---|---|---|---|
| `train-tokenizer` | `1,999,999,960`-byte sample from `584,303` documents, `32,000` vocabulary, `qualified_range_satisfied` | `25.1` min | `42.0` GiB |
| `tokenize` | `2,049,649,763` stored IDs, `490` shards, `3.82` GB | `51.7` min | `9.3` GiB |
| `plan-spans` | `976,562` complete spans, `1,024` partial span targets | `37.5` s | `3.4` GiB |

```
training_prefix_ids       2,000,000,001    exactly the frozen target
training_valid_targets    2,000,000,000    exactly the frozen target
training_target_satisfied true
training_unused_tail_ids  18
```

Token generation
`8dbd12d5f440e8db83241fbebb3848e27ceb7e83f5c31c055d71e212c5baa9c6`, over
tokenizer
`99ef8e60046c09f4278b131b584eaf6ae2821c178add3a32319524905ad29d4e`, over corpus
generation
`01a406801970c883e3709e3da81825f765c7a50c5b5b0cd00cfe2f50965c02f8`. Splits are
`2,000,000,019` stored train IDs across `2,936,749` documents, `24,047,158`
validation, `25,602,586` test.

**The overshoot did exactly what it was bought to do.** `479,222` train documents
and `1.62` GB of canonical text went unmaterialized, because the train stream
closes on the first document that reaches the target and everything after it is
skipped (`src/storage.rs:378`). That `13.7` percent of waste is the margin
working, not the margin being wasted: at `58` shards the projection was `2.01`
billion IDs, and had the tokenizer compressed one percent differently the chain
would have run for five hours and landed short. The last document overshot by
`18` IDs, and the prefix rule trimmed those.

Two model checks worth recording, because both were predictions made before the
run rather than after it. `train-tokenizer` peaked at `42.0` GiB against a
predicted `42.3` and took `25.1` minutes against `20`. And `3,500,856`
representatives were predicted from `4,546,453` accepted documents against
`3,486,672` projected — `0.4` percent out over a chain of three measured ratios.

Everything is under the ignored root `C:\python-slm-e3` on this host, which is
where it has to stay: an in-repo corpus aborts the quality gate at its `2` MiB
capture limit. `tokens-full\` is the installed generation, `corpus-full\` the
representative corpus behind it, `curated-*\` the `330` source generations,
`content-*\` the materialized documents, and `assets\` the fetched Parquet
shards.

Three traps that still apply to whoever runs this again.
`training_target_satisfied: false` is **not an error** and exits `0`
(`src/storage.rs:401-409`), so assert the field out of the `tokenize` result
rather than inferring it from an exit code — this run asserted it.
`shard_maximum_ids` is irreversible once published, and `read_range` re-reads and
re-hashes the *entire* backing shard on every sequence read
(`src/storage.rs:1110-1111`), so a 67M-ID shard means hashing `128` MiB per
training read; this run used `4,194,304`, giving `490` shards of about `8` MiB.
And keep the corpus out of the repository: `git status -uall` over tens of
thousands of untracked files exceeds the `2` MiB `CAPTURE_LIMIT_BYTES`
(`xtask/src/quality_gate.rs:14`) and aborts the quality gate with the misleading
`QUALITY_CAPTURE_LIMIT_EXCEEDED`.

**The checkbox is checked, and this is what it means.** An installed P8
generation reports `training_target_satisfied: true` at exactly `2,000,000,001`
stored training IDs and `2,000,000,000` valid targets, and `plan-spans` opened
that generation afterwards — which is the same verification on read that any
consumer performs. It does not mean the run is admitted: E5 still starts from a
measured `4.3x` SLA miss, and that is a separate question about hardware and
performance, not about data.

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

**E5 begins with a known negative result, not an open question.** E1B measured
`16,221` targets/s at the frozen micro-batch of 16 — a `34.2`-hour projection
against the `28,800`-second SLA, short by a factor of `4.3`. Both constants are
frozen and not retunable after measurement, so on current evidence **admission
fails** and E5's real work is either finding the missing factor or recording the
refusal with the gap quantified.

Two decisions E5 owns. The operating point: 16 sequences fits but leaves only
`728` MiB of headroom on a 32 GB device, so `8` sequences at `13,731` targets/s is
the safer choice for an unattended run and the throughput cost of that safety is
`15` percent. And the pinned-transfer deviation E2 carries: the frozen defaults
specify `cuda-page-locked`, the launch path uses a host staging ring instead, and
that can only change once a backend consumes device pointers directly — a
P12-contract change.

Where the remaining time is, measured rather than guessed: the graph sustains
roughly `19` TFLOP/s against `72`–`79` for isolated FP32 matmul on the same
shapes, so matmul is about a quarter of the wall clock and the rest is elementwise
traffic, launch overhead, and the `541` MB per-micro-batch gradient readback the
`TrainerBackend` contract requires. `docs/adr/0000` and P19 exist for the case
where the answer is that the frozen configuration cannot meet the frozen SLA.

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

E6 is unreachable until E5 admits the run, and on current measurements E5 does
not. The corpus is no longer part of that: E3 installed a token generation at
exactly `2,000,000,001` training IDs, so what stands between here and E6 is a
calibration that misses its SLA by `4.3x`, not a shortage of data.

The launch mode itself is ready: `train --config <defaults> --launch <launch>` is
the explicit execution mode, it demands `confirm_full_run`, and its SLA
measurement uses a suspend-inclusive clock so an overnight run that suspends does
not under-report against a wall-clock deadline.
