# Clean-Rebuild Execution Plan

Dependencies, not phase numbers, define admissible order. Never start a phase before its
dependencies pass. The GPU/backend track (P1B–P2), data track (P3–P9A), and CPU model
track (P3–P9B) may run in parallel where dependencies permit. The existing Rust code is
reference evidence, not code to copy. The target architecture is
`docs/ARCHITECTURE.md`.

## Operating Rules

- Start every phase by reading `AGENTS.md`, `docs/ARCHITECTURE.md`, this file, and
  `git status`. Preserve unrelated work.
- Do not run `cargo new`, `git init`, create a nested repository, broadly stage files,
  auto-commit, or tag. Use the existing repository.
- Implement only the active phase. Do not hide failed gates with fallbacks, relaxed
  assertions, ignored errors, or claims based on unexecuted commands.
- Normal tests are offline, deterministic, bounded, and credential-free. No Python
  executable or Python package is part of the build or pipeline.
- Record each phase's commands, exit codes, relevant measurements, and unresolved facts
  in `docs/receipts/<phase-id>.md`, for example `P1A.md`, `P6A.md`, `P7A.md`, `P9A.md`,
  or `P9B.md`. Mark a phase complete only after its PASS gate succeeds.
- A PASS list is not evidence without an exact VERIFY command. Each receipt records the
  command, working directory, relevant environment/configuration hashes, exit code,
  stdout/stderr or artifact hash, and status. Receipts never contain credentials, raw
  source, PII, or secrets.
- A suggested commit is optional and occurs only after human review.

