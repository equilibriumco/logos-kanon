# Manipulation analysis, and the minimum `maxAge` to run in production

SEC3 asks for two things. The first is a minimum recommended `maxAge`, documented
for both modes. The second is a manipulation analysis that covers signer
compromise, replay of stale packages, and signer-set update delays. This document
gives both.

The analysis covers the verification core as built in M1, and the account write in
`aggregator-program`. Two of its conclusions depend on decisions that M2 and M3
have not made yet. These are the authorisation of `submit_price`, and the pull
library, which is still a stub. The text says so at each place.

## The mechanisms the analysis rests on

Seven properties of `verifier-core`, `kanon-clock` and `aggregator-program` carry
the whole analysis. Most of what follows is a consequence of these seven.

| | property | where |
| --- | --- | --- |
| 1 | "Now" comes from the pinned every-block clock account `/LEZ/ClockProgramAccount/0000001`. Kanon refuses every other account before it reads the contents. A zero or undecodable reading refuses and does not guess. | [ADR 13][adr13], [ADR 18][adr18], `kanon-clock/src/lib.rs` |
| 2 | A package timestamp must fall in `[now − max_age_ms, now + 180_000]`. Both ends are inclusive, and both ends saturate. | [ADR 18][adr18], `verifier-core/src/feed.rs` |
| 3 | `max_age_ms` is per feed and set at registration. Registration refuses zero, and refuses more than `MAX_MAX_AGE_MS`, which is fifteen minutes. | [ADR 18][adr18], [ADR 29][adr29] |
| 4 | Kanon accepts every package that carries the requested feed, or refuses the payload. It ignores a package that cannot change the answer. The one exception is a signer whose slot is already full. | [ADR 15][adr15] |
| 5 | All packages that carry one feed in one payload must describe one moment. If they do not, Kanon refuses the payload with `TimestampMismatch`. The comparison runs before signature recovery. | [ADR 27][adr27] |
| 6 | The threshold counts distinct authorised signers, one slot each. The price is the median of the filled slots. An even count averages the two middle values. | [ADR 15][adr15], F3 |
| 7 | The aggregator writes only a timestamp strictly newer than the stored one. | `aggregator-program/src/publish.rs` |

RedStone's own bounds are `MAX_TIMESTAMP_DELAY_MS = 900_000` and
`MAX_TIMESTAMP_AHEAD_MS = 180_000`, from
`crates/redstone/src/protocol/constants.rs`. Kanon matches both.

## The recommended minimum `maxAge`

### The ceiling is fifteen minutes, and the code holds it

A `maxAge` of more than 900,000 ms admits packages that RedStone itself rejects.
Registration refuses such a configuration, so the ceiling is an invariant and not
advice. [ADR 29][adr29] records why it is enforced: `freshness` saturates, so a
very large `maxAge` puts the lower edge of the window at zero. Every timestamp in
the past then passes, and the feed still looks configured.

### The floor is the sum of three latencies

If `maxAge` is less than the time an honest package needs to reach the chain, Kanon
refuses honest updates. The floor is the sum of three terms:

1. **The upstream publish interval.** The package must exist before anyone can
   submit it. RedStone sets this cadence for the feed.
2. **Relayer latency.** The relayer must fetch the payload, assemble it, submit it,
   and wait for inclusion.
3. **Clock skew between a RedStone signer and the sequencer.** A signer clock
   behind the sequencer makes a package look older than it is.

Clock granularity is not a term here. The clock account updates every block, so a
reading can lag the wall clock. A lagging reading lowers `now`, which lowers the
lower edge of the window, and a package then looks younger. Block lag pushes
toward `FuturePackage` and away from `StalePackage`, so it belongs to the forward
tolerance and not to this floor.

Two of the three terms have no measurement today. Relayer latency needs a relayer,
and the relayer arrives in M4. M2-19 exercises the push path end to end, and it is
the first point where a submission figure becomes visible. M4-04 and M4-05 set the
cadence and the retry policy of the relayer. The publish interval for each feed is
RedStone's, and M2-00 is the task that captures it.

> **Provisional recommendation: a minimum of 120,000 ms (two minutes) for push.**
> This figure is a starting point and not a derivation. Two of the three terms are
> unmeasured, so the measurements can move it in either direction. A smaller value
> risks refusing honest updates. A larger value widens the window that section
> "Replay of stale packages" describes.

For pull, the second term almost disappears. The consumer fetches the payload and
verifies it in its own transaction, so no relayer hop exists. A pull consumer can
therefore run tighter than push, once it knows its own fetch-to-submit time. Until
then the same 120,000 ms applies, with the same uncertainty.

A pull consumer must not treat `maxAge` as a formality because its own fetch was
recent. Kanon verifies the timestamp of the signer, not the timestamp of the fetch.

## Signer compromise

### The threshold is a median, so half of it is enough

