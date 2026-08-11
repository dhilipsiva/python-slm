# Clean-Rebuild Execution Plan

Dependencies, not phase numbers, define admissible order. Never start a phase before its
dependencies pass. The GPU/backend track (P1B–P2), data track (P3–P9A), and CPU model
track (P3–P9B) may run in parallel where dependencies permit. The existing Rust code is
reference evidence, not code to copy. `AGENTS.md` governs work,
`docs/rebuild-contract.md` records the approved Phase 0 product decisions,
`docs/receipts/P0.md` is their signed approval authority, and `docs/ARCHITECTURE.md`
defines the target design. A conflict is a stop condition.

## Operating Rules

- Start every phase by reading `AGENTS.md`, `docs/rebuild-contract.md`,
  `docs/ARCHITECTURE.md`, this file, `git status`, and every dependency's authoritative
  acceptance record plus its referenced immutable run. Preserve unrelated work.
- Do not run `cargo new`, `git init`, create a nested repository, broadly stage files,
  auto-commit, or tag. Use the existing repository.
- Implement only the active phase. Do not hide failed gates with fallbacks, relaxed
  assertions, ignored errors, or claims based on unexecuted commands.
- Normal tests are offline, deterministic, bounded, and credential-free. No Python
  executable or Python package is part of the build or pipeline.
- Every attempt writes create-new
  `docs/receipts/<phase-id>/runs/<run-id>/evidence.json` using schema
  `python-slm-phase-evidence-v1`, with exact command transcripts and hashes. Failed and
  superseded runs remain immutable and are never selected. A reviewed passing run gets a
  create-new `acceptances/<sequence>.json` record; the root `evidence.json` is an atomically
  replaceable `python-slm-phase-evidence-pointer-v1` pointer containing that acceptance
  path and hash. A dependency passes only when the pointer, selected acceptance, referenced
  immutable run, `PASS` status, and every required named approval all validate. P0 is the
  sole receipt-model exception: its dependency passes only when the pinned P0 `VERIFY`
  block succeeds. `docs/receipts/P0.md` is its human approval authority, the sealed run is
  its machine evidence, and P0 requires no acceptance generation or root pointer. Machine
  evidence, checklist prose, silence, or an agent audit never substitutes for human approval.
- P1A is the sole automatic machine-qualification exception currently authorized. A passing
  P1A verifier publishes an acceptance with `required_approvals: []` and selects it through
  the root pointer. The verifier never edits this checklist or commits; both remain subject
  to human review. No later phase inherits this exception unless its card says so explicitly.
- Each receipt records the command, working directory, relevant environment/configuration
  hashes, exit code, stdout/stderr or artifact hash, and status. A phase remains blocked
  unless its exact invocation appears in the normative global table below or its card.
  Receipts never contain credentials, raw source, PII, or secrets.
- A suggested commit is optional and occurs only after human review.

Until Phase 3 replaces the command contract, the CPU quality gate is:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
cargo test --locked --features cpu-reference
```

From P3 onward, every phase runs `scripts/quality-gate.ps1` plus a named targeted
test/benchmark command for that phase and records both in its receipt.

The literal verification entry points are fixed below. P1/P2 create their named scripts;
P3 creates `scripts/quality-gate.ps1` and rejected-by-default `scripts/verify-phase.ps1`.
Each implementation phase installs its own case. The preceding implementation owner must
install no-code cases before freeze: P6 installs P6A, P7 installs P7A, P8 installs P9A,
and P13 installs P14-P16. The driver records every child argv and exit code.

```powershell
# P1A / P1B
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-env.ps1 -Mode Cpu -OutputRoot docs\receipts\P1A
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-env.ps1 -Mode Cuda -OutputRoot docs\receipts\P1B

# P2
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\qualify-backend.ps1 -OutputRoot docs\receipts\P2

# P3
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\quality-gate.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P3 -OutputRoot docs\receipts\P3

