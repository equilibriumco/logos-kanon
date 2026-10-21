# M0 re-run against LEZ v0.3 — findings

Run on 2026-10-05; §4.1 and §6.1 added 2026-10-07. Two workspaces: `m0/` is the
delivered M0 tree unchanged, and this directory is the same tree with `lez-probe`
ported to LEZ v0.3.0 (`db66590a`) and nothing else touched.

Toolchain as `m0/versions.md` pins it: r0vm 3.0.5 (the file records 3.0.6 and 3.0.5
as verified identical), guest rustc 1.97.0, cargo-risczero 3.0.6, risc0-zkvm =3.0.6.

---

## 1. M0 as delivered still reproduces, exactly

```
cargo test --release --locked --workspace   # from m0/
```

Exit 0. Every exact-equality guardrail in both suites passes, including
`MIXED_3_SIGNERS = 1_906_737`. The published figures are not stale in the sense of
being wrong about what they measured.

`m0/cost-baseline/` is unaffected by v0.3 **by construction** — it runs on bare
RISC Zero with no LEZ in the graph at all. Re-running it confirms the component
costs are unchanged:

| | accelerated | mixed |
| --- | ---: | ---: |
| 3 recoveries | 1,686,893 | 1,686,893 |
| 3 keccak256 over 77 B | 7,744 | 51,956 |
| zkVM floor | 2,940 | 2,940 |
| software recovery, 3 signers | 33,228,606 | 33,228,606 |

So the only half of M0 that v0.3 can move is `m0/lez-probe/`, and it moved.

---

## 2. What the port involved

v0.3 is a different program model, so this is a rewrite of the probe rather than a
pin bump:

- A program is now a `plan` run plus one `apply` run per shard the plan names.
  Both are metered, so the billed figure is the sum. The old probe measured one run.
- Call frames are borsh (`CallKind`, then `PlanInput`/`ApplyInput`), not risc0
  serde words. The instruction rides as borsh bytes inside `PlanInput.instruction_data`.
- `read_lee_inputs` / `ProgramOutput` / `AccountPostState` / `Claim` are gone,
  replaced by `run_program(plan, apply)` / `Plan::effect`.

The **workload is byte-for-byte identical**: same deterministic keys, same 77-byte
signed ranges, same keccak-then-recover per signer. That is what makes the two
generations comparable.

The port is roughly 250 lines of host and 130 of guest.

---

## 3. The headline numbers

### v0.2.1 (delivered M0)

| | 1 signer | 3 signers | 5 signers |
| --- | ---: | ---: | ---: |
| mixed, total cycles | 664,117 | **1,906,737** | 3,150,337 |
| share of the 33,554,432 budget | 1.98% | **5.68%** | 9.39% |
| accelerated, total cycles | 650,653 | 1,862,527 | 3,075,381 |
| framework floor | 39,103 | 39,103 | 39,103 |
| instruction handling | 39,869 | 119,187 | 198,505 |

### v0.3.0 (this re-run)

| | 1 signer | 3 signers | 5 signers |
| --- | ---: | ---: | ---: |
| mixed, total gas | 600,881 | **1,766,937** | 2,933,979 |
| &nbsp;&nbsp;of which plan | 594,597 | 1,760,653 | 2,927,695 |
| &nbsp;&nbsp;of which apply | 6,284 | 6,284 | 6,284 |
| share of the 10,000,000 limit | 6.01% | **17.67%** | 29.34% |
| accelerated, total gas | 587,414 | 1,722,716 | 2,859,004 |
| framework floor | 14,038 | 14,038 | 14,038 |
| instruction handling | 1,692 | 4,442 | 7,198 |

### What moved, and why

**The program got cheaper. The denominator got smaller. The second dominates.**

- **Total, 3 signers: 1,906,737 → 1,766,937, down 7.3%.** Entirely framework: the
  floor fell 2.8x (39,103 → 14,038) and per-package instruction handling fell
  **27x** (119,187 → 4,442), because borsh instruction bytes replaced risc0 serde
  words, which spend four bytes per byte.
