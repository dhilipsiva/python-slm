# ADR 0001 — Accelerator Numerical Conformance

Status: **ACCEPTED** under Simplified Implementation Mode (owner direction, 2026-08-16).

Supersedes the gradient-comparison clause of `PRECISION-001` in
`docs/decision-ledger-v2.md`. Introduces `PRECISION-002` in `docs/decision-ledger-v3.md`.
Creates no sealed run, receipt chain, approval record, acceptance, or pointer, because
`TODO.md` Simplified Implementation Mode disables that machinery for every remaining phase.

## Context

`PRECISION-001` requires that every parameter-gradient artifact equal the canonical
IEEE-754 reference bytes produced by the P9B scalar oracle, and forbids "gradient
tolerance, reassociation, contraction, candidate-specific comparison, and
nondeterministic reduction". `docs/ARCHITECTURE.md` states that no tolerance waives
exact gradients.

That requirement had never been executed. Required CI compiles the `cuda` feature but
does not run it, and the manual hardware workflow — which had never been dispatched —
ran only test paths that build their observation by copying the oracle's own gradient
bytes into the comparison, making the assertion a tautology that cannot fail on device
behavior. The same blind spot had already produced a real defect: the P2 fixture's
expected gradient constants were wrong and were corrected during E1 by running them.

Executing the parity fixture on the prototype RTX 5090 for the first time showed the
device reproducing the oracle's forward BF16 logits and FP32 loss exactly while its
gradients diverged.

## Measurements

`tests/e1a_numerical_probe.rs` isolates each candidate mechanism on the RTX 5090.

Contraction and reduction order are **exonerated**. Elementwise multiplication matches
the host on 64 of 64 operands. Device summation and `matmul` both reproduce host
left-to-right accumulation exactly at widths 2, 4, 8, and 64. Fused multiply-add and
tree reduction are therefore not the cause.

Transcendental implementations are the **sole** cause. Against Rust's host libm the
device differs by one ULP on `exp` (11 of 42 operands), `sin` (5 of 42), `cos` (2 of 42),
and `ln` (1 of 25). `sqrt` and `recip` are bit-identical. This split is exactly what
IEEE-754 predicts: it mandates correctly rounded square root and division but places no
such requirement on `exp`, `ln`, `sin`, or `cos`. Because every other primitive in the
graph is bit-exact, the transcendentals are the cause by elimination.

The resulting deviation profile tracks transcendental density along each backward path:

| Artifact | Worst relative deviation |
|---|---|
| `blocks.0.attn.q.weight` (through RoPE `sin`/`cos` **and** softmax `exp`) | `2.418e-4` |
| `blocks.0.attn_norm.weight` | `9.903e-5` |
| `tok_embeddings.weight` | `1.705e-5` |
| `blocks.0.ffn.gate.weight` | `3.259e-7` |
| `lm_head.weight` (adjacent to the loss) | `1.083e-7` |

Aggregate: relative L2 `5.714e-6`, cosine similarity `0.999999999984`.

The forward remains exact because every frozen BF16 storage point quantizes to eight
mantissa bits and absorbs a one-ULP FP32 difference. Gradients are stored raw FP32 with
an identity cast, so the same difference survives to the output. Forward parity is
therefore evidence that quantization hides the difference, not that the arithmetic is
identical.

## Decision

Split `PRECISION-001`'s single requirement, which conflates two different guarantees,
and amend only the one that is unattainable.

| Guarantee | Disposition |
|---|---|
| Device forward (BF16 logits, FP32 loss) equals the oracle exactly | **Retained as a hard gate** — passing on hardware |
| Device gradients equal the oracle byte for byte | **Replaced** by a predeclared provider-independent bound |
| Device repeated execution byte-identical to itself | **Retained at full strength** — proven on hardware |
| Byte-identical resume (`CKPT-001`) | **Untouched** — device-versus-itself, unaffected |

The bound reuses the already-frozen, provider-independent values in
`docs/schemas/P2/python-slm-backend-qualification-policy-v1.schema.json`:
`gradient_relative_l2_max 0.03`, `gradient_cosine_min 0.999`, an envelope of
`absolute_floor 0.0078125` and `reference_multiplier 0.03125`, with `nan_max 0`,
`infinite_max 0`, and `envelope_violations_max 0`. Those numbers predate and are
independent of the measurements above, which is what "predeclared" and "not
candidate-specific" require. Inventing a bound now, after observing `5.714e-6`, would be
post-hoc tuning of a gate to its own result.

## Alternatives rejected

**Engineer to bit-exactness.** Would require hand-written `exp`, `ln`, `sin`, and `cos`
that reproduce Rust's libm bit for bit on every operand. Rust's transcendentals are
themselves platform libm calls with no cross-platform bit-stability guarantee, so the
target is not fixed; `PORT-ACCEL-001` already requires literal gradient bytes on Linux
and macOS lanes, where the reference itself may move. This does not converge.

**Re-derive the oracle from a device run.** Restores byte equality trivially but bakes
CUDA arithmetic into the contract, destroying the provider independence that the P18
ROCm and Metal lanes exist to test.

**Loosen determinism instead.** Rejected outright. Determinism is the property that
protects resumable training, it is device-versus-itself, and it already passes on
hardware. It is not touched.

## Consequences

- The oracle remains authoritative for **semantics**. A bit-exact forward still catches a
  wrong causal mask, RoPE pairing, GQA head mapping, or loss normalization, which is what
  the oracle exists to detect.
- The accelerator conformance claim becomes honest: bounded numerical agreement with a
  provider-independent reference, plus byte-exact self-determinism and resume.
- Affected result schemas take create-new v2 versions rather than reinterpreting a v1
  field, per `RECEIPT-002`.
- `SPAN-001` seeds from the decision-ledger frozen byte range (`src/corpus.rs`), so a new
  ledger changes the training span order. No corpus exists yet (E3 has not started), so
  this costs nothing now and would have invalidated a materialized corpus later.
- The P9B oracle test joins the portable three-host CI lane, so cross-host stability of
  the reference itself becomes measured rather than assumed.

## Reopen trigger

A new precision semantic version, a change to the frozen bound, or evidence that the
oracle is not bit-stable across the required host matrix.
