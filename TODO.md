# Clean-Rebuild Execution Plan

Dependencies, not phase numbers, define admissible order. Never start a phase before its
dependencies pass. The accelerator/backend track (P1B–P2), data track (P3–P9A), and CPU
model track (P3–P9B) may run in parallel where dependencies permit. The existing Rust code is
reference evidence, not code to copy. `AGENTS.md` governs work,
`docs/rebuild-contract.md` records the approved Phase 0 product decisions,
`docs/receipts/P0.md` is their signed approval authority, and `docs/ARCHITECTURE.md`
defines the currently approved target design. Once P0A passes, its selected
`docs/rebuild-contract-v2.md`, prototype-first portable-interface ADR, revised architecture
hash, and decision-ledger hash become the active amendment authority. A conflict is a stop
condition.

This revision starts the `prototype-v2` qualification epoch and fixes the sole P1A–P16A
execution profile as `prototype-windows-5090-v1`: native Windows x86_64 on an AMD Ryzen 9
9950X3D, the Rust `x86_64-pc-windows-msvc` target with the qualified VS 2022 x64
MSVC/Windows SDK toolchain, and one NVIDIA GeForce RTX 5090 at compute capability 12.0/SM120
through CUDA. Portable interfaces, schemas, artifact formats, and typed capability errors
are designed now, but Linux, macOS, GCC/Clang host adapters, ROCm/HIP/AMD accelerators, and
Metal/Apple Silicon accelerators are explicitly `DEFERRED_POST_P16`; they are neither
implemented nor P1A–P16A PASS requirements. The earlier P1A and P1B acceptances remain
immutable historical evidence for their recorded Windows/MSVC/CUDA/SM120/RTX 5090 tuple,
and all existing P2 attempts remain immutable historical attempts. None satisfies a
`prototype-v2` dependency because none binds this amendment, profile, schemas, and complete
hardware identity. New P1A, P1B, and P2 evidence is published under
`docs/receipts/P1A-prototype-v2`, `docs/receipts/P1B-prototype-v2`, and
`docs/receipts/P2-prototype-v2`; old schemas, runs, acceptances, pointers, and seals are
never rewritten or reinterpreted.

## Operating Rules

- Start every phase by reading `AGENTS.md`, `docs/rebuild-contract.md`,
  `docs/ARCHITECTURE.md`, this file, `git status`, and every dependency's authoritative
  acceptance record plus its referenced immutable run. After P0A, also read and validate
  the selected P0A contract amendment, prototype-first portable-interface ADR, revised architecture,
  decision ledger, schema bundle, and owner-approval commit. Preserve unrelated work.
- Do not run `cargo new`, `git init`, create a nested repository, broadly stage files,
  auto-commit, or tag. Use the existing repository.
- Implement only the active phase. Do not hide failed gates with fallbacks, relaxed
  assertions, ignored errors, or claims based on unexecuted commands.
- Normal tests are offline, deterministic, bounded, and credential-free. No Python
  interpreter, executable, package, module, build backend, code generator, embedded
  runtime, or Python-launched subprocess is part of or invoked by the build, data
  preparation, verification, receipt, or training pipeline. Python-language source is
  input data only.
- Rust owns CLI orchestration, data preparation, tokenization, storage, model/training
  control, checkpointing, verification, and receipt publication. C and C++ are permitted
  only inside pinned, audited, feature-gated hardware-accelerated kernels, standard native
  ML libraries, and native accelerator or graphics API bridges such as CUDA, HIP/ROCm,
  and Metal, plus the pinned Tree-sitter generated C parser/runtime used solely for the
  frozen Python CST, comment, and lexical-token semantics. No native component may own
  orchestration or general data transformation. Accelerator components expose a narrow
  ABI, propagate native errors, retain resources through asynchronous completion, and are
  absent from CPU-only builds; the parser exception is independently pinned and audited.
- Historical `.ps1` capture artifacts remain byte-for-byte archival evidence only. They
  are never invoked as entry points, copied into active orchestration, or rewritten; every
  active verification and publication command is the Rust `xtask` interface.
- Host-specific process, filesystem, dynamic-library, compiler, and accelerator mechanisms
  exist only behind internal Rust abstractions. Public argv, schemas, exit categories,
  atomic publication, redaction, process-tree containment, cleanup, and receipt semantics
  are frozen as portable contracts. Through P16A, only their Windows x86_64/MSVC/CUDA
  implementations are required and qualified. Rust invokes platform tools directly; a
  platform shell script is never a normative entry point. Selecting a deferred OS or
  provider must fail before discovery or mutation with a stable `DEFERRED_POST_P16`
  capability error; it must never silently fall back or report stub success.
- A receipt-bearing attempt begins only after the phase's output-root safety, dependency
  authority, committed source-identity, clean-tree, and closed-policy admission gates pass.
  A failure before that boundary emits a typed CLI error and must not create a receipt that
  would falsely claim a validated dependency or source. Every attempt after that boundary
  writes create-new
  `docs/receipts/<phase-namespace>/runs/<run-id>/evidence.json`, where the phase card fixes
  the namespace, using the card's closed schema with exact command transcripts and hashes.
  Historical P1A retains its
  published v1 receipt schemas; historical P1B retains its published v2 schemas and CUDA
  manifest. The
  `prototype-v2` phases use new closed schemas that retain portable host/provider fields
  while recording the exact prototype profile without changing any published schema.
  Every prototype-v2 receipt has stable closed fields for `profile_id`, `support_tier`, the
  exact host/provider/device tuple, memory model, backend and native-library identities, and
  applicable fixed SLA constants. `support_tier` is exactly one of `designed`, `implemented`,
  `tuple_qualified`, or `full_run_qualified`; a receipt proves only its enumerated tuple and
  never implies a Cartesian support matrix. A field that is not applicable to a CPU/data
  phase uses the schema's explicit null/not-applicable form rather than omission or inference.
  The verifier builds in a same-filesystem private staging directory, converts every
  post-admission failure or recovered interrupted stage into a sealed `FAIL` run, and only
  then atomically publishes the create-new run directory. Failed and superseded runs remain immutable and are
  never selected. A reviewed passing run gets a create-new `acceptances/<sequence>.json`
  record; the root `evidence.json` is an atomically replaceable versioned pointer containing
  that acceptance path and hash. A dependency passes only when the pointer, selected
  acceptance, referenced immutable run, `PASS` status, and every required named approval
  all validate. P0 is the
  sole receipt-model exception: its dependency passes only when the pinned P0 `VERIFY`
  block succeeds. `docs/receipts/P0.md` is its human approval authority, the sealed run is
  its machine evidence, and P0 requires no acceptance generation or root pointer. Machine
  evidence, checklist prose, silence, or an agent audit never substitutes for human approval.
- Within an owner-approved `prototype-v2` contract, P1A, P1B, and P2 are the automatic
  machine-qualification exceptions. A
  passing verifier publishes an acceptance with `required_approvals: []` and selects it
  through the phase root pointer. The verifier never edits this checklist or commits. The
  owner must review and check the phase complete before a dependent phase begins; no later
  phase inherits this exception unless its card says so explicitly.
- Each receipt records the command, working directory, relevant environment/configuration
  hashes, exit code, stdout/stderr or artifact hash, and status. A phase remains blocked
  unless its exact invocation appears in the normative global table below or its card.
  Receipts never contain credentials, raw source, PII, or secrets.
- A suggested commit is optional and occurs only after human review.

