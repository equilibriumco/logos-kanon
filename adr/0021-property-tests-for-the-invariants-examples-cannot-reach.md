# 21. Property tests for the invariants examples cannot reach, inside the crate under test

- **Status**: accepted
- **Milestone**: M1 (`M1-24`)
- **Requirements**: S3
- **Artefacts**: `verifier-core/src/properties.rs`, `verifier-core/Cargo.toml`

## Context

`verifier-core` had 110 example tests before this. They are good at what examples
are good at: each one names a case a reader can follow and check by hand, and
between them they cover every branch the verification takes.

What an example cannot do is exhaust a domain. Three of this crate's decisions
are statements about *all* inputs rather than about a case:

- **The threshold.** `M` is supposed to gate a count it does not influence, so
  the price a payload verifies to must be the same at every threshold that
  payload meets. Nothing checked that; the examples check individual
  `(threshold, payload)` pairs.
- **Decoding.** The payload is attacker-supplied and every width, count and
  offset in it is declared by the payload itself. A decoder that read one of
  those from the wrong place still works on any payload where the two happen to
  agree, and the committed vectors are all payloads where they agree.
- **The scale conversion.** `to_q64_64` is an arithmetic identity over `2^128`
  inputs, with a representable range whose boundary is `2^64 * 10^decimals`. The
  examples check about a dozen points on it.

There is also a fourth, which cuts across all three and is the reason this task
was worth more than its three hours: ADR 4 makes a panic in guest-reachable code
an *aborted transaction* rather than a rejected payload. A malformed package that
panics does not fail for its sender, it fails for everyone reading the feed. That
is a property about arbitrary bytes, and arbitrary bytes are not something to
write examples of.

## Decision

`verifier-core/src/properties.rs`, ten properties under `proptest`, compiled
under `cfg(test)` as a module of the crate rather than as an integration test.

**Inside the crate, not in `tests/`.** The payload fixtures live in
`src/test_support.rs` behind `cfg(test)`. Reaching them from `tests/` would mean
publishing them behind a Cargo feature, which puts a payload builder and a
signing key in the API surface of a crate a third-party consumer links into its
own program. A generated test is still a test, and it belongs where the other
tests are.

**`proptest`, not a loop over a seeded PRNG.** Shrinking is the half of property
testing a hand-rolled loop does not give, and it is most of the value: a failing
payload of several hundred bytes with three mutations in it is not a bug report
until something has reduced it to the one that mattered. It also costs nothing —
`proptest` is already in the product lockfile through `alloy-primitives`, reached
from LEZ, so declaring it adds no crate. `handle-panics` replaces the default
`fork`, which would run every case in a subprocess and bring `rusty-fork`,
`tempfile` and `wait-timeout`; one of the two is needed, because without either a
panicking case aborts the run instead of shrinking to the input that caused it,
and the ADR 4 property is precisely about panics.

**The generators were checked by breaking the code.** A property that passes
proves nothing about the property. Four deliberate defects were introduced one at
a time — the median stops sorting, the conversion divides by `10^(d-1)`, the
threshold comparison goes off by one, the midpoint is written as the overflowing
`(a + b) >> 1` — and each was caught by the property that claims to cover it.
The fourth was not, at first, and that is the finding below.

### Randomness in CI, which nothing else here has

Every other assertion in this repository is exactly reproducible, and ADR 10
argues at length for exact equality over inequalities. These properties are the
exception: `proptest` draws a fresh seed per run, so two runs of the same commit
exercise different inputs, and a property can pass today and fail tomorrow with
nothing changed.

That is the point rather than a flaw — a fixed seed turns a property test into a
large example suite, and the inputs that find defects are the ones nobody chose.
What it needs is somewhere for a failure to land, which is
`verifier-core/proptest-regressions/`: `proptest` writes the failing case there,
and the file is committed, so the input becomes a permanent case that runs first
on every subsequent run. A failure that is not written down is a failure that may
not recur.

## Consequences

- **The median's overflow guard had never been reached.** `Value::midpoint` is
  written as `(a >> 1) + (b >> 1) + carry` rather than `(a + b) >> 1` because a
  32-byte sum does not fit. The first version of the generator drew reports from
  `u128`, which leaves the sixteen high bytes zero — so the overflowing
  implementation and the correct one agree on every case it could produce, and
  the mutation test passed with the guard removed. Widening the generator to the
  full 32-byte width catches it. The generator's own doc comment now says why it
  is wide, because the next person to simplify it will simplify it back.
- **The corrupted-payload property leaves the envelope alone, deliberately.** The
  first version mutated anywhere and truncated freely; six cases in ten died on a
  missing marker, which the decoder's own tests already cover, and fewer than one
  in twenty reached a signature at all. Confining the
  corruption to the packages and weighting truncation down puts about four cases
  in five past recovery and into the threshold and the conversion, which is the
  composed path the property exists to walk.
- **The threshold property subsumes several examples and is stronger than all of
  them.** For each generated payload it verifies at every threshold from one to
  the signer count and asserts one price across all of them, plus the exact
  `ThresholdNotMet { met, required }` above. An example asserts one point of that.
- **It is cheap.** Half a second for the module in release, which is what CI
  runs, and eight seconds in a debug `cargo test`. `proptest`'s default of 256
  cases per property therefore stands everywhere, including the two properties
  that sign and recover real signatures.
- **These test the host build of `verifier-core`, not the guest.** The
  cross-compile job and the cost harness are what say the same code runs under
  `riscv32im`; a property here that passes says nothing about the accelerated
  `k256`. That is the same division ADR 3 already draws, and the reason the
  no-panic property is worth having anyway is that the panics it looks for are in
  first-party bounds arithmetic, which is identical on both.
- **A red tick in `build-test` can now mean "a seed found something".** It is the
  one job where a failure is not necessarily a regression in the commit that
  triggered it. The committed regression file is what turns that into a normal
  failing test on the next run.

## Alternatives considered

- **A deterministic loop over a seeded PRNG, with no dependency.** Twenty lines
  and no lockfile change, which would have been the smaller diff. It gives up
  shrinking, and a property test without shrinking reports a 700-byte payload and
  leaves the reduction to a human at the moment they are least equipped to do it.
- **A fixed seed, for reproducibility with the rest of CI.** It makes the suite
  exactly reproducible and simultaneously makes it a fixed set of examples,
  which is the thing the crate already had 110 of.
- **`cargo-fuzz` for the no-panic property.** The right tool for a long soak and
  the wrong one for a pull request: it needs nightly, a separate target, and a
  corpus that has to live somewhere. The generated payload here is a better shape
  than raw bytes anyway — a fuzzer spends its first million cases learning to
  produce a valid marker.
- **An integration test in `tests/`, with `test_support` behind a feature.** It
  would let the properties use only the public API, which is a real benefit. It
  also puts fixture code in the published surface of a crate meant to be linked
  by third-party programs, and it would be the first Cargo feature in this crate.
  Not worth it for a test that has no reason to be outside.