# P4-P16: run quality-gate.ps1 first, then the exact phase command.
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P4 -OutputRoot docs\receipts\P4
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P5 -OutputRoot docs\receipts\P5
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P6 -OutputRoot docs\receipts\P6
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P6A -OutputRoot docs\receipts\P6A
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P7 -OutputRoot docs\receipts\P7
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P7A -OutputRoot docs\receipts\P7A
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P8 -OutputRoot docs\receipts\P8
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P9A -OutputRoot docs\receipts\P9A
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P9B -OutputRoot docs\receipts\P9B
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P10 -OutputRoot docs\receipts\P10
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P11 -OutputRoot docs\receipts\P11
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P12 -OutputRoot docs\receipts\P12
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P13 -OutputRoot docs\receipts\P13
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P14 -OutputRoot docs\receipts\P14
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P15 -OutputRoot docs\receipts\P15
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-phase.ps1 -Phase P16 -OutputRoot docs\receipts\P16
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

```powershell
$ErrorActionPreference = 'Stop'
$baseline = 'b1ebb455cdae94bbb9fc54f246cdf2758eedf1d1'
$sealed = @('docs/rebuild-contract.md', 'docs/receipts/P0/capture.ps1', 'docs/receipts/P0/evidence.json', 'docs/receipts/P0/runs')
git diff --exit-code $baseline -- $sealed
if ($LASTEXITCODE -ne 0) { throw 'sealed Phase 0 bytes changed' }
$sealedStatus = @(git status --porcelain=v1 --untracked-files=all -- $sealed)
if ($LASTEXITCODE -ne 0 -or $sealedStatus.Count -ne 0) { throw 'sealed Phase 0 paths are dirty or contain additions' }
$run = Resolve-Path docs\receipts\P0\runs\20260811T074740Z-d5008e94
Get-Content "$run\SHA256SUMS" | ForEach-Object { $expected, $relative = $_ -split '  ', 2; if ((Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $run $relative)).Hash.ToLowerInvariant() -ne $expected) { throw "seal mismatch: $relative" } }
$receiptCommit = '86fb1e4cc68efeb651e5362c4aca85c2827d8e4d'
$receiptSha256 = 'f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904'
git merge-base --is-ancestor $receiptCommit HEAD
if ($LASTEXITCODE -ne 0) { throw 'signed P0 receipt commit is not an ancestor of HEAD' }
git diff --exit-code $receiptCommit -- docs\receipts\P0.md
if ($LASTEXITCODE -ne 0) { throw 'signed P0 receipt differs from its approval commit' }
$receiptStatus = @(git status --porcelain=v1 --untracked-files=all -- docs\receipts\P0.md)
if ($LASTEXITCODE -ne 0 -or $receiptStatus.Count -ne 0) { throw 'signed P0 receipt is dirty' }
if ((Get-FileHash -Algorithm SHA256 -LiteralPath docs\receipts\P0.md).Hash.ToLowerInvariant() -ne $receiptSha256) { throw 'signed P0 receipt hash mismatch' }
$receipt = Get-Content -Raw docs\receipts\P0.md
$statusLines = [regex]::Matches($receipt, '(?m)^Status:[^\r\n]*$')
if ($statusLines.Count -ne 1 -or $statusLines[0].Value -notmatch '^Status:[ \t]+\*\*PASS\*\*[ \t]*$') { throw 'P0 receipt has no unique PASS status' }
foreach ($summaryName in @('Technical approval', 'Data-governance approval')) {
    $summaryPattern = '(?m)^' + [regex]::Escape($summaryName) + ':[^\r\n]*$'
    $summaryLines = [regex]::Matches($receipt, $summaryPattern)
    $approvedPattern = '^' + [regex]::Escape($summaryName) + ':[ \t]+\*\*APPROVED\*\*[ \t]*$'
    if ($summaryLines.Count -ne 1 -or $summaryLines[0].Value -notmatch $approvedPattern) { throw "missing or contradictory P0 summary: $summaryName" }
}
foreach ($sectionName in @('Technical approval', 'Data-governance approval')) {
    $sectionPattern = '(?ms)^###[ \t]+' + [regex]::Escape($sectionName) + '[ \t]*\r?\n(?<body>.*?)(?=^###[ \t]+|\z)'
    $sections = [regex]::Matches($receipt, $sectionPattern)
    if ($sections.Count -ne 1) { throw "missing or duplicate P0 section: $sectionName" }
    $body = $sections[0].Groups['body'].Value
    $decisions = [regex]::Matches($body, '(?m)^Decision:[^\r\n]*$')
    if ($decisions.Count -ne 1 -or $decisions[0].Value -notmatch '^Decision:[ \t]+\x60APPROVE\x60[ \t]*$' -or $body -match '(?i)\b(?:PENDING|REJECT)\b') { throw "ambiguous or non-approving P0 decision: $sectionName" }
    foreach ($pattern in @('(?m)^Owner name:[ \t]+\S[^\r\n]*$', '(?m)^Review reference/signature:[ \t]+\S[^\r\n]*$', '(?m)^UTC timestamp:[ \t]+\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z[ \t]*$')) {
        if ([regex]::Matches($body, $pattern).Count -ne 1) { throw "incomplete or duplicate P0 sign-off field in $sectionName" }
    }
}
```