Until Phase 3 replaces the command contract, the CPU quality gate is:

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
cargo test --locked --features cpu-reference
```

From P3 onward, every phase runs `cargo run --locked -p xtask --bin xtask -- quality-gate` plus a
named targeted test/benchmark command for that phase and records both in its receipt.

The literal verification entry points are fixed below. P0A creates the developer-only
`xtask` workspace package, its Windows process/filesystem adapter behind portable Rust
interfaces, `verify-p0`, and the rejected-by-default `verify-phase` dispatcher with only
P0A installed; it is not an installed product executable. P3 extends that dispatcher and
adds `quality-gate`.
Each implementation phase installs its own case. The preceding implementation
owner must install no-code cases before freeze: P6 installs P6A, P7 installs P7A, P8
installs P9A, and P13 installs P14–P16A and P19. P17 and P18 install their own cases. The
Rust driver records every child argv and exit code and obeys the frozen command and receipt
contract on the sole qualified P1A–P16A profile.

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- verify-p0
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --output-root docs/receipts/P0A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --finalize --output-root docs/receipts/P0A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P0A --check-selected --output-root docs/receipts/P0A
cargo run --locked -p xtask --bin xtask -- verify-env --mode host --profile prototype-windows-5090-v1 --output-root docs/receipts/P1A-prototype-v2
cargo run --locked -p xtask --bin xtask -- verify-env --mode accelerator --profile prototype-windows-5090-v1 --provider cuda --output-root docs/receipts/P1B-prototype-v2
cargo run --locked -p xtask --bin xtask -- qualify-backend --profile prototype-windows-5090-v1 --output-root docs/receipts/P2-prototype-v2
cargo run --locked -p xtask --bin xtask -- quality-gate
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P3 --profile prototype-windows-5090-v1 --output-root docs/receipts/P3
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P4 --profile prototype-windows-5090-v1 --output-root docs/receipts/P4
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P5 --profile prototype-windows-5090-v1 --output-root docs/receipts/P5
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P6 --profile prototype-windows-5090-v1 --output-root docs/receipts/P6
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P6A --profile prototype-windows-5090-v1 --output-root docs/receipts/P6A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P7 --profile prototype-windows-5090-v1 --output-root docs/receipts/P7
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P7A --profile prototype-windows-5090-v1 --output-root docs/receipts/P7A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P8 --profile prototype-windows-5090-v1 --output-root docs/receipts/P8
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P9A --profile prototype-windows-5090-v1 --output-root docs/receipts/P9A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P9B --profile prototype-windows-5090-v1 --output-root docs/receipts/P9B
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P10 --profile prototype-windows-5090-v1 --output-root docs/receipts/P10
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P11 --profile prototype-windows-5090-v1 --output-root docs/receipts/P11
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P12 --profile prototype-windows-5090-v1 --output-root docs/receipts/P12
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P13 --profile prototype-windows-5090-v1 --output-root docs/receipts/P13
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P14 --profile prototype-windows-5090-v1 --output-root docs/receipts/P14
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P15 --profile prototype-windows-5090-v1 --output-root docs/receipts/P15
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P16 --profile prototype-windows-5090-v1 --output-root docs/receipts/P16
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P16A --profile prototype-windows-5090-v1 --output-root docs/receipts/P16A
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P17 --output-root docs/receipts/P17
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P18 --output-root docs/receipts/P18
cargo run --locked -p xtask --bin xtask -- verify-phase --phase P19 --output-root docs/receipts/P19
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

- [ ] P0A complete

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

## Phase 1A — Qualify the Host Rust and C/C++ Toolchains

- [ ] P1A complete

Dependencies: P0A.

Historical note: the checked v1 P1A receipt remains valid only for its recorded
Windows/MSVC host. It does not bind P0A, `prototype-windows-5090-v1`, the Ryzen 9
9950X3D identity, or the new schema, so it is immutable evidence only and cannot satisfy
this phase.

Prompt:

> Implement `xtask verify-env --mode host --profile prototype-windows-5090-v1`. From an
> ordinary native Windows shell, require AMD64 Windows, the exact AMD Ryzen 9 9950X3D
> processor identity, Rust 1.96 or newer with host `x86_64-pc-windows-msvc`, a complete VS
> 2022 x64 MSVC C/C++ toolchain, and a selected Windows SDK/UCRT. Record normalized CPU
> vendor/model, topology and instruction-set facts needed by later profiling plus exact
> identities and hashes for Rust, the C compiler, C++ compiler, linker, archiver, target/ABI,
> runtime, and SDK. Do not install tools or mutate persistent state. Linux, macOS,
> GCC/Clang, and AppleClang adapters are reserved by the portable interface but return
> `DEFERRED_POST_P16` and are not attempted in this phase.
> Qualify CPU isolation explicitly: record the selected processor group, logical-processor
> topology, affinity mask, power/clock policy, and an idle/background-load baseline; reject
> an unapproved competing compute workload or affinity/topology drift. Ordinary OS activity
> is recorded and bounded rather than mislabeled as an accelerator or compiler failure.
>
> Use a create-new `P1A-prototype-v2` run and an owned temporary directory. From absent fresh target
> directories, compile, link, and execute minimal Rust↔C and Rust↔C++ ABI probes shaped
> only as the permitted future accelerator/native-ML bridge boundary, plus the locked CPU
> graph, then run the exact pre-P3 format, Clippy, and CPU-test gate offline.
> Reject compiler wrappers, build-affecting Cargo configuration, Python execution or
> linkage, every accelerator SDK/tool/feature/artifact/library, input mutation, redaction
> leaks, and incomplete cleanup. Platform adapters may differ internally, but output
> schemas, exit categories, containment, and publication semantics remain portable contracts.
>
> A PASS run publishes a new automatic hash-linked acceptance and atomically advances only
> the `P1A-prototype-v2` pointer. It never edits this checklist or commits. P1B must regression-run
> this same host qualification and bind its complete selected chain.

VERIFY:

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- verify-env --mode host --profile prototype-windows-5090-v1 --output-root docs/receipts/P1A-prototype-v2
```

PASS:

- A clean shell emits a closed, machine-readable manifest binding
  `prototype-windows-5090-v1` to the exact Windows/AMD64/Ryzen 9 9950X3D identity, target,
  Rust, C, C++, linker, runtime, and Windows SDK tuple.
- Fresh-target locked Rust, C ABI, C++ ABI, and CPU probes execute without discovering or
  linking CUDA, ROCm/HIP, Metal/MPS, a native ML backend, or Python.
- A nonmatching CPU, OS, target, or toolchain fails actionably; deferred hosts produce the
  stable capability error without being probed or treated as a PASS.
- PASS/FAIL seals, transcript hashes, schema bundle, acceptance chain, pointer, input
  stability, redaction, containment, and cleanup all validate before exit `0`.
- The checkbox remains open until a human reviews the selected machine qualification.

STOP/loop: missing host tools, an invalid P0A dependency, or a failed evidence chain blocks
P1B directly and P3 onward. Suggested commit:
`build: qualify prototype Windows host toolchains`.

## Phase 1B — Qualify the Prototype RTX 5090/CUDA Environment

- [ ] P1B complete

Dependencies: P1A.

Historical note: the checked v2 P1B receipt remains valid only for its recorded
Windows/CUDA/SM120/RTX 5090 tuple. It does not bind P0A,
`prototype-windows-5090-v1`, the Ryzen 9 9950X3D host, the derived model-allocation gate,
or the new schema, so it is immutable evidence only and cannot satisfy this phase.

Prompt:

> Extend `xtask verify-env` with `--mode accelerator --profile
> prototype-windows-5090-v1 --provider cuda`. Bind a fresh P1A prototype qualification,
> discover the installed SM120-capable CUDA Toolkit without using a shell script, and
> require exactly one selected runtime-visible `NVIDIA GeForce RTX 5090` at compute
> capability 12.0. If multiple matching devices exist, require an explicit stable device
> identity rather than discovery order. If using CUDA 12.x, require at least 12.8; a newer
> toolkit passes only when its compiler accepts the exact targets and its runtime/driver
> compatibility probe succeeds. Require the CUDA driver/runtime, cuBLAS, and cuBLASLt and
> record every exact compiler, SDK, runtime, driver, native-library, device, architecture,
> feature, stable-identity, and dedicated-VRAM fact. Never install an SDK, copy/rename
> libraries, or persistently mutate environment or OS configuration. ROCm/HIP and Metal
> selections return `DEFERRED_POST_P16` before SDK discovery and cannot satisfy or block P1B.
>
> Derive `minimum_accelerator_bytes` from the versioned, hash-bound canonical training
> memory formula approved in P0A. For `P = 135,285,504` parameters, use
> `bytes_per_parameter = 20`: `2P` BF16 parameters + `2P` BF16 gradients + `4P` FP32 master
> weights + `4P` first moments + `4P` second moments + `4P` (25% margin). Thus raw persistent
> bytes are `20P = 2,705,710,080`; align upward to `Q = 268,435,456` bytes to obtain
> `minimum_accelerator_bytes = 2,952,790,016`. This P1B allocation excludes activations,
> temporary workspaces, and backend allocator reserve; P10 and P14 must separately prove
> that the actual full forward/backward/training configuration fits. Require a fresh-process
> allocation of at least that amount
> of dedicated device VRAM on the selected RTX 5090, explicit synchronization, sentinel
> round-trip, release, and return to the recorded baseline. Reserved unified-memory fields
> remain in the portable schema for P18, but are not implemented or qualified here;
> advertised total VRAM alone is not evidence that the allocation succeeds.
>
> Compile and inspect a CUDA native-plus-PTX probe using `compute_120` with
> `sm_120,compute_120` code plus a PTX-only fallback artifact. Prove canonical SM120 SASS
> records rather than trusting metadata labels, prove compute_120 PTX, and launch both
> artifacts on the selected device through the required CUDA runtime/math libraries. Native
> C/C++ is allowed only inside this audited accelerator boundary. Bind a fresh
> P1A-prototype-v2 regression,
> verifier hash, schema bundle, source identity, and complete P0A/P0 chain.

