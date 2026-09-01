# [M3-06:02]. Where a pull consumer's trust comes from

- **Status**: accepted
- **Milestone**: M3 (`M3-06`)
- **Requirements**: SEC2, U7, F9
- **Artefacts**: `reference-consumers/pull/src/authority.rs`, `reference-consumers/pull/src/trust.rs`, `reference-consumers/pull/guest/src/bin/pull_consumer.rs`

## Context

SEC2 says the authorised signer set comes from the consumer and never from the payload.
`pull-lib` satisfies half of that and cannot satisfy the other half: the roster arrives
there as a slice, and a slice built from a constant is indistinguishable from one built
from the transaction's own instruction data. A consumer that read its signers off the
wire would pass every check in that crate and would have handed its caller the right to
speak for the feed. F9's and SEC2's rows have both said, since M3-01, that the boundary
is the reference consumer's to demonstrate.

So the question is where a consumer program's signer set, threshold, staleness window
and asset pair come from.

## Decision

**They live in a trust account the consumer program owns, one per feed, written only
behind an authority gate.**

`settle` takes an order account, a trust account, the clock and a payload. It writes
neither the trust account nor the config, and there is no signer, threshold, window or
pair parameter anywhere in its instruction data —
`nothing_in_settles_instruction_data_can_reach_the_signer_set` reads that off the
published IDL rather than off the source, because the IDL is what a caller builds
against and an added parameter shows up there as a failure rather than as a diff nobody
read. What protects the roster is `every_instruction_that_moves_what_the_program_trusts_declares_a_signer`
and the `authority::authorise` gate behind it.

**All of a feed's configuration is per feed, which is ADR 30 reaching its pull caller.**
That ADR rejects a global roster in as many words — it cannot represent an estate where
one feed's set has moved, and the repair once feeds are live is a migration rather than
an edit — and its consequence list says a pull consumer configures its own set per feed
too. The argument reaches the threshold and the window as well, because a threshold is
only meaningful against the set it counts, so `FeedTrust` carries all of it and is the
pull-side counterpart of the `FeedAccount` the push side stores, in an account governed
by this program's own authority rather than the aggregator's. Counterpart rather than
copy: everything a verification reads is the same, and the two differences are decisions —
this carries the `dataServiceId` that RFP-020 puts in a pull consumer's configuration, and
it has no pause flag, because pausing is a lever an aggregator needs over a feed it
publishes to others and a consumer that wants to stop acting stops sending settlements. One configuration
shape behind both modes is what makes ADR 2's "one verification, two modes" visible to
somebody reading the two programs side by side.

**The authority is established from a build input, and handover is two steps.** The
first write to the config account decides who governs the program, so an unguarded
`establish` is a race whoever watches for a deployment wins; the genesis key closes it,
and `establish` refuses a build that configured none. `nominate` and `accept` are
separate so a mistyped key costs a second nomination rather than the whole
administrative surface, and `accept` refuses a nominee whose account is pristine —
LEZ can never honour a write signed by a default-owned account, so an authority that is
one is an authority that cannot act. All three are the aggregator's choices, reached
independently here and deliberately spelled the same way; `[M2-06:01]` carries the
reasoning and the review that found the pristine-key case.

*Rejected: a roster compiled into the program.* It reads as the stricter choice —
nothing a caller sends can reach a constant — and it is the one this decision has to
argue against, because a reader will reach for it first.

It is not what F9 asks for. F9 wants a configured tuple and "no dependency on **the
aggregator's price account**"; registering a roster in an account this program owns is
not registering a feed against the aggregator, and SEC2's "comes from the consumer" is
satisfied better by governed state than by a constant, because governed state also shows
*how* a consumer governs its set.

And it is worse than unnecessary. RedStone rotates: ADR 30 records that the captured
addresses are unchanged since the capture and that a source two and a half years older
shares none of them, which is exactly why the push path has `update_signer_set` rather
than a table. A compiled roster makes that certain event a redeployment, and in LEZ a
redeployment is not a restart — a program's id is its RISC0 image id and every derived
address hashes the program id, so a changed build input presents fresh addresses and
abandons what the previous build created. Rotating a signer set would cost every open
order.

**A registration is retirable, because only the roster rotates.** The pair, the scale
and the window are fixed once written, for the reason above: they are what open orders
were priced against. That leaves a mistake with no in-place correction, and because a
trust account's address is its feed id's and LEZ never releases ownership, a single wrong
exponent would otherwise spend that feed id for the life of the build. So `deregister`
empties the account and `register` admits the emptied-and-ours state as a third
pre-state — the same round trip the push path's `deregister_feed` and `register_feed`
make, and the same three states.

A re-registration is bound to its feed by the account's *address* rather than by a stored
id, because a retired account is empty and remembers nothing. `deregister` and
`rotate_signers` compare the id they read; `register` compares the address the named feed
derives. Without that, registering `ETH` into `BTC`'s retired account writes a feed where
nothing will look for it and leaves `BTC` answering `AlreadyRegistered` to a registration
and `FeedMismatch` to a retirement — the only two ways out, both closed.

