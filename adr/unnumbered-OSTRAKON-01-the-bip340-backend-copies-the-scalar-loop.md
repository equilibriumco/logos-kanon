# [OSTRAKON-01]. The BIP-340 backend copies the scalar loop rather than sharing it

- **Status**: accepted
- **Milestone**: none. Ostrakon measurement work on top of the delivered recovery backend
- **Requirements**: none
- **Artefacts**: `verifier-core/src/backend/bip340.rs`, `verifier-core/src/backend/fast_recovery.rs`, `bip340-bench/`

## Context

BIP-340 verification computes `R = s·G − e·P`. Recovery computes
`Q = r⁻¹·(s·R − z·G)`. Both end in a sum of two scalar multiples, one against
the generator and one against a point the signature supplies, and
`fast_recovery` already drives that sum with a GLV split, four wNAF digit
streams, a compile-time generator table and a runtime table for the variable
point. It also already carries the addition-chain square root that BIP-340's
`lift_x` needs.

So the arithmetic is shared in substance, and the obvious structure is to share
it in code: lift the interleaved loop and the runtime table out of
`recover_signer` into two crate-visible functions, and have both verifiers call
them.

That was tried first, and it costs cycles. Extracting the loop moved the
recovery row of `methods/tests/cost.rs` from 109,835 to 110,631, about 0.7%,
and failed three exact-equality guardrails along with the figures `COSTS.md`
publishes. `#[inline(always)]` on the extracted functions did not restore the
original codegen; it made the row 112,607, worse than leaving the decision to
the optimiser.

A cycle count is a property of a compiled ELF. A function boundary is therefore
not free, and in a loop that runs about 130 iterations of circuit calls per
verification it is not small either.

## Decision: the loop and the runtime table are written out in both modules

**`bip340.rs` carries its own copy of the interleaved double-and-add and its
own runtime table construction. `fast_recovery.rs` is unchanged except for
widening the visibility of the primitives both use.**

The shared items are the ones where sharing costs nothing: `Pt` and its
circuit operations, `decompress`, `split_lambda`, `wnaf`, `Stream`, the
generator tables, the constants. Those are already function boundaries in the
delivered code, so exposing them moves no figure. The diff to
`fast_recovery.rs` is 28 lines, every one of them a visibility keyword.

This follows what the module already does with `ORDER`, duplicated from
`r0crypto`'s copy "on the same argument: the backends must be able to diverge
without dragging each other", and with `normalise_recovery_id`, "deliberately
duplicated from the siblings, as theirs are from each other". Duplication
between backends is this file's existing convention, for the same reason.

*Rejected: share the helpers and re-baseline the guardrails.* The regression
would be real and would be carried by delivered product code, to tidy a module
that has nothing to do with RedStone. The guardrails exist to catch exactly
this, and silencing them is the one response that wastes them.

*Rejected: fork `fast_recovery.rs` wholesale into a Schnorr copy.* It removes
the visibility diff at the cost of duplicating the GLV constants, the square
root chain and the point arithmetic, which is where a transcription error would
be both easy and invisible.

## Consequences

- **The duplicated region is about forty lines**, and it is the region most
  likely to be "cleaned up" by a later reader. This ADR is the record of why it
  should not be.
- **The two copies can diverge, and may need to.** The windows are tuned for
  recovery's half-scalars; nothing here establishes that 4 and 5 are the right
  pair for BIP-340, and the figure this work produced was measured with them
  inherited rather than chosen.
- **Enabling BIP-340 changes no delivered figure.** The module is behind the
  `bip340` feature, off by default, and the measurement lives in its own package
  with its own guest workspace and lockfile, because the reference arm needs
  `k256`'s `schnorr` feature and turning that on in `methods/guest` shifts the
  shared `k256` codegen on its own.
- **The same separation is repeated once more, for the same reason.**
  `m0/v0.3_rerun/lez-probe/methods-multisig` measures the approval check inside
  a LEZ program, and carries its own floor and plumbing guests rather than
  reusing `methods-mixed`'s: a subtraction holds only when both terms come out
  of one dependency graph, and the RedStone figures in `FINDINGS.md` are
  asserted by exact equality. Its floor comes out at 14,038 gas, the figure
  that document publishes, which is the check that the separation cost
  nothing.