The sealed capture remains authoritative for its frozen contract bytes and reference
observations. The signed receipt records technical and data-governance approval of the
separate architecture/TODO reconciliation commit without rewriting any sealed byte.

STOP/loop: a revoked or contradictory approval, failed P0 verifier, changed sealed byte,
or frozen-decision change reopens P0 and blocks P1A/P3. Suggested commit:
`docs: approve phase 0 rebuild contract`.

## Phase 1A — Verify and Pin the CPU Environment

- [x] P1A complete

Dependencies: P0.

Prompt:

> Implement the Windows PowerShell 5.1 entry point `scripts/verify-env.ps1 -Mode Cpu`.
> From an ordinary shell, discover a complete VS 2022 x64 toolchain and selected Windows
> SDK, qualify Rust 1.96 or newer on `x86_64-pc-windows-msvc`, and record normalized tool
> identities without installing tools or mutating persistent state. Use only a create-new
> P1A run and a unique owned temporary directory. Build the native ABI probe and locked
> CPU graph from an absent `CARGO_TARGET_DIR`, then run the exact pre-P3 format, Clippy,
> and CPU-test gate offline. Reject wrappers, build-affecting Cargo configuration, Python
> or CUDA tool execution, CUDA features/artifacts/linkage, input mutation, redaction leaks,
> and incomplete cleanup. Seal every completed attempt. `Cuda` is recognized only as the
> sealed P1B `MODE_NOT_IMPLEMENTED` failure until Phase 1B. A PASS run creates an automatic
> hash-linked acceptance and atomically advances the validated P1A root pointer; it never
> edits this checklist or commits. Phase 1B must regression-run CPU qualification after
> extending the shared verifier.

VERIFY:

```powershell
$ErrorActionPreference = 'Stop'
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\tests\verify-env.tests.ps1
if ($LASTEXITCODE -ne 0) { throw "P1A verifier tests failed: $LASTEXITCODE" }
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\verify-env.ps1 `
  -Mode Cpu `
  -OutputRoot docs\receipts\P1A
if ($LASTEXITCODE -ne 0) { throw "P1A qualification failed: $LASTEXITCODE" }
```

PASS:

- A clean shell produces a machine-readable CPU environment manifest.
- A clean-target locked CPU compile and probe run without CUDA discovery or linkage.
- Missing CPU tools fail with actionable messages.
- A passing immutable run is selected through a verified automatic acceptance with
  `required_approvals: []`; a failed run never changes the previously selected pointer.
- The closed schemas, transcript hashes, run seal, acceptance chain, root pointer,
  input-stability checks, redaction, and temporary cleanup validate before exit `0`.
- The checkbox remains open until a human reviews the selected machine qualification.

STOP/loop: missing CPU tools, an invalid P0 dependency, or a failed evidence chain blocks
P1B directly and P3 onward. Suggested commit:
`build: add reproducible CPU environment verification`.

## Phase 1B — Verify the RTX/CUDA Environment

- [ ] P1B complete

Dependencies: P1A.

Prompt:

> Extend `scripts/verify-env.ps1` with an explicit CUDA-required mode. Detect `nvcc`,
> toolkit libraries, driver, GPU name/memory/compute capability, and architecture flags
> accepted by the installed compiler. Require RTX 5090 compute capability 12.0 and an
> SM120-capable compiler. If using CUDA 12.x, require at least 12.8. A newer toolkit
> passes P1B when `nvcc` accepts the required SM120 image/PTX targets, the driver/runtime
> probe launches, and toolkit/driver compatibility passes; backend compatibility is a
> separate P2 gate. Record exact SASS/PTX targets. Compile and run one bounded CUDA device
> probe from an x64 VS 2022 Developer PowerShell. Do not persist local `CUDA_PATH` values
> in source control.

PASS:

- The CUDA-required manifest records compiler, driver, GPU, library, and SM evidence.
- The probe contains an SM120 image or supported forward-compatible code and launches on
  the target GPU.
