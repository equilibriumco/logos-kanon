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

**One function over `verifier-core`'s own types, adding as little as the
requirement allows.** `verify_price` takes a `PullConfig`, an `AssetPair` and a
`VerifierBackend`, and returns `VerifiedFeed` or `VerifyError`. No wrapper error,
no builder, and no second validation: `PullConfig` is two public fields, one of
them a `FeedConfig`, and it exists only because Functionality 9 names a tuple with
a fifth member that `FeedConfig` has no business holding — see below.

That is what makes two later claims structural instead of reviewed. F9's
independence from the aggregator holds because there is no aggregator type in the
signature and no path that could reach one, which M3-02 asserts mechanically.
U6's parity between the modes holds because both modes return the same enum from
the same call — M3-04 still asserts it, but it is asserting an identity rather
than a coincidence.

**The clock is the account, not a parameter**, and the guarantee is narrower here
than in push mode. `verify_price` takes the clock account's id and data and builds
the reading itself, refusing anything but the pinned every-block account, rather
than accepting a `TimeSource` the consumer supplies.

What that catches is the mistake, and the mistake is likely: the 10- and 50-block
accounts hold real timestamps, so reaching for the wrong one produces a plausible
answer rather than an obvious failure
([ADR 13](0013-staleness-is-measured-against-the-lez-clock-program.md)).

What it cannot catch is fabrication. The id and the data arrive as two independent
slices, so a consumer passing `CLOCK_ACCOUNT_ID` beside bytes of its own choosing
is believed. Binding them requires the `AccountWithMetadata` a dispatcher hands a
program, and that type is in a `std` crate this one cannot depend on — the same
constraint that makes `kanon-clock` declare the clock's layout rather than import
it. The push aggregator has no such residual, because there the dispatcher hands
`submit_price` an account whose id and data are fields of one struct.

That asymmetry is worth stating precisely, because it also settles who the check
protects. In push mode a relayer supplies the clock and a consumer reading the
account afterwards bears the risk, so pinning is a boundary against a third party
and the push path can enforce it. In pull mode the program supplies its own clock,
so a program that fabricates one is misleading its own users rather than being
attacked — and the residual is therefore an obligation on the consumer, not a hole
in this crate. `reference-consumers/pull` is where it is demonstrated (M3-06).

**The `dataServiceId` is carried, and it authenticates nothing.** RFP-020's
Functionality 9 asks for a configured
`(dataServiceId, feedId, authorised signer set, M-of-N threshold, maxAge)`, so
`PullConfig` is that tuple: the label, plus the `FeedConfig` that holds the other
four.

It cannot be verified. A data package carries a 32-byte feed id and a value, and
the service appears nowhere in the signed span. The only variable slot in the
envelope is the unsigned metadata, which sits outside every signature — so even
where a service id were present, whoever assembled the payload chose it. Both
sample envelopes carry zero metadata bytes, including the one RedStone serialised
themselves (`verifier-core/tests/vectors/redstone-sdk-sample-payload.hex`), but
the absence is the weaker half of the argument; the unsignability is the whole of
it.

So the signer set is the binding, and the label is the consumer's own record of
which service its roster came from — for its logs and its own error paths. Those
addresses *are* the data service: a package signed by anyone else is
`UnauthorisedSigner` whatever service it might claim to come from. `FEEDS.md`
records which roster served which service when it was measured, and
[ADR 30](0030-the-signer-set-stays-per-feed.md) records why the set stays per
feed.

**It stays out of the push path**, which the proposal does not ask for: `register_feed`
is specified as `(asset id, M-of-N, authorized signer set)` and RFP-020 names the
service only under Functionality 9, here, and Functionality 8, the relayer's
configurable fetch (M4-02). Putting it on the aggregator's stored feed as well
would change the account layout, the committed IDL and registration for something
nothing was promised.

## Consequences

- **A consumer's mistake is a configuration error, not a verification failure.**
  Passing a signer set from the wrong data service produces
  `UnauthorisedSigner`, which names the right thing — the keys — rather than
  implying the payload was for another service.
- **The `expected` pair is not the check it is in push mode.** `verify_feed`
  compares the caller's pair against the one in the `FeedConfig` it was handed,
  which ADR 16 makes a claim tested against a registration. Push mode reads that
  configuration from a registered feed account, so the comparison spans two
  parties. Pull mode has no registration: both operands come from the same
  consumer, so `AssetMismatch` means it disagreed with itself. Nothing weaker was
  available — no signer attests to which assets a feed prices — and the parameter
  is worth keeping, because a consumer holding one `FeedConfig` per feed and one
  expectation per call site still catches crossing them. What it does not catch is
  the same wrong feed id in both, which is why this is stated in the module header
  beside the other two limits of its kind rather than left to be inferred.
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

- **A `PullConfig` that validates, and converts to `FeedConfig` internally.**
  Distinct from the transparent two-field struct that exists: a wrapper that
  re-checked its contents would buy a second set of errors describing the same
  faults and a real question about which of the two `verifier-core`'s tests are
  about. `PullConfig` holds a `FeedConfig` rather than restating it, so there is
  still one validated configuration type behind both modes.
- **Leaving `dataServiceId` out and documenting why.** The technical argument
  stands on its own — the field verifies nothing — and it is not this repository's
  to make: Functionality 9 is a hard requirement that names the tuple, and an
  internal record cannot amend an accepted deliverable. Carrying an unread field
  is the smaller cost, and saying plainly that it is unread is what keeps it from
  being mistaken later for a check.
- **Take `&impl TimeSource`.** More flexible, easier to fake in a test, and it
  makes staleness advisory for every consumer that wants it to be. Refused for
  the reason ADR 13 gives.
- **Take the clock as a `LezClock` the consumer built.** A middle position, and
  it moves the pinning check to the consumer's own code, where it is one line to
  forget. The account is the thing a LEZ program is handed, so the account is
  what the function takes.