- **The cryptography did not move at all.** Backing recovery out as a remainder
  gives 1,696,501 on v0.3 against 1,696,491 on v0.2 — ten cycles apart on 1.7M.
  This is the strongest single result of the re-run: v0.3 changes the wrapper and
  leaves the thing M0 was actually about untouched.
- **The budget is now 10,000,000, not 33,554,432.** v0.3 bills gas as cycles one
  for one, capped at `MAX_GAS_EXEC = 2 × TARGET_GAS_EXEC`
  (`lez/programs/fee/core/src/market.rs`). A transaction declaring more is rejected
  (`FeeError::GasLimitAboveCap`).

So **5.68% becomes 17.67%** at three signers — a 3.1x worse share from a 7.3%
cheaper program.

---

## 4. Does any M0 decision change? None is withdrawn; two need restating.

### Accelerate secp256k1 — stronger

Software recovery for 3-of-N is 33,347,017 cycles. Under v0.2 that was 99.4% of the
budget and "does not fit". Under v0.3 it is **333% of the limit** — over by 3.3x
before the framework. The accelerator was already non-optional; it is now
non-optional by a wider margin.

### Leave keccak256 in software — stands, but re-decide it

The coprocessor buys 44,221 gas at three signers (1,766,937 → 1,722,716): 2.5% of
the program, 0.44 percentage points of the limit, and **no change to how many
updates fit a block** (five either way at three signers, three at five).

M0 gave three reasons for software keccak. §7 re-measures all three and **two
fail**: the 223 KB proof penalty vanishes under recursion, and the 6 GiB VRAM
cliff does not reproduce on the same card in either proving mode. What survives is
proving time, +21% succinct. Not reversed here — nothing operational turns on
2.5% — but it should be re-decided rather than inherited.

### Default to push, price pull — unchanged

Both modes run the same verifier over the same payload, so both cost the plan's
~1.77M gas. Nothing in v0.3 separates them.

### Recommend a secp256k1 precompile — superseded: it already exists

**M0 asked Logos to build this. It has been built, by Boundless, and we have
measured our own program running on it.**

`risc0-crypto-evm` (Apache-2.0) ships `secp256k1_ecrecover`, backed by the same
`risc0-bigint2` accelerator our patched `k256` already uses. It takes a
signature, recovery id and digest and returns a 20-byte address — exactly what
RedStone verification needs, since a signer is identified by address.

Measured end to end, not inferred: the probe carries a second copy of its LEZ
program, identical except for the recovery call.

| signers | `k256`, today | share | `risc0-crypto-evm` | share | |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 600,881 | 6.01% | 170,757 | 1.71% | 3.52x |
| 3 | **1,766,937** | **17.67%** | **481,474** | **4.81%** | **3.67x** |
| 5 | 2,933,979 | 29.34% | 792,176 | 7.92% | 3.70x |

Per recovery in isolation: 570,683 through `k256` against 140,542 — 4.06x, and
that understates it, since `secp256k1_ecrecover` also hashes the key to an
address where our path hashes separately.

**It would also dissolve §6's simultaneity bound**: five feeds at five signers
come to 3,960,880 gas, under 40% of a block — and 2,741,204 (27.4%) when BTC,
ETH and SOL ride one multi-point package (§6.1).

**Why the earlier estimates were wrong.** M0 projected 8–9x and an earlier draft
of this file projected 18–24x. Both assumed a recovery collapses to the cost of
one syscall. Measured, 29% of a recovery is curve arithmetic that `risc0-bigint2`
already accelerates — `k256` has called into it unconditionally in guest builds
all along:

| | cycles per recovery | |
| --- | ---: | --- |
| accelerator operations alone, 2 scalar mults + add | 164,447 | 28.8% |
| the same through `k256` | 274,289 | +109,841 wrapper |
| a full recovery through `k256` | 570,683 | +296,393 recovery logic |
| the same through `risc0-crypto-evm` | **140,542** | **4.06x** |