- Missing or incompatible CUDA components fail with actionable messages.

STOP/loop: P2 and P10–P16 remain blocked, but CPU/data work may continue. Suggested
commit: `build: verify RTX 5090 CUDA environment`.

## Phase 2 — Select the Backend by Measurement

- [ ] P2 complete

Dependencies: P1B.

Prompt:

> Create isolated, disposable spikes for current supported Burn/CubeCL and Candle
> candidates on Windows/MSVC/RTX 5090. Pin versions only for the experiment. Measure BF16
> allocation, representative GEMM, automatic differentiation/backward, synchronization,
> p50/p95 latency, and peak VRAM after warmup, using a minimal scalar CPU oracle. Do not
> invent model semantics or add FA3/custom attention. Before running, write the BF16
> forward/gradient tolerance table and comparison metrics into the receipt; changing them
> invalidates and reruns P2. Write `docs/adr/0001-ml-backend.md` with raw JSON,
> constraints, a provisional selection, and fallback boundary; P10 model/attention parity
> and P12 synchronized full-training-step evidence make it final.

PASS:

- At least one backend launches on SM120, backpropagates, synchronizes, and passes
  declared BF16 tolerances.
- The selection is based on target-host measurements, not feature lists.
- CPU-only compilation does not discover or link CUDA.

STOP/loop: if no backend passes, evaluate a narrowly scoped `cudarc`/cuBLASLt design
before changing the architecture. Suggested commit: `bench: select measured GPU backend`.

## Phase 3 — Clean In-Place Scaffold and Cutover

- [ ] P3 complete

Dependencies: P0, P1A. P2 may proceed in parallel and is required by P10.

Prompt:

> Recreate the implementation in this repository without copying the reference source.
> Keep one Cargo package and one installed `python-slm` executable initially. Expose
> independently restartable `plan`, `curate`, `train-tokenizer`, `tokenize`, `inspect`,
> `bench`, and `train` subcommands through internal config, data, tokenizer, storage,
> model, backend, and train modules. The
> the contract's historical phrase "independently restartable binaries" means these
> independently invocable subcommands, never multiple installed executables.
> Implement `CLI-001`, `CONFIG-001`, and `ERROR-001`:
> explicit versioned JSON production configurations, unknown-field rejection, no hidden
> production defaults, one terminal success JSON object, typed JSONL handled errors, and
> fixed exit categories 0 through 5. Feature-gate CUDA and native probes so the default
> CPU build needs no toolkit. Define a backend
> interface without selecting or importing a GPU framework; P10 consumes P2's ADR.
> Replace old code in small, reviewable diffs; preserve research documents and baseline
> receipts. Create stable `scripts/quality-gate.ps1` and rejected-by-default
> `scripts/verify-phase.ps1` entry points that record child argv/exit status. Do not leave
> executable `todo!()`, `unimplemented!()`, or silent stub success paths.

PASS:

- The canonical CPU gates pass with `CARGO_TARGET_DIR` set to a newly created empty
  temporary directory; never delete an existing target directory. CPU builds do not
  discover or link CUDA.
- If P2 is complete, its selected-backend compile gate also passes; otherwise that gate
  remains a recorded P10 prerequisite. `--help` exposes all seven subcommands, handled
  success/error streams and exit categories pass fixtures, and unfinished stages fail
  closed with explicit status.
- The canonical quality-gate commands are documented for all later phases.

STOP/loop: unexplained reference-code carryover or a CPU build requiring CUDA fails P3.
Suggested commit: `build: establish clean Rust pipeline scaffold`.

## Phase 4 — Source Access, Governance, and Provenance

- [ ] P4 complete

Dependencies: P3.

Prompt:

> Define a bounded `DocumentSource` contract carrying the exact `SOURCE-002` stable
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

> Implement `SOURCE-002/003` exactly: strict UTF-8/ASCII Python 3, recorded leading-BOM
> removal only, fixed canonical size `100..=1,000,000`, pinned
> `tree-sitter-python 0.25.0`, parser bounds, `root.has_error()` rejection, exact
> Tree-sitter comment-node byte accounting, and independent generated-marker scans over
> each comment intersection with `[0,8192)`. Equality at 50% comment bytes passes;
> strictly greater fails; docstrings are not comments. Before detector implementation,
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

