# 24. A duplicate package is skipped

- **Status**: accepted; the cost of
  skipping is bounded by [ADR 26](0026-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md),
  and which of a signer's packages counts is decided by
  [ADR 28](0028-one-price-comes-from-one-round-chosen-by-consensus.md)
- **Milestone**: M1 (`M1-13`)
- **Requirements**: F3, SEC1
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `COSTS.md`

## Context

ADR 15 decided count-and-ignore on two axes — an unrequested feed is skipped, an
unauthorised signer is skipped — and gave the reason: a package that costs an
attacker nothing to produce must not be able to fail the payload for everyone.
An unrecoverable signature is the clearest case. It needs no key and no valid
signature, so rejecting on it would be the cheapest denial of service in the
design.

A signer's second package for the same feed is a third axis, and the obvious
answer there is the opposite one: reject, because a signer occupying two slots
reaches any threshold alone.

The argument is right and the rejection does no work for it. `reported[index]`
is one cell per configured signer and `met` counts filled cells, so a signer
that sends fifty packages still moves the count by one. Failing the payload as
well would be doing nothing the data structure was not already doing — and it
would be reachable by anyone.

**A duplicate needs no key either.** A payload's packages are public, and adding
one means bumping the count and splicing bytes into the package region. Two
routes, neither of which involves signing anything:

- **Copy a package already in the payload**, byte for byte. Every field
  matches, including the signature, because it is the same signature.
- **Replay an older package from the same signer** that is still inside
  `maxAge`. RedStone rounds are about ten seconds apart and the default window
  is three minutes, so roughly eighteen validly-signed packages per signer are
  available at any moment.

Either one denies the feed to every consumer of that payload. This is precisely
the attacker ADR 15 built the rest of its semantics against, arriving through the
one door left open.

The second route also settles what the fix cannot be. Skipping only when the
values agree, and erroring when they disagree, closes the copy and leaves the
replay: two different prices from one signer is exactly what a replayed package
supplies.

## Decision

**A second package from a signer that already filled its slot is skipped, and
`ReoccurringSigner` is never constructed.** It stays in the enum, documented as
having no producer, alongside `InvalidSignature` and for the same reason: both
name a fault in one package, and failing a payload over one package is a free
denial of service. Both become reportable in a single-package API, where the
caller names one package and expects that package to verify.

**Which of a signer's packages then counts is not decided here.**
`for_each_package` walks the payload from the tail, so "whichever arrived first"
would mean "whichever an attacker appended last", and the verified price would
depend on where in the payload the bytes sit. Skipping the duplicate is what
makes that question askable at all — until it stops being fatal there is nothing
to choose between — and
[ADR 28](0028-one-price-comes-from-one-round-chosen-by-consensus.md) answers it,
by settling the round the payload speaks for and taking each signer's package
from that round. What matters here is the property the answer has to preserve:
the price is a function of the set of packages, not of their order.

## Consequences

- **The order-independence is asserted, not just intended.**
  `a_package_from_another_round_cannot_take_the_round_from_the_signers` runs the
  same replay, older and newer, and requires the same median and the same
  timestamp from both.
- **Three tests changed their expected answer** from `ReoccurringSigner` to the
  threshold failure it is. They are the same payloads asserting the same
  property — one signer counts once — which is the point: the property never
  depended on the error.
- **`no_two_failure_modes_answer_with_the_same_variant` compares nine causes
  rather than ten.** "One signer twice" is no longer a distinct failure mode,
  because it is no longer a failure.
- **One `[u64; MAX_SIGNERS]` more guest frame, and the published figures moved.**
  The floor grows by 5 cycles at every signer count; a whole update costs 122
  more at one signer and 132 *fewer* at five, as the remainder's codegen
  shuffled. Under 0.02% either way, against 585,274 cycles per recovery.
  `COSTS.md` and `methods/tests/cost.rs` carry the new numbers.
- **A signer that signs two different values under one timestamp is still
  resolved by position.** Reaching that needs the signer's key, so it is
  misbehaviour rather than an attack, and no ordering rule can make one of two
  equally-attested values the right one.

## Alternatives considered

- **Skip only when the values agree.** Closes the copy, leaves the replay. It
  reads as the conservative option and is the one that keeps the vulnerability.
- **First package walked wins, with no timestamps.** One line, no extra frame,
  and it closes the denial of service. Rejected because the walk runs from the
  tail: it would hand whoever appends last the choice of which of a signer's
  in-window prices counts. Whether that matters depends on how `submit_price`
  gates submission, which M2-02 has not decided — and a security fix should not
  rest on an assumption about a component that does not exist yet.
- **Keep the rejection and rely on the relayer to send clean payloads.** The
  relayer is not the threat model. Any party that can put bytes in front of the
  verifier can append a package.
