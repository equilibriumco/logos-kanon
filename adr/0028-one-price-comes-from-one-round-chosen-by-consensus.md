# 28. One price comes from one round, and the round is chosen by consensus

- **Status**: accepted
- **Milestone**: M1 (`M1-13`, `M1-15`)
- **Requirements**: F3, F4, SEC1, U6
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `COSTS.md`

## Context

`verify_feed` validated each package's timestamp on its own and, per ADR 24,
kept each signer's freshest. Nothing required the packages behind a price to
describe the same moment. A payload carrying one signer's report from 09:00:00,
another's from 09:00:10 and a third's from 09:00:20 — all validly signed, all
inside `maxAge` — produced a median RedStone never published, assembled by
whoever built the payload.

**RedStone's SDK requires one timestamp**, and this was missed rather than
decided. `crates/redstone/src/protocol/payload.rs`:

```rust
let first_timestamp = validator.validate_timestamp(0, first_package.timestamp)?;

if let Some(outstanding_ts) = self
    .data_packages
    .iter()
    .map(|package| package.timestamp)
    .skip(1)
    .find(|ts| *ts != first_timestamp)
{
    return Err(Error::TimestampDifferentThanOthers(first_timestamp, outstanding_ts));
}
```

ADR 18 read `core/validator.rs`, found the two-sided window, and recorded that
faithfully. The equality rule lives one level up in `protocol/payload.rs`, which
the SDK survey behind ADRs 15 and 18 did not inspect. The earlier
implementation's divergence was accidental rather than a decision.

The captured vectors say the premise holds upstream: all twenty-five packages
across five feeds carry the identical timestamp `1786710590000`.

**The SDK's remedy is not used here.** Refusing a payload whose timestamps
differ would make a copied or replayed, otherwise valid package fatal even
though ADR 24 deliberately keeps duplicates non-counting. That rules out every
rule whose reference timestamp an appender can move — newest-wins lets someone
orphan the honest packages by appending a newer one, oldest-wins by appending an
older one.

## Decision

**The price comes from one round, and the round is the timestamp the most
distinct configured signers agree on.** Ties to the newer, which is the price a
consumer would have got by asking again.

**Counted over distinct signers, never over packages.** A package can be copied
for free and a signature cannot, so counting packages would let five copies of
one signer's older report outvote three signers' current ones. Over signers,
appending can only add to some round's tally and never take from the one the
honest signers already hold.

**Reports are collected per package and resolved afterwards.** Which of a
signer's packages fills its slot depends on which round wins, and that is not
knowable until every package has been read, so the walk records
`(signer, timestamp, value)` and the slots are filled after it.

**One report per package, whatever the package repeats.** The list is sized at
`MAX_RECOVERIES` because that is what bounds the recoveries paying for it (ADR
26), and the bound only holds if a package cannot file more than one. A package
may carry many data points for one feed, so without the rule a single signature
could fill the list on its own and crowd every other signer out — and since the
walk runs from the tail, an attacker would place it last and the honest packages
after it would be dropped in silence. That is the M-of-N denial this crate keeps
closing, arriving through the buffer instead of through the error. Every
repeated point is still validated under ADR 15; only the first valid value is
filed as the package's single report.

**`MixedRounds { largest, required }` when the signers were there but not
together.** Distinct from `ThresholdNotMet` because the signers did report, and
distinct from `ValueOutOfRange` because their values were fine: what is wrong is
the payload, which points at whoever assembled it. Checked before the ordinary
threshold failure, which would otherwise imply the signers never reported.

## Consequences

- **This is the answer ADR 24 left open.** ADR 24 made a duplicate skippable and
  named the property whatever chose between them would have to keep: the price
  is a function of the set of packages, not of their order. A replay loses the
  round vote whether it is older or newer, which is that property with both
  directions covered.
- **`VerifiedFeed::timestamp_ms` is the round, not the oldest package behind the
  median.** A stronger statement, and a simpler one: every package behind the
  price now carries that exact timestamp.
- **The real vectors verify unchanged.** `a_published_payload_verifies_end_to_end`
  and the rest of the conformance suite pass without amendment, because RedStone
  rounds already share a timestamp. The rule costs nothing on honest data.
- **Two tests lost their premise**, both of them assertions that the timestamp is
  the oldest package behind the median. That behaviour was the defect.
- **Five mutations, five catches** — and three of them survived the first round
  of tests: counting packages instead of signers, breaking ties toward the older
  round, and letting one package file more than one report. All three were
  properties this ADR claims and nothing asserted until they were broken
  deliberately.
- **An update costs 425 cycles more** at three signers, 0.02%. The round vote is
  quadratic in the number of reports and the reports are capped at
  `MAX_RECOVERIES`, so the worst case is about a thousand comparisons against
  585,278 cycles for a single recovery.

## Alternatives considered

- **Refuse the payload when timestamps differ, as the SDK does.** Literal parity,
  and a keyless denial of the feed. The SDK runs where a caller supplies its own
  payload and wears its own failure; here a payload is published once and serves
  everyone reading it.
- **Take the newest timestamp as the round.** One pass, no vote. An appended
  newer package — obtainable ten seconds later, no key needed — would orphan the
  round the honest signers agreed on and deny the feed.
- **Take the oldest.** The same hole through the other door, and it also hands an
  attacker the choice of which in-window round the price comes from.
- **Allow a tolerance window instead of equality.** Packages within a few seconds
  of each other treated as one round. It admits the splice this ADR exists to
  refuse, just over a shorter interval, and it introduces a second staleness
  constant to justify.
