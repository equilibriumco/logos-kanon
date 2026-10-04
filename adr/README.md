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
| [31](0031-the-aggregator-pins-spel-and-lez-as-one-decision.md) | The aggregator pins SPEL and LEZ as one decision, spelled as the graph resolves them | accepted |
| [32](0032-one-price-account-per-feed-and-anyone-may-fill-it.md) | One price account per feed, at a derived address, and anyone may fill it | accepted |
| [33](0033-a-feed-lives-at-the-address-its-id-derives.md) | A feed lives at the address its id derives, so a client can find one | accepted |
| [34](0034-the-sequencer-runs-in-a-container-like-the-node-beside-it.md) | The sequencer runs in a container, like the node beside it | accepted, supersedes part of 12 |
| [M2-06:01](unnumbered-M2-06-where-the-admin-authority-comes-from.md) | Where the admin authority comes from, and how it is established | accepted, unnumbered |
| [M2-16:01](unnumbered-M2-16-where-a-payload-nobody-captured-comes-from.md) | Where a payload nobody captured comes from | accepted, unnumbered, supersedes part of 21 and 23 |
| [M2-19:01](unnumbered-M2-19-the-end-to-end-tests-resolve-in-their-own-workspace.md) | The end-to-end tests resolve in their own workspace | accepted, unnumbered, extends 7 |
| [M3-01:01](unnumbered-M3-01-the-pull-library-is-a-call-not-a-layer.md) | The pull library is a call, not a layer | accepted, unnumbered |
| [M3-04:01](unnumbered-M3-04-where-a-two-mode-test-can-live.md) | Where a test that needs both modes can live | accepted, unnumbered |
| [M3-05:01](unnumbered-M3-05-where-a-reading-consumers-trust-comes-from.md) | Where a reading consumer's trust comes from | accepted, unnumbered |
| [M3-06:01](unnumbered-M3-06-where-a-pull-consumers-program-lives.md) | Where a pull consumer's program lives | accepted, unnumbered |
| [M3-06:02](unnumbered-M3-06-where-a-pull-consumers-trust-comes-from.md) | Where a pull consumer's trust comes from | accepted, unnumbered |
| [M3-08:01](unnumbered-M3-08-what-a-read-costs-is-two-figures-per-mode.md) | What a read costs is two figures per mode | accepted, unnumbered |
| [M3-08:02](unnumbered-M3-08-the-guests-are-compiled-in-a-container.md) | The guests are compiled in a container, so a program id is a property of the source | accepted, unnumbered |
| [M3-09:01](unnumbered-M3-09-the-precompile-delta-is-a-function-not-a-figure.md) | The precompile delta is a function, not a figure | accepted, unnumbered |
| [M3-10:01](unnumbered-M3-10-a-transaction-is-not-its-body.md) | A transaction is not its body, and the difference is measured | accepted, unnumbered |

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

ADR 32 is where three of those decisions stop being about a verification and
start being about a transaction. ADR 13 chose a clock account, ADR 16 made the
asset pair the caller's claim, and ADR 22 wrote the six fields — but none of them
had a caller, so none had to say which account a price belongs in, who creates it,
or who may ask. Deriving the account from the feed answers the first two together:
the address a caller must reproduce is also the seed a first write claims. The
third goes the other way from how it looks — refusing to check the submitter is
the position that admits what a signature does and does not attest to, and it is
the one `MANIPULATION-ANALYSIS.md` had been written against.

It also finds the sharp edge in the mechanism. Making an account's address a
function of something also makes it unique, and uniqueness in LEZ is permanent:
ownership is never released, so an address claimed once is claimed for that
program build's lifetime. That is why the price account is derived from the feed
and the feed is not derived from the asset pair — one is a binding, the other
would be a registration that a single wrong exponent could burn.