Pinned by `the_r0crypto_program_is_unchanged` and
`the_irreducible_share_of_a_recovery_is_unchanged`.

**Do not adopt it on these numbers alone.** It is `0.1.0-rc.1`; it is a different
cryptographic implementation from the widely-reviewed `k256`, which makes this an
audit question first; it pulls `alloy-primitives`; and changing the guest changes
the image id, which is the program id, so every derived account address moves
(ADR 22). Worth a proper evaluation with a migration plan as later-milestone
work.

### 4.1 Superseded in part: a first-party backend now beats it, measured in the product

The evaluation happened, and went one step further
(`verifier-core/src/backend/fast_recovery.rs` in the product): a recovery
backend built directly on
`risc0-crypto`'s bigint2 curve circuits, driven by a GLV + wNAF + fixed-base
loop of our own instead of its 1-bit Strauss–Shamir — plus an addition-chain
square root for decompression. Measured in isolation inside the zkVM
(`methods/tests/recovery_cost.rs`):

| backend | cycles per recovery |
| --- | ---: |
| patched `k256` | 568,350 |
| `risc0-crypto-evm` ecrecover | 137,324 |
| **first-party GLV + wNAF + fixed-base** | **109,502** |

Correctness is pinned by a 50-case in-zkVM equivalence suite (all three
backends must agree exactly, errors included: GLV sign splits, both `v`
conventions, `r`/`s` at zero and at the order, decompression edges, a
signature recovering to the identity). The product guests run it; the phase
table there says **411,376 gas per 3-signer update and 642,321 at five**
(plan + both applies), with the relayer trimming payloads to the threshold.
The audit caveat above still applies in full — the field and curve arithmetic
underneath is the same rc-grade `risc0-crypto` — but the surface that is
first-party is the orchestration, which is small and separately testable.

## 5. One M0 claim is now wrong, and the product already acted on it

> "At 621,555 cycles per signer, the budget accommodates roughly **53 signers** in
> one transaction."

At v0.3 the marginal cost is 583,024 gas per signer against a 10,000,000 limit, so
the real figure is about **17 signers**, and that is the arithmetic ceiling before
payload bytes and the registration the submission carries. `MAX_SIGNERS` is 32 in
`verifier-core/src/feed.rs`, which this ceiling no longer supports: lowering it is
a product change rather than a measurement one, and it is the report's sentence
that is stale either way.

---

## 6. New, and not in M0 at all: the per-block cap

The one item here we could not answer ourselves — though see the correction below
on what it actually establishes.

`MAX_GAS_EXEC = 10,000,000` is **both** the most a transaction may declare and a
cap on the whole block: `accumulate_exec_gas` sums each transaction's actually
charged cycles across the block and rejects the block when the total passes it
(`lez/chain_state/src/apply.rs:452`). `TARGET_GAS_EXEC` is 5,000,000, and the base
fee rises up to 12.5% per block above target.

So, chain-wide, across all programs and all users:

| | gas | share of a whole block |
| --- | ---: | ---: |
| one 3-signer Kanon update | 1,766,937 | 17.7% |
| one 5-signer Kanon update | 2,933,979 | 29.3% |
| five feeds updating together, 5 signers | 14,669,895 | **147% — will not fit** |
| the same, BTC/ETH/SOL via one `___FAST___` package (§6.1) | 9,158,583 | 91.6% — fits, barely |
| `___FAST___` round with risc0-crypto-evm recovery (§6.1) | 2,741,204 | 27.4% |

M0 reasoned entirely about *fitting one transaction*, which was the right question
under v0.2 and is still answered yes with room. It never asked how many updates fit
a *block*, because v0.2 had no block-level meter.

**This is a bound on simultaneity, not on throughput** — an earlier draft of this
file and of the addendum overstated it as contention. Five feeds cannot settle in
one block at five signers; they settle in two or three, and at a one-hour
heartbeat there are 3,600 seconds in which to place five updates. The relayer
spreads them, which it would do anyway.

