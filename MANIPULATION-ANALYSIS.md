# Manipulation analysis, and the minimum `maxAge` to run in production

SEC3 asks for two things. The first is a minimum recommended `maxAge`, documented
for both modes. The second is a manipulation analysis that covers signer
compromise, replay of stale packages, and signer-set update delays. This document
gives both.

The analysis covers the verification core as built in M1. Two of its conclusions
depend on decisions that M2 and M3 have not made yet. These are the authorisation
of `submit_price`, and the pull library, which is still a stub. The text says so
at each place.

## The mechanisms the analysis rests on

Six properties of `verifier-core` and `kanon-clock` carry the whole analysis. Most
of what follows is a consequence of these six properties.

| | property | where |
| --- | --- | --- |
| 1 | "Now" comes from the pinned every-block clock account `/LEZ/ClockProgramAccount/0000001`. Kanon refuses every other account before it reads the contents. A zero or undecodable reading refuses and does not guess. | [ADR 13][adr13], [ADR 18][adr18], `kanon-clock/src/lib.rs` |
| 2 | A package timestamp must fall in `[now − max_age_ms, now + 180_000]`. Both ends are inclusive, and both ends saturate. | [ADR 18][adr18], `verifier-core/src/feed.rs` |
| 3 | `max_age_ms` is per feed and set at registration. The forward tolerance is a constant, RedStone's own `MAX_TIMESTAMP_AHEAD_MS`. | [ADR 18][adr18] |
| 4 | Kanon accepts every package that carries the requested feed, or refuses the payload. It ignores a package that cannot change the answer. The one exception is a signer whose slot is already full. | [ADR 15][adr15] |
| 5 | All packages that carry one feed in one payload must describe one moment. If they do not, Kanon refuses the payload with `TimestampMismatch`. | [ADR 27][adr27] |
| 6 | The threshold counts distinct authorised signers, one slot each. The price is the median of the filled slots. | [ADR 15][adr15], F3 |

RedStone's own bounds are `MAX_TIMESTAMP_DELAY_MS = 900_000` and
`MAX_TIMESTAMP_AHEAD_MS = 180_000`, from
`crates/redstone/src/protocol/constants.rs`. Kanon matches the forward bound
exactly. The operator sets the backward bound, which is the subject of the next
section.

## The recommended minimum `maxAge`

### Fifteen minutes is a ceiling, not a recommendation

At more than 900,000 ms, Kanon accepts packages that RedStone itself rejects. No
benefit comes from a bound more permissive than the source of the data. As a
result, fifteen minutes is a hard ceiling.

### The floor is the sum of four latencies

If `maxAge` is less than the time an honest package needs to reach the chain,
Kanon refuses honest updates. The floor is the sum of four terms:

1. **The upstream publish interval.** The package must exist before anyone can
   submit it. RedStone sets this cadence for the feed.
2. **Relayer latency.** The relayer must fetch the payload, assemble it, submit
   it, and wait for inclusion.
3. **One block of clock granularity.** The clock account updates every block. As a
   result, `now` can be one block behind the wall clock when a program reads it.
4. **Clock skew between a RedStone signer and the sequencer.** The fixed three
   minutes allows for this skew on the forward side. It also consumes part of the
   backward budget.

Two of the four terms have no measurement today. **This repository does not record
LEZ block time.** Relayer latency needs a relayer, and the relayer arrives in M4.
M2-19 exercises the push path end to end, and it is the first point where a
submission figure becomes visible. M4-04 and M4-05 set the cadence and the retry
policy of the relayer.

> **Provisional recommendation: a minimum of 120,000 ms (two minutes) for push.**
> This value is more than any plausible sum of the four terms, and much less than
> RedStone's fifteen minutes. It is provisional in one direction only. The
> measurements can justify a lower value, and a lower value shrinks the replay
> pool that this document describes further down.

For pull, the second term almost disappears. The consumer fetches the payload and
verifies it in its own transaction, so no relayer hop exists. As a result, a pull
consumer can run tighter than push. The recommendation is the same 120,000 ms. A
pull consumer that chooses a lower value is safe. A pull consumer that chooses a
higher value gets a proportionally larger replay pool.

