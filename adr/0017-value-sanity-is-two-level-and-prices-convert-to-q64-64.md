# 17. Value sanity is two-level, and prices convert to the account's Q64.64 scale

- **Status**: accepted
- **Milestone**: M1 (`M1-17`)
- **Requirements**: F4, U6
- **Artefacts**: `verifier-core/src/value.rs`, `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`

## Context

Two questions arrive together, and only the first was expected.

**Sanity.** F4 and U6 both ask for a "zero or negative price" to be rejected. ADR 15
decided the opposite for zero — it is *skipped*, exactly as RedStone skips it — and
corrected an earlier revision of itself that had rejected oversized values, because
rejecting let one configured signer deny a feed to everyone by sending one bad number.
That is the failure an M-of-N threshold exists to survive. So the requirement and the
existing decision point in different directions.

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

### Sanity applies at two levels, and the two answers differ

**Per data point: skip.** A zero, a value whose top bit is set, and a value over 32
significant bytes are all skipped. Each costs its own signer's slot and nothing more.
ADR 15's reasoning is unchanged and is the reason: one configured signer must not be able
to deny a feed.

The top-bit case is the new one. The wire field is unsigned, so nothing distinguishes a
genuinely astronomical price from a small negative one read as `int256`. Treating the
entire upper half of the range as unusable costs nothing real and is the only reading
under which "negative price" means anything at all here.

**Per result: report.** If the authorised signers that *did* report were skipped for
their values, and counting them would have reached the threshold, the values are the
fault and `ValueOutOfRange` says so. The rule is the causal test ADR 15 already
established for `UnauthorisedSigner`, applied to a second cause, and evaluated in this
order:

```
met >= threshold                     -> a price
met + spoiled >= threshold           -> ValueOutOfRange
met + distinct_unknowns >= threshold -> UnauthorisedSigner
otherwise                            -> ThresholdNotMet
```

`spoiled` counts distinct **configured** signers that supplied the requested feed and
whose every value for it was unusable. Three authorised signers all reporting zero against
a threshold of three is a different fault from three signers never signing, and an
operator told "threshold not met" would go looking for signers that are already there.

`ValueOutOfRange` is tested before `UnauthorisedSigner` because a configured signer that
did report is a nearer and more actionable fault than a stranger who was never configured.

The asymmetry is not a compromise between the requirement and ADR 15. One signer can only
spoil its own slot, so `spoiled` reaching the threshold means the fault is already spread
across signers the feed trusts to that degree — which is precisely the condition under
which reporting is safe.

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
- `ValueOutOfRange` and `ScalingOutOfRange` gain producers. Of the variants ADR 15 shipped
  declared and unused, `StalePackage` remains — M1-15 gives it one — and `InvalidSignature`
  remains without one by design, for the reason ADR 15 gives.
- `a_zero_value_does_not_count_toward_the_threshold` now asserts `ValueOutOfRange` where it
  asserted `ThresholdNotMet`. That is the behaviour change ADR 15 anticipated this task
  would make, named there in advance.
- Every consumer of the price account must know the Q64.64 convention out of band, since
  the account cannot carry it. A consumer assuming base-10 would be off by roughly
  `1.8 x 10^19` with no on-chain signal that anything is wrong. This is a question to put
  to Logos, not a choice this repository can make differently: Q64.64 is the only written
  specification that exists.

## Alternatives considered

- **Reject zero and negative per data point, as F4 reads literally.** Rejected for
  ADR 15's reason, which has not changed: it is a denial-of-service primitive costing one
  package. The two-level rule satisfies what the requirement is *for* — an actionable
  answer when values are the problem — without reintroducing it.
- **Write the RedStone integer unchanged and let consumers scale.** The account has no
  field to carry the exponent, so every consumer would need per-feed out-of-band knowledge
  of a different number, rather than one convention for all feeds.
- **Convert as `(median / 10^decimals) << 64`.** Divides first and throws away the entire
  fractional part, turning `$3000.12` into `$3000`. The shift-first form is why the
  intermediate needs more than 128 bits.
- **A full 256-bit divisor, dropping the `MAX_DECIMALS` bound.** More code in the
  guest-reachable path, for exponents no feed uses, and it would retire the argument that
  makes the zero sentinel unreachable.
