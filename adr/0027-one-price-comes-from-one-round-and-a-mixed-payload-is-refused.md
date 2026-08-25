# 27. One price comes from one round, and a payload that mixes rounds is refused

- **Status**: accepted
- **Milestone**: M1 (`M1-13`, `M1-15`)
- **Requirements**: F3, F4, SEC1, U6
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `COSTS.md`

## Context

`verify_feed` validated each package's timestamp on its own and, per ADR 24, kept each
signer's freshest. Nothing required the packages behind a price to describe the same
moment. A payload carrying one signer's report from 09:00:00, another's from 09:00:10 and
a third's from 09:00:20 — all validly signed, all inside `maxAge` — produced a median
nobody published, assembled by whoever built the payload. A price is a statement about one
observation, and that payload contains three.

The first answer to this chose a round by consensus: score each timestamp by the distinct
configured signers reporting it, take the winner, ties to the newer. It worked, and it
avoided making a copied or replayed package fatal.

It also contradicted the contract decided alongside it. ADR 15 rules that a package
carrying the requested feed with a bad signature, an unauthorised signer, an out-of-window
timestamp or an unusable value **refuses the payload**, and accepts what that costs: an
appended package refuses, and appending needs no key. Being strict about four properties of
a package and running a vote on the fifth is not a position, it is two positions. The
availability argument that justified the vote is the same argument ADR 15 heard and
declined.

The captured vectors say the strict rule costs nothing upstream: all twenty-five packages
across five feeds carry the identical timestamp `1786710590000`.

## Decision

**Every package carrying the requested feed must describe the same moment.** The first such
package the walk reaches sets the payload's timestamp; any later one that disagrees returns
`TimestampMismatch { expected, found }`, carrying the moment the payload settled on and the
first one that departed from it.

**Checked after the feed test and before the hash**, so a mismatch costs a field comparison
and no recovery. It joins ADR 15's checks as a fifth, in walk order, before the threshold
is evaluated.

**Scoped to the requested feed, not to the whole payload.** SEC1 is written per feed —
"reject any data package whose signer is not in the authorised signer set *for the
requested feed*" — and Reliability 3 requires that trouble with one feed not reach another
handled by the same node. A payload is multi-feed by construction, so a rule spanning it
would make another feed's timestamp a reason to refuse this one, and would move that
isolation out of this crate and into whatever assembles payloads.

**Duplicates stay non-counting rather than fatal (ADR 24).** A copy carries the moment it
was copied from, so it agrees by construction and fills no second slot. That keeps the
cheapest keyless denial closed: copying a package still buys nothing.

**The round vote and its report buffer are gone.** With one moment per payload there is
nothing to choose between, so slots are filled during the walk — first value per signer
holds — and `MixedRounds` no longer exists.

## Consequences

- **Whoever can add bytes to a payload can refuse it, and needs nothing to do so.** The
  moment check is first in walk order and compares a field, so it runs before recovery.
  An appended package therefore needs to carry the requested feed and a different
  timestamp, and nothing else: not a key, not a signature that recovers, not a timestamp
  inside `maxAge`, not a package that any signer ever produced. This is ADR 15's accepted
  trade at the lowest effort bar in the enum. Neither shipping mode grants that position —
  a push submitter spends its own transaction, a pull consumer owns the payload it passes —
  but anything that assembles payloads from a shared or untrusted source becomes a denial
  point. **A submitter must own the bytes it submits**, which is a constraint on M2's
  relayer, and one that a tighter `maxAge` does nothing to relax.
- **What this changes about replay is the *mixed* case only.** A package replayed into a
  payload built around another moment can no longer move a median, because it refuses the
  payload. Replaying a whole round is untouched: its packages agree about the moment by
  construction, so it verifies, and what bounds it is `maxAge` together with the
  aggregator's refusal to write a timestamp no newer than the stored one. Stated because
  the decision is easy to read as wider than it is.
- **`VerifiedFeed::timestamp_ms` is the moment every counted package shares**, and now by
  construction rather than by selection.
- **The real vectors verify unchanged.** The conformance suite passes without amendment,
  because a round's packages already share a timestamp.
- **Four tests changed premise and one split in two.** Every one of them asserted an
  outcome of the vote — that an appended round loses it, that copies cannot outvote
  signers, that a tie goes to the newer. Those outcomes were the vote's, and the vote is
  gone.
- **An update costs less, not more.** The vote was quadratic in the reports and its
  bookkeeping is deleted along with the per-package buffer; `COSTS.md` carries the
  re-measured figures.

## Alternatives considered

- **Choose a round by consensus, counting distinct signers.** What this replaces. It
  tolerates a replay, at the price of contradicting ADR 15 and of a quadratic vote,
  a per-package report buffer and an eighth error variant to explain.
- **Require one timestamp across the whole payload.** Simpler to state and one comparison
  cheaper, but it makes another feed's timestamp fatal to this feed's read — against
  Reliability 3, and against SEC1's per-feed framing.
- **Take the newest timestamp, or the oldest.** One pass, no vote, and either hands an
  appender the choice of which in-window round the price comes from.
- **Allow a tolerance window instead of equality.** It admits the splice this ADR exists to
  refuse, just over a shorter interval, and introduces a second staleness constant to
  justify.