A pull consumer must not treat `maxAge` as a formality because its own fetch was
recent. Kanon verifies the timestamp of the signer, not the timestamp of the
fetch.

Registration rejects a `max_age_ms` of zero. As a result, no configuration
disables the check.

## Signer compromise

### N does not dilute an attacker who assembles the payload

This is the most important finding, and the threshold is easy to read backwards. A
feed has a threshold of M and N authorised signers. The intuition is that an
attacker must outweigh the honest majority. This intuition is wrong, because the
author of the payload chooses which packages go into it.

An attacker that holds M signer keys submits a payload with only its own M
packages. An authorised signer signed every one of them, and they all describe one
moment. Kanon takes the median over exactly those M slots.

**M compromised keys give complete control of the reported price, for any N.** N
bounds the size of the set that an attacker draws from. N does nothing to dilute
what the attacker submits. M is the entire security parameter here.

Below M, a compromised key gives an attacker nothing. The payload fails with
`ThresholdNotMet`, which is a refusal and not a wrong price.

### What a higher M costs

Recovery is the dominant cost of verification. One package costs 585,276 cycles
against a public budget of 33,554,432 cycles (`COSTS.md`). A higher M raises the
cost of each update almost linearly. A higher M also raises the liveness bar,
because M authorised signers must publish and appear in the payload. If the
threshold approaches N, one signer outage stops the feed.

The default is 3-of-N (F3). As a result, **three compromised keys are enough** on
a default feed. For a five-signer service, that is three of five. Feeds that carry
more value are the argument for a higher M on those feeds. Both `maxAge` and the
threshold are per feed.

### What `maxAge` does not do

`maxAge` is not a defence against signer compromise. A compromised key signs fresh
packages, and fresh packages pass every freshness check. Two defences remain: M,
and the authorised set itself. The third section of this document covers the set.

### What the membership check does do

Kanon refuses the payload with `UnauthorisedSigner` at the first package whose
recovered signer is outside the configured set. It does this even when the
authorised packages already meet the threshold (SEC1, [ADR 15][adr15]). As a
result, an attacker cannot pad a payload with keys of its own making to reach M.
The admin authority must put every counted key into the set.

### An open dependency on M2-02

The authorisation of `submit_price` decides how much of this section an attacker
can reach. No requirement restricts who can submit a payload, because SEC2 governs
the signer set and not the submitter. M2-02 makes this decision.

- If submission is **permissionless**, this section holds as written. M
  compromised keys are enough on their own.
- If submission is restricted to an authorised relayer, an attacker needs M
  compromised signer keys and the position of the relayer. This is harder.

This is a security-relevant fork, and it belongs in the record of M2-02.

## Replay of stale packages

The answer to this threat changed during M1, and [ADR 27][adr27] is the record of
the change. A replay can try two different things, and the two need separate
treatment.

### To move a price: refused

Three properties stand in the way. A replayed package meets whichever comes first:

- Outside the window, the package is `StalePackage`.
- Inside the window, a package that describes a different moment from the first
  package for that feed refuses the whole payload with `TimestampMismatch`
  ([ADR 27][adr27], property 5).
- A same-moment copy from a signer with a full slot is skipped and changes nothing
  ([ADR 24][adr24], property 4).

No order of these three lets an old signed package contribute to a median.

### To deny a feed: live, and bounded by `maxAge`

Every package inside `maxAge` is public and validly signed. Anyone who can add
bytes to a payload before verification can append one such package, and the
payload then fails with `TimestampMismatch`. This needs no key and no signing,
only a copy of something public. [ADR 27][adr27] accepted this cost, because
[ADR 15][adr15] already made the same trade for the other four package properties.

Neither shipping mode gives anyone that position. A push submitter spends its own
transaction. A pull consumer owns the payload that it passes in. As a result, M2
gets one specific constraint:

> **A submitter must own the bytes that it submits.** Some relayers assemble
> payloads from a shared queue, a third-party aggregation service, or another
> source that a second party can append to. Such a relayer gives that party a
> denial switch on the feed.

`maxAge` bounds the pool that an attacker draws from. The number of replayable
rounds per signer is `maxAge` divided by the upstream publish interval. If you
halve `maxAge`, you halve the pool. This is the second reason to run at much less
than fifteen minutes. The first reason is plain freshness.

