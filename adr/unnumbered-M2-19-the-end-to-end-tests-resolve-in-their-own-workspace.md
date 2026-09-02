# [M2-19:01]. The end-to-end tests resolve in their own workspace

- **Status**: accepted, unnumbered
- **Milestone**: M2 (`M2-19`)
- **Requirements**: P1, S2
- **Extends**: [7](0007-two-workspaces-and-a-lockfile-per-resolution-domain.md),
  on the same argument applied to a third graph
- **Artefacts**: `e2e/Cargo.toml`, `e2e/Cargo.lock`, `e2e/tests/sequencer_e2e.rs`,
  `Cargo.toml`, `deny.toml`, `traceability/tests/source_boundary.rs`,
  `.github/workflows/ci.yml`

## Context

M2's done gate is the push path verifying and publishing across a standalone LEZ
sequencer. Reaching a sequencer means speaking its RPC, and the honest way to do
that is the client it publishes: `sequencer_service_rpc` for the calls, `lee` for
the transaction and message types, `common` for the envelope `sendTransaction`
takes. Hand-rolling the JSON-RPC would mean hand-rolling a transaction envelope,
which is the thing this test exists to stop guessing about.

Those three crates resolve the node they belong to. Added as dev-dependencies of
`aggregator-program`, which is where the first draft put them, they took the
**product lockfile from 384 packages to 798** and introduced **seven git sources**
`deny.toml` does not allow:

```
sequencer_service_rpc -> sequencer_service_protocol -> common
  -> logos-blockchain-common-http-client -> 38 logos-blockchain-* crates
  -> jf-poseidon2 -> jf-crhf   (github.com/EspressoSystems/jellyfish)
```

Measured on the branch that added them, and the cost was not only the lockfile.
CI's `cargo deny` job failed `source-not-allowed`; `build and test` spent about
eleven minutes compiling a node it never runs, and then failed, because
`cargo test --workspace` had begun running an end-to-end test in a job with no
sequencer to reach.

## Decision

**`e2e/` is its own workspace, in the root manifest's `exclude` list next to `m0`
and `methods/guest`, with its own `Cargo.lock`.** The client half of LEZ is
declared there and nowhere else.

This is ADR 7's argument, one graph further along. That record split measurement
from product so their pins could not constrain each other. The same holds here for
a third reason: the delivered lockfile should describe what Kanon ships, and a node
someone else wrote, reached only by a test, is not that.

The `deny.toml` change is deliberately the smaller one. The seven sources are
added to the shared `allow-git` rather than given a second config file, because two
configs would drift on the licence rules — which are the part that must stay
identical, since the licence gate is a deliverable. `cargo deny` runs over `e2e`
too, so its licences are still checked.

That leaves one real hole, which @frenzox found on #58: cargo-deny has no
per-workspace source policy, so one list permits those seven **anywhere**,
including in something that ships. The first draft of this record answered that the
guarantee is "structural rather than textual" — the product resolves none of them
and `e2e` is excluded so it cannot start to. That was an argument, not a gate, and
the repository's own standard is that a claim a test could hold should not be left
to prose.

So each of the seven carries a `# harness-only` comment, and
`traceability/tests/source_boundary.rs` reads the marker and asserts both
directions: **no marked source appears in the product `Cargo.lock`**, and **every
marked source appears in `e2e/Cargo.lock`**. The second is what keeps the marker
honest — a marker on something the harness does not use would be a live exemption
nobody is using. Both were checked by making them fail: marking `spel.git`, which
the product does resolve, fails the first; marking an unused URL fails the second.

The marker is a comment rather than a second TOML list on purpose. It sits on the
line it describes, so someone editing `allow-git` sees it, and there is no second
list to disagree with the first.

The tests run in the `sequencer` CI job, which is the only one with a chain to
reach. That job compiled nothing before this change; it now installs the RISC Zero
toolchain and builds the test and the guest, and its budget went from 25 to 45
minutes to cover a cold cache.

## Consequences

- **A third lockfile that has to agree with the product about LEZ.** It does, and
  by the existing mechanism rather than a new one: `scripts/lez-sequencer.sh`
  reads the resolved LEZ revision out of every non-`m0` `Cargo.lock` and refuses
  to run when they disagree. `e2e/Cargo.lock` is one of those, so a pin bump that
  misses it fails before the sequencer starts rather than at a confusing point
  afterwards. Verified: `lez-sequencer.sh pin` resolves one revision,
  `a58fbce2ff48c58b7bb5001b1a27e64b9596ee3a`, with the new lockfile in place.
- **`methods` and `aggregator-program` compile twice**, once per workspace, into
  separate target directories. The guest ELF does not: `methods/guest` has its own
  lockfile either way, so the artefact the test deploys is the one `risc0-build`
  produces for the product.
- **The guest the test deploys is not the guest CI ships.** It carries the test's
  admin key as `KANON_GENESIS_ADMIN`, so its image id — and therefore every
  address derived from it — differs. That is `[M2-06:01]` working as designed, and
  the test asserts the build carries the expected key rather than discovering it
  later as an unexplained `AdminUnowned`.
- **`e2e` has to be named in every per-workspace CI step.** It is now in the `fmt`,
  `clippy` and `cargo deny` loops. This is the standing cost of the exclude
  pattern, and the same one `m0` and the guests already carry.
- **A source the harness stops using has to lose its marker.** Otherwise
  `every_harness_only_source_is_one_the_harness_actually_resolves` fails, which is
  the intended direction: the alternative is an allowance left standing that the
  next dependency to reach that repository would inherit silently.
- **A product crate that genuinely needs one of those sources must say so.** The
  route is to take the marker off that line and record in `deny.toml` why the
  source is acceptable in a deliverable — not to weaken the test. The failure
  message says this, because the cheap way out of a red gate is to edit the gate.
- **The `sequencer` job is now the slow one.** It builds a 793-package graph on a
  cold cache — five fewer than the 798 above, because `e2e` does not resolve
  `pull-lib`, `kanon-sdk`, `kanon-relayer`, the two reference consumers or
  `traceability`, and adds itself. `Swatinem/rust-cache` covers both workspaces, so
  the warm cost is the test and the guest. Measured: 11 minutes cold, against the
  45-minute budget.

  All three counts are `Cargo.lock` entries rather than distinct crate names, which
  are 335, 708 and 703 respectively — the gap is crates the graph carries at two
  versions.

## Alternatives considered

**Keep the dev-dependencies in `aggregator-program` and widen the gates.** Add the
seven sources to `allow-git` and skip `sequencer_e2e` in the `build and test` job.
Two files, no new workspace. Rejected because the price is permanent and paid by
everything that ships: the delivered lockfile doubles, every `build and test` run
compiles the node graph, and the source allowlist that exists to make the licence
claim checkable gets widened for a test. The narrow version of that trade is
available and is this ADR.

**Hand-roll the JSON-RPC with an HTTP client.** Six methods, and it would drop the
node graph entirely. Rejected: `sendTransaction` takes LEZ's transaction envelope,
so this replaces a resolved dependency on upstream's types with a hand-written
copy of upstream's wire format — a restatement that can drift silently, in the one
test whose whole purpose is to stop restating what the chain does.

**Put the tests in `m0/`.** It is already an excluded workspace. Rejected: `m0`'s
pins are frozen to the toolchain its published figures were measured on, and this
test tracks the live product. Sharing that lockfile would recreate exactly the
constraint ADR 7 removed.