What it does establish, narrowly:

1. **A hard simultaneity bound.** No batching puts five five-signer feeds in one
   block, so atomic multi-feed updates — one transaction carrying several feeds so
   consumers see them move together — are ruled out at this roster size.
2. **Base-fee pressure when updates coincide.** Two five-signer updates exceed
   `TARGET_GAS_EXEC` (5,000,000) on their own. Small duty cycle at an hourly
   heartbeat, but the mechanism exists and a tighter cadence would exercise it.
3. **One transaction may take a third of a block** — a property of the chain
   rather than of this adaptor, and the part worth confirming as intended.

### 6.1 The multi-point round: `___FAST___` for BTC/ETH/SOL, singles for XMR and ZEC

RedStone already publishes **multi-point packages** on `redstone-primary-prod` — one signature covering
several data points. The live `___FAST___` package carries nine feeds including
BTC, ETH and SOL (not XMR or ZEC), signed by all five production signers every
10-second round like everything else. This is real, not configurable hope: a
captured `___FAST___` round reconstructs byte-for-byte under this decoder's view
of the signed span and every signature recovers to the published signer
(the product's `verifier-core/tests/multipoint_fast.rs`, four passing
conformance tests against live production signatures). The package set
is defined by a `multiPointPackages` map in RedStone's node manifest; adding
XMR/ZEC (or a dedicated package) is a one-line change only they can make.

The arithmetic consequence: BTC, ETH and SOL can share five recoveries instead
of costing five each. The round becomes one `___FAST___` update settling three
feeds plus two single-point updates (XMR, ZEC), **if** the aggregator learns to
settle several feeds from one submission — one plan, three applies (the two
extra applies, ~6k each, are excluded below as noise). Measured by this probe
(`--multifeed`), 9-point spans against the standard 77-byte spans, five signers:

| configuration | single-point update | `___FAST___` update | round, 5 singles | round, FAST+XMR+ZEC |
| --- | ---: | ---: | ---: | ---: |
| k256, software keccak | 2,933,979 | 3,290,625 | 14,669,895 — 147% | 9,158,583 — **91.6%** |
| k256, keccak coprocessor | 2,859,004 | 2,907,780 | 14,295,020 — 143% | 8,625,788 — **86.3%** |
| risc0-crypto-evm, software keccak | 792,176 | 1,156,852 | 3,960,880 — 39.6% | 2,741,204 — **27.4%** |

And measured end to end through the **product guest** — the aggregator with
every delivered optimization (first-party recovery backend, §4.1, coprocessor
keccak, relayer trimmed to the threshold), submitting the captured `___FAST___` payload
itself (`a_multipoint_fast_submission_measures` in `methods/tests/phases.rs`,
a harness that arrives with the push aggregator and is not in this tree --
see `README.md`):

| submission | plan | applies | total |
| --- | ---: | --- | ---: |
| via `___FAST___` (9 points), update, 5 signers | 665,895 | 15,652 + 12,072 | 693,619 |
| via `___FAST___` (9 points), update, 3 signers | 413,779 | 15,652 + 12,072 | **441,503** |
| single-point, update, 5 signers | 614,597 | 15,652 + 12,072 | 642,321 |
| single-point, update, 3 signers | 383,652 | 15,652 + 12,072 | 411,376 |

Nine points cost 30,127 more than one at the threshold — the span premium is
noise once keccak rides the coprocessor. One `___FAST___` submission carries
BTC, ETH and SOL, so the round at the production threshold is 441,503 (plus
roughly two more price applies, ~24k, once the aggregator settles three feeds
from one submission) + 411,376 (XMR) + 411,376 (ZEC) ≈ **1.29M gas, 12.9% of a
block** — against 155% for the same round through the `k256` path at five
signers, a factor of about twelve.

