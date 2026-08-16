# Decision Ledger v3

Status: **ACCEPTED** under Simplified Implementation Mode (owner direction, 2026-08-16).

Ledger version: `accelerator-numerical-conformance-v1`

This is the create-new amendment ledger for the numerical-conformance decision. It
supersedes exactly one row of `docs/decision-ledger-v2.md` and retains every other v2 and
v1 row unchanged. Its authority is `PRECISION-001`'s own reopen trigger, "New
precision/model semantic version and complete parity/resume requalification", and the
governing rationale is `docs/adr/0001-accelerator-numerical-conformance.md`.

This is **not** a P19 amendment. `FUTURE-001` reserves P19 for a larger model, more valid
targets, or a longer time budget; this ledger changes none of those. The canonical model,
target accounting, memory floor, admission ceiling, and completion SLA are unchanged.

Simplified Implementation Mode disables sealed runs, receipt chains, approval records,
acceptances, and pointers, so this ledger is an ordinary committed document. `SKIPPED`
remains a workflow decision and never means `PASS`, `APPROVED`, `tuple_qualified`, or
`full_run_qualified`.

## Superseded Decisions

| ID | v3 decision | Rationale | Reopen trigger |
|---|---|---|---|
| `PRECISION-002` (supersedes `PRECISION-001`) | Canonical storage remains BF16 for parameters/activations with the frozen FP32-sensitive accumulation and optimizer state, and the scalar BF16/FP32 oracle remains the provider-independent reference for operation, accumulation, reduction, contraction, and rounding order. Device **forward** artifacts — BF16 logits and the FP32 loss — must equal the canonical reference bytes exactly. Device **gradient** artifacts must satisfy one predeclared provider-independent bound: relative L2 at most `0.03`, cosine similarity at least `0.999`, an elementwise envelope of `absolute_floor 0.0078125` and `reference_multiplier 0.03125`, with zero NaN, zero infinite, and zero envelope violations. The bound is identical for every candidate and every provider; per-backend or post-measurement thresholds remain forbidden, as do nondeterministic reduction and any relaxation of determinism. Repeated execution on one tuple must remain byte-identical, and `CKPT-001` byte-identical resume is unchanged and unrelaxed. | Bit-exact gradient agreement with the scalar oracle is unattainable on any accelerator whose transcendental library is not the host's: IEEE-754 mandates correctly rounded `sqrt` and division but not `exp`, `ln`, `sin`, or `cos`. Measured on the prototype RTX 5090, contraction and reduction order reproduce the oracle exactly, while `exp`, `ln`, `sin`, and `cos` differ by one ULP, yielding relative L2 `5.714e-6` and cosine `0.999999999984`. The retained exact forward gate still detects every semantic error the oracle exists to catch, and determinism, which is what protects resumable training, is preserved at full strength. | A new precision semantic version, a change to the frozen bound, or evidence that the oracle is not bit-stable across the required host matrix. |

## Identity and Change Propagation

Any byte change to this file creates a new ledger identity.

`SPAN-001` derives its sampler seed from the frozen-decision byte range of the decision
ledger (`src/corpus.rs`), so adopting this ledger changes the training span order. That
seed operand change is recorded here explicitly, as `docs/decision-ledger-v2.md` requires.
No token corpus or span generation exists yet — the E3 data track has not started — so no
materialized artifact is invalidated and no rerun is owed. Any span order planned before
this ledger is superseded and may not be reused.

Downstream identity effects:

| Consumer | Effect |
|---|---|
| `SPAN-001` span order | New seed operand; must be planned against this ledger |
| `python-slm-accelerator-model-result` | Create-new v2 schema; v1 is not reinterpreted |
| `python-slm-provider-parity-result` | Create-new v2 schema; v1 is not reinterpreted |
| `MODEL-001-V2`, `MEMORY-001`, `ADMISSION-001`, `SLA-001`, `CKPT-001` | Unchanged |

## Requalification

`PRECISION-001`'s reopen trigger requires complete parity and resume requalification.
Under Simplified Implementation Mode this is the automated evidence, not a sealed receipt:

- device forward parity against the oracle, exact, on hardware;
- device gradient conformance against the frozen bound, on hardware;
- byte-identical repeated execution on one tuple, on hardware;
- byte-identical checkpoint restore and continuation, on hardware;
- the P9B oracle executed on the Windows, Linux, and macOS lanes, establishing whether the
  reference itself is bit-stable across the required host matrix.

Hardware qualification, performance admission, the completion SLA, and full-run evidence
remain `UNVERIFIED`, and manual qualification remains `SKIPPED`.
