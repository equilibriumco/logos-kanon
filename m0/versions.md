# Pinned versions

What to install for the figures in `m0/M0-report.pdf` to reproduce, and the one version
question still open.

This file is not the authority. The manifests and the six `Cargo.lock` files enforce
every crate version, and the exact-equality cycle assertions in the two guardrail
suites are what actually catch a wrong environment. Install the toolchain below and
run `cargo test --release --locked --workspace`; if it passes, the environment is
right.

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

Every requirement is `=` rather than a caret range. A range lets the software arm
resolve a different patch release than the fork tags the accelerated arms use, which
moves every recovery figure by about 0.01% and confounds the comparison, with nothing
visible in a green build.

## Open: which LEZ version

Two are in play, 784 commits and three months apart.

| | commit | tag | date |
| --- | --- | --- | --- |
| What the figures are built against (`lee_core`) | `15144ddb` | v0.2.1 | 2026-08-02 |
| What the `lgs` toolchain pins for its sequencer | `cf3639d8` | v0.1.2 | 2026-04-27 |

Upstream has since tagged v0.2.4, so neither is current. **No M0 figure is affected**:
cost is a property of the guest ELF and no sequencer takes part in producing one.
M1-05 is affected, because a test cannot transact against a sequencer until the two
agree.

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

**Decide the pin before M1 writes `verifier-core`.** Moving off v0.2.1 later means
re-measuring every figure and reissuing the report, and possibly API churn rather than
only new numbers. Two questions for Logos: which pin the estate is standardising on,
and the timeline for #246 including whether `test-node` is in scope.

## Changing any of this

Bump it, run both guardrail suites, and update the constants, the affected report
tables and this file in the same commit. A guardrail failure is a prompt to re-measure,
not necessarily a defect, but a published figure must never move without the table that
quotes it.
