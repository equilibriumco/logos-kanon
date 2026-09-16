# 15. Packages for the requested feed are rejected strictly

- **Status**: accepted
- **Milestone**: M1 (`M1-13`, `M1-18`)
- **Requirements**: F3, U6, SEC1
- **Artefacts**: `verifier-core/src/feed.rs`, `verifier-core/src/error.rs`, `verifier-core/src/value.rs`

## Context

RFP-020 says to reject any package whose signer is outside the authorised set,
whose signature is invalid, whose timestamp is outside the configured window,
or whose value is zero, negative, or otherwise invalid. The accepted proposal
also promises typed errors for each of those causes.

RedStone's Rust SDK sanitises some package failures while collecting a result.
Following that behaviour would make the verifier tolerant of a bad package, but
it would leave `InvalidSignature` and sometimes `UnauthorisedSigner` or
`ValueOutOfRange` unobservable whenever the other packages reached quorum. That
is not the interface promised by F4, U6, and SEC1.

There is a denial-of-service tradeoff, but it is payload-local rather than
feed-wide. In push mode a malicious relayer submits its own transaction:
rejecting it neither changes the price account nor prevents another relayer
from submitting clean bytes. In pull mode the consumer owns the transaction and
chooses its payload. A shared poisoned gateway response can make consumers of
that response fail, and a faulty authorised signer can make an unfiltered
aggregate fail, but RedStone packages are individually signed, so a relayer can
validate and submit a clean M-of-N subset. Transaction flooding is a sequencing
and fee problem common to all failing instructions, not a reason to accept a
payload that violates the verifier contract.

## Where the answer came from

The rejection contract comes from RFP-020 and the accepted proposal. RedStone
wire-format and median behaviour are conformed against `redstone-finance/rust-sdk`,
under the Boost Software License 1.0. The BUSL-1.1 EVM connector was not read or
ported; `NOTICE` records that boundary.

## Decision

**The rule the rest of this follows: a package that makes a claim about the
requested feed is either accepted or refuses the payload; a package that cannot
change the answer is ignored.** Every check below is that rule applied to one
property, and so is the single exception. A bad signature, an unauthorised
signer, a moment other than the payload's, a timestamp outside the window and an
unusable value are each a claim this verifier will not silently drop, because
dropping one means answering from a subset the payload's author chose. The
exception is a signer whose slot is already filled: one slot per configured
signer means a second package from that signer cannot fill another or move the
threshold, so refusing it would buy nothing and would hand anyone a denial for
the price of a copy (ADR 24). Which of two same-moment packages from one signer
holds the slot is settled by position, and reaching that at all needs the
signer's key.

**Every package carrying the requested feed is checked strictly**, in decoder
walk order:

1. returns `TimestampMismatch` when the package describes a different moment
   than the first package carrying this feed (ADR 27) — first, because it needs
   no signer and so costs no recovery;
2. returns `TooManyPackages` when the count passes `MAX_RECOVERIES` (ADR 25);
3. hashes the signed span and returns `InvalidSignature` if recovery fails;
4. returns `UnauthorisedSigner` if the recovered address is outside the feed's
   configured signer set;
5. returns `StalePackage` or `FuturePackage` when the timestamp is outside the
   two-sided window; and
6. returns `ValueOutOfRange` when the requested data point is zero, negative,
   or too wide to represent.

The first package failure encountered is returned before threshold evaluation.
This preserves the proposed typed-error interface without adding a second
single-package API or a diagnostic side channel.

**Packages that do not carry the requested feed are irrelevant.** They are
skipped before hashing, because they cannot fill a slot or change this feed's
answer and recovery is the dominant cost. ADR 25 bounds how many relevant
packages verification will recover.

**M-of-N counts distinct valid signers.** One slot is reserved per configured
signer. A repeated package cannot fill another slot, so it cannot inflate the
threshold; ADR 24 records why duplicates remain non-fatal, and a copy agrees
about the moment by construction. ADR 27 requires every package carrying this
feed to describe one moment, and refuses the payload otherwise.

The reported value is the median of the slots filled by the selected round,
using the overflow-safe midpoint.

## Consequences

- `InvalidSignature`, `UnauthorisedSigner`, `StalePackage`, `FuturePackage`, and
  `ValueOutOfRange` are direct package errors. They do not depend on whether
  ignoring the package would still leave quorum.
- A payload with three good packages and one bad package is rejected. A relayer
  receiving aggregates wider than the configured signer set must validate and
  select the packages it submits.
- Error precedence is deterministic in decoder walk order: moment, admission,
  signature, authority, age, then value within one package. The verifier reports the first
  defect it encounters rather than inventorying every defect in rejected input.
- `MAX_SIGNERS = 32` remains the fixed signer-set and median-buffer limit.
  `MAX_RECOVERIES` separately bounds the transaction cost of duplicate and
  multi-round packages.
- The implementation needs no unknown-signer or present-but-blocked tallies;
  removing them reduces state and keeps the strict path auditable.

## Alternatives considered

- **Sanitise bad packages and succeed when the remaining packages reach
  quorum.** This follows the RedStone SDK, but the returned `Result` exposes
  neither the rejected package nor its reason, so it does not provide the typed
  errors the accepted design promises.
- **Return a successful price plus per-package diagnostics.** This preserves
  quorum under a bad package, but requires a new result type, fixed diagnostic
  buffers, and a policy for what the aggregator exposes. It is a larger design
  than implementing the accepted contract.
- **Add a strict single-package API while leaving `verify_feed` permissive.** It
  gives the error variants producers but leaves both shipping modes on the
  non-conforming path.
- **Reject duplicate packages too.** Not required by F4 or SEC1, and the
  per-signer slot already prevents a duplicate from contributing to M-of-N.
  ADR 24 keeps that narrower exception.

## Carried forward

RedStone validates timestamps at both ends. ADR 18 records the future-window
tolerance and the distinct `FuturePackage` error. ADR 17 records the exact
value bounds and Q64.64 conversion.