> Before implementing, benchmarking, or tuning LSH, materialize the exact 10,000-pair
> `dedup-threshold-v1` generator, strata, seed, exhaustive truth, and manifest required by
> `DEDUP-003`; obtain human review and seal its source/artifact hashes. Then implement
> exact curated-hash dedup, `DEDUP-001` lexical traversal/encoding and transitive clusters,
> and `DEDUP-002`'s 256 fixed affine MinHash components, 32-by-8 LSH keys, short-document
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

> Without changing code, resolve the destination to an explicit ignored path, verify free
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

> Implement exact `TOKSAMPLE-001` selection over whole deduplicated, decontaminated train
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

> Without changing code, select only from the immutable governed training split using
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
> absolute paths, parent traversal, and symlink/reparse-point escapes in manifest paths.
> Map immutable files read-only and on Windows deny write/delete sharing, or document and
> test an equally strong invariant. Bulk-gather contiguous spans and explicitly decode
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

> Implement a small-shape-capable, bias-free, pre-norm decoder reference: embeddings;
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
> next-token loss on tiny shapes; do not select a GPU framework when P2 is incomplete.

PASS:

- Exact count, shape, causal-prefix invariance, RoPE scalar parity, GQA mapping,
  RMSNorm, SwiGLU, and finite-difference gradient checks on tiny shapes pass.
- The reference is deterministic and suitable as the oracle for optimized kernels.
- Both initialized artifacts reproduce byte-for-byte and their hashes are recorded.
- Full production shapes are not used in ordinary CPU tests.

STOP/loop: a range such as “135M ±5%” is not an acceptable count gate. Suggested
commit: `feat(model): add exact Llama reference semantics`.

## Phase 10 — SM120 Model, Attention, and Fused Loss

- [ ] P10 complete

Dependencies: P2, P9B.

Prompt:

> Port the validated model to the selected backend and load P9B's canonical initialized
> parameter artifact by its recorded hash; backend-native reinitialization is forbidden.
> Progress from unfused reference GQA
> to the framework's memory-efficient path; add an isolated SM120 custom operation only
> if profiling proves it necessary. The production path must support causal 12Q/4KV BF16
> forward and backward without materializing repeated K/V. Add chunked or fused LM-head
> cross-entropy so full `[B,L,V]` logits are not retained. Upstream FA3 is never mandatory;
> current Python/CuTeDSL FA4 is also only an isolated future experiment.
> Any custom kernel uses a tiny feature-gated `extern "C"` ABI, propagates CUDA errors,
> validates ABI layout and stream compatibility, and uses ownership guards through async
> completion. It emits an SM120 image plus PTX fallback when supported by the qualified
> toolchain. Before benchmarking, record the BF16 forward/gradient/loss tolerance table
> and comparison metrics in the ADR and receipt; changing them invalidates and reruns P10.
> Benchmark synchronized forward/backward at
> `B={2,4,8,16,32}`, `L=2048`, discard warmup, and emit JSON with error, p50/p95,
> workspace, and peak VRAM. Run sizes low-to-high in fresh child processes with timeouts;
> record OOM and skip larger sizes so one allocation cannot poison the suite. B=32 is a
> trial, not a PASS requirement. Update P2's ADR with model-level evidence and the
> unresolved P12 full-step gate.

PASS:

- Forward loss and gradients meet the predeclared BF16 tolerances against P9B.
- The backend loads the exact P9B initialization artifact/hash and parameter-name mapping.
- Causal and GQA semantics pass adversarial tests; optimized code does not repeat K/V.
- At least one full-layer and fused-loss path runs on SM120 without quadratic attention
  or full-logit retention.
- The provisional backend ADR is supported by model-level forward/backward evidence and
  explicitly remains open until P12.

STOP/loop: a fast forward-only kernel is not a training backend. Suggested commit:
`perf(model): add validated SM120 training path`.

## Phase 11 — Host Staging and H2D Transfer

- [ ] P11 complete

Dependencies: P8, P10.

Prompt:

> First benchmark the bulk pageable loader. Then implement a bounded reusable two- and
> three-slot page-locked staging pool with explicit CUDA streams/events and safe slot
> reuse. Pipeline mmap bulk gather/conversion, asynchronous H2D, and compute without
> pinning the whole corpus. Compare pageable synchronous copy, pinned synchronous copy,
> and pinned asynchronous overlap. Separate gather, conversion, H2D, synchronization,
> and compute timings. Keep the simplest path whose end-to-end measurements win.

