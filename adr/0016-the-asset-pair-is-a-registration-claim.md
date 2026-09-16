# 16. The asset pair is a registration claim, checked against the caller's expectation

- **Status**: accepted
- **Milestone**: M1 (`M1-16`)
- **Requirements**: F4, F5, U6, U7
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`

## Context

F4 asks the verifier to reject "asset identifiers that do not match the registered feed",
U6 names "asset mismatch" among the failure modes a caller must be able to act on, and U7
asks both reference consumers to demonstrate asset-pair verification. Read quickly, all
three sound like a property of the payload.

They cannot be. A RedStone data point carries one field of asset identity: a 32-byte
right-padded ASCII feed id, `BTC` or `ETH/BTC`. The canonical LEZ price account carries
two `AccountId`s:

```rust
/// Canonical identifier for the priced asset.
pub base_asset: AccountId,
/// Canonical identifier for the quote asset that denominates `price`.
pub quote_asset: AccountId,
```

There is no function from the string `BTC` to an `AccountId`, and no signer signs one.
The two live in different namespaces, and the payload bridges neither. Whatever the check
is, it is not a check on signed data.

## Decision

Registration binds `feed_id -> (base_asset, quote_asset)`. `FeedConfig` carries the pair.
`verify_feed` takes the caller's expected pair as an argument and returns
`VerifyError::AssetMismatch` when the two differ.

**The pair is a parameter, not a method.** `config.expect_assets(&expected)` was the
alternative, and it is one a caller can reach a price without ever calling. A check that
is optional at the call site is a check that will eventually be skipped, and U6 requires
push and pull to report the same failure the same way — which they cannot if one of them
forgets. Making the expectation an argument means no caller reaches a price without
having stated what it thought it was asking for.

**Checked before the package walk.** The comparison is 64 bytes. Reaching the same answer
after the walk would cost one secp256k1 recovery per signer, at 565,497 cycles each, to
learn something known before the first byte was read.

**The pair is stored as given.** `try_new` does not reject an all-zero identifier or a
pair whose base equals its quote. An all-zero `AccountId` is a legitimate LEZ value, and
which pairs are meaningful is the registering authority's judgement. A `no_std` crate
with no chain dependency has no basis for overruling it.

**Opaque bytes, not `AccountId`.** ADR 2 and ADR 4 keep `verifier-core` free of chain
dependencies, so `AssetPair` is two 32-byte arrays. The aggregator and `pull-lib` map the
real identifier in at their boundary.

## What this check does not establish

It compares an expectation against a claim. Both sides are configuration. If the
registering authority binds `BTC` to `(ETH_ACCOUNT, USD_ACCOUNT)`, every check here
passes and the account is written with a Bitcoin price labelled Ether — internally
consistent and externally wrong.

Nothing in the payload can catch that, because nothing in the payload knows what `BTC`
means. The trust root for the binding is F6's admin authority, and this record exists so
that a passing asset check is not later read as evidence the payload was verified to be
about a particular asset. It was not. It was verified to be about the feed the caller
named, and the caller was told which assets that feed is registered against.

## Consequences

- `FeedConfig::try_new` and `verify_feed` both change arity, which is a breaking change
  to the only public verification entry point. Made now, while the only callers are this
  crate's own tests, rather than after `pull-lib` and the aggregator are built over it.
- `AssetMismatch` gains a producer and is the cheapest rejection in the crate.
- A consumer that registers a feed and never states an expectation cannot exist: there is
  no such call. That is the intended cost.

## Alternatives considered

- **Derive the feed id from the pair.** Would need a canonical `AccountId -> symbol`
  table maintained in this repository — a second registry, with no authority behind it
  and a drift problem the moment RedStone renames a feed.
- **Carry the pair in the payload.** Not available: the wire format is RedStone's and it
  has one asset field. Extending it would mean signers signing a Kanon-specific
  structure, which is the opposite of consuming a public payload.
- **Check the pair in the consumer rather than the verifier.** This is what U7's
  reference consumers demonstrate, but demonstrating a pattern is not the same as
  enforcing it. Both push and pull go through `verify_feed`, so that is where the check
  binds all callers rather than the well-behaved ones.
