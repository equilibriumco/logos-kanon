# 24. A duplicate package is skipped, and the freshest one holds the slot

- **Status**: accepted, superseding the third paragraph of ADR 15's decision
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

On one axis it decided the other way. A second package from a signer that had
already filled its slot returned `ReoccurringSigner` and failed the whole
payload, on the argument that a signer occupying two slots reaches any threshold
alone.

The argument is right and the remedy was wrong, because the slot already
enforces it. `reported[index]` is one cell per configured signer and `met`
counts filled cells, so a signer that sends fifty packages still moves the count
by one. The error was doing no work the data structure was not already doing —
and it was reachable by anyone.

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

**The freshest package holds the slot, not the first one walked.**
`for_each_package` walks the payload from the tail, so "whichever arrived first"
would mean "whichever an attacker appended last" — the verified price would
depend on where in the payload the bytes sit, and an attacker chooses that. A
package only displaces what is in the slot if its timestamp is strictly greater,
which neither a copy (equal) nor a replay (older) is. The price is then a
function of the set of packages, not of their order.

**The feed's timestamp is taken after the walk**, as the minimum over the slots
as they finally stand. The running minimum it replaces was correct only while
slots could not be replaced.

## Consequences

- **The order-independence is asserted, not just intended.**
  `an_older_package_from_a_signer_that_already_reported_cannot_deny_the_feed`
  runs the same replay at both ends of the package list and requires the same
  median and the same timestamp from both.
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
