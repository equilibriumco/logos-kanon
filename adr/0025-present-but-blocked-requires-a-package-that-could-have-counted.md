# 25. "Present but blocked" requires a package that could have counted

- **Status**: accepted
- **Milestone**: M1 (`M1-18`)
- **Requirements**: F4, U6
- **Artefacts**: `verifier-core/src/feed.rs`

## Context

When the threshold is not met, `verify_feed` decides *why* before it answers.
The distinction it draws is between "too few signers signed at all", which is
`ThresholdNotMet`, and "enough signers were here but something blocked them",
which is `StalePackage`, `FuturePackage` or `ValueOutOfRange` depending on what
the something was. That choice is the whole value of the typed enum: the first
sends an operator to look for missing signers, the others send them to a
relayer, a clock or a data source.

Three per-signer flags feed it — `stale`, `future` and `supplied_feed` — and a
signer counts as present if any of them is set while its slot stayed empty.

The unknown-signer branch already guarded its own tally properly. A stranger is
recorded only if its package was current *and* carried a usable value for the
requested feed, because authorising a signer whose package was stale, or was
about another feed, would not have produced a price either.

The two age flags had no such guard. Any package from a configured signer that
fell outside the window set one, whatever the package was about.

That is reachable on production data rather than only on a contrived payload.
Signers on `redstone-primary-prod` subscribe to different feed subsets
(ADR 19), so a configured signer sending a stale package about ETH while the
caller asked for BTC is an ordinary shape. The signer reported nothing for BTC,
so BTC's slot was empty for want of a signer — and the answer flipped from
`ThresholdNotMet` to `StalePackage`, pointing an operator at a relayer that was
doing its job.

Under U6 this is not a cosmetic defect. The requirement is that a failure
carries a cause a caller can act on, and a cause that names the wrong failure
is worse than one that names none.

## Decision

**A stale or future-dated package sets its flag only if it carries a usable
value for the requested feed** — the same test the unknown-signer branch
applies, now extracted as `carries_usable_value` and called from both. The
question each tally asks is the same one: would this package have counted, if
the one thing being tested had been different?

The gate is evaluated inside the two out-of-window arms, so a payload that is
entirely current pays nothing for it.

## Consequences

- **A package that is both stale and unusable now reports `ThresholdNotMet`.**
  It falls out of the three questions rather than being decided separately: the
  age tally declines it because the value is unusable, and the value tally
  declines it because the package is not current. Fixing either one alone would
  not have produced a price, so neither `StalePackage` nor `ValueOutOfRange`
  names a repair that works. Covered by
  `a_stale_package_carrying_a_value_this_feed_cannot_use_is_not_staleness_either`.
- **The three tallies ask three different questions, and each now holds the
  right thing constant.** `unknown` asks whether the package would have counted
  had its signer been authorised, so it needs a current package with a usable
  value. `stale` and `future` ask whether it would have counted at the right
  age, so they need a usable value for this feed. `supplied_feed` asks whether
  it would have counted had the *value* been usable, so it needs a current
  package carrying the feed and must not require usability -- requiring it would
  make the tally that reports `ValueOutOfRange` unable to fire. Before this the
  age tallies held nothing constant, which is why they reported on packages that
  could not have counted under any repair.
- **One helper, two callers, no behavioural change to the unknown-signer
  branch.** It was already doing this; it just did it inline.
- **The published figures moved.** The gate sits on a path a well-formed payload
  never takes, but extracting the shared helper changed how the walk's body
  inlines: an update costs about 45 cycles more at one signer and 400 more at
  five, plus 23 for the Q64.64 conversion. A hundredth of a percent, and not the
  point of the change -- recorded because `COSTS.md` and `methods/tests/cost.rs`
  are pinned by exact equality and had to be re-measured either way.

## Alternatives considered

- **Gate on the package merely carrying the feed, usable or not.** It fixes the
  wrong-feed case and leaves the stale-and-zero one answering `StalePackage`,
  which is true and unhelpful. It also leaves the two tallies asking different
  questions, which is how this defect arrived.
- **Set the flag and let the post-walk ranking sort it out.** The ranking picks
  the largest cause among stale, future and spoiled; it cannot distinguish a
  signer that was blocked from one that was never there, because by then that
  information is gone.
- **Check freshness before recovery, so an out-of-window package never reaches
  the flags.** Rejected for the reason already recorded in `feed.rs`: an
  outsider could then append unsigned stale packages and turn a threshold
  failure into a staleness one, which is the same class of misdirection from the
  other side.
