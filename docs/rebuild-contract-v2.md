# Rebuild Contract v2

Status: **PENDING_P0A_ACCEPTANCE**

Contract version: `rebuild-contract-v2`

Design: `prototype-first-portable-interface-v1`

This document is a prospective, create-new amendment to
`docs/rebuild-contract.md`. It does not modify, replace, reinterpret, or validate any
historical P0, P1A, P1B, or P2 byte. Until a selected P0A acceptance binds this file,
`docs/decision-ledger-v2.md`, the portable-interface ADR, and the revised architecture,
the signed v1 P0 receipt remains the active product authority. A conflict is a stop
condition.

After P0A acceptance, this document governs the prototype-first execution boundary.
Every v1 decision not explicitly amended by `docs/decision-ledger-v2.md` remains in
force. The exact v1 contract and ledger hashes remain historical inputs to the v2
authority chain.

## Product Boundary

The first complete implementation and two-billion-target run use the sole pre-P16A
profile `prototype-windows-5090-v1`: native Windows x86_64, AMD Ryzen 9 9950X3D,
Rust target `x86_64-pc-windows-msvc`, the qualified VS 2022 x64 MSVC/Windows SDK
toolchain, one NVIDIA GeForce RTX 5090 at compute capability 12.0/SM120, dedicated VRAM,
and CUDA. The implementation must nevertheless expose provider-neutral Rust interfaces
from the first rebuild commit so later portability does not require changing product,
data, model, accounting, checkpoint, or receipt semantics.

The canonical product remains one installed `python-slm` executable with independently
invocable `plan`, `curate`, `train-tokenizer`, `tokenize`, `inspect`, `bench`, and `train`
subcommands. Developer qualification is owned by a non-installed Rust `xtask` binary.
Public argv, configuration schemas, stdout/stderr contracts, exit categories, artifact
formats, hash rules, atomic publication, and role-ledger arithmetic are provider-neutral.

Support claims use four distinct levels:

1. **Designed**: a provider-neutral interface and closed contract exist.
2. **Implemented**: the exact host or accelerator adapter exists on a named source tree.
3. **Tuple-qualified**: a selected immutable receipt passes for one exact OS, host target,
   toolchain, provider, SDK/runtime/driver, device, architecture, memory model, backend,
   and native-library tuple.
4. **Full-run-qualified**: that exact tuple has its own accepted two-billion-target durable
   run.

No lower level implies a higher one. No receipt proves an unlisted Cartesian combination
of operating systems and accelerator providers. Until P16A is accepted, selecting Linux,
macOS, ROCm/HIP/AMD, or Metal/Apple Silicon fails before provider discovery or persistent
mutation with stable capability code `DEFERRED_POST_P16`; it never falls back to the
prototype or reports stub success. The code names the post-P16 capability class; P16A is
the earliest authority that permits executing its implementation phases.

## Zero-Python and Native-Code Boundary

No Python interpreter, executable, package, module, wheel, build backend, code generator,
embedded runtime, or Python-launched subprocess may participate in build, curation,
tokenization, storage, training, qualification, verification, or receipt publication.
Python-language source is input data only. Rust owns CLI orchestration, process control,
general data transformation, model/trainer control, verification, and publication.
Historical `.ps1` capture files remain byte-for-byte archival evidence only and are never
invoked as active entry points; Rust `xtask` owns every active verification flow.

Native code is allowed only in these narrow, pinned, audited, feature-gated boundaries:

- provider kernels, standard native ML libraries, and CUDA, HIP/ROCm, or Metal bridges;
- `tree-sitter-python 0.25.0` and its generated C parser/runtime, used only through the
  Rust data-policy boundary for the frozen `SOURCE-002`, `SOURCE-003`, `DEDUP-001`, and
  `DECONTAM-001` CST, comment, and lexical-token semantics.

The Tree-sitter bundle must pin and hash the grammar source, generated C and any generated
scanner, runtime source/version, ABI, normalized per-host build flags, and a compatibility
corpus with expected parse, comment-range, and lexical-token outputs. Parser generation
at build or runtime is forbidden. C or C++ may not own orchestration or unrelated data
transformation. A parser-bundle or parser-derived output change creates a new artifact
identity and explicitly propagates through affected source-policy decisions, lexical
tokens, deduplication, decontamination, splits, tokenizer sampling, token corpora, sampler
seeds, checkpoints, and receipts.

CPU/data-only builds must discover and link no accelerator SDK, accelerator runtime,
native ML backend, or provider kernel. Host-specific mechanisms remain behind internal
Rust adapters. A platform shell script is never a normative entry point.

## Canonical Model

The canonical preset is `gqa-135m-v1` and has exactly `135,285,504` trainable parameters:

| Component | Arithmetic | Parameters |
|---|---:|---:|
| Token embedding | `32,000 * 768` | 24,576,000 |
| Untied LM head | `32,000 * 768` | 24,576,000 |
| Per-layer attention | `768*768 + 2*(768*256) + 768*768` | 1,572,864 |
| Per-layer SwiGLU | `3*(768*2,432)` | 5,603,328 |
| Per-layer norms | `2*768` | 1,536 |
| Twelve decoder layers | `12*(1,572,864 + 5,603,328 + 1,536)` | 86,132,736 |
| Final norm | `768` | 768 |
| **Total** | | **135,285,504** |

