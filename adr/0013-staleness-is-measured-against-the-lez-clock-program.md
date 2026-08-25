# 13. Staleness is measured against the LEZ clock program's every-block account

- **Status**: accepted
- **Milestone**: M1 (`M1-08` answered; implemented by `M1-14`, `M1-15`)
- **Requirements**: F4, SEC3
- **Artefacts**: `verifier-core/`, and two upstreams: the clock program at
  `lez/programs/clock` in `logos-blockchain/logos-execution-zone` `a58fbce2`, the revision
  the product's lockfiles resolve, and `programs/twap_oracle` in
  `logos-blockchain/lez-programs` `4363f139` for the precedent cited below

## Context

F4 requires rejecting stale packages against a configured `maxAge`. That needs a
trustworthy "now" inside a LEZ program, and M1-08 was the task of finding one.

Bedrock's time service is not the answer, despite the sequencer blocking on Bedrock's
`/time/info` at startup: that is a startup dependency and nothing more (ADR 12).

The real hazard is not finding *a* clock. It is where the clock comes from. A
caller-supplied timestamp — a parameter, or an account the caller chooses — makes a
stale price look current, which defeats the entire staleness check and, with it, the
main defence against replay of an old signed package.

## Decision

**The time source is LEZ's `clock` system program**, which the sequencer invokes as the
mandatory last transaction of every block. It maintains three accounts, each holding
`ClockAccountData { block_id: u64, timestamp: u64 }`:

| account id | refreshed |
| --- | --- |
| `/LEZ/ClockProgramAccount/0000001` | every block |
| `/LEZ/ClockProgramAccount/0000010` | every 10 blocks |
| `/LEZ/ClockProgramAccount/0000050` | every 50 blocks |

**The clock is taken as an input account, and any account other than the pinned
every-block one is rejected.** M1-14 wraps the account rather than accepting a
parameter; M1-15 rejects anything whose id is not the pinned one. This follows
`twap_oracle`'s `publish_price` in `logos-blockchain/lez-programs`, which panics unless
the clock it is given is the every-block account. Its stated reasoning is the same as
RFP-020's SEC3 requirement, arrived at independently upstream.

**Units are asserted, not assumed.** The clock timestamp is the sequencer's own wall
clock, `chrono::Utc::now().timestamp_millis()`, so it is **milliseconds**. RedStone
package timestamps are milliseconds too, so the comparison needs no conversion — which
is exactly the kind of assumption that is silently off by a factor of a thousand when it
goes unrecorded. M1-15 asserts it.

## Consequences

- Staleness cannot be defeated by a caller, which is what SEC3's manipulation analysis
  needs to be able to say.
- An existing LEZ convention is followed rather than a new one invented, so an auditor
  reading both programs sees the same pattern.
- The clock account becomes a required input to every verification that checks
  staleness, which shapes the aggregator's instruction signature and the pull library's
  API.
- **Open for M2:** reading the every-block account in every `submit_price` may put every
  submission in contention on one account. The 10- and 50-block variants exist for
  exactly that trade — freshness against contention — and which one an oracle should
  read is a question to settle with a measurement in M2 rather than by choosing now.
  Nothing in M1 forecloses that choice; only the *pinned-and-checked* property has to
  survive it.

## Alternatives considered

- **A caller-supplied timestamp parameter.** Rejected: it makes staleness advisory.
- **Bedrock's `/time/info`.** Rejected — it is a sequencer startup dependency, not a
  value a program can read.
- **The 10- or 50-block clock account, chosen now.** Deferred to M2, where a contention
  measurement can decide it.
