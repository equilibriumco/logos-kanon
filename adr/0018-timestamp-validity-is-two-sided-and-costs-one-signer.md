# 18. Timestamp validity is two-sided, and a bad timestamp costs one signer

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

### A bad timestamp costs its own signer

Skipped per package, reported on the aggregate. This is the third time the question has
arisen and the answer has not changed: a payload is published once and serves many
consumers, so letting one appended stale package fail it is a denial-of-service primitive,
and a threshold exists to survive one bad signer.

**A third divergence from RedStone**, which propagates `TimestampTooOld` out of the walk.
The same divergence ADR 15 recorded for an unrecoverable signature and for an oversized
value, made for the same reason and named here rather than left to be inferred from a
pattern.

The aggregate rule extends M1-17's ladder rather than adding a second one, but the shape
changed while implementing it. Asking each cause in isolation —

```
met + stale >= threshold   -> StalePackage
met + future >= threshold  -> FuturePackage
met + spoiled >= threshold -> ValueOutOfRange
```

— answers `ThresholdNotMet { met: 0 }` when two signers reported and *different* things
were wrong with each: one package too old, one value zero, threshold two. Both signers are
in the payload, fixing either would have met the threshold, and no single cause reaches it
alone. That is the same misdirection the ladder was built to remove, arriving through a
mix of causes rather than one.

So the question is asked once, over signers rather than over reasons:

```
met >= threshold                     -> a price
met + present >= threshold           -> the largest cause among stale, future, spoiled
met + distinct_unknowns >= threshold -> UnauthorisedSigner
otherwise                            -> ThresholdNotMet
```

where `present` counts configured signers whose slot stayed empty for any of the three
reasons, each counted once — a signer arriving stale in one package and useless in another
must not close two gaps by itself.

The largest cause is named because it is the one whose fixing moves the count furthest.
Ties go to age, then to skew, then to values: that is the order in which a cause resolves
without anybody acting, and a fresher payload fixes staleness where a bad value needs
someone to change something. Sending an operator to reconfigure a feed that will be fine
in thirty seconds is the wrong answer even when it is also a true one.

Single-cause payloads answer exactly as the sequential form did, which is what the
existing tests continue to assert.

### The timestamp is checked after recovery, not before

Checking it first is cheaper — two `u64` comparisons against 565,497 cycles — and it is
wrong. Recovery is skipped for a package that fails the window, so a stale package needs
no key and no valid signature: an outsider can append three of them and turn
`ThresholdNotMet` into `StalePackage`, telling an operator to refetch a payload when the
signer set is what does not match. That is a misdirection primitive rather than a denial
of service, but it costs an attacker nothing, and an error nobody can trust is worth less
than no error.

The saving was never real. A fresh payload has no stale packages, so nothing is skipped in
the ordinary case; the optimisation pays only in the replay and attack cases, which are
the two where naming the right cause matters most.
`strangers_sending_stale_packages_cannot_rename_a_threshold_failure` pins it.

## Consequences

- `StalePackage` gains a producer, leaving `InvalidSignature` as the only `VerifyError`
  variant without one — and that one is deliberate rather than pending (ADR 15).
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