It is bias-free, pre-normalized, width 768, FFN width 2,432, 12 layers, 12 query heads,
4 KV heads, head width 64, context 2,048, and has an untied output head. The
`gqa-124m-ref-v1` preset remains reference-only at exactly `124,668,672` parameters.
Changing the canonical parameter count or shape requires the optional P19 amendment; it
is not portability work.

## Accelerator Memory Compatibility Gate

Let:

```text
P = 135,285,504
Q = 256 * 1024 * 1024 = 268,435,456 bytes
raw_training_state_floor = 20 * P = 2,705,710,080 bytes
align_up(n, q) = ((n + q - 1) / q) * q, using exact integer arithmetic
minimum_accelerator_bytes = align_up(raw_training_state_floor, Q)
                          = 2,952,790,016 bytes
```

The `20*P` factor is a conservative compatibility floor, not a claim about the final
allocator layout or sufficient production memory. Environment qualification must perform
a fresh-process allocation of at least `2,952,790,016` accelerator-visible bytes on the
selected device, synchronize a sentinel operation and round trip, release the allocation,
and return to the recorded baseline. Dedicated and unified memory use the same logical
minimum but are reported separately. Advertised capacity, a compile check, or an
unsynchronized allocation is not evidence of passing.

## Fixed Training SLA

The contractual completion SLA remains fixed at `28,800` seconds. The admission ceiling
is fixed at `25,920` seconds, exactly 90 percent of the completion SLA. Neither value is
derived from a device roofline and neither may be retuned after observing a run.

After corpus materialization, the SLA clock starts immediately before the trainer opens
the frozen artifacts. It includes artifact re-verification, startup, model initialization,
run-time compilation or autotuning, data loading, forward/backward/optimizer work,
configured evaluation, synchronization, checkpointing, final durable save, and any
recovery downtime, host suspend/system sleep, and resumed-execution downtime. It is a
suspend-inclusive monotonic wall clock, not active-process CPU time, and stops only after
the final checkpoint is durable.

Before the P16 launch, a synchronized whole-run-equivalent projection must be no greater
than `25,920` seconds. The projection includes measured or count-scaled startup,
compilation/autotuning, loader stalls, synchronization, configured held-out evaluation,
planned checkpoints, recovery testing overhead where the contract charges it, and final
durable save. With `N = 2,000,000,000`, freeze exactly five fresh-process samples per
overhead class and compute:

```text
O_bound =
    max(startup + verification + initialization + JIT/autotune)
  + evaluation_count * max(evaluation)
  + nonfinal_checkpoint_count * max(nonfinal_checkpoint)
  + max(final_durable_save)

R_required = N / (25,920 - O_bound)
```

Require `O_bound < 25,920`, `R_qual >= R_required`, and
`ceil(N / R_qual + O_bound) <= 25,920`. Compute throughput, memory bandwidth,
operational intensity, roofline classification, and efficiency as synchronized diagnostic
evidence. No 85-percent bandwidth rule or other relative-efficiency target is an
independent PASS gate. A projection is only an admission gate. P16 passes the time gate only when
the actual continuous durable elapsed time is no greater than `28,800` seconds.

Completion remains exactly `2,000,000,000` valid predicted training targets, zero
overshoot, and a durable final checkpoint. The `2,000,000,001`-ID prefix, unused-tail,
unmaterialized-document, padding, boundary, update, evaluation, and checkpoint counters
continue to follow the v1 role-ledger rules.

## Prototype and Portability Phase Boundaries

### P1 through P16: Windows/NVIDIA prototype

P1 through P16 implement and qualify the first full product only on
`prototype-windows-5090-v1`. Provider-neutral interfaces and isolated
`cuda`, `rocm`, and `metal` feature boundaries are required, but P16 makes no claim that
Linux, macOS, AMD, or Apple adapters are implemented or qualified.

P16 is a technical execution receipt. It proves the exact source and tuple completed the
contracted target count, accounting, checkpoint, provenance, and `28,800`-second SLA. P16
does not by itself accept model quality and does not unlock portability or a model/budget
change.

### P16A: prototype quality acceptance

P16A depends on an accepted P16 technical receipt. Before P15 begins, freeze and hash one
complete P16A quality pack without changing model or training bytes. That pack contains:

- the immutable held-out validation/test sample manifest and exact aggregate-loss and
  perplexity calculation;
- the initialized-model checkpoint identity and its frozen aggregate held-out loss;
- the frozen unigram model definition, artifact identity, and aggregate held-out-loss
  baseline;
- the exact quantitative predicate: final-checkpoint aggregate held-out loss is finite and
  strictly below both the initialized-model aggregate loss and the frozen unigram aggregate
  loss, and final-checkpoint aggregate held-out perplexity is finite;
- the immutable qualitative prompt/sample pack, deterministic generation settings,
  output schema, scorer or owner-review rubric, and expected receipt fields.

