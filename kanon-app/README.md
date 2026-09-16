# `kanon-app`: the Basecamp mini-app

The off-chain feed dashboard (RFP-020 Usability 2), built in M4-19 to M4-21
against the Figma designs from M4-18.

Not a cargo crate, which is why this directory holds a README rather than a
`Cargo.toml`: it is a Logos mini-app that loads in Basecamp, and it talks to the
chain through `kanon-sdk` rather than reimplementing any part of the
verification or submission path.

## What it has to show

Per feed, for the push aggregator: the live price, the configured signer set,
the current M-of-N threshold, the latest data-package timestamp, and how stale
that makes the feed.

Plus a **public-mode pull dry-run panel** (M4-20): fetch a signed payload from
the RedStone DDL for a chosen `(dataServiceId, feedId)` and show what the pull
library would return on-chain, the verified price and timestamp or the typed
error, without sending a consumer transaction. The RFP asks for this so
developers can exercise the pull path before committing to it, and the same
dry-run is available from the CLI (M4-16).

## Before writing any of it

M4-18 produces the Figma designs and is a deliverable in its own right
(Supportability 7). The dashboard is built from those, not ahead of them.
