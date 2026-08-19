# Architecture decision records

The decisions behind this repository, one file each: what the situation was, what was
decided, and what it costs. Written after the fact for M0 and M1, and kept current from
here.

These are records, not documentation. For *what* the code does, the crate doc comments
and `TRACEABILITY.md` are the sources. An ADR answers **why**, and is the thing to read
before undoing one.

An ADR is not amended once it is accepted, except to change its status. If a decision is
reversed, the new ADR supersedes it and says so, and the old one stays where it is —
a record of a decision that was later reversed is more useful than no record of it.

| | decision | status |
| --- | --- | --- |
| [1](0001-dual-licence-enforced-by-a-machine-checked-allowlist.md) | Dual licence, enforced by a machine-checked allowlist | accepted |
| [2](0002-one-verification-core-shared-by-both-modes.md) | One verification core, shared by both modes, in a crate layout created up front | accepted |
| [3](0003-cryptographic-primitives-behind-a-verifier-backend-trait.md) | Cryptographic primitives behind a `VerifierBackend` trait | accepted |
| [4](0004-guest-reachable-code-is-no-std-unsafe-free-and-panic-free.md) | Guest-reachable code is `no_std`, unsafe-free, and must not panic | accepted |
| [5](0005-a-first-party-redstone-decoder-separate-from-verification.md) | A first-party RedStone decoder, kept separate from verification | accepted |
| [6](0006-mixed-accelerator-configuration.md) | The mixed accelerator configuration: accelerate secp256k1, leave keccak256 in software | accepted |
| [7](0007-two-workspaces-and-a-lockfile-per-resolution-domain.md) | Two workspaces, and a lockfile per resolution domain | accepted |
| [8](0008-exact-pins-and-tracking-the-estate.md) | Exact version pins, and the product tracks the estate | accepted |
| [9](0009-re-export-the-canonical-price-account-and-vendor-its-idl.md) | Re-export the canonical price account, and vendor its IDL as a conformance oracle | accepted |
| [10](0010-cycle-counts-as-exact-equality-guardrails-in-a-separate-workflow.md) | Cycle counts asserted by exact equality, in a workflow of their own | accepted |
| [11](0011-a-machine-checked-traceability-matrix.md) | A traceability matrix that is generated and machine-checked | accepted |
| [12](0012-a-standalone-lez-sequencer-without-lgs-run-from-a-prebuilt-image.md) | A standalone LEZ sequencer without `lgs`, run in CI from a prebuilt image | accepted |
| [13](0013-staleness-is-measured-against-the-lez-clock-program.md) | Staleness measured against the LEZ clock program's every-block account | accepted |
| [14](0014-build-admin-gating-against-the-unmerged-spel-admin-authority.md) | Admin gating built against the unmerged SPEL admin-authority, shim in reserve | accepted, contingency live |
| [15](0015-redstone-parity-verification-semantics.md) | Verification semantics follow RedStone's Rust SDK, with two divergences | accepted, duplicate rule superseded by 24 |
| [16](0016-the-asset-pair-is-a-registration-claim.md) | The asset pair is a registration claim, checked against the caller's expectation | accepted |
| [17](0017-value-sanity-is-two-level-and-prices-convert-to-q64-64.md) | Value sanity is two-level, and prices convert to the account's Q64.64 scale | accepted |
| [18](0018-timestamp-validity-is-two-sided-and-costs-one-signer.md) | Timestamp validity is two-sided, and a bad timestamp costs one signer | accepted |
| [19](0019-conformance-against-captured-redstone-payloads.md) | Conformance is asserted against captured RedStone payloads, with the signature as the oracle | accepted |
| [20](0020-per-component-costs-are-measured-in-the-product-guest.md) | Per-component costs are measured in the product guest, by differencing pipeline prefixes | accepted |
| [21](0021-property-tests-for-the-invariants-examples-cannot-reach.md) | Property tests for the invariants examples cannot reach, inside the crate under test | accepted |
| [22](0022-the-price-account-write-and-its-three-undecided-fields.md) | The price-account write, and the three fields nothing had decided | accepted |
| [23](0023-the-accept-and-reject-suite-is-shaped-by-the-contract.md) | The accept and reject suite is shaped by the contract, and asserts that the taxonomy discriminates | accepted |
| [24](0024-a-duplicate-package-is-skipped-and-the-freshest-holds-the-slot.md) | A duplicate package is skipped, and the freshest one holds the slot | accepted |
| [25](0025-present-but-blocked-requires-a-package-that-could-have-counted.md) | "Present but blocked" requires a package that could have counted | accepted |
| [26](0026-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md) | A payload cannot choose how much of the budget verification spends | accepted |

## How they fit together