Before P15 begins, a create-new named owner approval binds the exact quality-pack,
prompt/sample, decoding, output-schema, and rubric hashes, with identity,
signature/reference, and UTC timestamp. This artifact approval is distinct from the later
owner decision on the measured P16A result.

After P16, P16A binds the selected P16 receipt, fresh-process final-checkpoint reload,
machine evidence for the exact aggregate quantitative predicate, generated outputs for
the frozen qualitative pack, and a named owner's explicit quality decision, identity,
signature or review reference, and UTC timestamp.

No pack byte, baseline, aggregation rule, predicate, prompt, generation setting, rubric,
or exclusion may be selected or changed after P15 begins.
Machine evidence, an agent audit, or silence cannot substitute for owner approval. P16A
is the sole unlock for P17 and optional P19.

### P17: host and data portability

P17 depends on P16A. It implements and qualifies native Linux and macOS host/data adapters
while preserving a Windows regression on one frozen source tree. The minimum host matrix
is Windows x86_64 MSVC, Linux x86_64 GNU, and macOS arm64 Apple. Each profile runs the
same locked CPU/data quality gate and complete synthetic pipeline, including Tree-sitter
compatibility, Python canaries, path containment, mutation detection, atomic publication,
crash recovery, and cleanup.

Provider-independent data outputs must be byte-identical across profiles: accepted IDs and
reasons, canonical bytes, lexical tokens, dedup/decontamination decisions, splits,
tokenizer artifacts, indexes, shards, manifests, and their hashes. Host/toolchain evidence
is tuple-specific and is not expected to be byte-identical. P17 is not accelerator
qualification and does not reinterpret the P16 full-run receipt for the post-P17 source
tree.

### P18: exact accelerator-provider tuple matrix

P18 depends explicitly on both P16A and P17. Before measurement, it freezes an exact,
owner-reviewed tuple matrix including device stable identities and versions. All four
lanes are mandatory:

- a Windows/NVIDIA CUDA regression on the final P18 source tree;
- one Linux/NVIDIA CUDA tuple;
- one Linux/AMD ROCm/HIP tuple;
- one macOS arm64/Apple Silicon Metal tuple.

AMD on Windows and other unlisted Cartesian combinations are not implied.

Every required tuple independently passes environment/native probe, Python-free dependency
closure, backend selection, canonical initialization load, literal CPU-reference gradient
byte equality, deterministic state export/import, byte-identical interrupted resume,
provider-appropriate staging or unified-memory access, synthetic end-to-end, cleanup,
synchronized performance calibration, and the qualification ladder. P18 proves only
tuple-level implementation, correctness, deterministic resume, and qualification. It does
not prove performance equivalence, cross-provider checkpoint migration, or a two-billion-
target run on AMD or Apple.

### P19: optional larger-model or longer-budget amendment

P19 is optional and depends directly on P16A, not P17 or P18. It is the only authorized
path to increase model size, target count, or time budget. P19 requires a create-new
contract version, decision ledger, architecture/ADR impact analysis, named owner approval,
new model/memory/SLA/accounting identities, and rerun of every affected qualification and
training phase. It is not a portability phase and is not a portable-code P16 rerun.

## Receipt and Approval Governance

Historical receipt paths, schemas, runs, acceptances, pointers, and seals are immutable.
New phases use create-new, closed schemas and namespaces; no new schema may reinterpret an
old field or acceptance.

Every selected receipt binds its exact command argv, working directory, source/tree and
dirty state, `Cargo.lock`, contract and decision-ledger hashes, ADR and architecture hashes,
schema bundle, parser bundle where relevant, configuration and artifact hashes, exact host
and accelerator identities, transcripts, exit codes, gates, cleanup, and run seal. Failed
and superseded runs remain immutable. A failed run never advances a pointer.

P0A requires separate named technical and data-governance decisions with identities,
explicit `APPROVE` or `REJECT`, signatures or review references, and UTC timestamps. One
person may fill both roles only when their dual authority and two distinct decisions are
recorded. P16A separately requires the named quality-owner approval described above.

No checklist edit, receipt, or machine output is self-approving. Any later decision change
requires the applicable owner-approved amendment, a new identity, and rerun of the owning
phase plus affected downstream gates.

## Preserved Decisions

Unless `docs/decision-ledger-v2.md` explicitly says otherwise, all v1 decisions remain
unchanged, including:

- source authorization, provenance, removals, licensing, and sensitive-data policy;
- strict encoding, comment/generated-marker, Tree-sitter CST, and lexical-token semantics;
- deterministic custom byte-level BPE and special IDs;
- exact deduplication threshold, 256-component MinHash, 32-by-8 LSH, exhaustive final
  Jaccard check, split isolation, and decontamination;
- cryptographic hash chains, immutable artifact publication, path containment, and role
  ledgers;
- model arithmetic, initialization, parameter names, optimizer, schedule, packed spans,
  evaluation cadence, checkpoint contents, retention, exact gradient bytes, and
  byte-identical resume.

Portability is not permission to relax any preserved semantic or integrity gate.
