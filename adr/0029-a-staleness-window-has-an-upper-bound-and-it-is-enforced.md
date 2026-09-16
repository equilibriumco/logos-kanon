# 29. A staleness window has an upper bound, and it is enforced

- **Status**: accepted
- **Milestone**: M1 (`M1-15`)
- **Requirements**: F4, SEC3
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`

## Context

ADR 18 made the staleness window two-sided and gave each end its own error. It fixed
the forward tolerance at RedStone's three minutes and left `maxAge` to the feed,
because RFP-020 makes it configurable at registration and SEC3 asks for a documented
production minimum. Registration rejected only a `maxAge` of zero, on the reasoning
that no package could ever be young enough to satisfy it.

The other end went unbounded, and the gap is in the check itself. `freshness` compares
against `now_ms.saturating_sub(max_age_ms)`, and the saturation is deliberate: a chain
clock below the bound is possible on a fresh devnet, and a panic in a guest aborts the
transaction.

The consequence is that the check decays as `maxAge` grows, and never visibly breaks.
Saturation itself needs `max_age_ms` to reach the clock's own reading, about fifty-six
years at a real epoch timestamp, and at that point the window's lower edge is zero and
every past timestamp is current. But the check is useless long before that: at a `maxAge`
of one year the lower edge is an ordinary timestamp, nothing saturates, and every package
anyone would ever submit is inside the window. There is no value at which something goes
wrong that an operator could notice. The feed carries a `max_age_ms` like any other, and
`verify_feed` still runs the comparison.

This repository was itself the demonstration. `NO_MAX_AGE` in the `feed` tests and
`FOREVER` in the conformance suite were both `u64::MAX`, used to mean "this test is not
about the clock". They worked exactly because the check had been disabled.

So the range had a floor and no ceiling, and the end without one is the end where the
check stops existing.

## Where the answers came from

`redstone-finance/rust-sdk` at `50cd703e`, Boost licensed, as with ADR 15 and ADR 18; the
BUSL EVM connector stays unread. ADR 18 took `MAX_TIMESTAMP_DELAY_MS` from
`crates/redstone/src/protocol/constants.rs` and read it as the bound RedStone applies.
`crates/redstone/src/core/config.rs` says what it actually is: `Config::try_new` takes
`max_timestamp_delay_ms` and `max_timestamp_ahead_ms` as `Option<TimestampMillis>` and
resolves them with `unwrap_or` against those constants, documented as "If None is provided
then default config value is used."

So both bounds are the caller's to choose and the constants are a fallback. Nothing about
what RedStone rejects in general follows from either, which is what makes the ceiling below
a policy rather than an inherited property.

## Decision

**`MAX_MAX_AGE_MS`, and `try_new` refuses anything above it.** The value is fifteen
minutes, RedStone's own `MAX_TIMESTAMP_DELAY_MS`, so the widest window Kanon permits is
no looser than the upstream default. `ConfigError::MaxAgeTooLarge` carries both the
offered value and the bound, so a caller learns what to change.

**The value is borrowed, and the policy is ours.** Adopting RedStone's default is a
decision rather than an inherited guarantee, for the reason the section above gives. It
is an easy decision — being looser than the upstream's own fallback means accepting data
that fallback drops — but a reader who mistakes it for inherited will not know it can be
revisited.

**The bound is enforced rather than published**, which is the part worth recording,
because ADR 18 published the forward tolerance and this record does something else.
A recommendation is enough when ignoring it produces a visibly worse system. It is not
enough here, for the reason the Context gives: the check degrades continuously and there
is no threshold at which it announces itself. An operator who sets a huge `maxAge` sees a
feed that still verifies, still refuses bad signatures, still reports a threshold — and
has no staleness check worth the name. A smooth decay with no failure signal is the
wrong thing to leave to configuration.

**These are two decisions, and they answer different questions.** That the range needs a
ceiling at all follows from the decay above, and holds whatever value is chosen. That the
ceiling is *fifteen minutes* follows from RedStone's default, and would change if their
default changed. Keeping them apart matters because the first is not open to revisit and
the second is.

**A feed may still be as strict as it likes.** The ceiling constrains one direction
only. A production feed belongs well under this bound, and nothing here argues for
running at fifteen minutes.

## Consequences

- Twelve variants become thirteen. `MaxAgeTooLarge` joins `MaxAgeZero`, and the two
  bracket the range rather than guarding one end of it.
- **The test suites had to stop using an illegal configuration.** `NO_MAX_AGE` and
  `FOREVER` are now `MAX_MAX_AGE_MS`, and the tests that used a bare `1` or a small
  round number as a "the clock is not the subject" timestamp now use one the test clock
  calls current. That is fifty-odd sites, all of them mechanical, and the diff is
  larger than the check it accommodates.
- **Three ordered round constants replace three magic numbers.** `ROUND_A`, `ROUND_B`
  and `ROUND_C` sit relative to `NOW_MS`, because ADR 27's tests need distinct moments
  and a bounded window means a moment has to be a plausible one.
- The guest cost harness passes the bound rather than `u64::MAX`. Its clock is the
  vector's own timestamp, so nothing it measures moves.
- **SEC3 gains an enforced invariant.** Its documented minimum can now rest on a bound
  the code holds, rather than on advice alone.

## Alternatives considered

- **Document the ceiling and leave registration permissive.** Rejected once the
  saturation was traced: a ceiling that nothing enforces is indistinguishable from no
  ceiling, and the failure mode is a feed that looks configured.
- **Reject only `u64::MAX` and values near it.** Draws a line with no meaning behind it.
  Fifteen minutes has a reason: it is the default the upstream applies to the same
  packages.
- **Clamp silently to the bound instead of refusing.** Rejected for ADR 15's reason. A
  caller that asked for something the system will not do learns about it, rather than
  running a configuration it did not choose.
- **Make the ceiling configurable by the admin authority.** More knobs on the one
  parameter whose upper end disables a security check. The point of the bound is that
  it is not a choice.
