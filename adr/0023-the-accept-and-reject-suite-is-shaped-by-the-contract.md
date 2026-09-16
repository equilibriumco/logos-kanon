# 23. The accept and reject suite is shaped by the contract, and asserts that the taxonomy discriminates

- **Status**: accepted
- **Milestone**: M1 (`M1-22`)
- **Requirements**: F2, U6
- **Artefacts**: `verifier-core/src/accept_and_reject.rs`, `verifier-core/src/lib.rs`

## Context

`verifier-core` had 122 tests before this, and between them they already reach
every variant of `VerifyError` and every variant of `ConfigError`. So the obvious
reading of M1-22 — write tests for the accept and reject paths — was largely
satisfied by accident, as a by-product of M1-12, M1-13, M1-15, M1-16, M1-17,
M1-21 and M1-23 each testing what it introduced.

Two things were nevertheless missing, and they are different from each other.

**The suite had no shape a requirement could be read off.** Every test lives in
the module of the thing that introduced it: framing cases in `decode`, threshold
cases in `feed`, scale cases in `value`. That is the right arrangement for
locating a regression and the wrong one for answering U6's question, which is
whether *every* failure mode is covered and whether each one says something a
caller can act on. Answering it meant reading four modules and holding a
checklist in your head.

**Nothing asserted that the errors are distinguishable.** Each per-mode test
pins one cause to one error. None of them, individually or together, rules out
two causes collapsing onto the same answer — and a taxonomy that does not
discriminate fails U6 while passing every test that claims to cover it.

There is also a wrinkle in the task text. M1-22 asks for the paths exercised
"from the in-program call path and the consumer-side call path", and neither
caller exists: `pull-lib` is a re-export and nothing else until M3-01, and the
aggregator's `submit_price` is M2-02. Both land after this task's slot. The task
lists M1-18 and M1-16 as its dependencies, so nothing in the plan forced the
callers earlier.

## Decision

**One module organised by the contract, `verifier-core/src/accept_and_reject.rs`:
five accept cases, seventeen reject cases, each named after the mode rather than
the mechanism.** It calls nothing but the crate's public API. Where it overlaps
an existing test — and it overlaps several — the overlap is the point: the value
is a single place where a reviewer can check coverage against U6's list without
reconstructing it from four modules.

**A test that no two causes answer with the same variant.** Twelve distinct
failures, compared pairwise. Compared by `core::mem::discriminant` rather than by
value, because `ThresholdNotMet { met: 0 }` and `ThresholdNotMet { met: 1 }` are
different values and the same answer — comparing values lets exactly the collapse
this test exists to catch pass. That is not hypothetical: the first version
compared values, and it passed under a mutation that removed the
`UnauthorisedSigner` branch entirely.

**"Both call paths" is one implementation, tested once.** ADR 2 makes
`verify_feed` the only thing in the workspace that verifies, and it takes no
argument naming the mode that invoked it, so there is no per-mode behaviour a
per-mode test could reach. Testing it once through its public API is what F2's
"one audited implementation backing both modes" means as evidence. What the two
callers add on top is theirs: M2-02 for the write, M3-01 for the consumer-side
return, and M3-04 for the assertion that the two report the same taxonomy for
every shared failure mode.

**Inside the crate, under `cfg(test)`, for ADR 21's reason.** The payload
fixtures are in `src/test_support.rs` behind `cfg(test)`; reaching them from
`tests/` would mean publishing a payload builder and a signing key behind a Cargo
feature, in the surface of a crate third-party programs link.

## Consequences

- **`InvalidSignature` is a directly tested payload error.** An unrecoverable
  signature alongside an otherwise sufficient quorum still returns
  `InvalidSignature`, so the variant cannot regress into a threshold failure or
  an unobservable skip.
- **`Payload::decode` is the second place a caller handles a malformed payload.**
  An envelope that does not decode never reaches `verify_feed`, so a caller that
  matches only on `VerifyError` misses it. Two tests, one per site.
- **A sign check needs the full 32-byte width.** `Value::from_be_slice`
  right-aligns a shorter slice, so a four-byte `0xFFFFFFFF` is a large positive
  number and not minus one. The first draft asserted `ValueOutOfRange` for it and
  got a verified price instead. The fixture is now `[0xFF; 32]` with a comment,
  because the next person to shorten it will shorten it back.
- **Five mutations, five catches.** Collapsing `UnauthorisedSigner` into
  `ThresholdNotMet`, swapping the stale and future branches, making an
  unrecoverable signature non-fatal, letting zero count as usable, and letting
  a recurring signer fill two slots were each caught by the test that claims to
  cover it. The collapse was additionally caught by the discrimination test.
- **The contract module contains twenty-three tests**, including the taxonomy
  assertion. The full `verifier-core` unit suite currently contains 159 tests.
- **It tests the host build.** The cross-compile job and the cost harness are
  what say the same code runs under `riscv32im` — the same division ADR 3 and
  ADR 21 already draw.

## Alternatives considered

- **Wait for M3-01, then test through `pull-lib`'s API.** It honours the task
  text and it half-works: even with `pull-lib` in hand, the push caller does not
  exist until M2-02, so one of the two paths would still be absent. It also moves
  four hours of M3 into M1.
- **A thin fake caller per mode, standing in for the two real ones.** Two
  wrappers that forward to `verify_feed` and assert it was reached. They would
  test the wrappers, which are written for the test, and say nothing about the
  callers that eventually ship.
- **Record M1-22 as partial, with the second half attached to M3-04.** The same
  code, differently labelled. M3-04 already owns the cross-path parity evidence,
  which is the argument for it; against it, the clause it would defer is one this
  crate's structure makes vacuous rather than one left undone.
- **An integration test in `tests/`, with `test_support` behind a feature.** It
  would restrict the suite to the public API by construction rather than by
  discipline. Rejected for ADR 21's reason, which has not changed: fixture code
  does not belong in the published surface of a crate a consumer program links.
