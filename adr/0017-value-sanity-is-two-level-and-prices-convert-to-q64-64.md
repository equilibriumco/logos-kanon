# 17. Invalid values are rejected, and prices convert to the account's Q64.64 scale

- **Status**: accepted
- **Milestone**: M1 (`M1-17`)
- **Requirements**: F4, U6
- **Artefacts**: `verifier-core/src/value.rs`, `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`

## Context

Two questions arrive together, and only the first was expected.

**Sanity.** F4 and U6 ask for zero, negative, and otherwise invalid prices to be
rejected. ADR 15 applies that contract per package before threshold evaluation. This
record defines exactly which byte strings are invalid.

**Scale.** The canonical price account is:

```rust
/// Amount of `quote_asset` one unit of `base_asset` is worth.
///
/// `u128` keeps the consumer-side interface non-negative; zero is rejected on read.
pub price: u128,
```

and its constructor states what those bits mean:

> `initial_price` is a `Q64.64` fixed-point value: the real price is `initial_price / 2^64`,
> so `1.0` is `1 << 64`. The non-zero check rejects the sentinel but cannot validate scale —
> a caller is responsible for supplying a correctly-scaled fixed-point price, not a plain
> integer.

RedStone reports a price as an integer scaled by a power of ten. LEZ wants a binary
fraction. The account carries **no exponent field**, so nothing on chain reconciles the
two: a producer that writes the RedStone integer straight through is wrong by whatever
that feed's exponent happens to be, and no consumer can tell. This was not part of the
task as written; it is what reading the account's own definition produced.

## Decision

### Sanity is a package-level rejection

**A zero, a value whose top bit is set, and a value over 32 significant bytes return
`ValueOutOfRange`.** The error is immediate even when other packages already meet the
threshold, so an invalid package cannot disappear behind a valid quorum.

The top-bit case gives the requirement's "negative" price a wire-level meaning. The wire
field is unsigned, so nothing distinguishes a genuinely astronomical price from a small
negative one read as `int256`. Treating the entire upper half as invalid is conservative
and keeps a negative-looking value out of the account.

### RedStone's base-10 integers convert to Q64.64

```
price = (median << 64) / 10^decimals
```

with `decimals` configured per feed, because RedStone feeds do not all use the same
exponent. Truncation is toward zero, giving a relative error below `2^-64`, and rounding
up would let a published price sit above what the signers agreed on.

`MAX_DECIMALS = 19`. `10^19 < 2^64 < 10^20`, so this is exactly where `10^decimals` stops
fitting in a `u64` — the point at which the division would need a full 256-bit divisor
rather than a single-limb one. `ConfigError::DecimalsOutOfRange` rejects anything above it
at configuration time, so the runtime path has no exponent it cannot divide by.

`VerifyError::ScalingOutOfRange` reports a result the `u128` cannot hold. The alternative
is writing a wrapped number that looks like a price and is not one.

### The zero sentinel is unreachable by construction, so nothing checks for it

Writing zero to the price account means "no valid price" to every consumer that reads it,
which makes it worth being certain a real price can never land there. It cannot:
`2^64` exceeds `10^MAX_DECIMALS`, so the smallest non-zero median maps to `1` at every
exponent a feed may declare.

No runtime branch guards this, because a proof is stronger than a check. The inequality is
recorded here because raising `MAX_DECIMALS` would retire it silently — anyone doing so
has to reinstate the guard.

## Consequences

- `VerifiedFeed` carries both scales: `value` on RedStone's, `price` on the account's.
  M1-21 conforms this decoder against published RedStone vectors, and those vectors are in
  RedStone's scale, so dropping `value` would leave that test comparing a number this crate
  derived rather than one RedStone published.
- `ValueOutOfRange` and `ScalingOutOfRange` have separate producers: the first rejects a
  package's value and the second rejects an agreed value that cannot fit the account scale.
- Tests cover zero, negative, and over-width values both below threshold and alongside an
  otherwise sufficient quorum.
- Every consumer of the price account must know the Q64.64 convention out of band, since
  the account cannot carry it. A consumer assuming base-10 would be off by roughly
  `1.8 x 10^19` with no on-chain signal that anything is wrong. This is a question to put
  to Logos, not a choice this repository can make differently: Q64.64 is the only written
  specification that exists.

## Alternatives considered

- **Skip an invalid value and continue toward quorum.** Rejected because it makes
  `ValueOutOfRange` unobservable when the other packages suffice, contrary to F4 and U6.
- **Write the RedStone integer unchanged and let consumers scale.** The account has no
  field to carry the exponent, so every consumer would need per-feed out-of-band knowledge
  of a different number, rather than one convention for all feeds.
- **Convert as `(median / 10^decimals) << 64`.** Divides first and throws away the entire
  fractional part, turning `$3000.12` into `$3000`. The shift-first form is why the
  intermediate needs more than 128 bits.
- **A full 256-bit divisor, dropping the `MAX_DECIMALS` bound.** More code in the
  guest-reachable path, for exponents no feed uses, and it would retire the argument that
  makes the zero sentinel unreachable.
