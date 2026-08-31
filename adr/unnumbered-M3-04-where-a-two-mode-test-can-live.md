# [M3-04:01]. Where a test that needs both modes can live

- **Status**: accepted
- **Milestone**: M3 (`M3-04`)
- **Requirements**: U6
- **Artefacts**: `aggregator-program/tests/error_parity.rs`, `aggregator-program/Cargo.toml`

## Context

U6 asks for a clear, actionable error per failure mode, and this repository makes a
stronger claim about it than two taxonomies kept in step. One enum in `verifier-core`
decides every payload failure; `pull-lib` returns it unchanged and `submit_price` wraps
it. So the parity M3-04 asserts is an *identity* on the shared causes rather than a
correspondence, and U6's row has said so since M3-01.

Asserting it needs one test that can call both `verify_price` and `submit_price` over the
same inputs. Nothing in the repository could: `pull-lib` must not see the aggregator —
that is M3-02, asserted on the build graph — and `aggregator-program` had no reason to see
`pull-lib`.

## Decision

**`pull-lib` is a dev-dependency of `aggregator-program`, and the parity test lives
there.**

The direction is the one that costs nothing, and cargo is the reason rather than CI: a
dev-dependency sits outside `--edges normal,build`, so it cannot appear in `pull-lib`'s
build closure at all. Measured rather than assumed: both closures are byte-identical
before and after, and `pull-lib` does not appear in `aggregator-program`'s own normal
closure either.

**Two things make that safe and they are not the same thing.** Cargo is why the edge
cannot reach: dev-dependencies are outside the closure a `--edges normal,build` walk
sees. M3-02's `pull-independence` job is what refuses the reverse, by pinning that
closure to an allow-list. The job landed first and has been run against this edge, so
the claim is enforced rather than only argued -- but the two reasons are recorded
separately, because a reader who takes the second for the first will look for a CI
guarantee where the guarantee is cargo's.

*Rejected: a separate test-only crate depending on both.* Ownership would be cleaner —
the test is M3 work sitting in an M2 crate — but it costs a manifest, a lockfile entry
and a CI consideration for a file that is otherwise one of eight in a directory that
already reaches for `verifier-core`'s fixtures by path.

*Rejected: asserting the identity from `pull-lib`'s side.* It is the side that must not
know the aggregator exists. Reversing the dependency would make M3-02's job fail, which
is the correct outcome and the reason this direction is the only one available.

## Consequences

- **The parity is measured where a real payload reaches it, and structural elsewhere.**
  Six shared causes are asserted in both modes over the committed capture: a malformed
  payload, an unauthorised signer, a threshold the payload cannot reach, a stale package,
  one dated past the allowed skew, and an asset mismatch. `InvalidSignature`,
  `ValueOutOfRange`, `ScalingOutOfRange`, `TimestampMismatch` and `TooManyPackages` need a
  crafted payload, which only `verifier-core`'s `cfg(test)` builder can produce (ADR 21),
  so for those the identity rests on `verify_feed` taking no argument naming its caller
  (ADR 23) and on `pull-lib` returning its error unchanged. The row says which is which.
- **Four asymmetries are real and none is a defect.** The clock is `SubmitError::Clock` in
  push and folded into `VerifyError::NoClock` in pull — same leaf, same number, different
  wrapper. A configuration failure reaches push when it decodes a stored feed and reached
  a pull consumer earlier, when it built its `FeedConfig`, so `VerifyError::InvalidConfig`
  is unreachable through `verify_price`. The pair a caller claims arrives as account state
  in push and as an argument in pull. And the account layer — feed, price account,
  publish — is push's alone.
- **A new push cause has to be classified.** `every_submit_cause_is_classified` matches
  `SubmitError` without a wildcard arm, so the next variant fails to compile until someone
  decides whether pull reaches it. It does not prove the classification is right, and the
  comment says so; what it buys is that the decision is forced, which the four additions
  the push taxonomy took during M3 would otherwise have slipped past.
- **The test crate now builds `pull-lib`.** `cargo test -p aggregator-program` compiles it,
  and `Cargo.lock` gains one line. No shipped artefact changes.