The same round on the **storage market**, measured by the same envelope
harness as §7b (`the_final_round_on_the_storage_market` in
`kanon-chain/tests/storage_gas.rs`, which arrives with the chain client and is
not in this tree): the nine-point
submission is 2,506 storage gas against the single-point 970, so the round is
2,506 + 2 × 970 = **4,446 storage gas — 0.44% of the 1,000,000 block cap,
0.89% of the 500,000 target** (a three-feed settle adds two more 20-byte-per-
signer registrations, ≈ +342 bytes, which moves neither figure past half a
percent of the cap). So the final solution's block footprint, five feeds at
the 3-of-5 threshold, is **12.9% of execution and 0.44% of storage**: both
markets stated, neither near binding, execution still the only one worth
watching.

Three observations:

1. **The longer spans are a keccak bill, and the coprocessor pays it.** Nine
   points per package cost +356,646 across five packages under software keccak
   but only +48,776 under the coprocessor — multi-point packages are close to
   free except for the recoveries they eliminate. The delivered configuration —
   accelerated recovery with accelerated keccak — keeps nearly the full
   reduction.
2. **XMR and ZEC are unchanged**, in cost and in semantics: they stay on their
   single-point packages exactly as every figure above prices them. The
   conformance suite also pins the safety direction — a `___FAST___` payload
   reaches no threshold for XMR, so a package cannot vouch for a feed it never
   priced.
3. **This composes with, rather than competes against, everything else.** At
   the production threshold of three (payloads trimmed to the threshold) the
   recovery terms scale by 3/5. And under proof composition the same 15 → 9
   recovery reduction shrinks the *inner proof* the relayer must produce,
   cutting proving latency even when the on-chain side no longer executes
   recovery at all.

The dependency to flag: the composition of `___FAST___` is RedStone's, chosen
for other clients, and can change under us — a relayer using it must verify the
needed points are present and fall back to single-point packages. The durable
form of this win remains the one-line manifest ask (XMR and ZEC into a
package), which the live `___FAST___` demonstrates is their routine practice.

**Not measured: the block rate.** Shipped sequencer configs set
`block_create_timeout` to `10s` (`15s` in the debug profile), which is an upper
bound on how long a sequencer waits rather than a fixed interval. The live rate
could not be measured: the public testnet returned `502` throughout the attempt.
It matters only for the ceiling — updates per unit time is roughly five per block
times the block rate, and the RFP's cadence is orders of magnitude below any
plausible value of either term.

---

## 7. Proving: the private path

Three signers, one sample each, borsh over `receipt.inner` as the v0.2 probe
measured it — so these are comparable within this harness across the two
generations.

| | v0.2 proof bytes | v0.3 proof bytes | v0.3 proving time |
| --- | ---: | ---: | ---: |
| mixed (software keccak) | 564,637 | 563,287 | 141.18 s |
| accelerated (keccak coprocessor) | 787,749 | 786,399 | 174.23 s |
| **coprocessor penalty** | **223,112** | **223,112** | **+33.05 s (+23%)** |

The framework floor proves in 4.4 s for a 221,322-byte proof in both
configurations.

**The penalty is identical to the byte.** Both proofs shrank by exactly 1,350
bytes, so the keccak coprocessor costs the same 223,112 bytes on v0.3 as on v0.2,
and now costs 33 s rather than M0's 29.30 s. M0's reasoning — software keccak is
the configuration that is never badly wrong, because one ELF serves both execution
modes — survives intact.

### But v0.3 prices proof bytes, and prices them flat

v0.3 has a second market: storage gas, `TARGET_GAS_STOR` 500,000 and
`MAX_GAS_STOR` 1,000,000 bytes per block. A private transaction is charged
`PRIVATE_GAS_STOR = PROOF_BYTES + PRIVATE_PAD_BYTES` = 223,551 + 512 = 224,063.

Two things follow, and the second is the one to raise:

