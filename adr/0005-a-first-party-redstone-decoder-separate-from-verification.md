# 5. A first-party RedStone decoder, kept separate from verification

- **Status**: accepted
- **Milestone**: M1 (`M1-12`, with conformance in `M1-21`)
- **Requirements**: F4, U6
- **Artefacts**: `verifier-core/src/decode.rs`, `NOTICE`

## Context

A RedStone payload is a packed, separator-free byte format parsed **from the end** — on
EVM it is appended to a transaction's calldata, so the front of the buffer belongs to
the call and only the tail is RedStone's. Fields are fixed width and multi-byte
integers are big-endian.

There are two ways to get one decoded: take a dependency, or write it. The dependency
question is not purely technical. RedStone's Rust SDK is Boost licensed and could be
consumed; its EVM connector is partly BUSL-1.1 and cannot ship in an open-source
deliverable at all (ADR 1).

The subtler question is what "decode" is allowed to mean. It is tempting to have one
function that takes bytes and returns "a verified price", folding parsing, signature
recovery, threshold counting and staleness into one pass.

## Decision

**Write the decoder**, in `verifier-core/src/decode.rs`, zero-copy over a borrowed
buffer. Field widths and ordering follow RedStone's own Boost-licensed Rust SDK. The
BUSL EVM connector was deliberately not read for this purpose; where byte-level
conformance has to be checked it is used only as a black-box oracle against published
vectors. `NOTICE` records both boundaries and `deny.toml` fails a build that crosses
the second one.

**Decode and verify are separate steps, in separate modules.** `decode` decodes and
bounds-checks. It does not recover signers, count them against a threshold, or look at
the clock — those are M1-13, M1-14 and M1-15. Every `DecodeError` variant means *this
payload is not well formed*; none of them means *this payload is not authorised*.

Three details are load-bearing:

**The signed span.** The signature covers the data points **and** the timestamp, value
size and point count that follow them, but not itself. `DataPackage::signable` is that
span and it is the single most consequential thing in the file: a decoder that signs
too little lets an attacker rewrite the excluded field, and one that signs too much
rejects every genuine package. It has its own named test.

**Empty is rejected, not returned empty.** A payload declaring zero data packages is
`NoDataPackages`, not an empty set. An empty set satisfies "every signer in it is
authorised" *vacuously*, and a threshold check downstream is only as good as its input
being non-empty. Same reasoning for `NoDataPoints` and `ZeroWidthValue`.

**Trailing bytes are a decode failure.** Bytes remaining after the declared packages
are consumed mean the payload is not the payload that was signed, whatever else is true
of it.

## Consequences

- The verification path has no BUSL-adjacent code in it, and the boundary is enforced
  by the licence gate rather than by reviewer memory.
- Zero-copy suits the `no_std` guest constraint (ADR 4) and avoids allocating a copy of
  the payload inside the cycle budget.
- A malformed payload and an unauthorised payload fail in different places for
  different reasons, which is what lets U6 report distinguishable errors and what keeps
  the threshold logic from having to re-validate structure.
- Conformance with a third-party format becomes a local responsibility. That is the real
  cost, and M1-21 is the answer: conformance against RedStone's published vectors, so the
  decoder is checked against the format itself rather than against one reading of it.

## Alternatives considered

- **Depend on the RedStone Rust SDK for decoding.** Not ruled out by licence — Boost is
  allowlisted precisely so it could be — but rejected here because the guest constraints
  (`no_std`, panic-freedom, zero allocation) have to be guaranteed here, and a dependency
  moves them upstream.
- **One `verify(bytes) -> Price` entry point.** Rejected: it collapses "malformed" and
  "unauthorised" into one outcome, which U6 explicitly asks not to happen, and it makes
  the panic-freedom surface much harder to test exhaustively.
- **Port the EVM connector.** Excluded by licence, and stated as excluded in `NOTICE`
  so the boundary is a matter of record.