Scope bounds the denial as well as pool size. A refused payload is one
transaction. R3 requires that an error on one feed leaves the other feeds alone.
Kanon skips packages for other feeds before recovery ([ADR 15][adr15]). As a
result, a payload that is poisoned for BTC/USD does not touch ETH/USD.

## Signer-set update delays

RedStone rotates its signer roster. The configured set in Kanon is a copy of that
roster. As a result, a window always exists between the change upstream and the
change here. The window has two failure directions, and they are not symmetric.

| direction | during the lag | character |
| --- | --- | --- |
| A signer is **removed** upstream | its key still counts toward M here | **security**: a revoked key keeps its vote |
| A signer is **added** upstream | its packages are `UnauthorisedSigner` here | **liveness**: refusals, and M can become unreachable |

The removal direction is the one that matters. RedStone can revoke a signer
because its key leaked. Kanon continues to count that signer until an admin
updates the set. The exposure window is the detection time plus the admin
transaction. With the signer-compromise finding in the section that precedes this
one, a default 3-of-N feed needs only three keys in this state at one time.

The mitigation is operational and not architectural: a process tracks the
published roster of RedStone and alerts on divergence from the configured set. The
signer-roster tracking process in M5 owns that work. This analysis is the reason
that the work is not optional.

The addition direction fails closed, which is the correct direction. A set that
shrinks to less than the threshold produces `ThresholdNotMet`. This is a refusal,
and not a stale or wrong price. Registration also rejects a threshold larger than
the signer count. As a result, nobody can create the unreachable configuration on
purpose.

### Push and pull are not equal here, and pull is weaker

**Push.** The set is on chain, and only the admin authority can change it (SEC2).
One update fixes every consumer of that feed at once. M2-08 builds the update
path. M2-14 tests that only the admin can take it.

**Pull.** The set belongs to the consumer, and it never comes from the payload
(SEC2, enforced by M3-03). This independence from the aggregator is the point of
pull mode. It is also the weakness of pull mode, and it is easy to miss.

A pull consumer that runs a stale signer set carries the full exposure of the
removal direction. **Nobody can fix this centrally.** No admin transaction reaches
the configuration of a consumer. Every pull consumer must track the roster itself.

This obligation belongs in the documentation of the pull library and in the SDK
doc packet, and not only here.

## Summary of recommendations

1. **Set `maxAge` to a minimum of 120,000 ms in both modes.** Fifteen minutes is a
   hard ceiling. This value is provisional until a relayer exists to measure it.
2. **Raise M higher than the 3-of-N default on high-value feeds.** M is the only
   defence against signer compromise, and it is per feed.
3. **Own the bytes that you submit.** Do not assemble payloads from a shared or
   third-party source.
4. **Track the roster of RedStone, and alert on divergence.** The removal
   direction is a live security exposure for the length of the lag.
5. **Document the roster obligation for pull consumers.** Nobody can fix a pull
   consumer centrally.
6. **Decide the authorisation of `submit_price` in M2-02.** Use this analysis as
   input to that decision.

## What this document does not cover

- LEZ block time, which is one term of the `maxAge` floor. This repository does
  not record it.
- One question that [ADR 13][adr13] deferred to M2: does an oracle read the
  every-block clock account, or the 10-block or 50-block variant? No M2 task owns
  this question. Both this item and LEZ block time bear on how tight a `maxAge`
  can be.
- Manipulation of the price formation of RedStone, upstream of the signers. Kanon
  verifies that a quorum of authorised signers said something. It cannot verify
  that what they said was correct.
- Denial of service against LEZ itself, and fee exhaustion of the submission
  wallet. M4-07 covers the wallet.

[adr13]: adr/0013-staleness-is-measured-against-the-lez-clock-program.md
[adr15]: adr/0015-redstone-parity-verification-semantics.md
[adr18]: adr/0018-timestamp-validity-is-two-sided-and-costs-one-signer.md
[adr24]: adr/0024-a-duplicate-package-is-skipped.md
[adr27]: adr/0027-one-price-comes-from-one-round-and-a-mixed-payload-is-refused.md
