# Architecture decision records

The decisions behind this repository, one file each: what the situation was, what was
decided, and what it costs. Written after the fact for M0 and M1, and kept current from
here.

These are records, not documentation. For *what* the code does, the crate doc comments
and `TRACEABILITY.md` are the sources. An ADR answers **why**, and is the thing to read
before undoing one.

Once an ADR is part of a merged milestone, it is not amended except to change its status.
If a merged decision is reversed, the new ADR supersedes it and the old one stays where
it is — a record of a decision that was later reversed is more useful than no record of
it. ADRs still under review in an unmerged milestone are edited with that milestone so
they describe the design being proposed for acceptance.

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
| [15](0015-redstone-parity-verification-semantics.md) | Packages for the requested feed are rejected strictly | accepted |
| [16](0016-the-asset-pair-is-a-registration-claim.md) | The asset pair is a registration claim, checked against the caller's expectation | accepted |
| [17](0017-value-sanity-is-two-level-and-prices-convert-to-q64-64.md) | Invalid values are rejected, and prices convert to the account's Q64.64 scale | accepted |
| [18](0018-timestamp-validity-is-two-sided-and-costs-one-signer.md) | Timestamp validity is two-sided and rejects the package | accepted |
| [19](0019-conformance-against-captured-redstone-payloads.md) | Conformance is asserted against captured RedStone payloads, with the signature as the oracle | accepted |
| [20](0020-per-component-costs-are-measured-in-the-product-guest.md) | Per-component costs are measured in the product guest, by differencing pipeline prefixes | accepted |
| [21](0021-property-tests-for-the-invariants-examples-cannot-reach.md) | Property tests for the invariants examples cannot reach, inside the crate under test | accepted |
| [22](0022-the-price-account-write-and-its-three-undecided-fields.md) | The price-account write, and the three fields nothing had decided | accepted |
| [23](0023-the-accept-and-reject-suite-is-shaped-by-the-contract.md) | The accept and reject suite is shaped by the contract, and asserts that the taxonomy discriminates | accepted |
| [24](0024-a-duplicate-package-is-skipped.md) | A duplicate package is skipped | accepted |
| [25](0025-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md) | A payload cannot choose how much of the budget verification spends | accepted |
| [26](0026-a-maximum-payload-size-and-where-it-has-to-be-enforced.md) | A maximum payload size, and where it has to be enforced | accepted |
| [27](0027-one-price-comes-from-one-round-and-a-mixed-payload-is-refused.md) | One price comes from one round, and a payload that mixes rounds is refused | accepted |
| [28](0028-lgs-build-in-ci-triggered-by-the-manifests.md) | `lgs build` runs in CI, triggered by the manifests rather than by every push | accepted |
| [29](0029-a-staleness-window-has-an-upper-bound-and-it-is-enforced.md) | A staleness window has an upper bound, and it is enforced | accepted |
| [30](0030-the-signer-set-stays-per-feed.md) | The signer set stays per feed, though every feed currently shares one | accepted |

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
in order: ADR 15 settles strict package rejection and distinct-signer counting, ADR 16
settles what the feed is about, ADR 17 settles what the agreed number means, and ADR 18
settles the clock and timestamp window. ADR 18 also completes ADR 13, which chose the
clock long before anything read it. ADRs 16 and 17 both turn on the same observation —
that the canonical price account says less than it appears to, carrying neither the
asset semantics of a feed id nor the exponent of a price — so both decisions are about
supplying, and documenting, what the format leaves out.

ADR 22 is where those four stop being decisions about a verification and become the six
fields of an account. It answers what none of them had to: which of several package
timestamps a median is dated by, what a source identifier is when the source is off chain,
and what stops a replayed payload moving a published price backwards.

ADR 23 closes that group from the other side. The package and feed decisions test each
failure where it is introduced; ADR 23 is the one place they are read back as a contract,
and the one assertion none of them could make alone — that no two causes answer with the
same variant. `InvalidSignature` is observable in that suite rather than reserved for a
future API.

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

ADR 24 is the narrow exception to ADR 15's strict package rule, and copying a package needs
no key. Rejecting a second package from a signer whose slot is filled does no work the slot
is not already doing, because it cannot occupy another slot or increase the threshold count.
A package replayed from an older round is not that case -- it describes another moment, and
ADR 27 refuses it.

ADR 25 is the one that had to correct an earlier decision rather than an earlier
omission. ADR 24 stopped a duplicate package failing a payload, and in doing so removed
the thing that had been aborting a flood of duplicates after a handful of signature
recoveries: the denial it closed cost one package, and the denial it opened cost a whole
transaction's cycle budget. Bounding the work before the cryptography closes both, and
it also fixes something that was never an attack -- verifying one feed of a multi-feed
payload was paying to recover every other feed's packages.

ADR 26 finishes what ADR 25 started, and finds the boundary of what this repository
can decide. ADR 25 bounded the work a payload buys; ADR 26 bounds the payload itself,
because reading one costs cycles before any code here runs. That last part is not ours
to fix: LEZ reads a program's whole instruction data before the program's first
instruction, so the limit is a number to publish and for callers to enforce rather than
a check that can save the transaction it refuses.

ADR 27 is the one that found a gap nobody had decided on: a price is a statement about one
observation, and nothing required the packages behind one to describe the same moment, so a
payload's assembler could build a median out of three. It is also the decision this
milestone made twice. The first answer chose a round by vote, to avoid making a replayed
package fatal; the second refuses the payload, because ADR 15 had already ruled that a bad
package refuses and accepted what that costs. Being strict about four properties of a
package and holding a vote on the fifth was two positions rather than one. What the second
answer costs is written into ADR 27: an appended package is now a denial, so a submitter
has to own the bytes it submits. The moment check runs before recovery, so that package
needs no signer at all, which is why no amount of tuning relaxes the constraint.

ADR 29 closes a range that only had one end. ADR 18 gave staleness a two-sided window and
left `maxAge` to the feed; registration refused zero and nothing else. But `freshness`
saturates, so a `maxAge` near `u64::MAX` put the window's lower edge at zero and admitted
every timestamp in the past — the check was configurable into non-existence, and a feed in
that state looked configured like any other. The test suites had been relying on exactly
that to mean "this test is not about the clock", which is why a one-comparison check
arrives with fifty sites of churn behind it.

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
