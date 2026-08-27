# [M3-01:01]. The pull library is a call, not a layer

- **Status**: accepted
- **Milestone**: M3 (`M3-01`)
- **Requirements**: F9, U6, SEC1
- **Artefacts**: `pull-lib/src/lib.rs`, `pull-lib/tests/pull.rs`

## Context

Public-mode pull runs verification inside a *consumer's* own program: the
consumer holds the signer set, the threshold and the staleness window, passes a
payload it obtained however it likes, and gets a price or a reason. F9 adds that
such a consumer must be able to integrate without registering anything against
the aggregator.

Everything a verification decides already exists in `verifier-core`, tested once
for both modes (ADR 2). So the question this task actually had to answer was not
what to verify but how much crate to put in front of it — and one detail of that
turns out to carry a security property rather than an ergonomic one.

## Decision

**One function over `verifier-core`'s own types, adding none of its own.**
`verify_price` takes a `FeedConfig`, an `AssetPair` and a `VerifierBackend`, and
returns `VerifiedFeed` or `VerifyError`. No wrapper config, no wrapper error, no
builder.

That is what makes two later claims structural instead of reviewed. F9's
independence from the aggregator holds because there is no aggregator type in the
signature and no path that could reach one, which M3-02 asserts mechanically.
U6's parity between the modes holds because both modes return the same enum from
the same call — M3-04 still asserts it, but it is asserting an identity rather
than a coincidence.

**The clock is the account, not a parameter.** `verify_price` takes the clock
account's id and data and builds the reading itself, refusing anything but the
pinned every-block account. A `TimeSource` parameter is the obvious ergonomic
choice and it hands every consumer the ability to defeat its own staleness check,
which is exactly what [ADR 13](0013-staleness-is-measured-against-the-lez-clock-program.md)
exists to prevent. Pull mode is where that matters most, because here the caller
is the party the check protects. `kanon-clock` had already said as much — it
describes itself as the one implementation shared by both modes, "because the
pinned-account rejection is the whole of the guarantee" — so this is M1's
decision reaching its second caller rather than a new one.

**A payload names no data service, and the signer set is the binding.** RFP-020
lists `dataServiceId` in the pull configuration, and there is nothing to check it
against: a data package carries a 32-byte feed id and a value, and the service
appears nowhere in the signed span. The only variable slot in the envelope is the
unsigned metadata, which sits outside every signature — so even where a service
id were present, whoever assembled the payload chose it, and a check against it
would verify nothing. Both sample envelopes carry zero metadata bytes, including
the one RedStone serialised themselves
(`verifier-core/tests/vectors/redstone-sdk-sample-payload.hex`), but the absence
is the weaker half of the argument; the unsignability is the whole of it.

So the configuration carries no such field. What the documentation says instead
is that those addresses *are* the data service: a package signed by anyone else
is `UnauthorisedSigner` whatever service it might claim to come from. `FEEDS.md`
records which roster served which service when it was measured, and
[ADR 30](0030-the-signer-set-stays-per-feed.md) records why the set stays per
feed.

## Consequences

- **A consumer's mistake is a configuration error, not a verification failure.**
  Passing a signer set from the wrong data service produces
  `UnauthorisedSigner`, which names the right thing — the keys — rather than
  implying the payload was for another service.
- **`pull-lib` gains a dependency on `kanon-clock`.** Both are `no_std` and the
  CI `no-std` job already builds all three of `verifier-core`, `pull-lib` and
  `kanon-clock` for `riscv32im-unknown-none-elf`, so the constraint that keeps
  them guest-reachable is unchanged.
- **The pull and push paths take "now" the same way**, from an account whose id
  is checked, which is what lets M3-04 compare the two on `NoClock` as well as on
  the payload failures.
- **The clock is settled before the payload is even framed**, and that ordering
  is asserted rather than commented: with bytes that are not a payload and an
  account that is not the clock, the answer is the clock's.
- **`decimals` is the consumer's too, and the RFP does not list it.** The
  configuration needs it because the price account's `Q64.64` scale is derived
  from it (ADR 17), so a consumer that omits it cannot get a price at all. It is
  in `FeedConfig` for both modes and documented here rather than being treated as
  an extra.

## Alternatives considered

- **A `PullConfig` of its own, converted to `FeedConfig` internally.** It reads
  like the tidier public surface and it buys a second type to keep in step, a
  second set of validation errors, and a real question about which of the two
  `verifier-core`'s tests are about. The whole value of one verification core is
  that there is one configuration type in front of it.
- **Take `&impl TimeSource`.** More flexible, easier to fake in a test, and it
  makes staleness advisory for every consumer that wants it to be. Refused for
  the reason ADR 13 gives.
- **Take the clock as a `LezClock` the consumer built.** A middle position, and
  it moves the pinning check to the consumer's own code, where it is one line to
  forget. The account is the thing a LEZ program is handed, so the account is
  what the function takes.
- **Carry `dataServiceId` as an unchecked label** for diagnostics. Matches the
  RFP's wording literally, and adds a configuration field no verification reads —
  which is the kind of field that later gets mistaken for one that does. It is
  also asymmetric with the push path, whose stored feed has no such field, and
  M3-04 exists to keep the two comparable.
