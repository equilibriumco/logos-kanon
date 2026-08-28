# Pinned versions

What to install for the figures in `m0/M0-report.pdf` to reproduce, and the version
questions still open.

This file is not the authority. The manifests and the six `Cargo.lock` files enforce
every crate version, and the exact-equality cycle assertions in the two guardrail
suites are what actually catch a wrong environment. Install the toolchain below and
run `cargo test --release --locked --workspace` from `m0/`; if it passes, the
environment is right.

`m0/` is its own cargo workspace, separate from the product crates at the repository
root, and that is what keeps the pins below frozen: an ordinary product upgrade
cannot reach this lockfile, so it cannot move a published figure.

**The reasoning behind these pins lives in `adr/`, not here.** This file is the
environment, the open questions and the procedure for changing them. Each spike whose
finding settled a pin now has an ADR, and the pointers are in *Settled* below — one
statement of a decision, in one place, so the two cannot drift apart.

## Install this

rzup-managed, under `~/.risc0`, and recorded here because no lockfile can express
them: proving runs in an `r0vm` subprocess and the guest cross-compiles with RISC
Zero's own Rust.

| | |
| --- | --- |
| r0vm | **3.0.6** (3.0.5 verified identical) |
| guest Rust | **1.97.0** |
| cargo-risczero | **3.0.6** |

Both conditions are necessary. One machine gave different cycle counts until it came
off r0vm 3.0.3 with guest rustc 1.88.0. The **host** compiler is not a condition:
1.93.0 and 1.97.1 gave identical counts, because the host builds only the driver.

Everything else is in cargo: `risc0-zkvm` and `risc0-build` at `=3.0.6`, `lee_core` at
LEZ v0.2.1 (`15144ddb`), `k256` at `=0.13.3`, `tiny-keccak` at `=2.0.2`, plus the
`[patch.crates-io]` fork tags in each guest manifest.

### Why every requirement is an `=` pin

Every requirement is `=` rather than a caret range. A range lets the software arm
resolve a different patch release than the fork tags the accelerated arms use, which
moves every recovery figure by about 0.01% and confounds the comparison, with nothing
visible in a green build.

That is the measurement-integrity half of the argument. The product side of it — and
why the product's pins are deliberately *not* the same as these — is
`adr/0008-exact-pins-and-tracking-the-estate.md`.

## Settled: where the decisions behind these pins are recorded

Five spikes ran during M0 and M1 to answer questions these pins depend on. Each is now
an ADR, with the finding, the decision and what it cost:

| question | answer | ADR |
| --- | --- | --- |
| Which accelerator configuration should the guest use? | mixed: accelerated secp256k1, software keccak256 | [6](../adr/0006-mixed-accelerator-configuration.md) |
| Why does `m0/` have its own lockfile? | so a product upgrade cannot move a published figure | [7](../adr/0007-two-workspaces-and-a-lockfile-per-resolution-domain.md) |
| Can the product sit on risc0 3.0.6 and LEZ v0.2.1, as `m0/` does? | no — it tracks the estate at 3.0.5 and v0.2.0, and the figures stay comparable, measured rather than assumed | [8](../adr/0008-exact-pins-and-tracking-the-estate.md) |
| Does the canonical RFP-019 price account exist to write into? (M1-09) | yes, `twap_oracle_core::OraclePriceAccount`, re-exported rather than forked | [9](../adr/0009-re-export-the-canonical-price-account-and-vendor-its-idl.md) |
| Where does a `maxAge` comparison get "now"? (M1-08) | LEZ's clock program, the every-block account, never caller-supplied | [13](../adr/0013-staleness-is-measured-against-the-lez-clock-program.md) |
| Is there an admin-authority interface to build F6 against? (M1-07) | yes, `logos-co/spel` PR #212 — written, unmerged; built against with a shim in reserve | [14](../adr/0014-build-admin-gating-against-the-unmerged-spel-admin-authority.md) |

## Open: which LEZ version

Two are in play, 784 commits and three months apart.

| | commit | tag | date |
| --- | --- | --- | --- |
| What the figures are built against (`lee_core`) | `15144ddb` | v0.2.1 | 2026-08-02 |
| What the `lgs` toolchain pins for its sequencer | `cf3639d8` | v0.1.2 | 2026-04-27 |

Upstream has since tagged v0.2.4, so neither is current. **No M0 figure is affected**:
cost is a property of the guest ELF and no sequencer takes part in producing one.

The product has since moved to v0.2.0 for reasons unrelated to either — it tracks the
LEZ that `lez-programs` is built against (ADR 8) — so there are now three revisions in
the estate and `m0/` deliberately stays on the one its figures were measured against.

**M1-05 is not affected either.** M1-04a resolved it by not using `lgs`: see below.
What remains open is SPEL, which matters to M2.

A spike established the following, and it is all reproducible from
`logos-co/scaffold` at `9fcc3766`:

- `lgs test-node prepare --lez-ref 15144ddb…` **builds** a v0.2.1 sequencer in 2m02s,
  so this is not a deep API break.