This is the most important finding, and the threshold is easy to read backwards. A
feed has a threshold of M and N authorised signers. The intuition is that an
attacker must hold M keys, or must outweigh the honest majority. Both readings are
wrong.

The author of a payload chooses which packages go into it, and every honest package
is public. An attacker that holds k keys therefore assembles its own payload. It
puts in k packages of its own, and M − k honest packages from the same round. The
payload meets the threshold. Kanon takes the median over those M slots, and the
attacker placed k of them.

A median of M values follows the attacker when the attacker holds more than half:

| threshold | keys for full control | keys for unbounded movement |
| --- | ---: | ---: |
| 3 | 2 | 2 |
| 4 | 3 | 2 |
| 5 | 3 | 3 |
| 7 | 4 | 4 |

**The default of 3-of-N needs two compromised keys, not three.** An even threshold
averages the two middle values. That gives M / 2 keys unbounded upward movement
without full control, which is no better in practice.

Two consequences follow for the recommendation:

- **N does nothing.** N bounds the set an attacker draws from. It does not dilute
  what the attacker submits.
- **Only odd increases help.** A move from 3 to 4 leaves the bar at two keys. A
  move from 3 to 5 raises it to three.

With fewer keys than the table requires, a compromised key gives an attacker
nothing on value. If a payload cannot reach M distinct authorised signers, it fails
with `ThresholdNotMet`, which is a refusal and not a wrong price.

### What a higher threshold costs

Recovery is the dominant cost of verification. One package costs 585,276 cycles
against a public budget of 33,554,432 cycles (`COSTS.md`). A higher threshold
raises the cost of each update almost linearly. It also raises the liveness bar,
because M authorised signers must publish and appear in the payload. If the
threshold approaches N, one signer outage stops the feed.

Feeds that carry more value are the argument for a higher threshold on those feeds.
Both `maxAge` and the threshold are per feed.

### What `maxAge` does not do

`maxAge` is not a defence against signer compromise. A compromised key signs fresh
packages, and fresh packages pass every freshness check. Two defences remain: the
threshold, and the authorised set itself. The third section of this document covers
the set.

### What the membership check does do

Kanon refuses the payload with `UnauthorisedSigner` at the first package whose
recovered signer is outside the configured set. It does this even when the
authorised packages already meet the threshold (SEC1, [ADR 15][adr15]). An attacker
therefore cannot pad a payload with keys of its own making. Every counted key is
one the admin authority put in the set.

### An open dependency on M2-02

The authorisation of `submit_price` decides how much of this section an attacker can
reach. No requirement restricts who can submit a payload, because SEC2 governs the
signer set and not the submitter. M2-02 makes this decision.

- If submission is **permissionless**, this section holds as written. The keys in
  the table are enough on their own.
- If submission is restricted to an authorised relayer, an attacker needs those
  keys and a way to reach the relayer's payload.

A relayer restriction is worth less than it looks. It helps only to the degree that
the relayer builds payloads from packages it fetched itself, and refuses packages
from anywhere else. A relayer that forwards a payload another party assembled adds
nothing. M2-02 must decide the sourcing rule together with the authorisation rule.

## Replay of stale packages

A replay can try three different things. The three have different answers, and
[ADR 27][adr27] changed only the first.

### A package replayed into another round: refused

Three properties stand in the way. A replayed package meets whichever comes first:

- Outside the window, the package is `StalePackage`.
- Inside the window, a package that describes a different moment from the first
  package for that feed refuses the whole payload with `TimestampMismatch`
  ([ADR 27][adr27], property 5).
- A same-moment copy from a signer with a full slot is skipped and changes nothing
  ([ADR 24][adr24], property 4).

No order of these three lets an old package join a round it did not belong to.

### A whole round replayed intact: accepted, and this is the real replay case

An attacker that replays every package of one earlier round submits a payload that
verifies. The packages agree about the moment by construction, the signatures are
genuine, and the threshold is met. Nothing in [ADR 27][adr27] refuses it, because
there is no mixing to detect.

Two properties bound what the attacker gains:

- **`maxAge` bounds how old the replayed round can be.** Past that bound, the first
  package is `StalePackage`.
- **The aggregator refuses a timestamp no newer than the stored one** (property 7).
  A replay therefore cannot move a price backwards.

What remains is real. If the stored timestamp is older than the replayed round, the
account advances to a price that is genuine but out of date. If the account holds
no observation yet, the replay sets the first one. In both cases the price is a
real RedStone price from up to `maxAge` ago, published as though it were current.

This is the one manipulation vector that a smaller `maxAge` directly shrinks, and
it is the strongest argument for running well under fifteen minutes.

### A payload denied by an appended package: free

Property 5 compares timestamps before signature recovery. An appended package
therefore needs to carry the requested feed and a different timestamp, and nothing
else. It needs no key, no signature that recovers, no timestamp inside `maxAge`,
and no package that a signer ever produced. The payload fails with
`TimestampMismatch`.