[M3-01:01] is ADR 13 reaching its second caller, and the place where pinning a
clock turns out to guarantee less than it does in the first. In push mode a relayer
supplies the clock and a consumer reading the account afterwards bears the risk, so
the check is a boundary against a third party — and the dispatcher hands
`submit_price` an account whose id and data are fields of one struct, which is what
makes it enforceable. In pull mode the program supplies its own clock as two
separate slices, so pinning catches the wrong-account mistake, which is the likely
error, and cannot catch fabrication at all. A program that fabricates one is
misleading its own users, so what is left is an obligation on the consumer rather
than a hole in the library, and the reference consumer is where it is shown. The
rest of that record is about adding nothing: one function over `verifier-core`'s
own types is what makes ADR 2's "one verification, two modes" an identity rather
than a claim two crates have to keep agreeing on.

[M3-06:01] and [M3-06:02] are that obligation being discharged, and each turns out
to be about a boundary the library could describe but not hold. The first is a
question about packaging that became a question about evidence: a SPEL program is a
guest binary, `cargo tree` resolves a package rather than a binary, and the package
holding the aggregator's guest cannot host a consumer whose whole claim is that it
reaches no aggregator crate. Giving the consumer its own guest workspace is what
makes the closure an assertion about the program and not only about the library it
calls. The second is where the signer set comes from, and the answer is less obvious than
it looks. A shared borrow proves a roster was not mutated during a call and says
nothing about where it came from, so the demonstration had to be about provenance —
and provenance reads at first like a compiled constant, since nothing a caller sends
can reach one. F9 asks for a *configured* tuple with no dependency on the
aggregator's price account, though, and a roster in an account the consumer itself
owns is neither a dependency on the aggregator nor a registration against it; SEC2's
"comes from the consumer" is answered better by governed state than by a literal,
because governed state also shows how a consumer governs its set. ADR 30 settles the
rest: RedStone rotates, which is why the push path has `update_signer_set` and not a
table, and a compiled roster would meet that certain event with a redeployment —
which in LEZ moves every derived address and abandons every open order. So the
consumer carries an authority, a trust account per feed, and a rotation that is a
transaction.

[M3-05:01] is the same question put to the other mode, and it is worth reading beside
[M3-06:02] because the answer matches while the argument does not. A reading consumer
sends the aggregator no transaction, so what it has to decide is which account to
believe — and nothing in the six published fields says who wrote one. `source_id` names
RedStone rather than a program, so the only binding is the address, which hashes the
aggregator's program id, which is its image id. A compiled id looked defensible here in
a way a compiled roster never did: the address would simply go dead and every read would
refuse, which is what U7 asks for. What decides it is the cost of recovery rather than
the failure itself. Following the rebuild would mean rebuilding the consumer, and that
moves the consumer's own accounts — so an aggregator fix would cost the order book. The
trigger differs, the mechanism is the one ADR 33 describes, and the answer is the same
account-behind-a-gate on both sides.

[M3-08:01] is the same image-id argument arriving from a direction nobody was
watching. [M3-05:01] and [M3-06:02] both turn on what happens when a program id
moves, and both reason about it as a *deployment* risk: a rebuilt aggregator, a
rotated roster. What M3-08 found is that a measurement can move one by accident.
Adding two dependency edges to the pull consumer's guest manifest — for a cost
binary that ships nowhere — moved `PULL_CONSUMER_ID`, which would have abandoned
every account a deployed consumer owns. The fix is that the crates re-export what
their guests need, so nothing is added to a workspace whose ELF is a program id;
and the re-exports turn out to be owed anyway, because `read_price` takes a
`LezClock` a caller previously could not name. The lesson worth carrying is that
the closure of a guest workspace is part of the deployed artefact, and only a
measurement says whether an edge is free.

[M3-09:01] is the answer [M3-08:01] made computable, and it inherits that ADR's
discipline about what a measurement is allowed to claim. M3-08 refused to publish a
mode's read that contained the consumer's bookkeeping; M3-09 refuses to publish a
precompile delta that contains our guess at somebody else's syscall cost. In both cases
the fix is the same shape — name the term, put it where it cannot contaminate the
measured part, and let the reader see which is which. The band is also the reason the two
ADRs disagree about presentation: M3-08's figures are exact because they are measured, and
M3-09's are a function because one of them is not.