Four of these are one argument in four parts. ADR 7 splits the workspaces so measurement
and product can move independently; ADR 8 then moves the product onto the estate's pins,
which is what made ADR 9's canonical price account linkable; ADR 10 is what made that
move provably safe rather than hopeful, by asserting every published figure exactly.

Three more are the shape of the verification path: ADR 2 puts one implementation behind
both modes, ADR 3 puts the expensive primitives behind one swappable trait, and ADR 4
states what the guest environment demands of anything either of them touches. ADR 5 and
ADR 13 are the two places where an external format and an external clock enter that
path, and both are decided the same way — take the untrusted thing, bound it, and refuse
to let a caller choose it.

Four describe what the verifier decides once a payload is in front of it, and they read
in order: ADR 15 settles who counts and what a skip costs, ADR 16 settles what the feed is
about, ADR 17 settles what the agreed number means, and ADR 18 settles when it stops being
true. ADR 18 also completes ADR 13, which chose the clock long before anything read it. The last two both turn on the same
observation — that the canonical price account says less than it appears to, carrying
neither the asset semantics of a feed id nor the exponent of a price — so both decisions
are about supplying, and documenting, what the format leaves out.

ADR 22 is where those four stop being decisions about a verification and become the six
fields of an account. It answers what none of them had to: which of several package
timestamps a median is dated by, what a source identifier is when the source is off chain,
and what stops a replayed payload moving a published price backwards.

ADR 23 closes that group from the other side. ADR 15 through ADR 18 each decided a
failure mode and tested it where it was introduced; ADR 23 is the one place those
decisions are read back as a contract, and the one assertion none of them could make
alone — that no two of the causes they settled answer with the same variant.

The remaining seven exist to keep claims verifiable: the licence gate (1), the guardrails
(10), the traceability checker (11), a sequencer that cannot drift from the code it tests
(12), conformance against payloads nobody here controls (19), a cost table generated
by the code that asserts it (20), and generated inputs for the claims that are about every
input rather than a case (21). ADR 19 is the answer to a question the others cannot
settle -- whether the decoder is right about a format this repository did not define --
and ADR 20 answers the other one, where an update's cost actually goes. ADR 20 also
completes ADR 6: the accelerator configuration is a `[patch.crates-io]` section that
fails silently, and measuring keccak256 and recovery as separate rows is what makes both
failure directions visible in cycles alone. ADR 21 is the odd one in this group, and
deliberately so: it is the only place where CI runs on inputs nobody chose, which is the
opposite of what ADR 10 argues for everywhere else, and the reason is that the decisions
it covers -- the threshold in ADR 15, the scale in ADR 17, the framing in ADR 5 -- are
statements about all inputs, where a cycle count is a statement about one.

ADR 24 reopened one of them. ADR 15's own argument -- a package an attacker can
produce for free must not be able to fail the payload for everyone -- turned out to
apply to the one axis it had decided the other way, because a duplicate package needs no
key: copy one already in the payload, or replay an older one still inside `maxAge`. The
rule it replaces was doing no work the per-signer slot was not already doing.

ADR 25 is a correction of the same kind one layer up. ADR 18 decided that a package outside the
window costs its signer, and ADR 15 decided when a signer set is named as the fault.
Each was tested where it was introduced, and between them the two age tallies ended up
holding nothing constant: a configured signer's stale package about another feed counted
as present-but-blocked, so a threshold failure reported as staleness. Each tally now
varies exactly the thing it names, which is the question the unknown-signer tally had
been asking all along.

ADR 26 is the one that had to correct an earlier decision rather than an earlier
omission. ADR 24 stopped a duplicate package failing a payload, and in doing so removed
the thing that had been aborting a flood of duplicates after a handful of signature
recoveries: the denial it closed cost one package, and the denial it opened cost a whole
transaction's cycle budget. Bounding the work before the cryptography closes both, and
it also fixes something that was never an attack -- verifying one feed of a multi-feed
payload was paying to recover every other feed's packages.

## Open questions carried by these decisions

Four are with Logos, and all four are stated in full in `m0/versions.md`, *Open:
questions outstanding with Logos*:

- **Which LEZ pin the estate is standardising on**, and the timeline for scaffold
  PR #246 including whether `test-node` is in scope (ADR 8, ADR 12).
- **Is `#212` merging, and when** (ADR 14).
- **Is LEZ's transitive LGPL-3.0 dependency an accepted position for the estate**
  (ADR 1) — and, separately, three Logos crates ship without a `license` field, which a
  one-line manifest change would fix for every downstream consumer (ADR 9).

One is settled locally, with a measurement in M2: which clock account an oracle should
read (ADR 13). The CI ceilings that stop the accelerator configuration regressing
silently (ADR 6) landed with M1-25, and ADR 20 records that both of them turned out to
be cycle counts.
