# Decision Ledger v5

Status: **ACCEPTED** under Simplified Implementation Mode (owner direction, 2026-08-18).

Ledger version: `content-bearing-parquet-source-v1`

This is the create-new amendment ledger for the source decision. It supersedes exactly one
row of `docs/rebuild-contract.md` and retains every other row of that contract, of
`docs/decision-ledger-v2.md`, of `docs/decision-ledger-v3.md`, and of
`docs/decision-ledger-v4.md` unchanged. Its authority is `SOURCE-001`'s own reopen trigger,
"Source-access failure plus named owner approval of an alternate", and both halves of that
trigger are satisfied and recorded below.

This is **not** a P19 amendment. `FUTURE-001` reserves P19 for a larger model, more valid
targets, or a longer time budget; this ledger changes none of those. The canonical model,
target accounting, memory floor, admission ceiling, and completion SLA are unchanged, and
so is every numerical decision in v3 and the native-code boundary set in v4.

Simplified Implementation Mode disables sealed runs, receipt chains, approval records,
acceptances, and pointers, so this ledger is an ordinary committed document. `SKIPPED`
remains a workflow decision and never means `PASS`, `APPROVED`, `tuple_qualified`, or
`full_run_qualified`.

## Superseded Decisions

| ID | v5 decision | Rationale | Reopen trigger |
|---|---|---|---|
| `SOURCE-002` (supersedes `SOURCE-001`) | The primary source is governed content-bearing Parquet: the BigCode Stack v1 deduplicated Python subset (`bigcode/the-stack-dedup`, `data/python`), whose shards carry the source text itself together with its repository, path, revision, and detected-licence metadata. `SOURCE-001`'s Stack v2 metadata plus authorized Software Heritage content remains an implemented and preferred route whenever that access exists, and the adapter for it is retained rather than removed. Every other source rule is unchanged and unrelaxed: shards are hash-bound before they are read, the frozen `PERMISSIVE_LICENSES` allowlist decides admission, the `1,000,000`-byte document ceiling applies to decoded bytes, each row's content is verified against the blob identity its own metadata declares, and publication stays create-new. This substitution is explicit and recorded here; it is never inferred, defaulted to, or silently applied. | `SOURCE-001` names content-bearing Parquet as the designated alternate precisely for this case, and both halves of its trigger are met. **Source-access failure:** the Stack v2 content path requires a bulk-access agreement with Software Heritage and INRIA, an AWS account for a requester-pays bucket, and AWS SigV4 request signing that the acquisition client does not implement — measured, not assumed, by fetching the real shard and reading the dataset's own terms. **Named owner approval:** granted 2026-08-18. The alternate also removes roughly 1.45 million sequential blob transfers, which at ten per second would run about forty hours, and it does so without weakening a single verification: the content-bearing rows carry the same licence vocabulary and the same blob identity, so the hash chain from a pinned shard to a published document is unbroken. | Restored Software Heritage bulk access, evidence that the content-bearing shards disagree with the identifiers they declare, or any proposal to admit a source that is not hash-bound end to end. |

## Identity and Change Propagation

Any byte change to this file creates a new ledger identity.

`SPAN-001` derives its sampler seed from the frozen-decision byte range of
`docs/rebuild-contract.md`, delimited by `## Frozen Decision Ledger` and
`## Deferred Qualification Facts`. That file is immutable historical evidence and is not
modified by this amendment, so **the span seed is unchanged**.

No token corpus, span generation, checkpoint, or receipt exists yet, so no materialized
artifact is invalidated and no rerun is owed. The corpus this decision produces will be the
first one.

Downstream identity effects:

| Consumer | Effect |
|---|---|
| Source generations | Produced by the content-bearing adapter; `adapter_namespace` distinguishes them from the Software Heritage route |
| `curate` and everything after it | Unchanged; the adapter ends at the same `MaterializedSourceManifestV1` contract |
| `SPAN-001` span order | Unchanged; the seed operand is not this file |
| `SCOPE-002` | Unchanged; the Zstandard decoder admitted in v4 is what makes these shards readable too |
| `PRECISION-002`, `MODEL-001-V2`, `MEMORY-001`, `ADMISSION-001`, `SLA-001`, `CKPT-001` | Unchanged |

## Requalification

`SOURCE-001`'s reopen trigger requires source-access failure plus named owner approval,
both recorded above. Under Simplified Implementation Mode the supporting evidence is
automated rather than a sealed receipt:

- the full quality gate passes with the content-bearing adapter present;
- a content-bearing shard projects, filters, and publishes against fixtures, including the
  cases that must fail: a row whose content disagrees with its declared identity, a licence
  outside the frozen allowlist, and a document over the frozen ceiling;
- the published generation is consumed unchanged by `curate`, which is what establishes
  that the alternate ends at the same contract the primary route does.

Hardware qualification, performance admission, the completion SLA, and full-run evidence
remain `UNVERIFIED`, and manual qualification remains `SKIPPED`.