- `lgs test-node start` **fails** on it: v0.2.x replaced `genesis_id`,
  `is_genesis_random`, `initial_accounts` and `initial_commitments` with a single
  tagged `genesis` array plus `bedrock_config.funding_key`, and scaffold requires a
  numeric `genesis_id` (`src/testnode/mod.rs`). Its state seeding writes
  `initial_public_accounts` / `initial_private_accounts`, which no longer exist.
- **`localnet` is unaffected.** `prepare_sequencer_config` discards `genesis_id`, so
  running a v0.2.x sequencer is not blocked; only the isolated test-node harness is.
- **Already solved downstream, with no upstream change required.**
  `logos-co/eth-lez-atomic-swaps` runs LEZ **v0.2.2** (`d6e4ae69`) via a
  caller-provided `[repos.lez].path` plus a local bridge script. A v0.2.x-aligned SPEL
  exists: scaffold's default `73fc462e` vendors v0.1.2, while `3d639076` vendors
  v0.2.0-rc3.
- Upstream scaffold #240 / PR #246 covers `setup`, `run` and `doctor`, but not
  `test-node`.

### How M1-04a resolved this, for integration tests

The blocker was never LEZ. It was `lgs`: `test-node` generates a v0.1.2-shaped
`sequencer_config.json` and cannot start a v0.2.x sequencer, so using it forced a
choice between the revision the figures are built against and the revision the
toolchain pins.

`scripts/lez-sequencer.sh` skips scaffold and runs the two services LEZ itself runs,
deriving the revision from the committed product lockfiles so no mismatch can enter.
The full decision, including why CI pulls a prebuilt image rather than building LEZ, is
[ADR 12](../adr/0012-a-standalone-lez-sequencer-without-lgs-run-from-a-prebuilt-image.md);
the script's own header carries the operational detail.

One distinction to keep in view: the sequencer blocks on Bedrock's `/time/info` at
startup and never opens its RPC port without it, so **Bedrock is not optional** — but
that is a startup dependency and nothing more. It is *not* a `maxAge` time source. That
answer is the clock program, ADR 13.

**Closed by M2-01, and it was never a choice.** This section used to read that SPEL sat
at v0.1.2 and that M2-01 would have to pick between that and a v0.2.x-aligned scaffold.
Both figures were stale. SPEL is at v0.6.0, `scaffold.toml` pins `0cb7e098`, and
`Cargo.lock` already resolved `spel-framework-core` at that same commit before the
aggregator existed -- `twap_oracle_core` uses the framework's `#[account_type]` macro, so
the canonical price account brought SPEL into the graph.

What remains is a constraint rather than a question. `spel-framework` names its own LEZ
tag (`v0.2.0`, as `nssa_core`, which is `lee_core` renamed), so a SPEL bump and a LEZ bump
are one decision: cargo unifies the two spellings only when source and tag match exactly,
and `AccountId` is defined there. [ADR
31](../adr/0031-the-aggregator-pins-spel-and-lez-as-one-decision.md) records it.
Deploying a program through `lgs` is still untried, and is M4's rather than M2-01's.

## Open: the LGPL-3.0 dependency in LEZ's host graph

M1-04's licence gate found copyleft in the dependency graph. It needs a stated
decision rather than an unread exception.

| crate | licence | how it arrives |
| --- | --- | --- |
| `malachite`, and its `-base`, `-float`, `-nz`, `-q` crates | LGPL-3.0-only | `lee_core` → `risc0-zkvm` → `risc0-circuit-rv32im` |
| `downloader` | LGPL-3.0-or-later | build-time only, under `risc0-circuit-recursion` |
| `option-ext` | MPL-2.0 | build-time only, under risc0's kernel build |

The build-time two are unremarkable: they run during a build and are linked into
nothing that ships. `malachite` is the one to decide about, and the scope is narrower
than the table implies. It arrives only when risc0's **proving** feature is enabled,
which is this measurement workspace's configuration: the five LGPL crates are in
`m0/Cargo.lock` and **absent from the product lockfile**, so no shipped host binary
links them today. That will change if a product binary ever proves locally rather than
delegating, which is worth knowing before the relayer grows a proving path. No guest is
affected either: the guest workspaces resolve separately and the gate is clean against
all six of their lockfiles.

No first-party code pulls it in, and LEZ itself ships `MIT or Apache-2.0` while carrying
it, so this is an inherited position rather than a chosen one. But RFP-020 requires an
open-source deliverable and the RFP-020 proposal committed to a gate that fails on
copyleft, transitive dependencies included, so it is recorded as a narrow per-crate
exception in `deny.toml` and raised here rather than waved through. The gate itself is
[ADR 1](../adr/0001-dual-licence-enforced-by-a-machine-checked-allowlist.md).

## Open: questions outstanding with Logos

Five, in descending order of how much they block delivery. All five concern
repositories outside this one, which is why they are tracked here rather than resolved
in code.

**1. Which LEZ pin the estate is standardising on**, and the timeline for scaffold
PR #246 — including whether `test-node` is in scope. Three revisions are now live
across `m0/`, the product and `lgs`. Detail in *Open: which LEZ version* above.