- **A private transaction is 22% of a block's entire storage budget**, and 45% of
  the target above which the storage base fee starts climbing. That is a real
  constraint on how much private traffic a block holds, and M0 had no way to see
  it.
- **That 224,063 is a flat constant, not a measurement of the proof actually
  carried.** It is marked `FIXME: Provisional: re-pin with the LEZ wire-format
  numbers`. So today a 786,399-byte proof and a 563,287-byte one are charged the
  same storage gas, and the keccak coprocessor's 223,112-byte penalty does not
  reach the fee at all. If that constant is ever replaced by the real proof size —
  which the FIXME says is intended — the penalty becomes a doubling of a private
  transaction's storage bill, and the software-keccak decision goes from "never
  badly wrong" to "measurably cheaper".

### The penalty does not survive recursion — measured, and it corrects M0

Those are **composite** receipts: one segment proof per execution segment, so
size grows with the program. LEZ's private path puts none on the wire. It proves
the program composite in stage 1, then proves the privacy circuit over the result
with `ProverOpts::succinct()` (`privacy_preserving_transaction/circuit/mod.rs`
lines 348–350 and 378, verified on v0.3). A succinct receipt is constant-size by
construction.

Proved succinct, three signers:

| | succinct bytes | time |
| --- | ---: | ---: |
| mixed (software keccak) | 223,789 | 162.39 s |
| accelerated (keccak coprocessor) | 223,789 | 194.01 s |
| **coprocessor penalty** | **0** | **+31.62 s, +19%** |

**Identical to the byte.** The 223,112-byte penalty is an artefact of the
uncompressed receipt and costs nothing a private transaction would be charged
for.

Corroboration: 223,789 is within 238 bytes of LEZ's own provisional
`PROOF_BYTES` of 223,551, which suggests that constant was measured from a real
succinct receipt rather than estimated.

### Nor does the memory cliff reproduce

M0's other reason for software keccak was hardware: it proved on a 6 GiB GPU
where the accelerated build exhausted VRAM. Re-measured on that same card
(GTX 1660 SUPER, 6144 MiB), both modes, sampling system and GPU memory every
200 ms via `measure-proving-memory.sh`:

| mode | configuration | time | peak RSS | peak VRAM over idle |
| --- | --- | ---: | ---: | ---: |
| composite | mixed | 147.25 s | 9.16 GiB | 1,286 MiB |
| composite | accelerated | 168.32 s | 9.18 GiB | **1,228 MiB** |
| succinct | mixed | 160.94 s | 9.18 GiB | 1,212 MiB |
| succinct | accelerated | 194.02 s | 9.16 GiB | 1,574 MiB |

**Neither configuration approaches the limit in either mode**, and in composite —
the mode M0 measured — the accelerated build uses marginally *less* VRAM. System
RAM is identical within noise across all four. Proof sizes reproduced to the byte,
which is a second independent confirmation of the pinned constants.

### What is left of M0 §5.1

| M0's reason for software keccak | status |
| --- | --- |
| saves 223 KB of proof | **does not hold** — vanishes under recursion |
| avoids exhausting a 6 GiB GPU | **does not reproduce** — in either mode |
| saves proving time | **holds** — +21% succinct, +14% composite |

One of three. The surviving cost is ~33 s per private settlement, borne by the
consumer that proves, against 44,221 gas saved on every public update — 2.5% of a
submission, a thirteenth of what one extra signer costs, and no change to block
occupancy (five transactions either way).

Not reversed here, because nothing operational turns on 2.5%. But it should be
**re-decided on its own merits rather than inherited**, and reversing it is a
two-line `[patch.crates-io]` change, so the option stays cheap.

Caveat: one machine, one GPU, 3 signers, and a desktop already holding 2.2 GiB of
VRAM that a headless prover would not.

Pinned by `the_coprocessor_penalty_does_not_survive_recursion`, which also fails
if the coprocessor ever proves *faster* than software keccak — the one outcome
that would reopen the decision.

---

## 7b. Storage gas on the public path — measured, and it never binds