Until Phase 3 replaces the command contract, the CPU quality gate is:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features cpu-reference -- -D warnings
cargo test --locked --features cpu-reference
```

From P3 onward, every phase runs `scripts/quality-gate.ps1` plus a named targeted
test/benchmark command for that phase and records both in its receipt.

## Phase 0 — Freeze the Rebuild Contract

- [ ] P0 complete

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
> reject-versus-transform policy; train/validation/test split and
> benchmark-decontamination policy; dedup-only normalization, lexical-token
> 5-gram semantics, MinHash seed/signature width, LSH layout, labeled-suite corpus/version,
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
- The diff contains only the contract and receipt.

STOP/loop: unresolved product decisions stay in P0. Suggested commit:
`docs: freeze clean-rebuild contract`.

## Phase 1A — Verify and Pin the CPU Environment

- [ ] P1A complete

Dependencies: P0.

Prompt:

> Add a read-only `scripts/verify-env.ps1` with a CPU-only mode and an
> environment-manifest schema. Detect `rustc -Vv`, Cargo, the MSVC Rust host, linker/C
> compiler resolution, and Windows SDK without embedding machine-specific absolute paths
> or mutating user environment variables. From an ordinary PowerShell, locate VS 2022 via
> supported discovery and spawn an x64 Developer environment for the clean native build,
> or instruct the operator to use an x64 VS 2022 Developer PowerShell. Perform a locked
> CPU build and trivial Rust/native-dependency probe without silently relying on cached
> objects. Record exactly how the toolchain was located.

PASS:

- A clean shell produces a machine-readable CPU environment manifest.
- A clean-target locked CPU compile and probe run without CUDA discovery or linkage.
- Missing CPU tools fail with actionable messages.

STOP/loop: missing CPU tools block P3 onward. Suggested commit:
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
> Keep one Cargo package initially. Establish typed, versioned configuration and the
> module/CLI boundaries specified in `docs/ARCHITECTURE.md`: config, data, tokenizer,
> storage, model, backend, train, and independently restartable binaries. Feature-gate
> CUDA and native probes so the default CPU build needs no toolkit. Define a backend
> interface without selecting or importing a GPU framework; P10 consumes P2's ADR.
> Replace old code in small, reviewable diffs; preserve research documents and baseline
> receipts. Create a stable `scripts/quality-gate.ps1`. Do not leave executable `todo!()`,
> `unimplemented!()`, or silent stub success paths.

PASS:

- The canonical CPU gates pass with `CARGO_TARGET_DIR` set to a newly created empty
  temporary directory; never delete an existing target directory. CPU builds do not
  discover or link CUDA.
- If P2 is complete, its selected-backend compile gate also passes; otherwise that gate
  remains a recorded P10 prerequisite. `--help` exposes stage commands and unfinished
  stages fail closed with explicit status.
- The canonical quality-gate commands are documented for all later phases.

STOP/loop: unexplained reference-code carryover or a CPU build requiring CUDA fails P3.
Suggested commit: `build: establish clean Rust pipeline scaffold`.

## Phase 4 — Source Access, Governance, and Provenance

- [ ] P4 complete

Dependencies: P3.

Prompt:

> Define a bounded `DocumentSource` contract carrying source/repository IDs, source
> snapshot, original encoding, license/provenance, removal-list version, raw hash, and
> content bytes. Implement separate adapters for genuinely content-bearing Parquet and
> authorized Stack-v2 metadata plus Software Heritage content retrieval. Add HTTPS,
> checksums, size limits, timeout, retry/backoff, bounded concurrency, resumable cache,
> and environment-only credentials. Build deterministic local HTTP/Parquet/SWH fixtures;
> normal tests must never call the public Internet. Bound and revalidate redirects, never
> forward credentials across origins, reject local/link-local targets outside explicit
> fixture mode, enforce approved HTTPS hosts/destinations, cap compressed and decompressed
> bytes, and make cache keys/path joins traversal-safe. Emit acquisition manifests and
> reason-coded failures.

PASS:

- Fixture tests cover resume, timeout, retry, checksum mismatch, oversize/decompression
  bomb, hostile redirect, cross-origin credential forwarding, absolute/`..` IDs,
  malformed metadata, encoding metadata, and credential redaction.
- Provenance survives a source round trip and removal snapshots are versioned.

STOP/loop: fixture-complete adapters may pass P4 without credentials. The authorized
live-source gate is deferred to P6A and cannot be self-approved. Use an explicitly
approved alternate source, never a hidden substitution. Suggested commit:
`feat(data): add governed document source adapters`.

## Phase 5 — Decode, Scrub, CST, and Quality Filters

- [ ] P5 complete

Dependencies: P4.

Prompt:

> Implement the P0 encoding policy deterministically, retaining the raw hash and
> recording any Rust-native transcode. Use one pinned-version Tree-sitter Python parser
> per Rayon worker. Add configurable minimum/maximum bytes, parser resource bounds,
> `root.has_error()` rejection, Tree-sitter comment-node byte ratio, header-scoped
> generated markers, and metadata policy. Equal to 50% comment bytes is accepted; greater
> than 50% is rejected. Docstrings are not comments. Apply the P0-fixed PII/secret policy.
> Preserve original bytes in the governed raw layer. Any permitted deterministic
> transformation creates separate curated bytes, pre/post hashes, a policy version, and
> a transformation record; never claim transformed text is original or log scrubbed
> values. Re-run CST validation after transformation and reject a transform that changes
> the configured Python-validity outcome. Define stable all-reasons ordering or explicit
> first-failure precedence.

PASS:

- Boundary/property fixtures cover encodings, 99/100 bytes, exactly/over 50%, malformed
  trees, docstrings, marker location/case, huge files, time budgets, PII, and secrets.
- Repeated runs produce identical accepted IDs, bytes, reasons, and hashes.
- CPU memory remains within the configured bound under adversarial batches.
- No PII/secret fixture value appears in logs, errors, or receipts.

STOP/loop: ambiguous encoding or scrub policy returns to P0 rather than becoming an
implicit heuristic. Suggested commit: `feat(data): add deterministic source policy`.

## Phase 6 — Scalable Exact/Near Deduplication

- [ ] P6 complete

Dependencies: P5.

Prompt:

> Implement exact-content deduplication, then the P0-frozen dedup-only normalization and
> lexical-token 5-grams. Build deterministic 256-component seeded MinHash signatures and
> LSH candidate retrieval using the frozen seed/layout. Apply exact source-shingle
> Jaccard only to candidates; reject when similarity is strictly greater than 0.85.
> Cluster matches and choose a deterministic representative using documented quality and
> stable-ID tie-breaks.
> Partition or persist production indexes; never perform global O(N²) comparison or keep
> every document/shingle set in RAM. Keep normalization used for dedup separate from
> tokenizer text. Duplicate clusters retain every source/provenance/license record;
> representative selection chooses training bytes only.

PASS:

- Fixtures built from rational set intersections clearly below, exactly at, just above,
  and well above 0.85 behave as specified.
- Results and representative choices are invariant across thread counts and input order.
- A labeled clone suite reports LSH recall/precision; a 100K-document benchmark proves
  bounded memory and no all-pairs path.
- Measured recall and precision meet the P0-frozen minima for the recorded suite version;
  otherwise P6 remains open.

STOP/loop: exact checks do not justify claiming removal of pairs LSH never retrieved.
Record measured recall. Suggested commit: `feat(data): add deterministic LSH dedup`.

## Phase 6A — Materialize Governed Documents and Splits

- [ ] P6A complete

Dependencies: P4, P5, P6.

Prompt:

> Resolve the destination to an explicit ignored path, verify free space/quota, and bound
> caches before bulk work; never recursively clean an output root or commit raw/generated
> artifacts. Run synthetic, authorized 1,000-document, larger bounded, then production
> source snapshots through acquisition, decoding/policy, and deduplication without changing
> code. Persist immutable governed-document and duplicate-cluster manifests with every
> provenance/license/removal record. Apply the P0 repository/cluster split algorithm and
> benchmark-decontamination registry before tokenizer sampling. Record counts, hashes,
> rejection reasons, dedup recall, resource peaks, and policy versions. Obtain named human
> governance approval; an agent cannot self-approve source rights or transformations.

PASS:

- Authorized live acquisition succeeds and every stage verifies from a fresh process.
- Train/validation/test splits are repository/duplicate-cluster isolated and repeat from
  the recorded algorithm, seed, and registry hash.
- Named governance approval covers the immutable snapshot, and the training split has
  enough approved bytes for the P0 tokenizer budget.

STOP/loop: missing access, approval, provenance, removal freshness, scrub policy, disk,
or approved bytes blocks the production tokenizer. Suggested commit:
`docs: record governed curated corpus`.

## Phase 7 — Tokenizer Engine and Fixture Qualification

- [ ] P7 complete

Dependencies: P6.

Prompt:

> Implement deterministic sample selection with a configurable decimal-byte budget,
> stable ranking/order, per-repository caps, and exclusion of validation/test and
> decontamination matches. Implement Rust-native byte-level BPE training with no case
> folding, Unicode normalization, or whitespace stripping; explicitly seed the complete
> 256-byte alphabet; use fixed special IDs and EOS policy; and emit versioned artifact
> metadata. Qualify the engine on a sufficiently varied immutable synthetic training
> split. Add byte-roundtrip, repeated-build hash, whitespace/indentation, arbitrary-byte
> policy, unknown-token, max-ID, and pathological repeated-pair overflow tests. Do not
> require or create production data.

PASS:

- Vocab size is 32,000; maximum ID is at most 31,999; all special IDs match the contract.
- Two clean fixture runs over the same ordered manifest produce byte-identical artifacts.
- Supported source bytes round-trip exactly with zero unknown IDs; unsupported input fails
  explicitly before tokenization.

STOP/loop: nondeterministic trainer behavior, pair-count overflow, or byte-roundtrip
failure stays in P7. Suggested commit: `feat(tokenizer): add deterministic 32k BPE`.

## Phase 7A — Train the Production Tokenizer

- [ ] P7A complete

Dependencies: P6A, P7.

Prompt:

> Without changing code, select only from the immutable governed training split using the
> P0 byte budget (production target: up to 2,000,000,000 decimal bytes; never duplicate
> input to fill it), stable order, and per-repository caps. Train twice from clean outputs,
> verify both complete hash chains, and publish the production tokenizer/sample receipt.
> Validation/test and benchmark-decontamination matches are ineligible.

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
> little-endian values. Generate 2,049 stored IDs for each 2,048-token input/target pair;
> never wrap corpus tail to head or cross forbidden document/split boundaries.

PASS:

- Round-trip, cross-shard, endianness, boundary, truncation, concurrent mutation,
  corruption, tokenizer-mismatch, wrong-count, absolute/parent/symlink path, crash between
  shard and manifest finalization, and out-of-range fixtures pass.
- Readers reject incomplete generations and enforce the backing-file immutability
  invariant documented by the architecture.
- Bulk-loader benchmark avoids per-token file lookup and records host throughput.

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
> root or commit raw/token artifacts. Run a bounded sample, then materialize enough stored
> IDs to expose exactly 2,000,000,000 valid predicted targets under the P0 policy. Treat
> stored-ID count as a measured artifact output, not a preselected target. Preserve and
> verify the entire source→split→tokenizer→shard hash chain. Record host resource peaks,
> tokenization wall time, boundary exclusions, unused tails, and held-out artifacts.

PASS:

- Every manifest/hash verifies from a fresh process; no partial artifact is accepted.
- Valid predicted targets equal 2,000,000,000; stored-ID, boundary-exclusion, and
  unused-tail counts reconcile exactly to manifests and P0 accounting rules.
- Tokenized train/held-out artifacts trace to the approved P6A split and tokenizer hashes.

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
> tensor layout, RoPE pairing, causal-mask convention, parameter names, dropout, and
> initialization from P0. Implement an analytical per-component parameter counter
> independent of any backend registry and require exact agreement. Implement shifted
> next-token loss on tiny shapes; do not select a GPU framework when P2 is incomplete.

PASS:

- Exact count, shape, causal-prefix invariance, RoPE scalar parity, GQA mapping,
  RMSNorm, SwiGLU, and finite-difference gradient checks on tiny shapes pass.
- The reference is deterministic and suitable as the oracle for optimized kernels.
- Full production shapes are not used in ordinary CPU tests.

STOP/loop: a range such as “135M ±5%” is not an acceptable count gate. Suggested
commit: `feat(model): add exact Llama reference semantics`.

## Phase 10 — SM120 Model, Attention, and Fused Loss

- [ ] P10 complete

Dependencies: P2, P9B.

Prompt:

> Port the validated model to the selected backend. Progress from unfused reference GQA
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

> Implement BF16 pre-training with explicit FP32-sensitive reductions/state and a
> versioned configurable preset defaulting to AdamW (`beta1=.9`, `beta2=.95`, `eps=1e-8`,
> decay `.1`), P0 decay exemptions/clipping policy, and 1,000-optimizer-step warmup to
> `2.5e-3` followed by cosine decay to the P0-fixed floor/end. Distinguish stored IDs,
> consumed input IDs, valid predicted targets, padding/boundary exclusions, microsteps,
> optimizer steps, and zero overshoot. Normalize accumulated loss/gradients by the actual
> valid-predicted-target count; define the final partial update; if scaling, unscale before
> finite checks and clipping; clip only after all microsteps; then step AdamW and the
> scheduler and zero gradients once at the completed optimizer update. Save atomic
> generation checkpoints with
> model, master/moment state, scheduler, scaler if used, host/device RNG, data order/cursor,
> counters, and all config/artifact hashes, only at completed optimizer boundaries unless
> accumulated gradients are also serialized. Do not run the full corpus. Evaluation
> consumes immutable held-out spans, does not advance training cursor/RNG/scheduler,
> restores mode and state, records loss/perplexity, and includes enabled cadence/time in
> SLA projections. Before measurement, record full-step numerical and performance gates
> in the ADR. Measure synchronized representative full training steps, including optimizer
> work, and finalize the backend ADR only if numerical parity, p50/p95 latency, throughput,
> and peak VRAM meet those gates.

PASS:

- Scalar AdamW/schedule/accumulation references and boundary tests pass.
- A fixed tiny corpus overfits without non-finite values.
- Interrupted/resumed execution matches uninterrupted execution within a declared BF16
  tolerance and refuses mismatched artifacts/configuration.
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

- Fresh-clone CPU CI passes locked fmt/check/Clippy/tests without CUDA.
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
> context 2,048. Adjust gradient accumulation independently to preserve the selected
> valid predicted-target batch. Run each OOM-prone candidate in a fresh child process with
> an explicit timeout and result artifact; record failure and stop increasing that family.
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

> Freeze code, dependencies, corpus, tokenizer, model, and training configuration. Before
> running, verify all immutable hashes, exclusive GPU availability, target-host power,
> sleep/update state, sufficient disk for all checkpoint generations plus safety margin,
> and that no superseding mandatory removal snapshot exists. Report blockers but do not
> mutate OS policy without explicit authorization. Qualify at 1M valid predicted targets,
> 100M valid predicted targets, 30 minutes, and then 60 minutes. At each rung run a
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

> Run the frozen training command against the immutable governed corpus. After corpus
> preparation, start the SLA clock immediately before the trainer opens the frozen
> artifacts and stop it only after the final checkpoint is durable. Finalize the receipt
> immediately using that captured elapsed time. Process exactly the contracted number of
> valid predicted training targets, handling final
> alignment and masks explicitly. Monitor
> without modifying code. During training, validate durable checkpoint manifests/hashes
> without interrupting the run. After completion, load a copied mid-run checkpoint and
> the final checkpoint in fresh processes. Do not deliberately interrupt the timed final
> run; any recovery downtime that occurs remains inside the continuous SLA clock.
> Archive the frozen source-tree hash, Git commit and status, `Cargo.lock` hash,
> Rust/MSVC/CUDA/driver/GPU manifest, selected backend/kernel IDs,
> tokenizer/corpus/split/config hashes, counters, benchmark JSON, complete logs,
> checkpoint hashes, wall time, peak VRAM, and measured loss diagnostics.

PASS:

- The actual durable run completes in less than 28,800 seconds.
- Exactly 2,000,000,000 valid predicted targets, zero overshoot, hashes,
  checkpoints, and fresh-process reload all verify.
- The receipt distinguishes measured facts from interpretation and records any deviation.

STOP/loop: do not invent an arbitrary real-corpus loss-halving criterion or report a
projection as completion. On non-finite values, OOM, artifact mismatch, or an impossible
SLA, perform only the already-tested safe checkpoint/abort path, preserve a failure
receipt, and do not report partial work as completion. Suggested commit:
`docs: publish reproducible 2b-token receipt`.