What the recovery path costs is not hidden: orders against a retired feed answer
`Deregistered` until it is registered again, and if it returns on different terms they
never fill — a changed pair
answers `AssetMismatch`, a changed scale or window answers `ScaleChanged` or
`WindowChanged`.

**Which means an order binds every term it was priced under, not only its pair.** "The
pair, the scale and the window are fixed at registration" is true of `rotate_signers` and
untrue of retire-and-re-register, so the recovery path is exactly what makes the other two
mutable. A feed re-registered with a fifteen-minute window would let a ten-minute-old
payload fill an order whose owner accepted one minute, and one re-registered at six
decimals rather than eight would rescale every limit by a hundred. So `OrderAccount` carries `decimals` and
`max_age_ms` as well, captured from the registration rather than supplied — they are the
program's parameters and not claims an owner would know to make — and `terms_still_hold`
destructures `FeedTrust` with no `..`, so a field added later stops the crate compiling
until somebody decides whether it binds an open order. That is the intended outcome rather than a gap — an order priced against a pair
this feed no longer claims is an order whose meaning changed — and it is only safe
*because* an order carries its own pair. The two decisions hold each other up: without
the recovery path a mis-registration is permanent, and without the order's own pair the
recovery path would silently reprice open orders.

*Rejected: amending a registration in place while no order exists.* It is the obvious
alternative and this program cannot implement it, because it cannot see whether an order
exists: orders live at addresses derived from order ids nobody enumerates. Counting them
on the trust account would work and would put a write on the trust account in every
`open_order`, turning a read-only account into a contended one for a check the authority
can make off-chain. Retire-and-re-register reaches the same place without that.

*Rejected: letting an owner cancel an order.* There is no withdrawal: `open_order` and
`settle` are the only entry points, `settle` is permissionless, and an order stays
settleable by anybody the moment the market crosses it, for as long as its feed is
registered. An owner who regrets a limit has no lever, and the only stop is the authority
retiring the feed, which hits every order against it at once.

That is a real property of this program and not an oversight, so it is recorded here
rather than left to be discovered. The reason is what the domain is for: the order exists
to be an *action gated on a verified price*, and cancellation is a venue's concern that
would demonstrate nothing about pull verification while adding an instruction, an account
state and a race — a cancel and a settle for the same order in one block have to resolve,
and resolving them is a trading question this consumer has no business answering.

Anybody lifting this into something real needs it, and needs it before they take the
program live rather than after. It is a signed instruction on an account its owner
already owns, so it costs one handler and no new trust: check the signer against the
stored `owner`, then empty the account the way `deregister` empties a trust. What it must
not do is reopen the id, for the reason the module header gives.

*Rejected: an authority with a revoke.* The aggregator has one because RFP-001 asks for
it. A consumer that revoked its own authority would freeze every trust account it owns
with no way back and nothing gained, so the surface is not carried here.

## Consequences

- **A rotation is a transaction, and it changes what verifies.**
  `a_rotation_changes_which_payloads_verify_without_a_redeployment` asserts it over the
  captured signatures in both directions: narrowing the roster below the set that signed
  a payload refuses that payload outright (ADR 15), and widening it invalidates nothing,
  because an added signer that did not report contributes nothing towards the threshold.
- **An authority can change who speaks for a feed under an open order.** The exposure
  that comes with the decision, and
  `an_order_opened_before_a_rotation_is_settled_under_the_roster_in_force` states it
  rather than leaving it implicit. It is the same exposure a push consumer has to
  `update_signer_set`. What an authority cannot change is what an order is priced
  against: the pair, the scale and the window are fixed at registration, and
  `rotate_signers` moves only the roster and its threshold.
- **Three derived addresses, and all three can be squatted.** The config account's
  address derives from the program id alone and a reproducible build makes the next
  one computable from published source; a trust account's derives from a feed id and an
  order's from an order id. An account that holds a balance but is still unowned can
  never be written by any program, so each of the three refuses it with its own cause
  rather than reporting "already there" — the advice differs, and for the config account
  it means the build is unusable until an input changes. This is the same hazard #48
  found on the aggregator's feed accounts.
- **An order carries the pair its owner signed for, which is what makes asset-pair
  verification real here.** U7 asks the reference consumer to *show* it, and the obvious
  reading of `verify_price` does not: passing the trust account's own pair as the expected
  one holds a value against itself, so the comparison cannot fail. What it would miss is
  concrete — a registration labelling RedStone's `BTC` feed as ETH/USD verifies real BTC
  packages, meets the threshold and passes every freshness check, and nothing in a payload
  can contradict it, because no signer attests to which assets a feed prices. So
  `open_order` takes the owner's expected pair, refuses the order if the registration
  disagrees, and stores it; `settle` passes that. Two independently written records, which
  is what the push path does with the price account's pair against the feed account's
  configuration (ADR 32) and for the same reason. Sourcing that one value from the
  registration instead fails
  `an_order_cannot_be_filled_under_a_pair_its_owner_never_signed_for` and nothing else,
  which is what a check being load-bearing looks like.
- **Eight instructions, which is more than a reference consumer would need to show
  verification alone.** Two are the domain, six are governance. That ratio is the honest
  one: the verification call is a single line, and everything around it is what makes the
  roster the consumer's rather than its caller's.