`maxAge` does not bound this one. A smaller window removes nothing that the attack
uses. Neither shipping mode gives anyone the position, because a push submitter
spends its own transaction and a pull consumer owns the payload it passes in. So
the constraint falls on how a payload is assembled:

> **A submitter must own the bytes that it submits.** Some relayers assemble
> payloads from a shared queue, a third-party aggregation service, or another source
> that a second party can append to. Such a relayer gives that party a denial switch
> on the feed.

Scope bounds the denial. A refused payload is one transaction. R3 requires that an
error on one feed leaves the other feeds alone. Kanon skips packages for other feeds
before recovery ([ADR 15][adr15]). A payload that is poisoned for BTC/USD therefore
does not touch ETH/USD.

## Signer-set update delays

RedStone rotates its signer roster. The configured set in Kanon is a copy of that
roster. As a result, a window always exists between the change upstream and the
change here. The window has two failure directions, and they are not symmetric.

| direction | during the lag | character |
| --- | --- | --- |
| A signer is **removed** upstream | its key still counts here | **security**: a revoked key keeps its vote |
| A signer is **added** upstream | its packages are `UnauthorisedSigner` here | **liveness**: refusals, and the threshold can become unreachable |

The removal direction is the one that matters. RedStone can revoke a signer because
its key leaked. Kanon continues to count that signer until an admin updates the set.
The exposure window is the detection time plus the admin transaction. With the
finding in the section that precedes this one, a default 3-of-N feed needs only two
keys in this state at one time.

The mitigation is operational and not architectural: a process tracks the published
roster of RedStone and alerts on divergence from the configured set. The
signer-roster tracking process in M5 owns that work. This analysis is the reason
that the work is not optional.

The addition direction fails closed, which is the correct direction. A set that
shrinks to less than the threshold produces `ThresholdNotMet`. This is a refusal,
and not a stale or wrong price. Registration also rejects a threshold larger than
the signer count, so nobody can create the unreachable configuration on purpose.

### Push and pull are not equal here, and pull is weaker

**Push.** The set is on chain, and only the admin authority can change it (SEC2).
One update fixes every consumer of that feed at once. M2-08 builds the update path.
M2-14 tests that only the admin can take it.

**Pull.** The set belongs to the consumer, and it never comes from the payload
(SEC2, enforced by M3-03). This independence from the aggregator is the point of
pull mode. It is also the weakness of pull mode, and it is easy to miss.

A pull consumer that runs a stale signer set carries the full exposure of the
removal direction. **Nobody can fix this centrally.** No admin transaction reaches
the configuration of a consumer. Every pull consumer must track the roster itself.

This obligation belongs in the documentation of the pull library and in the SDK doc
packet, and not only here.

## Summary of recommendations

1. **Treat the threshold as a median, not a majority.** Two compromised keys
   control a 3-of-N feed.
2. **Raise the threshold from 3 to 5 on high-value feeds.** A move to 4 leaves the
   bar at two keys, because an even count averages the middle two.
3. **Set `maxAge` to 120,000 ms until the latencies are measured.** The figure can
   move in either direction. Fifteen minutes is an enforced ceiling.
4. **Own the bytes that you submit.** Do not assemble payloads from a shared or
   third-party source, and do not forward a payload another party assembled.
5. **Track the roster of RedStone, and alert on divergence.** The removal direction
   is a live security exposure for the length of the lag.
6. **Document the roster obligation for pull consumers.** Nobody can fix a pull
   consumer centrally.
7. **Decide the authorisation of `submit_price` in M2-02, with its sourcing rule.**
   Authorisation without a sourcing rule buys little.

## What this document does not cover

- The measured value of the three floor terms. The recommendation in this document
  stays provisional until M2-00 captures the publish intervals and a relayer exists
  to time.
- One question that [ADR 13][adr13] deferred to M2: does an oracle read the
  every-block clock account, or the 10-block or 50-block variant? No M2 task owns
  this question.
- Manipulation of the price formation of RedStone, upstream of the signers. Kanon
  verifies that a quorum of authorised signers said something. It cannot verify that
  what they said was correct.
- Denial of service against LEZ itself, and fee exhaustion of the submission wallet.
  M4-07 covers the wallet.

[adr13]: adr/0013-staleness-is-measured-against-the-lez-clock-program.md
[adr15]: adr/0015-redstone-parity-verification-semantics.md
[adr18]: adr/0018-timestamp-validity-is-two-sided-and-costs-one-signer.md
[adr24]: adr/0024-a-duplicate-package-is-skipped.md
[adr27]: adr/0027-one-price-comes-from-one-round-and-a-mixed-payload-is-refused.md
[adr29]: adr/0029-a-staleness-window-has-an-upper-bound-and-it-is-enforced.md
