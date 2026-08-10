# 2. One verification core, shared by both modes, in a crate layout created up front

- **Status**: accepted
- **Milestone**: M1 (`M1-01`)
- **Requirements**: F1, F2, U6, SEC1
- **Artefacts**: `Cargo.toml`, `verifier-core/`, `pull-lib/`, `aggregator-program/`, `README.md`

## Context

RFP-020 asks for two delivery modes over the same data. **Push**: an aggregator
program verifies a signed RedStone package and publishes the price into a canonical
account that consumers read. **Pull**: a consumer program verifies a payload inline,
in its own transaction, with its own signer set and its own `maxAge`.

The obvious shape is two implementations that happen to agree. The requirement is that
they cannot disagree: F2 asks for "one audited implementation backing both modes",
SEC1 for one membership check, U6 for one set of typed errors. Divergence between push
and pull on *who is an authorised signer* or *what counts as stale* is a security
defect, not a documentation problem.

## Decision

`verifier-core` is the only crate that verifies, and it **depends on neither mode**.
Both modes depend on it:

```
aggregator-program ──┐
                     ├──> verifier-core ──> backend::VerifierBackend
pull-lib ────────────┘
```

The direction is what carries the guarantee. A dependency from `verifier-core` to
either mode would let mode-specific behaviour leak into shared code; keeping the arrow
one-way means the shared path physically cannot know which mode it is running in.
`pull-lib` re-exports `verifier_core` so a consumer program pins one verification
implementation rather than two, and M3-02 asserts mechanically that the pull side
carries no aggregator dependency rather than leaving it to review.

Alongside that, the **whole crate layout was created in M1-01**, before most of it had
code: `kanon-sdk`, `kanon-relayer`, `reference-consumers/*`, `kanon-idl`, `methods/`.
Each placeholder carries a doc comment naming the tasks that fill it and the
requirement those tasks serve, and the relayer binary exits non-zero with the same
message rather than pretending to run.

## Consequences

- One audit covers both modes, which is what F2 asks for and what makes the M5 audit
  tractable.
- A reader can see from `Cargo.toml` alone that the invariant holds; it does not
  depend on anyone remembering it.
- The empty crates make the shape of the delivery legible from the first commit, and
  make each later task an edit to a named place rather than an argument about layout.
  The cost is a repository that looks more finished than it is, which is why every
  placeholder says *"Not yet implemented"* in its own doc comment and `TRACEABILITY.md`
  keeps the accurate count.
- `methods/guest` links `verifier-core` and `kanon-idl` from M1-01 onward, so the
  riscv32 cross-compile of *product* code is exercised continuously. A dependency that
  turns out not to be guest-compatible fails on the commit that adds it rather than at
  the end of the milestone. See ADR 4.

## Alternatives considered

- **Verification in the aggregator, with pull calling into the program.** Rejected:
  pull mode exists precisely so a consumer does not depend on the aggregator's state.
- **Two implementations with a shared test vector suite.** Rejected: shared vectors
  catch disagreement only on the cases that were written down, which is not the same as
  being unable to disagree.
- **Adding crates as each milestone needs them.** Rejected: it defers the layout
  argument to the moment there is least time for it, and hides the delivery shape.
