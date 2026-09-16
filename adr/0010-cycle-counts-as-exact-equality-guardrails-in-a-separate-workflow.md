# 10. Cycle counts are asserted by exact equality, in a workflow of their own

- **Status**: accepted
- **Milestone**: M0 (guardrails), M1 (CI split)
- **Requirements**: P1, P2, P3, S2
- **Artefacts**: `.github/workflows/guardrails.yml`, `.github/workflows/ci.yml`, `m0/*/host/tests/guardrails.rs`

## Context

RFP-020 Performance 2 makes cost measurement a **primary deliverable**, "reproducible
from the test suite". A report full of numbers that nothing checks is a report that
becomes wrong quietly: cycle counts are a deterministic function of the guest ELF and
its input, so any toolchain or dependency change moves them, and nothing about an
ordinary green build would say so.

There is also a signalling problem. A failing cycle assertion and a failing unit test
mean completely different things. The first says *re-measure and decide*; the second
says *the code is wrong*. If both arrive as "CI is red", the first gets treated like the
second, and the usual response — make the test pass — is exactly the wrong one.

## Decision

**Every published figure is asserted by exact equality**, not by a tolerance band, in
one of the two guardrail suites, and both run in CI. Twelve assertions cover recovery
cycles, keccak cycles per hash, full 3-of-N update cycles, the accelerated
configuration, proof sizes and the budget-headroom claim.

A guardrail failure is **a prompt to re-measure and re-tag deliberately, not necessarily
a defect**. That is the stated contract, and it is why the assertion is exact: the job
of the test is to notice movement, and a tolerance band is a decision to not notice
small movement, taken in advance without knowing what caused it.

**CI is split into two workflows so a red tick means one thing rather than two.**

| workflow | question | a failure means |
| --- | --- | --- |
| `guardrails.yml` | did the measured cycle counts move? | re-measure |
| `ci.yml` | does the code work? | the code is wrong |

`ci.yml`'s jobs are `lint`, `build-test`, `no-std`, `guest`, `licenses`, `traceability`
and `sequencer` — each gating one thing, so the failing job names the problem.

**Guardrails run on x86_64 only.** M0-11 established that the figures are architecture
independent, across three machines and two architectures, and the report records it.
ARM64 is not a delivery target, so that finding is not re-confirmed on every push —
and `deny.toml`'s `aarch64-apple-darwin` target was removed for the same reason, with a
comment saying it should come back the same day ARM64 does.

The proving figures are `#[ignore]`d and run on demand: they take minutes to hours, and
proving wall-clock is noisy enough that a single run can order two configurations
wrongly.

## Consequences

- P2's "reproducible from the test suite" is satisfied the strict way rather than by
  pointing at a document.
- Any toolchain, dependency or codegen change that moves a figure fails loudly. That
  property is what made the risc0 3.0.5 experiment in ADR 8 conclusive rather than
  hopeful.
- Guardrail failures will happen on legitimate upgrades. The procedure is written down
  (`m0/versions.md`, *Changing any of this*): bump, run both suites, and update the
  constants, the affected report tables and that file in the same commit. A published
  figure must never move without the table that quotes it.
- The split means a contributor must know that `cargo test --workspace` at the root does
  not run the guardrails (ADR 7).
- ARM64 regressions would not be caught. Accepted, and reversible the day ARM64 becomes
  a target.

## Alternatives considered

- **Tolerance bands (±1%).** Rejected: it decides in advance not to notice a class of
  change, and the 0.01% figure movement from a mismatched patch release (ADR 8) is
  exactly what a band would hide.
- **One CI workflow.** Rejected on signal quality: the two failures call for opposite
  responses.
- **Publishing figures without asserting them.** Rejected — that is the thing P2 asks
  not to do.
