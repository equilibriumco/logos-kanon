#!/usr/bin/env python3
"""Capture which feeds a data service publishes, who signs them, and how often.

Run by hand, like the vector capture next to it. What it writes is `FEEDS.md`'s
source: the authorised signer set per feed, and the interval between rounds.

    python3 scripts/capture-signer-sets.py --minutes 5 > /tmp/capture.json

# Why the served signer address is not taken at its word

The gateway labels every package with a `signerAddress`. Trusting that field
would make the roster a claim by whoever answers the request, and the roster is
the security parameter a feed is registered with -- a wrong one either locks a
feed out or lets a stranger in. So each address is recovered from the signature
over the reconstructed package, using the same code that builds the conformance
vectors, and the served label is compared against it. A disagreement is reported
rather than resolved.

# Why the whole data service is swept, and on weaker evidence

The five feeds F7 names are recovery-verified: every address is recovered from the
signature. That is too slow to run over every feed the service serves, several thousand
packages a poll, so the sweep that asks whether any other feed has a different roster
reads the addresses the gateway labels packages with. It is the weaker source, and it is
answering the weaker question -- whether a differing roster exists anywhere -- where a
gateway with no reason to lie is good enough. The five feeds being registered do not rest
on it.

# Why the interval needs many samples and not two

A round's timestamp is the round's, not the observer's, so the interval between
rounds can be read off the served packages. But the nominal cadence is not the
number a staleness window has to survive: rounds are skipped. Only a sample long
enough to contain skips shows the worst gap, and the worst gap is what a
consumer's `maxAge` floor has to clear.
"""

import argparse
import importlib.util
import json
import pathlib
import sys
import time
import urllib.request
from collections import defaultdict
from datetime import datetime, timezone
from decimal import Decimal

POLL_SECONDS = 2


def load_capture_module():
    """The vector capture's recovery, reused rather than reimplemented."""
    path = pathlib.Path(__file__).with_name("capture-redstone-vectors.py")
    spec = importlib.util.spec_from_file_location("capture_redstone_vectors", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--minutes", type=float, default=5.0)
    args = parser.parse_args()
    if args.minutes <= 0:
        parser.error("--minutes has to be positive: a window of zero observes nothing")

    cap = load_capture_module()
    url = f"{cap.GATEWAY}/data-packages/latest/{cap.DATA_SERVICE}"

    service_rosters = defaultdict(int)
    signers = defaultdict(lambda: defaultdict(int))
    rounds = defaultdict(set)
    intra_round_spread = defaultdict(int)
    mislabelled = []
    unrecoverable = []
    polls = 0
    answered = 0
    started = time.monotonic()

    while time.monotonic() - started < args.minutes * 60:
        polls += 1
        try:
            with urllib.request.urlopen(url, timeout=30) as response:
                served = json.load(response, parse_float=Decimal)
        except Exception as error:
            print(f"poll {polls}: {error}", file=sys.stderr)
            time.sleep(POLL_SECONDS)
            continue
        answered += 1

        for feed, packages in served.items():
            roster = frozenset(
                (p.get("signerAddress") or "").lower() for p in packages
            )
            if roster:
                service_rosters[roster] += 1

        for feed in cap.FEEDS:
            packages = served.get(feed, [])
            if not packages:
                continue
            instants = [p["timestampMilliseconds"] for p in packages]
            rounds[feed].add(max(instants))
            intra_round_spread[feed] = max(
                intra_round_spread[feed], max(instants) - min(instants)
            )
            for package in packages:
                resolved = cap.resolve(package)
                if resolved is None:
                    unrecoverable.append([feed, package["timestampMilliseconds"]])
                    continue
                points, timestamp_ms, signature, _ = resolved
                recovered = cap.recovered_signer(
                    cap.signed_span(points, timestamp_ms), signature
                )
                if recovered != package["signerAddress"].lower():
                    mislabelled.append([feed, package["signerAddress"], recovered])
                signers[feed][recovered] += 1
        time.sleep(POLL_SECONDS)

    feeds = {}
    for feed in cap.FEEDS:
        # Publication is what was observed, not the absence of an observation.
        # Deriving it the other way round reported every feed as published when
        # nothing had been seen at all, which is the one output of this script
        # that must never be produced by accident: it is M2-00's evidence.
        if not rounds[feed]:
            feeds[feed] = {"published": False}
            continue
        ordered = sorted(rounds[feed])
        gaps = [b - a for a, b in zip(ordered, ordered[1:])]
        feeds[feed] = {
            "published": True,
            "signers": dict(sorted(signers[feed].items())),
            "rounds": len(ordered),
            "gap_ms": {
                "min": min(gaps) if gaps else None,
                "max": max(gaps) if gaps else None,
                "counts": {str(g): gaps.count(g) for g in sorted(set(gaps))},
            },
            "intra_round_spread_ms": intra_round_spread[feed],
        }

    json.dump(
        {
            "provenance": {
                "gateway": url,
                "data_service": cap.DATA_SERVICE,
                "captured_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
                "polls": polls,
                "polls_answered": answered,
                "poll_seconds": POLL_SECONDS,
                "note": (
                    "Every signer address was recovered from the package signature and "
                    "compared against the address the gateway served alongside it."
                ),
            },
            "service_wide": {
                "note": (
                    "Every feed the data service serves, by the signer address the "
                    "gateway labels each package with rather than by recovery."
                ),
                "distinct_rosters": len(service_rosters),
                "roster_sizes": sorted({len(r) for r in service_rosters}),
                "feed_observations": sum(service_rosters.values()),
            },
            "mislabelled_packages": mislabelled,
            "unrecoverable_packages": unrecoverable,
            "feeds": feeds,
        },
        sys.stdout,
        indent=1,
    )
    print()

    missing = [feed for feed in cap.FEEDS if not rounds[feed]]
    if answered == 0:
        print(
            f"no poll was answered out of {polls}: nothing here was observed",
            file=sys.stderr,
        )
    elif missing:
        print(f"served no packages for {', '.join(missing)}", file=sys.stderr)

    if mislabelled or unrecoverable or answered == 0 or missing:
        sys.exit(1)


if __name__ == "__main__":
    main()
