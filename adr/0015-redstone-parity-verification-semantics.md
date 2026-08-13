# 15. Verification semantics follow RedStone's Rust SDK, with two divergences

- **Status**: accepted
- **Milestone**: M1 (`M1-13`, `M1-18`)
- **Requirements**: F3, U6, SEC1
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `verifier-core/src/value.rs`

## Context

RFP-020 says "reject any package that does not meet" the threshold (F3) and "reject any
package whose signer is not in the authorised signer set" (SEC1). Neither settles
whether an unknown signer fails the whole payload or is merely uncounted. The two
readings diverge sharply in consequence: the strict reading hands a denial-of-service
primitive to anyone who can append a package to a payload that other consumers rely on,
since RedStone payloads are multi-consumer by design and routinely carry signers no
single consumer configured.

## Where the answer came from

`redstone-finance/rust-sdk`, under the Boost Software License 1.0, verified from its own
`LICENSE` file rather than assumed from the package name. `deny.toml` allowlists Boost
for exactly this reason. The BUSL-1.1 EVM connector was not consulted: `NOTICE` states
that no part of it is copied, ported or read for the purpose of writing this decoder or
its verification semantics, and following its logic here would have made that statement
false.

## Decision

Count-and-ignore on both axes. An unrequested feed in a package is skipped; a signer not
in the configured set is skipped, not fatal. One slot is reserved per configured signer,
so a repeat from an already-slotted signer is rejected rather than counted twice — a
signer that could occupy two slots reaches any threshold alone. The reported value is
the median across whatever slots filled, using the overflow-safe midpoint (ADR-adjacent
to `value.rs`, not restated here).

`UnauthorisedSigner` is returned on the causal test `met + distinct_unknowns >=
threshold`: only when the skipped, unauthorised signers would themselves have been
enough to reach quorum is the *signer set* named as the problem. Two simpler rules were
considered and rejected on payloads that refute them:

- **"Any unknown signer is unauthorised."** Refuted by
  `an_unknown_signer_is_skipped_and_the_payload_still_verifies`: three authorised signers
  plus a stranger still verifies, because RedStone payloads routinely carry more signers
  than one consumer configures.
- **"Zero matched signers always means the signer set is wrong."** Refuted by
  `a_threshold_that_was_unreachable_anyway_is_not_blamed_on_the_signer_set`: met = 0, one
  distinct unknown, threshold 3 — authorising the stranger still leaves one of three, so
  the honest answer is `ThresholdNotMet`, not `UnauthorisedSigner`. Distinct signers are
  counted rather than packages, for the same reason:
  `repeated_packages_from_one_unknown_signer_count_as_one` shows that three packages
  from one stranger must not read as three missing authorisations.

## Two current divergences, and two corrected

- **Below-threshold is an error here; a zero value is not, yet.** The SDK can return a
  degraded result that simply reflects fewer inputs. Kanon writes one canonical
  on-chain price, and U6 wants both a value and a reason when one cannot be produced, so
  `ThresholdNotMet` is a typed `VerifyError` variant rather than a degraded number — that
  is the live divergence. A zero value, today, is **skipped**, exactly as RedStone skips
  it: `a_zero_value_does_not_count_toward_the_threshold` asserts `ThresholdNotMet { met:
  2, required: 3 }`, not a zero-specific rejection. Making zero its own error is
  value-sanity work M1-17 delivers; it is not something the code does now, and this ADR
  should not be read as claiming it does.
- **`alloc` is not available.** The SDK's `Matrix<Option<Value>>` and its use of
  `alloc::vec::Vec` assume an allocator, which ADR 4 forbids in guest-reachable code.
  ADR 5 names zero allocation among the guest constraints a decoder must guarantee, but
  it argues from wire-format control and audit surface, and never records that the SDK
  *in fact* requires one. That concrete fact — not merely the constraint it would
  violate — is what settles the choice not to depend on it, and this is where the fact
  is written down.
- **A former divergence, now corrected.** An earlier revision of this decision made a
  value over 32 significant bytes `ValueOutOfRange` rather than skipped. That let one
  configured signer deny the entire feed by sending a single oversized value —
  precisely the failure mode an M-of-N threshold exists to survive, and precisely
  backwards from what SEC1 and F3 are for. RedStone's own SDK sanitises an oversized
  value rather than rejecting the package that carries it, so erroring was itself an
  unjustified divergence from parity. It is now skipped, exactly like a zero value —
  matching RedStone rather than diverging from it:
  `an_unrepresentable_value_costs_only_that_signer_not_the_whole_feed` shows three good
  signers plus one oversized still producing a price, and
  `an_unrepresentable_value_can_still_leave_the_threshold_unmet` shows the skip is not
  silent — too few good signers left still rejects, naming the threshold rather than the
  payload's shape.
- **A second former divergence, now corrected: an unrecoverable signature.** An earlier
  revision propagated a failed signature recovery out of the package walk with `?`,
  failing the entire payload. That is a denial-of-service primitive, and the cheapest one
  in the design: a stranger can append one package carrying sixty-five bytes of garbage —
  no key and no valid signature needed — and deny the feed to every consumer that payload
  serves, where every other skip case needs at least a valid signature to reach. RedStone
  skips it (`Some(address) => address, _ => continue`), so it is skipped here too, and the
  package is *not* recorded as an unknown signer: recovery failure leaves no address to
  deduplicate against, and recording it would let one attacker inflate the unknown tally
  by repeating the same garbage. `VerifyError::InvalidSignature` stays declared with no
  producer; a future single-package API, where the caller names one package and expects
  it to verify, is where it becomes reportable.
  `an_outsiders_garbage_signature_cannot_deny_the_feed` pins it: three authorised signers
  plus one package signed with sixty-five bytes of `0xFF` still produce a price.

## Consequences

- `MAX_SIGNERS = 32` is a real limit, not a guard rail: it sizes three fixed-size stack
  buffers `verify_feed` walks -- `reported` (`[Option<Value>; 32]`, 1,056 bytes),
  `unknown` (`[SignerAddress; 32]`, 640 bytes) and `collected` (`[Value; 32]`,
  1,024 bytes) -- about 2.7 KB in one frame, which is how the threshold runs with no
  allocator. `collected` exists only because `median` takes `&mut [Value]` while
  `reported` holds `Option<Value>`; sorting `reported` in place would remove it and save
  a kilobyte of guest stack, and is deliberately left for later rather than folded into
  this decision. RedStone caps a signer set at 255 and live feeds run ten to twenty, so
  32 leaves headroom, and raising it costs stack and nothing else.
- A value over 32 significant bytes is skipped, like a zero value, for the reason above:
  so that one configured signer cannot deny a feed by sending a single malformed report.
- `StalePackage`, `AssetMismatch`, `ValueOutOfRange` and `ScalingOutOfRange` are declared
  in `VerifyError` now and returned by nothing until M1-15 through M1-17 land. `feed.rs`
  never constructs them; documenting that gap here is what keeps `TRACEABILITY.md`
  honest about what M1-13 and M1-18 actually deliver.
- `InvalidSignature` is declared and, unlike the four above, has no producer by design
  rather than by sequencing: an unrecoverable signature is skipped, not reported, for as
  long as verification runs over a multi-consumer payload rather than one named package.

## Carried forward

RedStone validates a package's timestamp two-sided: it rejects both a package that is
too old and one dated too far in the future. U6 names only staleness. That a
future-dated package should also be a rejection is therefore a finding for M1-15 to
carry forward and confirm, not an inference to make here — this ADR's artefacts do not
touch timestamps at all.
