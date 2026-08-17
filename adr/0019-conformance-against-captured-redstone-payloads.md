# 19. Conformance is asserted against captured RedStone payloads, with the signature as the oracle

- **Status**: accepted
- **Milestone**: M1 (`M1-21`)
- **Requirements**: F4, S3
- **Artefacts**: `verifier-core/tests/redstone_conformance.rs`, `verifier-core/tests/vectors/`, `scripts/capture-redstone-vectors.py`

## Context

M1-21 asks for "decode conformance tests against published RedStone wire-format test
vectors, using the EVM connector only as a black-box oracle". Two of those three things
turned out not to exist.

**There are no published vectors.** `redstone-finance/rust-sdk` has no captured payloads:
`crates/redstone/src/core/test_helpers.rs` builds `DataPackage` values programmatically
from feed-id strings and signer addresses, and the serialised bytes only ever exist at
runtime. There is no fixture directory, no `.hex`, no sample payload anywhere in the crate.

**The EVM connector cannot be the oracle.** It is BUSL-1.1, `deny.toml` denies that
licence outright, and `NOTICE` states no part of it is read for this decoder. Reading its
fixtures would falsify a claim this repository ships.

That left the decoder's evidence at: the wire-format specification, our own
`PayloadBuilder` round-tripping — which proves only that the builder and the decoder agree
with each other — and a constant-by-constant comparison against RedStone's
`protocol/constants.rs`. None of that is a real signed payload, and everything in
`m1-part2` rests on the decoder being right.

## Decision

Capture live packages from RedStone's public Data Distribution Layer gateway, freeze them
as committed vectors, and **use RedStone's own signature as the oracle**.

The signature is computed by a RedStone signer over RedStone's serialisation of the
package. It therefore recovers to the address they publish only if this decoder's view of
the signed span — field order, field widths, and which bytes the signature covers — matches
theirs byte for byte. Nothing in this repository can influence that.

`scripts/capture-redstone-vectors.py` does the capture, by hand rather than in CI;
`verifier-core/tests/redstone_conformance.rs` reads the frozen result. The tests never
reach the network: one that did would fail on a bad day for reasons unrelated to this
code, and would stop meaning anything the moment the market moved.

### The value has to be searched for, and that is the point

The gateway serves each data point's value as a JSON number, which is a lossy rendering of
the integer that was signed. A price of `62787.07322193565` is `6278707322194` on the wire
at eight decimals, and the JSON cannot say whether the last digit was rounded or truncated.

So the capture tries each candidate and keeps the one whose reconstructed package recovers
to the published signer. That is not a workaround for a broken test — it is the test. A
candidate can only recover correctly if the layout used to rebuild the package is right, so
the search succeeding *is* the conformance result. It also settled the exponent: eight
decimals for `redstone-primary-prod`, because no other reproduces a signature they signed.

### What is captured and what is assembled

The packages and signatures are RedStone's. The envelope around them — package count,
unsigned metadata, its size, the marker — is assembled by the capture script per the wire
format, because the gateway serves packages rather than a finished payload and RedStone's
own SDK assembles it client-side in the same way.

The honest claim is therefore narrower than "we tested against a published payload": the
**signed span is conformance-tested against real signatures**, and the **envelope is built
to spec**. The test file says so, in those words, so that nobody later reads the file name
and infers more than it establishes.

### A negative control, because a check that cannot fail is not a check

`a_wrong_signed_span_recovers_to_nobody` hashes deliberately wrong spans — short of
`point_count`, short of the trailing fields, the data points alone, one byte short — and
asserts none of them recovers a signer RedStone published.

Measured at capture time across 25 packages and five feeds, the correct span recovers
every package and each of seven wrong candidates recovers none: omitting `point_count`,
omitting `value_size` and `point_count`, the points alone, timestamp before value,
timestamp at eight bytes, a twenty-byte feed id, and little-endian trailing fields. That
gap between everything and nothing is what makes the positive assertion worth making.

## Consequences

- The decoder is conformant against a system this repository does not control, which is
  what F4 and S3 needed and what neither the specification nor a self-built payload could
  give.
- The repository now carries third-party captured data: 12 KB of public market data, with
  the gateway, data service and capture time recorded alongside it. RedStone publish these
  payloads for on-chain redistribution, so holding twenty-five of them in a test fixture
  is within their intended use rather than at the edge of it. Nothing here is credentials
  or personal data: feed ids, timestamps, public signer addresses, prices and signatures.
- The vectors go stale as prices and signer rosters move. They are not meant to track
  either: the format is what is under test, so a re-capture is only needed if the wire
  format changes. `FOREVER` is used as `maxAge` in these tests for that reason, with
  staleness tested separately against a clock the test controls.
- Five feeds are covered: BTC, ETH, SOL, XMR and ZEC — F7's five, so M2-11's registration
  work has a decoded example of each.
- The capture confirms XMR/USD and ZEC/USD are live on `redstone-primary-prod` with five
  signers each. That is M2-00's question, answered as a side effect rather than by the
  spike scheduled for it.

## Alternatives considered

- **Reconstruct vectors from the SDK's `test_helpers.rs`.** They are built in code, so a
  vector derived from them would be our serialisation of their structure — the same
  circularity `PayloadBuilder` already has, with more ceremony.
- **Read the EVM connector's fixtures.** BUSL, denied by `deny.toml`, and it would make
  `NOTICE` untrue.
- **Fetch from the gateway during the test run.** Makes CI depend on a third party's
  uptime and on the current market, and turns a format regression and an outage into the
  same red build.
- **Commit the gateway's JSON verbatim instead of reconstructed payloads.** Keeps the
  provenance purer and tests nothing: the JSON has no wire bytes in it, which is the whole
  problem this record starts from.