v0.3 bills a transaction on two markets. §7 covers the private side. The public
side is a property of the **signed envelope** rather than of the guest —
`classify` prices a charged public transaction at `borsh::to_vec(tx)?.len()`
(`lez/chain_state/src/classify.rs`) — so it cannot be measured in the probe at
all. It is measured in `kanon-chain/tests/storage_gas.rs` -- not in this tree --
where the envelope is built, over the same payloads and rosters the cycle
figures use.

`submit::encode` writes `[0u8]` then borsh of the `PublicTransaction`, and
`Public` is the first variant of `LeeTransaction`, so the base64-decoded wire
value **is** the byte string `classify` measures. `the_wire_bytes_are_what_the_chain_prices`
pins that identity rather than assuming it, which matters because `common` — the
crate that owns `LeeTransaction` — is one `deny.toml` keeps out of everything
delivered.

| signers | as captured | padded to `MAX_PAYLOAD_BYTES` | share of a block's storage |
| ---: | ---: | ---: | ---: |
| 1 | 646 | 33,258 | 3.33% |
| 3 | **970** | 33,298 | 3.33% |
| 5 | 1,294 | 33,338 | 3.33% |
| 12 | 2,428 | 33,478 | 3.35% |

A signer costs a flat **162 bytes**: one 20-byte address in the registration the
submission carries, one signed package in the payload.

**Execution binds 182x sooner than storage** for a 3-signer submission — 17.67%
of the per-block execution budget against 0.10% of the storage one — and still
~22x sooner at the worst case the design permits (12 signers, payload padded to
32 KiB: 73.2% against 3.35%). Storage is **0.05% of a submission's own fee**, and
about 340 million base units a year against §4.1's 619 billion.

So the §6 simultaneity finding is an execution finding, and the second market
neither softens nor complicates it. No conclusion in this document rests on a
storage figure.

## 8. What was NOT re-measured

- **Proving at signer counts other than three**, and more than one sample each.
- **What a fee costs in LGO.** Gas × base fee gives base units (3 signers at the
  floor base fee of 8 is 14,135,496 units per update, 619 billion a year at
  43,800 updates). Converting that to LGO needs the native token's denomination,
  which is not stated in LEZ, the wallet, or our repo. Worth asking.
- **`m0/cost-baseline/`'s proving figures**, unchanged by construction.

---

## 9. Recommendation

A one-to-two page addendum to M0, not a reissued report. It should say:

1. M0's figures reproduce exactly and its four recommendations all stand.
2. Here is the same decomposition at v0.3, and the cryptography is identical to
   within ten cycles.
3. The denominator changed: 5.68% → 17.67% at three signers.
4. Withdraw the precompile ask: `risc0-crypto-evm` already ships it, 3.67x
   cheaper measured — and it has since been superseded by a first-party
   GLV + wNAF + fixed-base backend at 109,502 cycles per recovery (§4.1),
   integrated and equivalence-tested in the product. **This, with item 7, is
   the headline.**
5. The "53 signers" line is withdrawn; the real ceiling is ~17 and the product
   ships 12.
6. Flag the per-block cap and the simultaneity bound it implies, which M0 could not have asked about.
7. RedStone's live `___FAST___` multi-point package carries BTC/ETH/SOL under
   one signature per signer (§6.1), and the product guest settles BTC from it
   for 441,503 gas at the threshold — 30k over a single-point submission. The
   round lands at ~12.9% of a block (was 155% through the `k256` path), with XMR
   and ZEC unchanged on single-point packages; the remaining work is the
   aggregator's three-feeds-from-one-submission settle, and the one-line
   manifest ask (XMR/ZEC into a package) is demonstrably RedStone's routine
   practice.

The port is done and the figures are measured, so the addendum is writing, not
measurement. The one open input is whether to also re-pin the probe's guardrails at
the v0.3 figures — which would mean `m0/` carrying two LEZ revisions, and that is a
decision rather than a detail.
