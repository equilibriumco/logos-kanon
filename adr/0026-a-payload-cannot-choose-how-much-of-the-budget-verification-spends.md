# 26. A payload cannot choose how much of the budget verification spends

- **Status**: accepted
- **Milestone**: M1 (`M1-13`, `M1-18`)
- **Requirements**: P1, SEC1, U6
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `methods/tests/cost.rs`, `COSTS.md`

## Context

`verify_feed` hashed and recovered every package in the payload. Recovery is
585,274 cycles and the hash before it 17,475, so each package cost 602,749, and
nothing bounded how many packages there were. The count is a two-byte field in
the envelope, outside every signature, so anyone handling a payload can raise it
and a package can be copied without a key.

LEZ allows 33,554,432 cycles in a public transaction. 56 packages exhaust it.
Measured in the guest rather than extrapolated from the table: an 8 KB payload of
repeated packages came to **34,746,599 cycles, 103.6% of the budget**, and a
transaction that reaches the limit aborts instead of publishing.

**ADR 24 is what made that cheap.** Before it, a repeated package returned
`ReoccurringSigner` from the point loop as soon as a signer's slot was already
filled, so a flood aborted the walk after a handful of recoveries — the same 56
packages measured 4,545,420 cycles, 13.6%. Skipping the duplicate instead was
right for the reason ADR 24 gives, and it turned a denial that cost one package
into a denial that costs a whole transaction. Trading one for the other was not
the intention and is not a defensible resting place.

The second half is not an attack at all. **RedStone payloads carry several feeds
at once**, and every package was recovered whatever feed it named. Verifying one
feed of a five-feed payload measured 15,477,759 cycles — 46% of the budget for 25
packages, 20 of which could not have changed the answer. A consumer wanting all
five feeds pays that five times. That is a scaling defect in ordinary operation,
and it puts a bound on the *whole payload* out of the question: the limit has to
be about the feed being verified, not about how many feeds the relayer bundled.

## Decision

**A package carrying no data point for the requested feed is skipped before it is
hashed.** It is the one test cheap enough to run before recovery and the only one
that does not need the signer. Nothing is lost: such a package cannot fill a
slot or change this feed's error result.

**`MAX_RECOVERIES` packages for the requested feed, then `TooManyPackages`.** Its
own variant, because it is its own failure: the payload is well formed, its
signers may all be authorised, and what is wrong is its size. Reported so the
contract in ADR 23 stays complete and
`no_two_failure_modes_answer_with_the_same_variant` covers it.

**Refused, not truncated.** Stopping at the ceiling and answering from what had
been read would let whoever assembled the payload choose which of a signer's
packages counts, by placing the ones it wants inside the ceiling and the rest
outside — the manipulation ADR 24 closed, reopened through the bound meant to
protect the budget. A denial is a worse outcome than a price; a price the
attacker picked is worse than both.

**32, kept separate from `MAX_SIGNERS` though it shares its value.**
`MAX_SIGNERS` costs guest stack and `MAX_RECOVERIES` costs the transaction, so
raising one is not an argument for raising the other. At 32 the ceiling measures
19,885,240 cycles, 59% of the budget — 19,352,503 of that the verifier's own work
and the rest the harness reading a longer payload — and no honest payload
approaches it: one package per signer per feed, on a service running ten to
twenty.

## Consequences

- **The worst case is now a measured, asserted number.**
  `the_most_a_payload_can_cost_still_fits_in_one_transaction` runs a payload at
  the ceiling and three well past it, requires the ceiling to fit inside the
  budget, and requires the ones past it to do no more verification work. P1's
  row carries it, which is what turns "an update is 5.68% of the budget" into a
  statement about every payload rather than about the captured one.
- **An honest update costs 851 cycles more**, at three signers: the feed scan
  runs on every package before the hash. 0.05%, against a five-feed payload
  getting three times cheaper.
- **Unknown signers no longer need a buffer.** ADR 15 returns
  `UnauthorisedSigner` at the first relevant package outside the configured set.
- **Four mutations, four catches**, one of which the tests missed first time:
  counting every package toward the ceiling rather than only this feed's passed,
  because the payload put the feed's packages where the tail-first walk reached
  them before the ceiling could bite. The test now runs both orders.
- **A payload past the ceiling still costs about 19.9M cycles.** The refusal is
  bounded, not free: the ceiling is reached before it is reported. What it buys
  is that the number is fixed and under the budget, where before it was the
  payload's to choose.
- **Strict package failures can only reduce the work when encountered before
  the limit.** The ceiling admission check still runs before each recovery;
  within it, an invalid signature or unauthorised signer returns immediately.
  The ceiling remains necessary for valid copied packages and valid packages
  spread across rounds.

## Alternatives considered

- **Bound the whole payload's package count in `decode`.** Cheapest to write and
  wrong: it would refuse the multi-feed payloads RedStone is built around, and a
  bound loose enough to admit them is loose enough to exhaust the budget.
- **Stop at the ceiling and answer from what was read.** Turns a denial into a
  wrong price chosen by the attacker. Rejected above.
- **Check freshness before recovery too.** It would cut the cost of a stale
  flood, but it would report an unauthorised or invalidly signed package as a
  clock problem. ADR 15 keeps the actionable error order: signature, authority,
  then age.
- **Leave it to the caller.** The aggregator could bound the payload before
  calling. It would work for push and do nothing for pull, where the payload is
  the caller's argument and `verify_feed` is the whole of what a consumer runs.