VERIFY:

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- verify-env --mode host --profile prototype-windows-5090-v1 --output-root docs/receipts/P1A-prototype-v2
cargo run --locked -p xtask --bin xtask -- verify-env --mode accelerator --profile prototype-windows-5090-v1 --provider cuda --output-root docs/receipts/P1B-prototype-v2
```

PASS:

- A fresh selected P1A-prototype-v2 regression pins the same verifier/schema bundle and P1B-prototype-v2
  records and validates the complete dependency chain.
- The portable-schema manifest binds the exact profile, Windows/Ryzen host,
  SDK/runtime/driver, CUDA native libraries, RTX 5090 stable identity, SM120 feature set,
  and dedicated-memory model without leaking machine-local absolute paths.
- Inspection proves canonical SM120 SASS plus compute_120 PTX in the mixed CUDA artifact and
  compute_120 PTX without SASS in the fallback artifact. Both probes allocate at least
  `minimum_accelerator_bytes = 2,952,790,016`, launch, synchronize, copy, validate, release
  all resources, and restore the memory baseline on the selected device.
- Missing, ambiguous, or incompatible CUDA components fail actionably; deferred providers
  return the frozen capability error without being probed.
- PASS and FAIL runs are sealed; FAIL never moves the pointer. PASS publishes and fully
  revalidates only the new automatic acceptance/pointer chain before exit `0`.
- The checkbox remains open until the owner reviews the selected machine qualification.

STOP/loop: P2 and P10–P16 remain blocked, but CPU/data work may continue. Suggested commit:
`build: qualify prototype RTX 5090 CUDA environment`.

## Phase 2 — Select the Accelerator Backend by Measurement

- [ ] P2 complete

Dependencies: P1B.

Prompt:

> Create isolated, disposable spikes for every viable Rust CUDA backend candidate on the
> exact `prototype-windows-5090-v1` P1B-qualified tuple. Compare at least two when two
> satisfy the Python-free Windows/MSVC/CUDA dependency and support preflight. A single
> candidate may proceed only after the
> sealed receipt proves that all named alternatives are unsupported on the tuple or fail a
> hard gate; documentation or feature lists alone are insufficient. Before fetching,
> building, or measuring any candidate, seal a profile-specific candidate inventory
> with the discovery method, every considered backend/native path, exact direct versions,
> support status, and exclusion criteria. Changing that inventory invalidates the run and
> prevents post-measurement cherry-picking. Candidates may be
> Rust-native or use audited
> C/C++ FFI to pinned native kernels or standard ML libraries, including Candle, Burn with
> a supported native backend such as CubeCL or LibTorch, direct LibTorch bindings, or a
> narrowly scoped custom CUDA binding. The selected crate-facing backend interface must
> remain accelerator-neutral, but ROCm and Metal implementations and candidate measurements
> are outside this phase and deferred to P18. Python bindings, Python build steps, Python wheels, and
> Python runtimes are forbidden. Pin every direct and transitive candidate/native dependency
> in experiment-only manifests and lockfiles; do not modify the root package or lockfile.
>
> Generate identical deterministic BF16 fixtures outside every candidate and evaluate the
> same graph against a backend-independent scalar CPU oracle. The oracle must decode the
> exact BF16 fixture bits, execute the frozen FP32 accumulation/reduction order without
> contraction or reassociation, and emit canonical IEEE-754 output, loss, and gradient
> bytes. Require BF16 parameters/activations with explicitly verified FP32 accumulation for
> GEMM, loss, reductions, and gradients. Both input-gradient artifacts must be byte-for-byte
> identical to the canonical CPU-reference gradient artifacts; no gradient tolerance,
> candidate-specific comparison, nondeterministic reduction, or cross-run variation is
> permitted. Forward/loss diagnostics may additionally use one backend-independent,
> predeclared elementwise/norm table, but cannot waive exact gradient equality. Changing
> fixtures, graph, operation order, metrics, or thresholds invalidates the whole run.
>
> Measure allocation/round-trip, representative GEMM, forward loss, automatic
> differentiation/backward, explicit synchronization, post-warmup p50/p95 latency, and
> peak accelerator memory. Record dedicated RTX 5090 VRAM; unified memory remains a deferred
> schema variant. Audit every loaded
> native library and require it to resolve to the P1B-qualified SDK/runtime or sealed native
> dependency bundle. CPU-only builds must discover and link no accelerator SDK, native ML
> backend, or Python component. Deterministic execution is a hard gate. Reject a candidate
> that cannot exactly export and restore every parameter, layout identity, deterministic-mode
> setting, RNG, and backend-visible state required for P12 byte-identical resume.
>
> Write `docs/adr/0001-ml-backend.md` with immutable result paths and hashes, constraints,
> the provisional selection, rejected candidates, and fallback boundary; raw JSON remains
> in the sealed receipt. P10 supplies model-level parity and state round-trip evidence; P12
> supplies synchronized full-step and interrupted/resumed evidence before the ADR is final.

VERIFY:

```text
cargo test --locked -p xtask
cargo run --locked -p xtask --bin xtask -- qualify-backend --profile prototype-windows-5090-v1 --output-root docs/receipts/P2-prototype-v2
```

PASS:

- At least one backend launches on the exact P1B-qualified device architecture,
  synchronizes, uses verified BF16/FP32 accumulation, and produces both gradient artifacts
  byte-for-byte equal to the canonical CPU reference.
- Selection uses measurements from the qualified tuple, not feature lists or results from
  another OS, SDK, or accelerator.
- A multi-candidate tuple records a fair measured comparison; a single-candidate tuple
  records sealed, reproducible hard-gate failures or unsupported-status evidence for every
  named alternative.
- Native-library provenance, CPU/accelerator/Python isolation, deterministic replay, exact
  state export/import, cleanup, seals, acceptance chaining, and pointer publication validate.
- The qualifier publishes an automatic passing acceptance with `required_approvals: []`;
  the checkbox stays open until owner review, and only that reviewed commit unblocks P10.
- The ADR defers full model parity to P10 and exact full-step resume to P12 and makes no
  production-throughput, production-memory, or final-SLA claim.

STOP/loop: if no framework passes, evaluate a bounded diagnostic binding to the selected
CUDA SDK's native primitives, such as cuBLASLt. A diagnostic
primitive cannot satisfy automatic differentiation or exact-resume gates without an
owner-approved architecture revision. Suggested commit:
`bench: select measured accelerator backend`.

## Phase 3 — Clean In-Place Scaffold and Cutover

- [ ] P3 complete

Dependencies: P0A, P1A. P2 may proceed in parallel and is required by P10.

Prompt:

> Recreate the implementation in this repository without copying the reference source.
> Keep one installed product Cargo package and one installed `python-slm` executable;
> the developer-only `xtask` workspace package created by P0A is never installed. Expose
> independently restartable `plan`, `curate`, `train-tokenizer`, `tokenize`, `inspect`,
> `bench`, and `train` subcommands through internal config, data, tokenizer, storage,
> model, backend, and train modules. The
> contract's historical phrase "independently restartable binaries" means these
> independently invocable subcommands, never multiple installed executables.
> Implement `CLI-001`, `CONFIG-001`, and `ERROR-001`:
> explicit versioned JSON production configurations, unknown-field rejection, no hidden
> production defaults, one terminal success JSON object, typed JSONL handled errors, and
> fixed exit categories 0 through 5. Define one accelerator-neutral Rust interface and
> stable `cuda`, `rocm`, and `metal` provider identities, but implement only the mutually
> isolated `cuda` provider feature before P16A. A `rocm` or `metal` selection must return
> `DEFERRED_POST_P16` before native discovery; it is not a compile, test, or PASS gate and
> must not be represented by a success stub. Feature-gate every native
> probe, kernel, and ML library so the default CPU/data build discovers and links no
> accelerator SDK or native ML backend. Define the backend interface without selecting or
> importing a framework; P10 consumes P2's ADR.
> Replace old code in small, reviewable diffs; preserve research documents and baseline
> receipts. Extend stable `xtask quality-gate` and rejected-by-default `xtask verify-phase`
> commands that record exact child argv/exit status. Freeze their portable interface
> behavior, implement and qualify the Windows adapter, and defer the Linux/macOS adapters
> to P17. Do not leave
> executable `todo!()`, `unimplemented!()`, or silent stub success paths.

PASS:

- The canonical CPU gates pass with `CARGO_TARGET_DIR` set to a newly created empty
  temporary directory; never delete an existing target directory. CPU builds do not
  discover or link CUDA, ROCm/HIP, Metal/MPS, a native ML backend, or Python.
- If P2 is complete, its selected-backend compile gate also passes; otherwise that gate
  remains a recorded P10 prerequisite. `--help` exposes all seven subcommands, handled
  success/error streams and exit categories pass fixtures, and unfinished stages fail
  closed with explicit status.
- The canonical quality-gate commands are documented for all later phases.

STOP/loop: unexplained reference-code carryover, shell-specific orchestration, a deferred
provider reporting success, or a CPU
build requiring any accelerator/native-ML dependency fails P3.
Suggested commit: `build: establish clean Rust pipeline scaffold`.

## Phase 4 — Source Access, Governance, and Provenance

- [ ] P4 complete

Dependencies: P3.

Prompt:

> Implement and qualify this phase on `prototype-windows-5090-v1`. Keep `DocumentSource`,
> acquisition manifests, cache identities, and reason codes OS-neutral; implement only the
> Windows process/filesystem/network adapter before P16A, with Linux/macOS adapters deferred
> to P17. Define a bounded `DocumentSource` contract carrying the exact `SOURCE-002` stable
> `source_id` and always-present `repository_group_id`, adapter namespace, provider record
> identity, source snapshot, original encoding, license/provenance, removal-list version,
> raw hash, and content bytes. Provider IDs must be unique in their contract namespace;
> use the frozen conservative fallback grouping when no repository identity exists.
> Implement separate adapters for genuinely content-bearing Parquet and
> authorized Stack-v2 metadata plus Software Heritage content retrieval. Add HTTPS,
> checksums, size limits, timeout, retry/backoff, bounded concurrency, resumable cache,
> and environment-only credentials. Build deterministic local HTTP/Parquet/SWH fixtures;
> normal tests must never call the public Internet. Bound and revalidate redirects, never
> forward credentials across origins, reject local/link-local targets outside explicit
> fixture mode, enforce approved HTTPS hosts/destinations, cap compressed and decompressed
> bytes, and make cache keys/path joins traversal-safe. Emit acquisition manifests and
> reason-coded failures. Acquire and hash every authoritative `GOV-003` removal manifest,
> preserve provider ordering/freshness metadata, and fail closed when a required authority
> is missing, unverifiable, or unorderable. Each adapter must map removal entries to the
> exact `SOURCE-002` identity namespace and reject matching documents before P5; an
> ambiguous or unresolvable mandatory removal entry fails closed rather than passing data.

PASS:

- Fixture tests cover resume, timeout, retry, checksum mismatch, oversize/decompression
  bomb, hostile redirect, cross-origin credential forwarding, absolute/`..` IDs,
  malformed metadata, encoding metadata, and credential redaction.
- Provenance survives a source round trip and removal snapshots are versioned.
- Fixtures prove source-ID determinism, non-unique-provider-ID rejection, fallback grouping,
  removal-manifest ordering/hash verification, exact membership rejection, ambiguous-entry
  failure, and stale/superseding-snapshot behavior.

STOP/loop: fixture-complete adapters may pass P4 without credentials. The authorized
live-source gate is deferred to P6A and cannot be self-approved. Use an explicitly
approved alternate source, never a hidden substitution. Suggested commit:
`feat(data): add governed document source adapters`.

## Phase 5 — Decode, Scrub, CST, and Quality Filters

- [ ] P5 complete

Dependencies: P4.

Prompt:

> Build the pinned generated Tree-sitter C parser/runtime with P1A's qualified MSVC
> toolchain behind the narrow Rust parser ABI; GCC/Clang builds are deferred to P17 and are
> not a P5 gate. Implement `SOURCE-002/003` exactly: strict UTF-8/ASCII Python 3, recorded leading-BOM
> removal only, fixed canonical size `100..=1,000,000`, pinned
> `tree-sitter-python 0.25.0`, parser bounds, `root.has_error()` rejection, exact
> Tree-sitter comment-node byte accounting, and independent generated-marker scans over
> each comment intersection with `[0,8192)`. Equality at 50% comment bytes passes;
> strictly greater fails; docstrings are not comments. The generated C parser/runtime is
> the sole data-path native-code exception and may expose only the pinned CST boundary.
> Before detector implementation,
> obtain named human review of an ADR that freezes and hashes `sensitive-rules-v1`, with
> exact patterns, validators, entropy rules, reserved domains, precedence, and quarantine
> behavior. Apply `GOV-002`: reject confirmed sensitive documents, quarantine uncertain
> cases, preserve restricted raw evidence, never expose values in logs/receipts, and never
> rewrite tokenizer-visible source in v1. Implement `GOV-001` with an SPDX-expression
> parser—not substring matching—and the exact `permissive-v1` allowlist. An OR expression
> passes only when a complete branch passes; every term in an AND branch must pass. Reject
> missing, unknown, conflicting, copyleft, exception-bearing, `LicenseRef`, malformed, and
> otherwise unapproved expressions while retaining the original expression and provenance.
> Define stable all-reasons ordering or explicit first-failure precedence.

PASS:

- Boundary/property fixtures cover encoding cookies/BOM conflicts, 99/100,
  1,000,000/1,000,001 bytes, exactly/over 50%, malformed trees, docstrings, marker
  location/case, time budgets, PII, and secrets.
- The pinned Tree-sitter parser reproduces every P0A-sealed compatibility outcome and the
  receipt binds its grammar, generated C/runtime source, build flags, fixture, and output
  hashes without invoking Python.
- Repeated runs produce identical accepted IDs, bytes, reasons, and hashes.
- CPU memory remains within the configured bound under adversarial batches.
- No PII/secret fixture value appears in logs, errors, or receipts.
- License fixtures cover every allowlisted identifier, nested parentheses, OR/AND,
  malformed/unknown/missing expressions, `LicenseRef`, exceptions, conflicts, and
  representative copyleft terms; exact `GOV-001` outcomes and reason codes pass.

STOP/loop: missing human review of `sensitive-rules-v1`, ambiguous encoding, or ambiguous
sensitive-data behavior blocks P5 rather than becoming an implicit heuristic. Suggested
commit: `feat(data): add deterministic source policy`.

## Phase 6 — Scalable Exact/Near Deduplication

- [ ] P6 complete

Dependencies: P5.

Prompt:

> Qualify semantic determinism and resource bounds on the Windows/Ryzen prototype and record
> its exact thread, memory, and storage context without making those performance facts part
> of the portable artifact contract. Before implementing, benchmarking, or tuning LSH,
> materialize the exact 10,000-pair
> `dedup-threshold-v1` generator, strata, seed, exhaustive truth, and manifest required by
> `DEDUP-003`; obtain human review and seal its source/artifact hashes. Then implement
> exact curated-hash dedup, `DEDUP-001` lexical traversal/encoding over the pinned
> Tree-sitter CST and the same transitive clusters, and `DEDUP-002`'s 256
> fixed affine MinHash components, 32-by-8 LSH keys, short-document
> shingle, and exact candidate Jaccard. Reject strictly above 0.85; equality passes.
> Partition or persist production indexes; never perform global O(N²) comparison or keep
> every document/shingle set in RAM. Duplicate clusters retain every provenance/license
> record while the deterministic representative supplies training bytes. Also implement
> and fixture-qualify `DECONTAM-001` and `SPLIT-001`: import the pinned benchmark registry
> without Python, apply exact/Jaccard/protected-span exclusions, then assign repository/
> duplicate connected components deterministically to 98/1/1 splits.

PASS:

- Fixtures built from rational set intersections clearly below, exactly at, just above,
  and well above 0.85 behave as specified.
- Results and representative choices are invariant across thread counts and input order.
- The sealed suite reports end-to-end recall at least `0.995`, final precision exactly
  `1.0`, and candidate amplification; a 100K-document benchmark proves bounded memory and
  no all-pairs path.
- The receipt names the human reviewer and records sealed generator/source, exhaustive
  truth, pair-manifest, and suite hashes created before LSH work began.
- Decontamination and split fixtures reproduce fixture import/extraction hashes, protected
  matches, component isolation, and bucket boundaries without invoking Python. Actual
  EvalPlus asset/decoded hashes remain a P6A gate.

STOP/loop: missing human review/sealed suite identities, or a recall/precision failure,
keeps P6 open. Exact checks do not justify claiming removal of pairs LSH never retrieved.
Record measured recall. Suggested commit: `feat(data): add deterministic LSH dedup`.

## Phase 6A — Materialize Governed Documents and Splits

- [ ] P6A complete

Dependencies: P4, P5, P6.

Prompt:

> Run this materialization only on the qualified Windows/Ryzen prototype and bind its host,
> filesystem, storage, thread, and resource identities; the governed artifact formats and
> hash chains remain portable. Without changing code, resolve the destination to an explicit
> ignored path, verify free
> space/quota, and bound caches before bulk work; never recursively clean an output root or
> commit raw/generated artifacts. Run synthetic, authorized 1,000-document, larger bounded,
> then production snapshots through the P4-P6 qualified engines. Pin EvalPlus v0.3.1,
> HumanEval+ asset v0.1.10, and the real MBPP+ asset v0.2.0; verify imported and decoded
> hashes before decontamination. Persist immutable governed-document, duplicate-cluster,
> split, benchmark-registry, license/provenance, and removal manifests. Record counts,
> rejection reasons, dedup metrics, resource peaks, and policy versions. Obtain named human
> governance approval; an agent cannot self-approve source rights or sensitive-data policy.

PASS:

- Authorized live acquisition succeeds and every stage verifies from a fresh process.
- Train/validation/test splits are repository/duplicate-cluster isolated and repeat from
  the recorded algorithm, seed, and registry hash.
- Named governance approval covers the immutable snapshot, and the training split has
  enough approved bytes for the P0 tokenizer budget.
- Every EvalPlus asset/import hash and `GOV-003` removal authority verifies from a fresh
  process, and the removal recheck is current for materialization.

STOP/loop: missing access, approval, provenance, removal freshness, scrub policy, disk,
or approved bytes blocks the production tokenizer. Suggested commit:
`docs: record governed curated corpus`.

## Phase 7 — Tokenizer Engine and Fixture Qualification

- [ ] P7 complete

Dependencies: P6.

Prompt:

> Qualify the Rust tokenizer engine on the prototype host while keeping the tokenizer and
> sample manifests canonical and OS-neutral. Implement exact `TOKSAMPLE-001` selection over
> whole deduplicated, decontaminated train
> documents: domain-separated SHA-256 rank, stable order, 10,000,000-byte repository-group
> cap, 2,000,000,000-byte global cap, and skip-non-fitting-then-continue behavior. Implement
> Rust-native byte-level BPE training with no case
> folding, Unicode normalization, or whitespace stripping; explicitly seed the complete
> 256-byte alphabet, minimum merge frequency two, and source encoding that bypasses
> special-token matching; use fixed special IDs and EOS policy; and emit versioned artifact
> metadata. Qualify the engine on a sufficiently varied immutable synthetic training
> split. Add byte-roundtrip, repeated-build hash, whitespace/indentation, arbitrary-byte
> policy, unknown-token, max-ID, and pathological repeated-pair overflow tests. Do not
> require or create production data.

PASS:

- Vocab size is 32,000 contiguous IDs and maximum ID is exactly 31,999; all special IDs
  match the contract.
- Two clean fixture runs over the same ordered manifest produce byte-identical artifacts.
- Supported source bytes round-trip exactly with zero unknown IDs; unsupported input fails
  explicitly before tokenization.

STOP/loop: nondeterministic trainer behavior, pair-count overflow, or byte-roundtrip
failure stays in P7. Suggested commit: `feat(tokenizer): add deterministic 32k BPE`.

## Phase 7A — Train the Production Tokenizer

- [ ] P7A complete

Dependencies: P6A, P7.

Prompt:

> Run both clean production builds on the prototype host and record its identity; do not
> make host scheduling or filesystem enumeration part of tokenizer ordering. Without
> changing code, select only from the immutable governed training split using
> exact `TOKSAMPLE-001`: whole documents in domain-separated SHA-256 rank order, at most
> 10,000,000 decimal bytes per repository group and 2,000,000,000 globally, skipping a
> non-fitting document and continuing without splitting or duplication. The qualified
> sample must contain `1,999,000,000..=2,000,000,000` bytes. Train on separate documents
> in rank order with no inserted specials. Train twice from clean outputs, verify both
> complete hash chains, and publish the production tokenizer/sample receipt. Validation,
> test, and benchmark-decontamination matches are ineligible.

PASS:

- Vocab is exactly 32,000 contiguous IDs `0..31,999`, with canonical special IDs.
- Two clean runs produce byte-identical tokenizer and metadata artifacts.
- Unknown-token rate is exactly zero on supported curated source; unsupported bytes fail
  before tokenization. The receipt records mean bytes/token, p50/p95 tokens/file, sample
  byte count/hash, configuration/seed, and tokenizer hash.

STOP/loop: insufficient approved bytes fail the production tokenizer gate; record the
actual budget and return to P6A. Suggested commit:
`docs: record production tokenizer qualification`.

## Phase 8 — Versioned Token Corpus and Bulk Loader

- [ ] P8 complete

Dependencies: P7.

Prompt:

> Implement create-new raw little-endian `u16` shards, a versioned JSON manifest,
> document/split indexes, and sequence indexes. Record tokenizer/source/config hashes,
> token counts, shard hashes, byte order, special IDs, and boundary policy. Validate IDs
> before narrowing. Create temporary shards/manifests on the destination filesystem, sync,
> and atomically finalize; never overwrite, recursively clean an output root, or treat
> partial output as complete. Resume completed shards only after hash verification. Reject
> absolute paths, parent traversal, drive-relative paths, alternate data streams, device/UNC
> namespace escapes, symlink/junction/reparse-point escapes, volume-mount substitution, and
> equivalent Windows path escapes in manifest paths. Freeze a portable immutable-reader and
> mutation-detection interface, but implement and qualify its Windows behavior before P16A:
> map immutable files read-only from held handles opened with write/delete sharing denied;
> bind stable volume/file identity, size/metadata checks, and full hash revalidation at
> defined read boundaries. P17 must implement equivalent POSIX descriptor/mount protections;
> inode identity alone is never sufficient. Concurrent
> in-place mutation must be detected and rejected. Bulk-gather contiguous spans and explicitly decode
> little-endian values. Represent each 2,048-target sample as a 2,049-ID logical view over
> global offsets; never duplicate stored anchors. Samples may cross physical shards and
> EOS/document transitions, but never split boundaries or corpus-end wrap. Implement the
> exact `SPAN-001` planner on synthetic manifests: 976,562 complete spans shuffled by the
> specified ChaCha12/rejection-sampled descending Fisher-Yates algorithm, followed by the
> unshuffled final 1,024-target partial span. Preserve token order within every span.

PASS:

- Round-trip, cross-shard, endianness, boundary, truncation, concurrent mutation,
  corruption, tokenizer-mismatch, wrong-count, absolute/parent/symlink path, crash between
  shard and manifest finalization, and out-of-range fixtures pass.
- Readers reject incomplete generations and enforce the backing-file immutability
  invariant documented by the architecture.
- Bulk-loader benchmark avoids per-token file lookup and records host throughput.
- Golden `SPAN-001` fixtures prove the exact seed operands/order, no duplicate or omitted
  complete span, the partial span last, deterministic replay, and corpus-end no-wrap.

STOP/loop: unsafe casts require documented alignment, lifetime, mutation, and endian
proof; explicit decoding is the default. Suggested commit:
`feat(storage): add immutable u16le corpus format`.

## Phase 9A — Materialize and Verify the Governed Token Corpus

- [ ] P9A complete

Dependencies: P6A, P7A, P8. May run in parallel with P9B.

Prompt:

> Materialize on `prototype-windows-5090-v1`, bind the exact Windows/Ryzen/storage/resource
> identities, and preserve the same portable manifests and hashes for later P17 replay.
> Consume the immutable P6A curated training and held-out splits; do not rerun acquisition,
> filtering, deduplication, or splitting. Resolve explicit ignored destinations, verify
> free space/quota plus safety margin, bound caches, and never recursively clean an output
> root or commit raw/token artifacts. Run a bounded sample, then order/pack complete train
> documents exactly as `ACCOUNT-001`. Stop after completing the first document whose EOS
> reaches at least 2,000,000,001 IDs; contract the first 2,000,000,001 IDs, retain only
> that document's remainder as stored unused tail, and count all later approved documents
> and bytes as unmaterialized. Preserve and verify the complete source→split→tokenizer→shard
> hash chain. Generate the production `SPAN-001` order manifest from the raw decision-ledger
> and corpus-manifest hashes using P8's qualified implementation; record its seed operands,
> ordered offsets, final partial-span descriptor, and artifact hash. Materialize and hash
> the exact 1,000,000-target `EVAL-001` validation index manifest. Record host resource
> peaks, tokenization wall time, every role-ledger counter, and held-out artifacts.

PASS:

- Every manifest/hash verifies from a fresh process; no partial artifact is accepted.
- The prefix has 2,000,000,001 IDs and exposes exactly 2,000,000,000 consumed real inputs
  and valid targets with zero boundary exclusions. Stored unused tail, unmaterialized
  documents/bytes, and the final 1,024 runtime PAD/masked positions reconcile exactly.
- Tokenized train/held-out artifacts trace to the approved P6A split and tokenizer hashes.
- The `SPAN-001` manifest contains every complete span exactly once in qualified order,
  keeps the 1,024-target partial span last, and verifies from its recorded seed operands.
- The deterministic validation-sample index contains exactly 1,000,000 targets and
  verifies from its recorded split-manifest hash.

STOP/loop: disk, hash-chain, boundary, or count failure blocks the production run and
returns to the owning phase. Suggested commit:
`docs: record governed token-corpus materialization`.

## Phase 9B — CPU/Reference Model Semantics

- [ ] P9B complete

Dependencies: P3. May run in parallel with P9A.

Prompt:

> Implement and seal the CPU oracle on the Windows/Ryzen prototype, but define it entirely
> by canonical artifact bytes and frozen arithmetic so P17/P18 can replay it without using
> host-specific math behavior. Implement a small-shape-capable, bias-free, pre-norm decoder reference: embeddings;
> 12Q/4KV causal GQA; RoPE base 10,000; FP32-accumulating RMSNorm epsilon `1e-5`;
> SwiGLU; residuals; final norm; and untied LM head. The canonical preset uses
> `d_ff=2432` and must equal 135,285,504 parameters; retain the named `d_ff=2048`
> 124,668,672 preset. Map query head `q` to KV head `q/3`; do not use concatenation that
> produces interleaved ordering. Use a backend-independent scalar/f64 oracle. Freeze
> tensor layout, adjacent-pair RoPE with reset only at packed-sample start, inclusive
> causal masking across EOS, and the stable `PARAM-001` names. Materialize and hash
> canonical initialized parameter artifacts for both presets using `INIT-001`'s exact
> parameter order, pinned `rand_chacha 0.10.0`/`rand_distr 0.6.0`, domain-separated seed, f32 normal
> sampling, and BF16 conversion. Implement an analytical per-component parameter counter
> independent of any backend registry and require exact agreement. Implement shifted
> next-token loss on tiny shapes. Alongside the f64 semantic oracle, implement a canonical
> scalar BF16/FP32 execution oracle with frozen operation, accumulation, reduction,
> contraction, and rounding order. For sealed small fixtures, materialize the canonical
> IEEE-754 forward, loss, and every parameter-gradient artifact and hash; P10 requires exact
> gradient-byte equality. Do not select an accelerator framework when P2 is incomplete.

PASS:

- Exact count, shape, causal-prefix invariance, RoPE scalar parity, GQA mapping,
  RMSNorm, SwiGLU, and finite-difference gradient checks on tiny shapes pass.
- The reference is deterministic and suitable as the oracle for optimized kernels.
- Both initialized artifacts reproduce byte-for-byte and their hashes are recorded.
- Canonical small-fixture BF16/FP32 forward/loss/gradient artifacts reproduce byte-for-byte,
  and the scalar reference forbids reassociation or contraction that changes their bits.
- Full production shapes are not used in ordinary CPU tests.

STOP/loop: a range such as “135M ±5%” is not an acceptable count gate. Suggested
commit: `feat(model): add exact Llama reference semantics`.

## Phase 10 — Prototype CUDA Model, Attention, and Fused Loss

- [ ] P10 complete

Dependencies: P2, P9B.

Prompt:

> On `prototype-windows-5090-v1`, port the validated model to the selected CUDA backend and
> load P9B's canonical initialized
> parameter artifact by its recorded hash; backend-native reinitialization is forbidden.
> Progress from unfused reference GQA to the framework's memory-efficient path; add an
> isolated custom operation only if profiling proves it necessary. The production path
> must support causal 12Q/4KV BF16
> forward and backward without materializing repeated K/V. Add chunked or fused LM-head
> cross-entropy so full `[B,L,V]` logits are not retained. Python, CuTeDSL, and
> Python-generated kernels are forbidden. FlashAttention or another standard native ML
> library may be used only when its exact CUDA/SM120 path, dependency closure,
> and build are Python-free and sealed.
>
> Custom native code may use CUDA C/C++ or a pinned standard ML library, but only behind a
> tiny `cuda`-feature-gated ABI. It propagates CUDA errors, validates tensor layout and
> stream compatibility, and retains ownership through asynchronous completion. Emit an
> exact SM120 image plus compute_120 PTX fallback when the P1B-qualified SDK supports it.
> HIP and Metal implementations remain `DEFERRED_POST_P16` for P18 and are not P10 gates.
>
> Require the frozen BF16 storage rules and verified FP32 accumulation for every sensitive
> operation. Before benchmarking, record one backend-independent forward/loss tolerance
> table and comparison metrics in the ADR and receipt; changing them invalidates and reruns
> P10. Every parameter-gradient artifact must match P9B's canonical scalar BF16/FP32
> reference byte-for-byte; gradient tolerances are forbidden. Demonstrate exact deterministic
> export/import of all parameter tensors,
> backend-visible state, parameter names, layouts, and deterministic-mode settings needed
> for P12 exact resume. Benchmark synchronized forward/backward at
> `B={2,4,8,16,32}`, `L=2048`, discard warmup, and emit JSON with error, p50/p95,
> workspace, and peak dedicated VRAM. Run sizes low-to-high
> in fresh child processes with timeouts;
> record OOM and skip larger sizes so one allocation cannot poison the suite. B=32 is a
> trial, not a PASS requirement. Update P2's ADR with model-level evidence and the
> unresolved P12 full-step gate.

PASS:

- Forward/loss meet the predeclared backend-independent BF16/FP32 gates, and every parameter
  gradient is byte-for-byte identical to P9B's canonical scalar BF16/FP32 artifact.
- The backend loads and round-trips the exact P9B initialization artifact/hash, stable
  parameter-name mapping, tensor layouts, and backend state without reinitialization.
- Causal and GQA semantics pass adversarial tests; optimized code does not repeat K/V.
- At least one full-layer and fused-loss path runs on the P1B-qualified device architecture
  without quadratic attention or full-logit retention.
- The provisional backend ADR is supported by model-level forward/backward evidence and
  explicitly remains open until P12.

STOP/loop: a fast forward-only kernel is not a training backend. Suggested commit:
`perf(model): add validated SM120 CUDA training path`.

## Phase 11 — Prototype Host Staging and CUDA Transfer

- [ ] P11 complete

Dependencies: P8, P10.

Prompt:

> First benchmark the bulk Windows host loader. Then qualify transfer for the discrete RTX
> 5090 dedicated-memory model. Compare pageable synchronous H2D, bounded page-locked
> synchronous H2D, and a reusable two- or three-slot page-locked asynchronous ring using
> CUDA streams/events. Pipeline mmap bulk gather/conversion, H2D, and compute without
> pinning or retaining the whole corpus. Separate gather, conversion, transfer,
> synchronization, and compute timings. Keep the simplest safe path whose end-to-end
> measurements win. ROCm and Metal/unified-memory transfer implementations remain
> `DEFERRED_POST_P16` for P18.

PASS:

- No buffer is reused before its completion event; allocation is bounded and stable.
- Data parity holds under long randomized stress, cancellation, and error paths.
- Benchmark evidence, not terminology, selects the production transfer path.
- Every registered/page-locked slot, CUDA stream, and event is released after success,
  cancellation, timeout, panic boundary, and CUDA error;
  repeated runs return host and accelerator memory use to baseline.

STOP/loop: mmap, `Bytes`, or ordinary vectors are never mislabeled as page-locked transfer
buffers. Suggested commit: `perf(loader): benchmark bounded CUDA staging`.

## Phase 12 — BF16 Trainer and Exact Resume

- [ ] P12 complete

Dependencies: P8, P10, P11.

Prompt:

> Initialize the trainer only from P9B's canonical parameter artifact and verified hash;
> backend-native reinitialization is forbidden. Implement the named canonical BF16
> training preset with explicit FP32-sensitive
> reductions/state and exact `OPT-001` AdamW equations, bias correction, epsilon placement,
> decay groups, global-L2 clipping, and master-weight cast. Preserve exactly 65,536 valid
> targets per full update, 30,517 full updates, one final 37,888-target update, and 30,518
> updates total. Implement `SCHED-001`: 1,000-update linear warmup to `2.5e-3`, then cosine
> decay so update 30,518 uses exactly `2.5e-4`. Distinguish stored IDs,
> consumed input IDs, valid predicted targets, padding/boundary exclusions, microsteps,
> optimizer steps, and zero overshoot. Normalize accumulated loss/gradients by the actual
> valid-predicted-target count; define the final partial update; if scaling, unscale before
> finite checks and clipping; clip only after all microsteps; then step AdamW and the
> scheduler and zero gradients once at the completed optimizer update. Save atomic
> generation checkpoints with
> model, master/moment state, scheduler, scaler if used, host/accelerator RNG, data
> order/cursor,
> counters, and all config/artifact hashes. Consume a verified `SPAN-001` manifest
> directly—P8's synthetic fixture in this phase and P9A's production artifact in later
> qualification—never regenerate or reorder it in the backend, and checkpoint its identity
> and exact next-span cursor. Version 1 forbids mid-update checkpoints.
> Implement `EVAL-001`'s fixed 1,000,000-target sample and cadence plus `CKPT-001`'s same
> post-update thresholds, completion checkpoint, atomic publication, and retention policy.
> Evaluation consumes immutable held-out spans, does not advance training cursor/RNG/
> scheduler, restores mode and state, records loss/perplexity, and includes its time in SLA
> projections. Do not run the full corpus. Before measurement, record full-step numerical
> and performance gates
> in the ADR. Measure synchronized representative full training steps, including optimizer
> work, and finalize the backend ADR only if numerical parity, p50/p95 latency, throughput,
> and peak dedicated accelerator memory meet those gates.

PASS:

- Scalar AdamW/schedule/accumulation references and boundary tests pass.
- Trainer initialization reproduces the P9B artifact hash before the first update.
- Tests cover updates 1, 1,000, 30,518; the 37,888-target final normalization; threshold
  evaluation/checkpoint coincidences; retention; and zero overshoot.
- A fixed tiny corpus overfits without non-finite values.
- On the exact qualified tuple, interrupted/resumed execution is byte-for-byte identical
  to uninterrupted execution for every subsequent parameter, master weight, optimizer
  moment, scheduler/scaler/RNG state, per-update gradient artifact, evaluation result,
  counter, cursor, and checkpoint. It resumes the next exact `SPAN-001` entry without
  repeat/skip and refuses mismatched span manifests, artifacts, environment, backend, or
  configuration. A backend without deterministic execution cannot pass P12.
- Checkpoints preserve byte-exact serialized tensors/state, counters, cursor, artifact
  identities, selected SDK/device/backend identity, and next-span position. Fresh-process
  reload and continued execution reproduce the uninterrupted bytes exactly.
- Held-out evaluation repeats from the same checkpoint/sample order without changing the
  subsequent training state.
- Crash-injection tests ignore incomplete/corrupt checkpoint generations and reload the
  last complete generation.
- The finalized backend ADR includes passing full-training-step evidence.

STOP/loop: weights-only checkpoints are not resumable. A backend full-step failure returns
to P2/P10 and reruns affected downstream gates. Suggested commit:
`feat(train): add resumable BF16 trainer`.

## Phase 13 — Synthetic End-to-End and CI Separation

- [ ] P13 complete

Dependencies: P4, P5, P6, P7, P8, P9B, P10, P11, P12. P9A is not required.

Prompt:

> On `prototype-windows-5090-v1`, generate an offline synthetic corpus containing valid,
> invalid, encoded, duplicate,
> near-threshold, generated, comment-heavy, PII/secret, tiny, and oversized documents.
> Exercise source, policy, dedup, tokenizer, storage, training, checkpoint, and resume from
> one reproducible xtask command. Create Windows-appropriate failing canary launchers for
> `python`, `python3`, versioned Python names, `pythonw`, `py`, `pip`, `pip3`, and versioned
> pip names first on `PATH`; audit every child process, native import table, dynamic module,
> build command, and absolute executable path. Any Python interpreter, library, package,
> generator, or embedded runtime fails the gate.
>
> Add ordinary Windows x86_64 CPU/data CI plus a separately provisioned
> `prototype-windows-5090-v1` CUDA job. The accelerator job runs environment qualification,
> backend/model parity, tiny training, byte-exact resume, and benchmark artifact generation
> only for the exact prototype tuple. Linux/macOS and non-CUDA jobs remain
> `DEFERRED_POST_P16` until P17/P18. Keep noisy performance thresholds out of ordinary
> correctness CI. Generate workflow files and xtask cases only. Do not
> register a self-hosted runner, change repository settings, or trigger external CI without
> explicit authorization. When no runner is provisioned, execute the same xtask accelerator
> qualification locally on the target host.

PASS:

- An authorized fresh clone of the exact commit, or a clean content-addressed export of
  the exact tree, passes locked fmt/check/Clippy/tests on Windows x86_64 without any
  accelerator/native-ML dependency.
- The local xtask accelerator qualifier—or an already authorized accelerator CI job—publishes
  parity JSON and a complete environment/reproducibility manifest for its exact tuple.
- Data-side artifacts (accepted IDs/reasons, tokenizer, indexes, and shards) repeat
  byte-for-byte. On the qualified prototype tuple, interrupted/resumed training and
  checkpoints reproduce the uninterrupted subsequent state and outputs byte-for-byte.
- No Python canary is hit and no Python executable, library, package, or embedded runtime
  is observed.

STOP/loop: Clippy is not evidence of accelerator correctness, memory safety, or
performance. Suggested commit: `ci: add prototype Windows and CUDA qualification gates`.

## Phase 14 — Autotune and Project the Fixed Prototype SLA

- [ ] P14 complete

Dependencies: P12, P13.

Prompt:

> In one sealed release build on the exact `prototype-windows-5090-v1` host, CUDA,
> SDK/runtime/driver, backend/native-library, clock, and power-state tuple, profile the bulk
> loader, transfers, kernels, full step, evaluation, checkpoint, and final-save paths.
> Record synchronized compute throughput, device-memory bandwidth, operational intensity,
> the resulting roofline classification, and compute/bandwidth efficiency, plus counters
> where available, compiler flags, warmup/sample rules, telemetry, and every identity as
> diagnostic evidence. Efficiency is diagnostic only: no theoretical or relative target,
> including 85% bandwidth efficiency, is an independent PASS gate or may replace the fixed
> SLA or admission threshold.
>
> Autotune microbatches 2, 4, 8, 16, and 32 at context 2,048. Adjust gradient accumulation
> independently to preserve exactly 65,536 valid predicted targets per full update. Run
> each OOM-prone candidate in a fresh contained child process with an explicit timeout and
> result artifact; record failure and stop increasing that family. Synchronize timings,
> discard compilation/JIT/autotune warmup from numerator and denominator, compute throughput
> from actual valid targets rather than nominal `B*L`, and select the fastest stable
> configuration below the dedicated-VRAM safety ceiling.
>
> After selection, run at least five independent fresh-process synchronized representative
> ordinary-step trials. Each includes bulk loading, H2D, synchronization, optimizer work,
> and ordinary stalls but excludes only the separately measured event intervals below. Set
> `R_qual` to the minimum exact valid-targets/elapsed-seconds ratio among those trials; do not
> average away a slow trial or round the rate before comparison.
>
> Measure each non-steady-state event class—the combined startup plus frozen-artifact
> verification/open plus initialization plus compilation/JIT/autotune interval, one configured
> held-out evaluation, one planned non-final checkpoint, and final durable save—in exactly
> five independent fresh processes under the frozen profile.
> Measure each event's incremental blocked-trainer duration beyond any ordinary step already
> counted by `R_qual`. For each class freeze the sample set and use its maximum observed
> synchronized duration; multiply recurring class maxima by their exact frozen P16 event
> counts and add every one-time maximum to derive the conservative overhead bound `O_bound`.
> No interval may be omitted from or counted in both `R_qual` and `O_bound`. Require
> `0 <= O_bound < admission_seconds` and define
> `N = 2_000_000_000`. Compute, in seconds and valid targets/second with exact rational
> arithmetic and declared final ceiling rules:
>
> `R_required = N / (admission_seconds - O_bound)`
>
> `projected_run_seconds = ceil(N / R_qual + O_bound)`
>
> Freeze `sla_seconds = 28_800`, `admission_seconds = 25_920`, and
> `actual_elapsed_limit_ns = 28_800_000_000_000`; derive
> `runtime_reserve_seconds = sla_seconds - admission_seconds = 2_880`. Promotion requires
> both `R_qual >= R_required` and
> `projected_run_seconds <= admission_seconds`; thus at least 2,880 seconds, exactly
> 10% of the fixed SLA, remain between the qualifying projection and P16's hard limit.
> Compare the measured rate to the exact rational `R_required` by cross multiplication so
> display rounding cannot create a PASS. Hash the profile, source/config/artifact identities,
> fixed thresholds, per-class samples/maxima/counts, `O_bound`, `R_required`, `R_qual`,
> projection, and rounding rules
> into one canonical `admission_hash`.
> Report loader, kernel-only, trainer, and whole-run-equivalent throughput; p50/p95;
> forward/backward/optimizer/H2D/evaluation/checkpoint/final-save time; stalls; and
> allocated/reserved/peak dedicated VRAM. The P14 receipt locks the selected configuration,
> overhead model, `O_bound`, `R_required`, `R_qual`, formula inputs/units/rounding, projection,
> `admission_hash`, and all three fixed SLA values.
> P14 changes configuration only; a code bottleneck returns to P10, P11, or P12 and reruns
> P13/P14.

PASS:

- A reproducible configuration is selected from complete machine-readable evidence bound
  to the exact prototype tuple.
- No unexplained non-finite values, OOM, hidden synchronization, or valid-target overcount
  exists.
- A synchronized short-run preflight reproduces `R_qual`; every overhead class has exactly
  five fresh-process samples and its frozen maximum/count reconciles into `O_bound` without
  omissions or double counting.
- `O_bound < 25_920`, `R_qual >= R_required`, and
  `ceil(2_000_000_000 / R_qual + O_bound) <= 25_920`; the immutable SLA is exactly `28_800`,
  the difference is exactly `2_880`, and all formula inputs and artifact hashes revalidate.

STOP/loop: a hardcoded batch, absolute VRAM goal, unsynchronized timing, omitted overhead,
or any attempt to derive, relax, or retune the fixed `28,800`/`25,920`-second gates fails
P14. Suggested commit: `perf(train): qualify prototype configuration against fixed SLA`.

## Phase 15 — Frozen-Code Qualification Ladder

- [ ] P15 complete

Dependencies: P9A, P14.

Prompt:

> Freeze code, dependencies, corpus, tokenizer, model, training configuration, approved
> full-contract hash, decision-ledger hash, P9B initialization-artifact hash, P14 calibration
> identity, `O_bound`, `R_required`, `R_qual`, `admission_hash`, `sla_seconds = 28_800`,
> `admission_seconds = 25_920`, `runtime_reserve_seconds = 2_880`, and
> `actual_elapsed_limit_ns = 28_800_000_000_000`. Before P15 begins, also freeze and seal
> the exact P16A
> quality-evaluation specification: held-out index; initialization checkpoint and metrics;
> Rust-computed training-only unigram baseline algorithm/configuration and artifact; prompt
> and sample pack; decoding parameters, seeds, and output-normalization/comparison rules.
> Before the first P15 trial, require a create-new named owner approval that binds the exact
> quality-pack, prompt/sample, decoding, output-schema, and rubric hashes, with identity,
> signature/reference, and UTC timestamp. This pre-P15 artifact approval is distinct from
> the later decision on the measured P16A result.
> None may be chosen or changed after a P15 run or after seeing P16 outputs. Before running,
> verify all immutable hashes, absence of unapproved foreign accelerator compute, the exact
> `prototype-windows-5090-v1` identity, and target-host power.
> Record unavoidable OS/display activity as a frozen baseline and reject a run when
> contention, timing drift, or telemetry exceeds its predeclared interference limits. Verify
> sleep/update state, sufficient disk for all checkpoint generations plus safety margin,
> and that every `GOV-003` authority was rechecked within the preceding 24 hours with no
> superseding mandatory removal snapshot. Report blockers but do not
> mutate OS policy without explicit authorization. Observe the 1M and 100M target
> thresholds at the first completed optimizer boundary that crosses each threshold; do not
> invent exact partial updates. Run 30-minute and then 60-minute trials, ending each only at
> a completed optimizer boundary. At each rung run a
> restart-correctness trial and a separate uninterrupted performance trial; only the
> uninterrupted trial supplies throughput, while measured checkpoint/restart overhead is
> added separately. Archive
> logs, JSON, hashes, host/accelerator telemetry, loss/gradient diagnostics, compute/memory
> efficiency diagnostics, and non-finite counts.
> Calculate the two-billion-token projection from measured whole-run-equivalent throughput
> including startup, JIT/autotune, loader stalls, synchronization, configured held-out
> evaluation, planned checkpoints, and final durable save. Do not edit code while
> measuring.

PASS:

- All rungs finish without correctness, resume, OOM, or thermal-stability failure.
- Every rung preserves completed-boundary checkpoint semantics and the frozen contract,
  ledger, initialization, corpus, tokenizer, code, backend/native libraries, exact device,
  P14 calibration, and configuration identities.
- The uninterrupted 30-minute trial and the uninterrupted 60-minute trial each reproduce
  the P14-qualified configuration and rate within its predeclared measurement bounds;
  restart-correctness trials never supply performance numerator data.
- Each uninterrupted trial independently computes its own `R_rung` and proves
  `ceil(2_000_000_000 / R_rung + O_bound) <= 25_920` using the identical frozen `O_bound`.
  Both projections are no greater than `25,920` seconds, leaving exactly the fixed
  2,880-second admission-to-SLA
  reserve; all per-event samples/maxima/counts, `O_bound`, `R_required`, `R_qual`, and
  `admission_hash` reproduce exactly, and
  `ceil(2_000_000_000 / R_qual + O_bound) <= admission_seconds` holds.

STOP/loop: on failure, preserve evidence and return to the phase that owns the cause—P10
model/kernel, P11 loader, P12 trainer/checkpoint, or P14 configuration. Rerun that phase
and every affected downstream gate, then freeze a new qualification identity. Never patch
only the benchmark or launch 2B. Any host, SDK/runtime/driver, device, backend, native
library, or power-state identity drift returns to P1B, P2/P10, or P14 as appropriate.
Suggested commit: `test: record frozen prototype qualification`.

## Phase 16 — Final Two-Billion-Token Run and Release Receipt

- [ ] P16 complete

Dependencies: P15.

Prompt:

> Recheck every `GOV-003` authority within 24 hours before launch and stop on a superseding
> mandatory removal. Run the frozen training command against the immutable governed
> corpus only on `prototype-windows-5090-v1`. Revalidate the complete P14/P15 projection,
> fixed-SLA, and admission chain plus the exact host OS,
> architecture, Rust/C/C++ toolchains, accelerator SDK/runtime/driver, selected device and
> architecture, memory model, backend, and native-library identities. After corpus
> preparation, start the SLA clock immediately before the trainer verifies and opens the
> frozen artifacts and stop it only after the final checkpoint is durable.
> Finalize the receipt
> immediately using that captured elapsed time. Process exactly the contracted number of
> valid predicted training targets, handling final
> alignment and masks explicitly. Monitor
> without modifying code. During training, validate durable checkpoint manifests/hashes
> without interrupting the run. After completion, load a copied mid-run checkpoint and
> the final checkpoint in fresh processes. Do not deliberately interrupt the timed final
> run; recovery, host suspend/system sleep, and resumed-execution downtime all remain inside
> the same suspend-inclusive continuous SLA clock. The clock is monotonic across suspend and
> cannot be reconstructed from active-process CPU time.
> Publish the complete `PROV-001` schema: approved full-contract and decision-ledger hashes;
> frozen source tree; Git commit/dirty status; `Cargo.lock`; source/removal approval
> references and hashes; host OS/architecture; Rust, C, and C++ toolchains; selected
> accelerator provider, SDK/runtime/driver, device vendor/model/architecture and memory
> model; backend/kernel/native-library build and
> configuration identities; tokenizer/source/corpus/split/configuration hashes;
> telemetry/log/benchmark hashes; every role-ledger counter; evaluation/checkpoint counts;
> P14 calibration receipt/hash, formula inputs, overhead-event samples/maxima/counts,
> `O_bound`, `R_required`, `R_qual`, `admission_hash`,
> projected training seconds,
> fixed `admission_seconds = 25_920`, `runtime_reserve_seconds = 2_880`,
> `sla_seconds = 28_800`, and `actual_elapsed_limit_ns = 28_800_000_000_000`; achieved
> diagnostic efficiency; peak dedicated VRAM;
> loss/evaluation diagnostics, actual elapsed time, and final checkpoint hash.

PASS:

- The measured `actual_elapsed_ns` for the continuous durable run is no greater than the
  immutable `actual_elapsed_limit_ns = 28_800_000_000_000` (`sla_seconds = 28_800`).
- Exactly 2,000,000,000 valid predicted targets, zero overshoot, hashes,
  checkpoints, and fresh-process reload all verify.
- The receipt distinguishes measured facts from interpretation and records any deviation.
- The `PROV-001` role ledger records and reconciles 2,000,000,001 prefix IDs, stored
  unused tail, unmaterialized documents/bytes, 2,000,000,000 consumed real inputs, 1,024
  runtime PAD inputs, 2,000,000,000 valid targets, 1,024 masked padding targets, zero
  boundary exclusions, 30,518 optimizer updates, and all evaluation/checkpoint counters.
- P16 establishes technical completion of the frozen prototype run; it does not by itself
  claim useful language-model quality or unlock portability/scale-up work. P16A owns that
  acceptance.

STOP/loop: do not replace P16A's pre-P15-frozen quality gates or report a projection as
completion. On non-finite values, OOM, artifact mismatch, or a fixed-SLA breach, perform
only the already-tested safe checkpoint/abort path, preserve a failure
receipt, and do not report partial work as completion. Suggested commit:
`docs: publish reproducible 2b-token receipt`.

## Phase 16A — Prototype Quality Acceptance

- [ ] P16A complete

Dependencies: P16.

Prompt:

> Treat P16 as technical execution evidence, not proof of a useful language model. Verify
> the complete P0A–P16 acceptance, seal, source, profile, corpus, tokenizer, initialization,
> checkpoint, role-ledger, fixed-SLA, and `admission_hash` chain. In fresh processes, load
> the P9B initialization checkpoint and the final P16 checkpoint and evaluate both on the
> exact held-out index and metric implementation frozen before P15. Revalidate the
> independently frozen Rust unigram-baseline artifact and compute all losses and perplexities
> over the same valid targets, masks, accumulation order, and normalization. The final
> held-out loss and perplexity must be finite, and the final held-out loss must be strictly
> lower than both the initialization loss and the frozen unigram-baseline loss. No metric,
> baseline, held-out sample, or comparison direction may be selected after seeing P16.
>
> Run the exact pre-P15-frozen prompt/sample pack, decoding configuration, seeds, stop rules,
> and output comparison in at least two fresh processes from the final checkpoint. Require
> byte-identical generated token-ID sequences and decoded output bytes for every sample;
> malformed output, non-finite logits, hidden sampling state, or cross-run variation fails.
> Revalidate the distinct pre-P15 owner approval of the fixed prompt/sample and rubric
> artifact before evaluating the final checkpoint.
> Publish a sealed technical PASS/FAIL run. Technical PASS does not select the phase pointer:
> require an explicit named owner `APPROVE` decision with identity, signature/reference, UTC
> timestamp, the exact run/seal hashes, and an acknowledgement that these bounded gates do
> not establish broad capability, safety, or benchmark superiority. The acceptance is
> create-new/hash-linked, the root pointer atomically selects it, and neither artifact
> rewrites P16.

PASS:

- The entire prototype dependency and receipt chain, including P16 technical PASS and its
  fixed-SLA result, revalidates.
- Final held-out loss/perplexity are finite and final loss is strictly below both the
  initialization loss and frozen unigram-baseline loss on identical valid targets.
- Every frozen prompt/sample produces deterministic byte-identical token IDs and decoded
  bytes across fresh-process replays.
- A named owner explicitly approves the exact sealed P16A run; the selected acceptance and
  pointer bind that approval and contain no post-result metric or sample change.

STOP/loop: without both machine PASS and explicit owner approval, the project may report a
completed technical training run but not an accepted prototype LLM. P17, P18, and P19 remain
blocked. Suggested commit: `docs: accept prototype model quality`.

## Phase 17 — Portable Host/Data Implementation

- [ ] P17 complete

Dependencies: P16A.

Prompt:

> After prototype acceptance, implement the deferred host/process/filesystem/toolchain
> adapters without changing the frozen public argv, schemas, error categories, artifact
> formats, data semantics, or receipt model. Qualify one immutable source tree and lockfile
> across the mandatory CPU/data host matrix: Windows x86_64 with MSVC, Linux x86_64 with a
> recorded supported GCC/G++ or Clang/Clang++ toolchain, and macOS arm64/Apple Silicon with
> AppleClang. Implement each platform's process-tree containment, redaction, atomic
> publication, path containment, stable file identity, concurrent-mutation rejection,
> dynamic-library audit, temporary cleanup, and toolchain discovery directly in Rust; no
> platform shell script becomes a normative entry point.
>
> Compile the exact pinned Tree-sitter generated C parser/runtime with each selected native
> toolchain through the same narrow Rust ABI. On every host, run CPU-only quality gates,
> zero-Python canaries, P4–P9B compatibility fixtures, adversarial path/mutation fixtures,
> and fresh-process verification of the immutable governed production artifacts. Require
> byte-identical accepted IDs/reasons, Tree-sitter compatibility outputs, exact/near-dedup
> clusters and LSH decisions, split/sample manifests, custom BPE artifacts, token shards and
> `SPAN-001` order, P9B initialization artifacts, and canonical BF16/FP32 oracle gradients.
> Host performance may differ and is recorded but is never part of semantic equality.
>
> Each host publishes an independently sealed child qualification from the same P17 portable
> source, lockfile, schema bundle, parser, rules, seeds, and fixture hashes, and binds the
> unchanged P0A contract/ledger and selected P16A acceptance as dependencies.
> Publish P17 PASS only from an aggregate receipt that revalidates all three selected child
> acceptances. P17 never rewrites or generalizes the Windows prototype's P16/P16A evidence.

PASS:

- Windows x86_64, Linux x86_64, and macOS arm64 host qualifications all pass from the same
  immutable source and semantic identities, with exact toolchain/OS tuples recorded.
- Public xtask/CLI behavior and receipt semantics match the frozen portable contract, while
  every OS-specific containment, path, atomicity, and mutation fixture passes.
- All required data, tokenizer, storage, and CPU-oracle artifacts/decisions reproduce
  byte-for-byte; CPU-only builds discover or link no accelerator SDK/native ML backend or
  Python component.
- The aggregate receipt validates every child seal and acceptance and makes no accelerator,
  performance-equivalence, or new two-billion-token-run claim.

STOP/loop: one passing OS, a compile-only lane, or matching high-level counts cannot satisfy
P17. Preserve failing child evidence and fix the owning adapter or semantic regression.
Suggested commit: `feat(portability): implement portable host and data pipeline`.

## Phase 18 — Accelerator Provider Matrix

- [ ] P18 complete

Dependencies: P17 and P16A.

Prompt:

> Implement the deferred accelerator adapters and qualify all four mandatory tuples from one
> immutable source tree: (1) the Windows x86_64/NVIDIA RTX 5090/CUDA prototype regression;
> (2) Linux x86_64/NVIDIA/CUDA; (3) Linux x86_64/AMD/ROCm-HIP; and (4) macOS
> arm64/Apple Silicon/Metal. Before work, seal the exact host compiler, SDK/compiler,
> runtime/driver, native-library, device stable identity, architecture, feature, memory-model,
> backend-candidate inventory, and minimum model-allocation formula for every tuple. A
> substitute tuple or omitted mandatory tuple fails; qualifying one tuple never implies
> support for an untested OS/provider pairing.
>
> For every tuple, independently run the applicable correctness portions of the
> P1A/P1B/P2/P10–P15 protocol: native
> environment/artifact inspection and launch; minimum accelerator-visible allocation;
> measured Python-free Rust/native backend selection; exact P9B parameter loading; BF16
> storage with frozen FP32 accumulation; byte-for-byte equality of every canonical gradient;
> memory-efficient forward/backward and loss; bounded transfer or unified-memory access;
> synthetic end-to-end zero-Python qualification; byte-identical interrupted/resumed state
> within that tuple; synchronized per-tuple profiling; and a P15-style bounded
> correctness/stability ladder without the prototype's fixed performance-admission gate.
> Provider-specific C/C++ or Metal native code remains behind the same narrow audited Rust
> ABI and cannot own orchestration or general data transformation.
>
> Publish independent profile receipts and a create-new aggregate P18 acceptance from the
> same P18 source, lockfile, schemas, parser, data artifacts, P9B oracle, and numerical rules;
> bind the unchanged P0A contract/ledger and selected P16A/P17 acceptances as dependencies.
> Record per-tuple performance and memory facts, but do not
> require performance equivalence, apply the prototype's fixed 28,800-second SLA, claim
> cross-provider checkpoint migration, claim every OS/provider combination, or run/claim a
> two-billion-token completion on the new providers.

PASS:

- All four mandatory tuples independently qualify their exact host/device/backend chain;
  Windows/NVIDIA reproduces the accepted prototype regression on the portable source.
- Every tuple produces canonical gradient bytes exactly equal to P9B and proves exact
  deterministic interrupted/resumed continuation within the tuple.
- Transfer/unified-memory ownership, synchronization, cleanup, cancellation, error, and
  repeated-run baseline tests pass for each selected memory model.
- The aggregate acceptance revalidates all four child receipts from one immutable source and
  states the matrix's exact limits without extrapolation.

STOP/loop: missing hardware, an unavailable SDK/backend, tolerated gradient drift,
nondeterministic resume, or any missing mandatory tuple keeps P18 open; another provider is
not a fallback. Suggested commit: `feat(accelerator): qualify mandatory provider matrix`.

## Phase 19 — Optional Scale-Up Amendment

- [ ] P19 complete (optional)

Dependencies: P16A. P17 and P18 are independent and neither blocks nor satisfies P19.

Prompt:

> Begin only after an explicit owner request to scale the accepted prototype to a larger
> model and/or longer training budget. Create a new change-control amendment and receipt
> namespace; never alter, supersede by implication, or reinterpret the P0A–P16A prototype
> contract, artifacts, receipts, SLA result, or quality acceptance. Freeze and obtain named
> owner approval for every changed model shape/count, initialization artifact, corpus/token
> budget and role-ledger arithmetic, optimizer/schedule, evaluation/checkpoint cadence,
> memory formula, qualification profile, performance/admission rule, and SLA before any
> scale-up training or result-dependent tuning.
>
> Re-run every affected gate from P9B through P16 in dependency order under create-new phase
> and artifact identities, including canonical CPU initialization/oracle bytes, backend exact
> gradients, exact deterministic resume, profiling/admission, qualification ladder, governed
> source/removal freshness, and final technical receipt. If the longer budget changes the
> governed token prefix, span order, held-out index, or corpus identities, return to P9A (and
> any earlier owning phase) first. No prototype PASS may be copied forward merely because an
> implementation or provider is unchanged.

PASS:

- The owner-approved amendment explicitly identifies every changed and unchanged decision
  and binds a new noncolliding receipt namespace and complete dependency graph.
- Every affected P9B–P16 gate, plus any earlier artifact owner invalidated by the amendment,
  passes with new immutable evidence while preserving exact-gradient, exact-resume,
  cryptographic, governance, BPE, deduplication, LSH, and role-ledger requirements.
- The outcome is reported as the exact approved scale-up run, not as a portable-provider
  rerun or a reinterpretation of the original two-billion-token prototype.

STOP/loop: absent owner approval, post-result target selection, reused incompatible
artifacts, or a skipped invalidated gate blocks P19. Suggested commit:
`docs: approve and qualify optional scale-up amendment`.