ADR 29 closes a range that only had one end. ADR 18 gave staleness a two-sided window and
left `maxAge` to the feed; registration refused zero and nothing else. But `freshness`
saturates, so a `maxAge` near `u64::MAX` put the window's lower edge at zero and admitted
every timestamp in the past — the check was configurable into non-existence, and a feed in
that state looked configured like any other. The test suites had been relying on exactly
that to mean "this test is not about the clock", which is why a one-comparison check
arrives with fifty sites of churn behind it.

## Open questions carried by these decisions

Seven are with Logos. `m0/versions.md`, *Open: questions outstanding with Logos*,
states six of them in full and the mapping is not one-to-one, so it is spelled
out per bullet rather than by a count:

- **Is a SPEL release coming, or is pinning `main` sanctioned** (ADR 8, ADR 14). SPEL
  resolves LEZ, so this settles the LEZ pin with it — which is why M0's separate
  "which LEZ pin is the estate standardising on" is no longer asked on its own. The
  scaffold PR #246 half of that question is withdrawn: it merged on 2026-08-10, and
  M1-04a had already removed the dependency on `lgs test-node` (ADR 12).
  `m0/versions.md` item 1.
- **Where the RFP-001 admin-authority extension is going to live**, now that the work
  has moved off PR #212 and onto a separate extension crate in a personal namespace
  (ADR 14). `m0/versions.md` item 2.
- **Is LEZ's transitive LGPL-3.0 dependency an accepted position for the estate**
  (ADR 1) — and, separately, three Logos crates ship without a `license` field, which a
  one-line manifest change would fix for every downstream consumer (ADR 9). The
  second half is `m0/versions.md` item 3; the LGPL question is stated in
  `deny.toml` beside the exceptions it justifies rather than there.
- **May a program claim a non-default, default-owned account** (`[M2-06:01]`, ADR 33).
  One rule pair, met twice: it strands a signer a program declines to claim, and it lets
  anyone put one unit of balance on a publicly derivable PDA and make that address
  permanently unwritable — including the admin config account, whose address takes no
  input an attacker cannot predict. `m0/versions.md` item 4.
- **`OraclePriceAccount` is harder to depend on than it needs to be** (ADR 8,
  ADR 9): the account-type crate pulls risc0 and `uniswap_v3_math` in to carry six
  Borsh fields, and `=3.0.5` is an exact pin, which is what makes the canonical
  account awkward for the external adaptors it was written for.
  `m0/versions.md` item 5 has the detail.
- **Two things SPEL's IDL generator does that a program author cannot see**
  (ADR 32), found while declaring `submit_price`'s accounts and worth reporting
  rather than working around twice. It parses `#[account(owner = ...)]` and
  discards it, so an owner constraint is enforced at runtime and absent from
  every generated client — which is why Kanon's is a Rust check with a test
  recording why. And the documented `const("...")` seed spelling cannot parse,
  because a seed is read as an expression and a bare keyword is not one; only the
  raw `r#const("...")` and the legacy `literal("...")` work. Not in
  `m0/versions.md`, which predates it.
- **What happens to a price account when the program that created it is upgraded**
  (ADR 22). Nothing LEZ documents answers it. `m0/versions.md` item 6.

One that `m0/versions.md` carried as M2's is now closed rather than answered: which SPEL
revision the aggregator builds against had already been decided by the dependency graph,
because the canonical price account brings SPEL in through its own macro (ADR 31).

One is settled locally and still needs its measurement: which clock account an
oracle should read (ADR 13). M2-02 made the read real, on the every-block account,
so the freshness-against-contention trade the 10- and 50-block accounts exist for
is now a live property of every submission rather than a hypothetical one.
Measuring it needs a running sequencer, which no M2 task currently carries.

The CI ceilings that stop the accelerator configuration regressing silently
(ADR 6) landed with M1-25, and ADR 20 records that both of them turned out to be
cycle counts.
