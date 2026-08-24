# 18. Timestamp validity is two-sided and rejects the package

- **Status**: accepted
- **Milestone**: M1 (`M1-14`, `M1-15`)
- **Requirements**: F4, U6, SEC3
- **Artefacts**: `verifier-core/src/time.rs`, `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `kanon-clock/`

## Context

ADR 13 settled where "now" comes from: LEZ's clock program, the every-block account,
pinned and never caller-chosen. It left three things for M1-15, and this record answers
them.

The first was carried forward explicitly by ADR 15: RedStone validates a package's
timestamp at **both** ends, rejecting one dated too far in the future as well as one too
old, while U6 names only staleness. Which of those Kanon follows was called out there as
a finding to confirm rather than an inference to make.

## Where the answers came from

`redstone-finance/rust-sdk`, Boost licensed, as with ADR 15; the BUSL EVM connector stays
unread. `crates/redstone/src/core/validator.rs` returns `TimestampTooOld` and
`TimestampTooFuture` as two distinct errors, against
`MAX_TIMESTAMP_DELAY_MS = 15 * 60 * 1000` and `MAX_TIMESTAMP_AHEAD_MS = 3 * 60 * 1000`
from `crates/redstone/src/protocol/constants.rs`.

ADR 13's millisecond claim is confirmed at its source: `lez/sequencer/core/src/lib.rs`
computes `chrono::Utc::now().timestamp_millis()` and passes it straight into
`clock_invocation`. RedStone package timestamps are milliseconds too, so the comparison
needs no conversion — and the same file's constants gave a free check on the M1-12
decoder, whose eight wire-format widths all match RedStone's own.

## Decision

### Two ends, two errors

`StalePackage` for a package older than the feed's `maxAge`; `FuturePackage` for one
dated beyond the clock-skew tolerance. `FuturePackage` is a variant U6 does not name,
added for the reason ADR 15 gives for the whole enum: a caller that cannot tell two
failures apart cannot act on either. These two send an operator to different machines —
stale means the relayer is behind or a payload was replayed, future-dated means a signer's
clock is wrong or this node's is.

**`maxAge` is per feed; the forward tolerance is a constant.** RFP-020 makes `maxAge`
configurable at registration and SEC3 asks for a documented production minimum, so it is a
knob. The forward bound is not: it is an allowance for skew between a RedStone signer's
clock and the sequencer's, which no feed operator is better placed to judge. Fixed at
RedStone's own three minutes, so a package RedStone would accept is never one Kanon
rejects.

The window is inclusive at both edges, matching RedStone's `is_same_or_after` and
`is_same_or_before`. An exclusive bound would reject, for one millisecond, a package the
system we are conforming to accepts.

### The clock is a trait, and its one implementation is a crate

`TimeSource` in `verifier-core`, `LezClock` in `kanon-clock`. Exactly ADR 3's shape, and
for a stronger reason: ADR 13 already established that a caller-supplied timestamp makes
staleness advisory, so the pinned-account rejection *is* the guarantee.

It is a crate of its own rather than a module in either mode, because F9 forbids the pull
library depending on the aggregator and both modes need this check. In the aggregator it
would leave pull mode with either no rejection or a second copy of it, and a security
check that exists twice eventually disagrees with itself.

`clock_core` and `lee_core` are `std` crates while `pull-lib` is `no_std` (ADR 4), so
`kanon-clock` declares the account id and the sixteen-byte layout rather than importing
them, with `tests/clock_conformance.rs` asserting both against the real types. This is
ADR 9's arrangement for the price account's IDL, applied again: a copy that is checked
cannot drift quietly, and the `no-std` CI job now builds `kanon-clock` for the bare target
so that nobody can "simplify" the declaration back into an import.

A clock that cannot be read is `VerifyError::NoClock`, never a guess. Verifying without a
clock accepts a package of any age, which is the replay the check exists to stop. A zero
timestamp — upstream's "no valid time" sentinel — is reported rather than believed, since
believing it would date every package impossibly far in the future and fail a feed for a
reason unrelated to the feed.

### A bad timestamp rejects the payload

F4 requires an out-of-window package to be rejected. `StalePackage` and
`FuturePackage` therefore return immediately for any authorised package carrying the
requested feed, even if other packages already meet the threshold. Age is not a
post-threshold tally and is not ranked against other package failures.

### Authority is checked before age

Timestamp checks follow signature recovery and signer-set membership. That order makes
the error actionable: an unauthorised stale package is `UnauthorisedSigner`, not evidence
that the configured relayer is behind. Within an authorised package, age precedes value;
there is no reason to parse and classify a value from a package already known to be stale.

Packages that do not carry the requested feed are skipped before recovery under ADR 25,
so a configured signer's stale package about another feed does not affect this one.

## Consequences

- `StalePackage` and `FuturePackage` are direct package errors. `InvalidSignature` is
  direct too, under ADR 15.
- `verify_feed` takes a fifth argument. Every caller now supplies a clock, which is what
  makes the check unforgettable rather than conventional.
- `kanon-clock` is a fourth guest-reachable crate under ADR 4's `no_std` rule, and is
  built for `riscv32im-unknown-none-elf` in CI alongside `verifier-core` and `pull-lib`.
- The production `maxAge` recommendation and SEC3's manipulation analysis are not here.
  This decision gives them their mechanism and RedStone's fifteen minutes as the reference
  point; the recommendation itself is a document, in M5.
- ADR 13's deferred question is untouched: whether an oracle should read the 10- or
  50-block clock account is still an M2 measurement. Only the *pinned-and-checked*
  property had to survive it, and it does — swapping the constant is the whole change.

## Alternatives considered

- **One error for both ends of the window.** Cheaper, and it hides the distinction that
  tells an operator which machine to look at.
- **A `u64` timestamp parameter instead of a trait.** Rejected by ADR 13 already: it makes
  staleness advisory. The trait costs nothing and is what lets tests drive a clock at all.
- **Accepting any of the three clock accounts.** The 10- and 50-block accounts hold real
  timestamps, so this would silently accept a clock up to fifty blocks stale. Which one an
  oracle *should* read is a live question; accepting all three answers it by not asking.
- **Putting `LezClock` in `aggregator-program`.** This design's first answer, and it
  violates F9.
- **Importing `clock_core` in `kanon-clock`.** Would make the crate `std`, which takes
  `pull-lib` with it and breaks ADR 4.
