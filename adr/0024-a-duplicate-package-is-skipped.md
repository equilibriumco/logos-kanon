# 24. A duplicate package is skipped

- **Status**: accepted; the cost of
  skipping is bounded by [ADR 25](0025-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md),
  and which of a signer's packages counts is decided by
  [ADR 27](0027-one-price-comes-from-one-round-and-a-mixed-payload-is-refused.md)
- **Milestone**: M1 (`M1-13`)
- **Requirements**: F3, SEC1
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `COSTS.md`

## Context

ADR 15 rejects signatures, signers, ages, and values strictly according to the
accepted contract. A signer's second otherwise-valid package is different: none
of those checks failed, and the security property is only that one signer must
not occupy two threshold slots.

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

Rejecting on either one would poison that exact payload without strengthening
the threshold: the duplicate still cannot fill another slot. This is the narrow
case where ignoring a package has a direct security argument independent of the
strict rejection contract.

The second route also settles what the fix cannot be. Skipping only when the
values agree, and erroring when they disagree, closes the copy and leaves the
replay: two different prices from one signer is exactly what a replayed package
supplies.

## Decision

**After ADR 15's strict checks, a second valid package from one signer cannot
fill another signer slot and is not an error.** Reports are retained long enough
for ADR 27 to select a round, then the per-signer slot enforces the
anti-inflation property. The public error enum has no duplicate-specific
variant because callers can never observe one.

**Which of a signer's packages then counts is not decided here.**
`for_each_package` walks the payload from the tail, so "whichever arrived first"
would mean "whichever an attacker appended last", and the verified price would
depend on where in the payload the bytes sit. Skipping the duplicate is what
makes that question askable at all — until it stops being fatal there is nothing
to choose between — and
[ADR 27](0027-one-price-comes-from-one-round-and-a-mixed-payload-is-refused.md) answers it
by removing the choice: every package carrying the feed describes the same
moment or the payload is refused, so a signer's packages are interchangeable and
the first the walk reaches holds the slot. What matters here is the property that
answer preserves: the price is a function of the set of packages, not of their
order. A copy is still non-fatal, because a copy agrees about the moment; a
package replayed from an older round does not, and ADR 27 records what that
costs.

## Consequences

- **The order-independence is asserted, not just intended.**
  `a_package_from_another_round_cannot_take_the_round_from_the_signers` runs the
  same replay, older and newer, and requires the same median and the same
  timestamp from both.
- **Three tests changed from duplicate rejection to the threshold failure it
  is.** They are the same payloads asserting the same property — one signer
  counts once — which is the point: the property never depended on an error.
- **Every public error variant is observable.** "One signer twice" is not in
  the taxonomy because it is not a distinct failure mode.
- **The extra report bookkeeping is included in the product measurements.**
  `COSTS.md` and `methods/tests/cost.rs` carry the current exact numbers.
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
- **Reject duplicates and rely on relayers to send clean payloads.** Feasible,
  but it adds a failure mode without adding anti-inflation: the slot already
  makes the duplicate inert.
