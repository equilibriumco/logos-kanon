# 30. The signer set stays per feed, though every feed currently shares one

- **Status**: accepted
- **Milestone**: M2 (`M2-00`)
- **Requirements**: F7, SEC2
- **Artefacts**: `FEEDS.md`, `scripts/capture-signer-sets.py`, `verifier-core/src/feed.rs`

## Where the answers came from

`scripts/capture-signer-sets.py`, sampling `redstone-primary-prod` for five minutes. The
five registered feeds have every signer address recovered from the package signature; the
sweep across all 878 feeds the service serves reads the gateway's own labels, which is
weaker evidence for the weaker claim it supports. `FEEDS.md` carries the figures.

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

The simplification is not tempting because of a small sample. Sweeping every feed
`redstone-primary-prod` serves, 878 of them, found one roster and no exception anywhere,
so a global set matches everything measurable about this data service rather than just
about F7's five feeds.

What the sweep does not reach is anything outside this data service. Nothing in the wire
format ties one feed's signer set to another's: signers subscribe to feeds, a new feed can
arrive with a different set, and a feed can move between sets without a payload changing
shape. One data service agreeing with itself today is not a property of the format.

There is a second reason, and it survives whatever RedStone does. A feed's other security
parameters — its threshold, its `maxAge`, its asset pair — are already per feed in
`verifier-core`'s config. Moving the signer set alone to a global slot would split one
feed's security parameters across two places with different update paths, so reading what
governs a feed would mean reading both and knowing which wins.

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

- **One global signer set for the whole program.** Smaller, and it matches every feed the
  data service serves rather than only the five being registered. Rejected because it
  encodes a property of one data service's current configuration into the account layout,
  where the repair for being wrong is a migration rather than a transaction, and because
  it would leave a feed's signer set governed from somewhere other than the rest of the
  feed's parameters.
- **Per-feed sets, with a test asserting they are identical.** Keeps the layout and adds
  a tripwire for divergence. Rejected because the tripwire fires on ordinary upstream
  behaviour: RedStone rotating one feed's signers is not a failure, and a red build that
  means "the world changed, as it may" trains people to ignore it.
- **A shared set with per-feed overrides.** Both layouts at once, with a resolution rule
  between them. Rejected as the cost of the global set plus the cost of the per-feed set,
  bought to save four copies of five addresses.
