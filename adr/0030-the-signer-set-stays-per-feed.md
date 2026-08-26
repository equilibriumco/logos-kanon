# 30. The signer set stays per feed, though every feed currently shares one

- **Status**: accepted
- **Milestone**: M2 (`M2-00`)
- **Requirements**: F7, SEC2
- **Artefacts**: `FEEDS.md`, `scripts/capture-signer-sets.py`, `verifier-core/src/feed.rs`

## Where the answers came from

`scripts/capture-signer-sets.py`, sampling `redstone-primary-prod` for five minutes and
recovering every signer address from the package signature. `FEEDS.md` carries the figures.

## Context

M2-00 went to confirm that XMR/USD and ZEC/USD are published at all, since F7 names five
feeds and the two privacy assets were the ones nobody could assume. They are, and the
capture answered a question it was not sent to ask: all five feeds are signed by the same
five addresses, in every round of every sample.

That makes a simplification available. `verifier-core`'s feed config carries a signer set
per feed, so registering F7's five means storing the same five addresses five times, and
M2-07's `register_feed` has to take a set it could instead read from one place. A single
global roster would be smaller, cheaper to update — one admin transaction instead of five
— and, today, exactly as correct.

It is correct today because of an accident of how one data service is configured, and the
capture is not evidence that it will stay that way. Signers subscribe to feeds, and
nothing in the wire format ties one feed's set to another's. A new feed can arrive with a
different set, and a feed can be moved between sets without anything in a payload
changing shape. The measurement covers five feeds on one data service over five minutes.

The cost of being wrong is not symmetric. Per-feed sets that turn out to be identical
waste four copies of five addresses and four admin transactions. A global set that turns
out to be wrong cannot represent the estate at all, and the repair is a migration of the
account layout with feeds already registered against it — reached by the same route in
either case, since finding out means a feed's set diverging in production.

## Decision

**The signer set stays a property of a feed.** `register_feed` takes the set for the feed
it registers, `update_signer_set` moves one feed's set, and the identical rosters observed
today are stored as five identical sets rather than collapsed into one.

**Nothing in the repository asserts that the sets agree.** The capture reports what it
finds and `FEEDS.md` records it as an observation. A test that required the five to match
would fail the day RedStone rotated one feed's signers, which is upstream behaviour this
adaptor has no standing to call a defect.

## Consequences

- **Registering F7's five feeds is five transactions with the same payload.** M2-11 does
  the work five times, and that is the intended shape rather than a duplication to factor
  out.
- **`update_signer_set` is per feed, so an upstream rotation touching every feed needs
  five admin transactions.** With the roster turning over on a scale of years — the
  addresses are unchanged since the conformance capture twelve days earlier, and a source
  from about two and a half years back shares none of them — the operational cost of that
  is close to nothing, and the path is the same one a single-feed rotation uses.
- **A pull consumer configures its own set per feed too.** SEC2's pull half is unchanged
  by this: the set is the consumer's, and it reaches `verifier-core` the same way the
  aggregator's does.
- **The roster in `FEEDS.md` is the observed set, not an authorised one.** A signer
  authorised upstream but silent through the sample does not appear in it, so what M2-11
  registers is what was seen signing, and `update_signer_set` is the correction path.

## Alternatives considered

- **One global signer set for the whole program.** Smaller, and matches everything
  measured. Rejected because it encodes a property of one data service's current
  configuration into the account layout, where the repair for being wrong is a migration
  rather than a transaction.
- **Per-feed sets, with a test asserting they are identical.** Keeps the layout and adds
  a tripwire for divergence. Rejected because the tripwire fires on ordinary upstream
  behaviour: RedStone rotating one feed's signers is not a failure, and a red build that
  means "the world changed, as it may" trains people to ignore it.
- **A shared set with per-feed overrides.** Both layouts at once, with a resolution rule
  between them. Rejected as the cost of the global set plus the cost of the per-feed set,
  bought to save four copies of five addresses.