PASS:

- No buffer is reused before its completion event; allocation is bounded and stable.
- Data parity holds under long randomized stress, cancellation, and error paths.
- Benchmark evidence, not terminology, selects the production transfer path.
- Every registered or allocated page-locked slot is unregistered/freed after success,
  cancellation, timeout, panic boundary, and CUDA error; repeated runs return pinned-host
  memory use to baseline.

STOP/loop: mmap, `Bytes`, or ordinary vectors are never labeled pinned. Suggested
commit: `perf(loader): benchmark bounded GPU staging`.

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
> model, master/moment state, scheduler, scaler if used, host/device RNG, data order/cursor,
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
> and peak VRAM meet those gates.

PASS:

- Scalar AdamW/schedule/accumulation references and boundary tests pass.
- Trainer initialization reproduces the P9B artifact hash before the first update.
- Tests cover updates 1, 1,000, 30,518; the 37,888-target final normalization; threshold
  evaluation/checkpoint coincidences; retention; and zero overshoot.
- A fixed tiny corpus overfits without non-finite values.
- Interrupted/resumed execution matches uninterrupted execution within a declared BF16
  tolerance, resumes the next exact `SPAN-001` entry without repeat/skip, and refuses
  mismatched span manifests, artifacts, or configuration.
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

> Generate an offline synthetic corpus containing valid, invalid, encoded, duplicate,
> near-threshold, generated, comment-heavy, PII/secret, tiny, and oversized documents.
> Exercise source, policy, dedup, tokenizer, storage, training, checkpoint, and resume from
> one reproducible command. Put failing canary launchers for `python`, `python3`, `py`, and
> `pip` first on `PATH`, audit the child-process tree, and statically inspect repository
> build scripts/commands for Python launch paths; an absolute-path Python process also
> fails the gate. Add normal Windows CPU CI and a separate explicitly provisioned RTX 5090
> job for environment checks, CUDA parity, tiny training, and benchmark artifacts. Keep
> noisy performance thresholds out of ordinary correctness CI. Generate workflow files
> and local qualification scripts only. Do not register a self-hosted runner, change
> repository settings, or trigger external CI without explicit authorization. When no
> runner is provisioned, execute the GPU qualification script locally on the target host.

PASS:

- An authorized fresh clone of the exact commit, or a clean content-addressed export of
  the exact tree, passes locked fmt/check/Clippy/tests without CUDA.
- The local GPU qualification script—or an already authorized GPU CI job—publishes parity
  JSON and a complete environment/reproducibility manifest.
- Data-side artifacts (accepted IDs/reasons, tokenizer, indexes, and shards) repeat
  byte-for-byte. Training/checkpoint results meet declared numerical resume tolerances;
  bitwise CUDA equality is required only if deterministic mode was qualified.
- No Python canary is hit and no Python or pip process is observed.

STOP/loop: Clippy is not evidence of CUDA correctness, memory safety, or performance.
Suggested commit: `ci: add offline CPU and RTX qualification gates`.

## Phase 14 — Autotune and Profile the Full Step

- [ ] P14 complete

Dependencies: P12, P13.

Prompt:

> In a release build on the target host, autotune microbatches 2, 4, 8, 16, and 32 at
> context 2,048. Adjust gradient accumulation independently to preserve exactly 65,536
> valid predicted targets per full update. Run each OOM-prone candidate in a fresh child
> process with an explicit timeout and result artifact; record failure and stop increasing
> that family.
> Synchronize timings, discard JIT/autotune warmup from numerator and denominator, compute
> throughput from actual valid predicted targets consumed rather than nominal `B*L`, and
> maximize steady-state targets/s below the VRAM safety ceiling rather than maximizing
> occupancy.
> Report loader, kernel-only, trainer, and whole-run-equivalent throughput; p50/p95;
> forward/backward/optimizer/H2D/checkpoint time; stalls; and allocated/reserved/peak VRAM.
> P14 changes configuration only. A code bottleneck returns to P10 (model/kernel), P11
> (loader), or P12 (trainer/checkpoint); rerun P13 and P14 afterward.

PASS:

- A reproducible configuration is selected from complete JSON evidence.
- No unexplained non-finite values, OOM, hidden synchronization, or valid-target overcount
  exists.
