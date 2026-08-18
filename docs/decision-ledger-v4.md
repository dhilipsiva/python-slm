# Decision Ledger v4

Status: **ACCEPTED** under Simplified Implementation Mode (owner direction, 2026-08-18).

Ledger version: `data-lane-parquet-codec-v1`

This is the create-new amendment ledger for the data-lane native-code decision. It
supersedes exactly one row of `docs/rebuild-contract.md` and retains every other row of
that contract, of `docs/decision-ledger-v2.md`, and of `docs/decision-ledger-v3.md`
unchanged. Its authority is `SCOPE-001`'s own reopen trigger, "Only an owner-approved
contract revision", which the owner granted on 2026-08-18 after the conflict was measured
rather than predicted.

This is **not** a P19 amendment. `FUTURE-001` reserves P19 for a larger model, more valid
targets, or a longer time budget; this ledger changes none of those. The canonical model,
target accounting, memory floor, admission ceiling, and completion SLA are unchanged, and
so is every numerical decision in v3.

Simplified Implementation Mode disables sealed runs, receipt chains, approval records,
acceptances, and pointers, so this ledger is an ordinary committed document. `SKIPPED`
remains a workflow decision and never means `PASS`, `APPROVED`, `tuple_qualified`, or
`full_run_qualified`.

## Superseded Decisions

| ID | v4 decision | Rationale | Reopen trigger |
|---|---|---|---|
| `SCOPE-002` (supersedes `SCOPE-001`) | Windows-native Rust control plane; no Python interpreter, package, or subprocess in build, data, training, or qualification. Native code is limited to three named boundaries and nothing else: feature-gated CUDA/C ABI accelerator backends; the pinned `tree-sitter-python 0.25.0` generated C parser/runtime, reached only through the Rust data-policy boundary; and the Parquet Zstandard decoder (`zstd-sys`), reached only through the Rust Parquet reader in `src/stack.rs` and used only to decompress governed metadata shards. The Zstandard boundary is decode-only, owns no orchestration and no data transformation, and every byte it produces remains subject to the same digest verification as any other input. No further native dependency may be added to the data lane without a new owner-approved revision. | The Stack v2 Python metadata that `SOURCE-001` names as the primary source is Zstandard-compressed Parquet, measured on the real shard rather than assumed: `train-00000-of-00009.parquet` fails to decode with "Disabled feature at compile time: zstd". HuggingFace's auto-converted branch is byte-identical to the main branch, so no supported-codec copy of the same data exists, and `parquet-rs` wires only the C-backed `zstd` crate with no pure-Rust backend in any published version. The alternatives were worse against this repository's own priorities: an alternate source is itself a named-owner substitution under `SOURCE-001` and abandons the contract-primary source; a hand-integrated pure-Rust decoder places unaudited decompression under the corpus hash chain; and transcoding outside the product would break the chain from the published dataset to the corpus, which is the property the repository exists to preserve. The data lane already compiles C for the Tree-sitter parser, so this adds a second pinned C dependency to a lane that already requires a C toolchain rather than introducing a new class of requirement. | A pure-Rust Zstandard backend usable by the Parquet reader, a source whose metadata does not require it, or any proposal to widen the data lane beyond these three boundaries. |

## Identity and Change Propagation

Any byte change to this file creates a new ledger identity.

`SPAN-001` derives its sampler seed from the frozen-decision byte range of
`docs/rebuild-contract.md`, delimited by `## Frozen Decision Ledger` and
`## Deferred Qualification Facts`. That file is immutable historical evidence and is not
modified by this amendment, so **the span seed is unchanged** and no planned span order is
superseded. This ledger records a build-surface decision, not a frozen-decision-range one.

`PROV-001` records the `Cargo.lock` SHA-256 in every final receipt, and enabling the
Parquet `zstd` feature changes it. No token corpus, span generation, checkpoint, or receipt
exists yet, so no materialized artifact is invalidated and no rerun is owed.

Downstream identity effects:

| Consumer | Effect |
|---|---|
| `Cargo.lock` identity | New digest; any future receipt records the post-amendment value |
| Data-lane build surface | Adds `zstd`, `zstd-safe`, `zstd-sys`; requires a C toolchain, as the Tree-sitter parser already did |
| `SPAN-001` span order | Unchanged; the seed operand is not this file |
| `SOURCE-001` | Unchanged; this amendment is what makes its primary source readable |
| `PRECISION-002`, `MODEL-001-V2`, `MEMORY-001`, `ADMISSION-001`, `SLA-001`, `CKPT-001` | Unchanged |

## Requalification

`SCOPE-001`'s reopen trigger requires only an owner-approved revision, which this is. Under
Simplified Implementation Mode the supporting evidence is automated rather than a sealed
receipt:

- the full quality gate passes on the post-amendment build surface;
- the accelerator-free surfaces still compile, so the codec is confined to the data lane
  and has not leaked into an accelerator feature;
- the Stack v2 metadata shard decodes and projects, which is the fact the amendment exists
  to obtain and the one that would make it pointless if it failed.

Hardware qualification, performance admission, the completion SLA, and full-run evidence
remain `UNVERIFIED`, and manual qualification remains `SKIPPED`.