**2. Is `logos-co/spel` PR #212 merging, and when.** Not "is there an admin-authority
interface" — there is, and it is written. The PR has had no activity since 2026-05-20,
and M2-06 through M2-10 are designed against its `#[require_admin(config)]` plus
`AdminConfig` shape with a shim held in reserve (ADR 14). One naming trap: the RFP-001
milestone text promises `renounce_admin` where the code implements `revoke_admin`.

**3. Three Logos crates ship without licence metadata.** `twap_oracle_core`,
`spel-framework-core` and `spel-framework-macros` declare no `license` field, so
cargo-deny reports them as unlicensed. They are not: `lez-programs` ships a LICENSE
file (MIT) and `spel` ships LICENSE-MIT and LICENSE-APACHE-v2, and the terms are
permissive and compatible with the delivery licences. `deny.toml` carries three
`[[licenses.clarify]]` entries rather than a widened allowlist, because a
clarification names one crate and can be checked. This is the cheapest of the five
asks: a one-line `license = "..."` in each manifest removes all three entries and
stops every downstream consumer having to make the same judgement call privately.

**4. The rule pair that makes a default-owned account permanently unwritable, and what
it costs.** LEZ increments every signer's nonce after applying a state diff, outside
program execution (`lee/state_machine/src/state.rs:212-216`); rule 6 refuses a data
change unless the executing program owns the account or the pre-state is default; and
rule 7 refuses any post-state whose program owner is the default one unless the
pre-state was `Account::default()` (`lee/state_machine/core/src/program.rs:711-731`).
The claim mechanism cannot undo either, because the claim loop runs *after*
`validate_execution` (`lee/state_machine/src/validated_state_diff.rs:215` then `:267`).

Together those make an account that is non-default and default-owned permanently
unwritable by any program. We have now met it twice, from opposite directions, and the
second is the sharper one.

**As a trap.** A program that takes a signer and does not claim it strands that account:
the signer passes once while it is still pristine, LEZ bumps its nonce, and every later
post-state naming it is refused. `aggregator-program` refuses a default-owned admin key
rather than claim the operator's wallet (`AdminUnowned`), because rule 4 would make that
ownership permanent and rule 5 would let the program move the balance. So: **is a
program obliged to claim or reject a default-owned signer?** "You must claim" is a
change to every program that takes one; "claim or reject" is satisfied by a guard.

**As a griefing vector, which is the part worth answering.** Rule 5 guards balance
*decreases* only, so anyone may *increase* the balance of an account they do not own,
and rule 7 admits a pristine target. One unit of balance on a publicly derivable PDA
therefore puts that address into the unwritable state for good, before the program that
owns the derivation has ever run. Measured on this branch: the transfer validates, the
generated `init` check then answers `AccountAlreadyInitialized` forever — with
`RegisterError::AlreadyRegistered` one layer further in, which is what a host test sees
— and any post-state writing the account is refused by rule 6.

The reach is the whole of this adaptor's account model. Feed accounts derive from the
feed id, and the five production ids are published in `FEEDS.md`. The **admin config
account is the worst case**: its address derives from the program id and a constant,
both of which an attacker can compute, so squatting it makes the generated `init` check
answer `AccountAlreadyInitialized` forever — `AlreadyInitialised` is what the pure
function returns, one layer further in — and the deployment is dead before the operator
can bootstrap it. Price accounts derive from the feed account's id and go the same way.
Rebuilding is not an escape either: the program id is the image id and ADR 8's exact pins make the build
reproducible, so the next build's addresses are computable from published source too.

**So the question is whether a program may claim a non-default, default-owned account.**
If it may, both problems close at once — the squatted address becomes recoverable and
the signer trap stops being a trap. If it may not, then every program in the estate
using derived addresses can be denied for one unit of balance by anyone who can read its
source, and that is worth knowing before mainnet. Pinned at both layers rather than
assumed: `aggregator-program/tests/register_feed.rs::a_derived_address_can_be_squatted_and_this_pins_the_refusal`
and `tests/admin.rs` for the pure functions,
`a_squatted_feed_account_is_refused_by_the_generated_validator` and
`a_squatted_admin_config_is_refused_by_the_generated_validator` in
`methods/guest/src/bin/aggregator.rs` for the dispatcher.

**5. `OraclePriceAccount` is harder to depend on than it needs to be**, and this is
closer to a defect report than a preference: the account-type crate needs neither
risc0 nor `uniswap_v3_math` to carry six Borsh fields. Splitting it into a standalone
crate with relaxed pins — or at minimum loosening `=3.0.5` to `^3.0.5` — would make the
canonical account usable by the external adaptors it was explicitly written for. What
its current pins cost this repository is in ADR 8 and ADR 9.

## Changing any of this

Bump it, run both guardrail suites, and update the constants, the affected report
tables and this file in the same commit. A guardrail failure is a prompt to re-measure,
not necessarily a defect, but a published figure must never move without the table that
quotes it.
