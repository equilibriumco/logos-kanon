# 6. The mixed accelerator configuration: accelerate secp256k1, leave keccak256 in software

- **Status**: accepted
- **Milestone**: M0 (measured), M1 (adopted in product code)
- **Requirements**: P1, P2, P3
- **Artefacts**: `methods/guest/Cargo.toml`, `m0/cost-baseline/README.md` (*Recommendations* 1), `m0/lez-probe/README.md`

## Context

RISC Zero publishes accelerated forks of `k256`, `crypto-bigint`, `sha2` and
`tiny-keccak`. They are adopted through `[patch.crates-io]` in a guest manifest, which
means the *same source* compiles against either the upstream crate or the accelerated
one. Nothing in `verifier-core` changes — which is what made a controlled three-way
comparison possible in the first place (ADR 3, ADR 7).

M0 measured three configurations — software, accelerated, mixed — on both bare RISC
Zero and inside a real LEZ program. The two accelerators behave very differently.

## Decision

**Mixed**: patch in the secp256k1 (`k256`, `crypto-bigint`, `sha2`) accelerators, and
leave keccak256 in software.

The secp256k1 accelerator is roughly a **20× cycle reduction with no side effects**. It
is not a judgement call and it is not optional.

The keccak accelerator is the finer one, and it is declined:

| | accelerated keccak | software keccak |
| --- | --- | --- |
| public mode, cycles per update | 1,769,222 | 1,813,434 |
| proving cost | +26.90 s, +223 KB proof per update | — |
| 6 GiB VRAM | failed to fit | proved successfully |

In public mode — where LEZ executes without proving and charges cycles — accelerated
keccak is marginally *better*, a 2.4% saving. It is declined anyway, because **LEZ runs
the same program publicly or privately**. A consumer holding private accounts has this
code proven, and there the coprocessor charge needs roughly 16 hashes to amortise
against an update's three.

Negligibly worse in public mode, decisively better in private. Software keccak is the
configuration that is never badly wrong, and "never badly wrong" is the right objective
for a configuration chosen once and inherited by every consumer.

Patches are pinned by **tag, not branch**, so what is built is what M0 measured.

## Consequences

- The product guest is built in M0's recommended configuration rather than a default,
  and the published figures describe the shipping configuration.
- A dropped or moved patch tag silently restores *software* recovery — a 20× cycle
  regression — in a green build. M0's *Recommendations* item 2 asks CI to guard this on
  two metrics: a cycle ceiling on one recovery catches losing secp256k1, and a
  proof-size ceiling on a 3-of-N update catches accidentally *gaining* keccak. Those
  land with M1-25 and are not in place yet.
- The three guest workspaces under `m0/` must keep their distinct patch sets, which is
  the direct reason guests are excluded from the parent workspaces (ADR 7). Folding
  them in would collapse the three configurations into one and destroy the comparison.
- If LEZ ever charges for proving, or the private-mode assumption changes, this is the
  decision to revisit — the measurement to revisit it with is already in `m0/`.

## Alternatives considered

- **Fully accelerated.** Rejected: 2.4% better in public mode, materially worse in
  private, and it failed to fit in 6 GiB of VRAM where mixed proved successfully.
- **Fully software.** Rejected outright: forgoing the secp256k1 accelerator is a 20×
  cycle cost on the path that already dominates the program at 88.97%.
- **Per-deployment choice.** Rejected: LEZ runs the same program in both modes, so
  there is no deployment-time moment at which the choice could be made correctly.
