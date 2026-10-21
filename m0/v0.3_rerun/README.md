# M0, re-measured against LEZ v0.3

M0 was measured against LEZ v0.2.1 (`15144ddb`). The testnet has since moved to
v0.3.0 (`db66590a`), which replaced the program model and introduced a fee market.
This directory is the re-measurement, and the answer is that **the report stands**:
all four recommendations hold, and the cryptography they rest on is unchanged to
within ten cycles on 1.7M.

Two things found along the way grew past the rerun and became the headline
(`FINDINGS.md` §4.1 and §6.1): a first-party recovery backend that measures
109,502 cycles per recovery in the product — 5.2x under the `k256` the report
priced — and RedStone's live multi-point `___FAST___` package, which carries
BTC, ETH and SOL under one signature per signer and takes the five-feed round
to ~12.9% of a block with every delivered optimization. The probe's
`--multifeed` flag measures the package shapes; the product-side tests in this
tree are `methods/tests/recovery_cost.rs`, `methods/tests/backend_equivalence.rs`
and `verifier-core/tests/multipoint_fast.rs`.

**Two harnesses the findings cite are not in this tree.**
`a_multipoint_fast_submission_measures` (`methods/tests/phases.rs`) and
`the_final_round_on_the_storage_market` (`kanon-chain/tests/storage_gas.rs`)
belong to the push aggregator and the chain client, which arrive with the
milestones after M3. The figures they produced are quoted in `FINDINGS.md` §7
and §7b and marked there; the recovery backend and the multi-point package
shapes — the two headline claims — are reproducible here in full.

| file | what it is |
| --- | --- |
| `M0-addendum-A.pdf` | **the deliverable** — an extension to `m0/M0-report.pdf`, keyed to its sections |
| `FINDINGS.md` | the working: what was run, what moved, what was not re-measured |
| `lez-probe/` | `m0/lez-probe/` ported to v0.3's program model |
| `raw/` | unedited output of the runs behind both documents |

The addendum's typst source is held with the other milestone reports, outside this
repository. Only the built PDF is tracked here, as `m0/M0-report.pdf` is.

## Why this is a separate workspace

`m0/` is frozen. Its own lockfile and its exact-equality assertions exist so that an
ordinary product upgrade cannot move a figure in a delivered document. Re-measuring
inside it would defeat that. So this is a separate workspace for the same reason
`m0/` is separate from the product crates: neither tree's figures can move the
other's.

Every pin is identical to `m0/`'s — risc0 `=3.0.6`, `k256` `=0.13.3`, `tiny-keccak`
`=2.0.2`, the same guest toolchain — and **only the LEZ revision differs**. That is
what makes a difference between the two sets of figures attributable to LEZ and to
nothing else.

## Why only `lez-probe/` is here

M0 has two harnesses, and only one of them can see a LEZ version.

`m0/cost-baseline/` measures the primitives on bare RISC Zero, with no LEZ, no SPEL
and no sequencer in the dependency graph. No LEZ version can reach it. That is a
fact about the build rather than a judgement, and re-running it under the frozen
workspace confirms every primitive figure in M0 §3.1 and §3.2 unchanged. Copying it
here would duplicate frozen code to no end.

`m0/lez-probe/` links `lee_core`, so it is the half that moved, and it is the half
that is ported here.

## What the port involved

v0.3 is a different program model, so this is a rewrite of the probe rather than a
pin bump:

- A program is a `plan` run plus one `apply` run per shard the plan names. Both are
  metered, so the billed figure is the sum, and the harness reports the split.
- Call frames are borsh — `CallKind`, then `PlanInput` or `ApplyInput` — not risc0
  serde words. The instruction rides as borsh bytes inside
  `PlanInput.instruction_data`.
- `read_lee_inputs` / `ProgramOutput` / `AccountPostState` / `Claim` are gone,
  replaced by `run_program(plan, apply)` and `Plan::effect`.
- Gas is cycles one for one, against `MAX_GAS_EXEC` = 10,000,000 rather than v0.2's
  `MAX_NUM_CYCLES_PUBLIC_EXECUTION` = 33,554,432.

**The workload is byte-for-byte what the v0.2 probe measured**: the same
deterministic keys, the same 77-byte signed ranges, the same keccak-then-recover per
signer. That is what licenses comparing the two generations at all, and
`the_cryptography_did_not_move_between_lez_generations` is the assertion that keeps
it honest.

## Running it

```sh
nix-shell                                   # NixOS only

cargo test --release                        # the published figures, seconds
cargo test --release -- --ignored           # proof sizes, minutes
cargo run  --release -p lez-probe           # print the tables
cargo run  --release -p lez-probe -- --signers 1,3,5,10,20
cargo run  --release -p lez-probe -- --signers 3 --prove --repeat 2
cargo run  --release -p lez-probe -- --multifeed    # the ___FAST___ round shapes
```

Toolchain as `m0/versions.md` pins it: r0vm 3.0.5 (recorded there as verified
identical to 3.0.6), guest rustc 1.97.0, cargo-risczero 3.0.6.

## Changing any of this

As in `m0/`: bump it, run the suite, and update the constants, `FINDINGS.md` and
the addendum's tables in the same commit. A guardrail failure is a prompt to
re-measure, not necessarily a defect — but a published figure must never move
without the table that quotes it.