- Before promotion to P15, a synchronized short-run preflight reaches at least 75K
  steady-state valid predicted targets/s and projects below 28,800 seconds using measured
  startup, evaluation, checkpoint, and final-save overhead.

STOP/loop: hardcoded batch 32, 28GiB-as-goal, or unsynchronized timing fails P14.
Suggested commit: `perf(train): select measured RTX 5090 configuration`.

## Phase 15 — Frozen-Code Qualification Ladder

- [ ] P15 complete

Dependencies: P9A, P14.

Prompt:

> Freeze code, dependencies, corpus, tokenizer, model, training configuration, approved
> full-contract hash, decision-ledger hash, and P9B initialization-artifact hash. Before
> running, verify all immutable hashes, exclusive GPU availability, target-host power,
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
> logs, JSON, hashes, host/GPU telemetry, loss/gradient diagnostics, and non-finite counts.
> Calculate the two-billion-token projection from measured whole-run-equivalent throughput
> including startup, JIT/autotune, loader stalls, synchronization, configured held-out
> evaluation, planned checkpoints, and final durable save. Do not edit code while
> measuring.

PASS:

- All rungs finish without correctness, resume, OOM, or thermal-stability failure.
- Every rung preserves completed-boundary checkpoint semantics and the frozen contract,
  ledger, initialization, corpus, tokenizer, code, and configuration identities.
- Synchronized steady-state throughput is at least 75K valid predicted targets/s for
  30–60 minutes.
- Measured whole-run projection, including overhead, is below 28,800 seconds.

STOP/loop: on failure, preserve evidence and return to the phase that owns the cause—P10
model/kernel, P11 loader, P12 trainer/checkpoint, or P14 configuration. Rerun that phase
and every affected downstream gate, then freeze a new qualification identity. Never patch
only the benchmark or launch 2B. Suggested commit:
`test: record frozen RTX 5090 qualification`.

## Phase 16 — Final Two-Billion-Token Run and Release Receipt

- [ ] P16 complete

Dependencies: P15.

Prompt:

> Recheck every `GOV-003` authority within 24 hours before launch and stop on a superseding
> mandatory removal. Run the frozen training command against the immutable governed
> corpus. After corpus preparation, start the SLA clock immediately before the trainer
> opens the frozen artifacts and stop it only after the final checkpoint is durable.
> Finalize the receipt
> immediately using that captured elapsed time. Process exactly the contracted number of
> valid predicted training targets, handling final
> alignment and masks explicitly. Monitor
> without modifying code. During training, validate durable checkpoint manifests/hashes
> without interrupting the run. After completion, load a copied mid-run checkpoint and
> the final checkpoint in fresh processes. Do not deliberately interrupt the timed final
> run; any recovery downtime that occurs remains inside the continuous SLA clock.
> Publish the complete `PROV-001` schema: approved full-contract and decision-ledger hashes;
> frozen source tree; Git commit/dirty status; `Cargo.lock`; source/removal approval
> references and hashes; Rust/MSVC/CUDA/driver/GPU/SM; backend/kernel build and
> configuration identities; tokenizer/source/corpus/split/configuration hashes;
> telemetry/log/benchmark hashes; every role-ledger counter; evaluation/checkpoint counts;
> peak VRAM; loss/evaluation diagnostics; elapsed time; and final checkpoint hash.

PASS:

- The actual durable run completes in less than 28,800 seconds.
- Exactly 2,000,000,000 valid predicted targets, zero overshoot, hashes,
  checkpoints, and fresh-process reload all verify.
- The receipt distinguishes measured facts from interpretation and records any deviation.
- The `PROV-001` role ledger records and reconciles 2,000,000,001 prefix IDs, stored
  unused tail, unmaterialized documents/bytes, 2,000,000,000 consumed real inputs, 1,024
  runtime PAD inputs, 2,000,000,000 valid targets, 1,024 masked padding targets, zero
  boundary exclusions, 30,518 optimizer updates, and all evaluation/checkpoint counters.

STOP/loop: do not invent an arbitrary real-corpus loss-halving criterion or report a
projection as completion. On non-finite values, OOM, artifact mismatch, or an impossible
SLA, perform only the already-tested safe checkpoint/abort path, preserve a failure
receipt, and do not report partial work as completion. Suggested commit:
`docs: publish reproducible 2b-token receipt`.
