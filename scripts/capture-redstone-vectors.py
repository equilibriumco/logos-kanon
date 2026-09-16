#!/usr/bin/env python3
"""Capture live RedStone packages and freeze them as offline conformance vectors.

Run by hand, not in CI: the vectors it writes are committed, and the tests read
them from disk so a test run needs no network and no live market.

    python3 scripts/capture-redstone-vectors.py > verifier-core/tests/vectors/redstone-primary-prod.json

# Why the wire value has to be searched for

The gateway serves each data point's value as a JSON number, which is a lossy
rendering of the integer that was actually signed: a price of 62787.07322193565
is 6278707322194 on the wire at eight decimals, and the JSON cannot say whether
the last digit was rounded up or truncated. So each candidate is tried and the
one whose reconstructed package recovers to the signer address RedStone
published is the true one.

The numbers are parsed as `Decimal` rather than as `float`, so the only
uncertainty is the gateway's rendering. Through float64 the scaling adds an
error of its own, which can put the product on the wrong side of the true
integer and leave every candidate wrong -- a package that then drops without
comment.

That search is the test's strength rather than a workaround. The signature is
RedStone's, over RedStone's own serialisation, so a candidate can only recover
correctly if the byte layout used to rebuild the package matches theirs exactly
-- the field order, the widths, and which bytes the signature covers. If any of
that were wrong, no candidate would recover for any package.

# What is captured and what is assembled

The packages are RedStone's, signatures included. The envelope around them --
package count, unsigned metadata, size, marker -- is assembled here per the wire
format, because the gateway serves packages rather than a finished payload and
RedStone's own SDK assembles it client-side in the same way. The honest claim is
therefore: the signed span is conformance-tested against real signatures, and
the envelope is built to spec.
"""

import base64
import json
import sys
import urllib.request
from datetime import datetime, timezone
from decimal import Decimal

from eth_hash.auto import keccak
from eth_keys import keys

GATEWAY = "https://oracle-gateway-1.a.redstone.finance"
DATA_SERVICE = "redstone-primary-prod"
DECIMALS = 8
FEEDS = ["BTC", "ETH", "SOL", "XMR", "ZEC"]

FEED_ID_BYTES = 32
TIMESTAMP_BYTES = 6
VALUE_SIZE_BYTES = 4
POINT_COUNT_BYTES = 3
PACKAGE_COUNT_BYTES = 2
METADATA_SIZE_BYTES = 3
MARKER = bytes([0, 0, 2, 237, 87, 1, 30, 0, 0])
VALUE_SIZE = 32


def signed_span(points, timestamp_ms):
    """The bytes a package signature covers: points, then the three trailing fields."""
    body = b""
    for feed_id, value in points:
        body += feed_id.encode().ljust(FEED_ID_BYTES, b"\x00")
        body += value.to_bytes(VALUE_SIZE, "big")
    return (
        body
        + timestamp_ms.to_bytes(TIMESTAMP_BYTES, "big")
        + VALUE_SIZE.to_bytes(VALUE_SIZE_BYTES, "big")
        + len(points).to_bytes(POINT_COUNT_BYTES, "big")
    )


def recovered_signer(span, signature):
    v = signature[64]
    v = v - 27 if v >= 27 else v
    digest = keccak(span)
    public = keys.Signature(signature[:64] + bytes([v])).recover_public_key_from_msg_hash(digest)
    return public.to_address().lower()


def resolve(package):
    """The exact on-wire integers, found by asking which one the signature agrees with."""
    signature = base64.b64decode(package["signature"])
    if len(signature) != 65:
        return None
    timestamp_ms = package["timestampMilliseconds"]
    expected = package["signerAddress"].lower()

    # Exact decimal arithmetic, on the digits the gateway actually sent. Scaling
    # a float64 introduces an error of its own on top of the rendering's, which
    # can land the product on the wrong side of the true integer -- and then
    # every candidate offset from it is wrong too, the package silently drops,
    # and a feed arrives with four signers where the tests expect five.
    scaled = [(p["dataFeedId"], int(p["value"] * 10**DECIMALS)) for p in package["dataPoints"]]
    # Both directions. The gateway's rendering may have rounded up or down, so
    # the signed integer is within one either way.
    for offset in (0, 1, -1):
        points = [(feed, value + offset) for feed, value in scaled]
        if any(v <= 0 or v.bit_length() > VALUE_SIZE * 8 for _, v in points):
            continue
        try:
            if recovered_signer(signed_span(points, timestamp_ms), signature) == expected:
                return points, timestamp_ms, signature, expected
        except Exception:
            continue
    return None


def payload(packages):
    """A complete payload: packages, count, empty unsigned metadata, size, marker."""
    body = b""
    for points, timestamp_ms, signature, _ in packages:
        body += signed_span(points, timestamp_ms) + signature
    return (
        body
        + len(packages).to_bytes(PACKAGE_COUNT_BYTES, "big")
        + b""
        + (0).to_bytes(METADATA_SIZE_BYTES, "big")
        + MARKER
    )


def main():
    url = f"{GATEWAY}/data-packages/latest/{DATA_SERVICE}"
    with urllib.request.urlopen(url, timeout=30) as response:
        served = json.load(response, parse_float=Decimal)

    vectors = []
    skipped = False
    for feed in FEEDS:
        if feed not in served:
            print(f"{feed}: not served by {DATA_SERVICE}", file=sys.stderr)
            continue
        # Every served package or none. A vector short one signer is a valid
        # payload and passes the conformance suite, then panics the cost
        # harness, which slices `signers[..5]`. Whatever made a package
        # unreconstructible is worth knowing about at the capture rather than
        # three tests later.
        resolved = [r for r in (resolve(p) for p in served[feed]) if r]
        if len(resolved) != len(served[feed]):
            print(
                f"{feed}: {len(resolved)}/{len(served[feed])} packages "
                "reconstructed -- not captured",
                file=sys.stderr,
            )
            skipped = True
            continue
        # Each signer timestamps its own package from its own clock, so a
        # capture taken while a round is landing can hold packages milliseconds
        # apart. One `timestamp_ms` per vector would then be one signer's, and
        # the conformance suite compares it against the whole feed's -- which
        # `verify_feed` reports as the oldest package behind the price, not the
        # first one in the payload. Skip the feed and capture it again rather
        # than commit a vector that fails on arrival.
        instants = {timestamp_ms for _, timestamp_ms, _, _ in resolved}
        if len(instants) != 1:
            spread = max(instants) - min(instants)
            print(
                f"{feed}: signers {spread}ms apart, mid-round -- not captured",
                file=sys.stderr,
            )
            skipped = True
            continue
        vectors.append(
            {
                "feed_id": feed,
                "timestamp_ms": resolved[0][1],
                "signers": [signer for *_, signer in resolved],
                "values": [str(points[0][1]) for points, *_ in resolved],
                "payload_hex": payload(resolved).hex(),
            }
        )

    json.dump(
        {
            "provenance": {
                "gateway": url,
                "data_service": DATA_SERVICE,
                "captured_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
                "decimals": DECIMALS,
                "note": (
                    "Packages and signatures are RedStone's, served live. The payload "
                    "envelope is assembled per the wire format, as RedStone's own SDK "
                    "assembles it client-side. Every value here was confirmed by "
                    "recovering the published signer address from the published "
                    "signature."
                ),
            },
            "vectors": vectors,
        },
        sys.stdout,
        indent=1,
    )
    print(file=sys.stdout)

    if skipped:
        sys.exit(1)


if __name__ == "__main__":
    main()
