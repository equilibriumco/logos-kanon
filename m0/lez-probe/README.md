# Kanon: LEZ framework cost probe

The cost baseline in `m0/cost-baseline/` established what RedStone verification costs
on bare RISC Zero. This probe answers what remains: what the LEZ program framework
adds around that verification, and how the total sits against LEZ's
per-transaction budget.

It is deliberately **independent of `m0/cost-baseline/`** in what it measures. The two
share the root workspace, the risc0 pin and the host-side signing crates, so that
`cargo build --workspace` covers the repository and so that the two sets of figures
are directly comparable. What is not shared is the measured code:
the crypto is duplicated in `methods/guest/src/bin/lez_verify.rs` rather than
imported, so each project's guest ELF stands alone. M1 resolves that duplication by
promoting a single `verifier-core` that both consume.

Each guest configuration keeps its own workspace and its own lockfile, which is the
mechanism that distinguishes the accelerated and mixed builds; only host crates are
unified.

## Setup

Same toolchain as `m0/cost-baseline/`: a host Rust and the rzup-managed guest
toolchain. See that README for install steps, including the NixOS
`programs.nix-ld.enable` prerequisite.

`lee_core` comes from a **git dependency pinned by revision** rather than a
submodule or a path. The repo is public, so https needs no credentials, and
Cargo.lock records the exact commit, which is what makes these numbers
reproducible.

When a standalone sequencer is needed, it will arrive by a different route: the `lgs`
toolchain maintains its own LEZ checkout and builds `sequencer_service` from it, at a
pin of its own. So there will be two independent LEZ pins in play, this git revision
and the toolchain's, and they are currently different versions. That has to be
reconciled before any test transacts against a sequencer; see *The two LEZ versions*
in `m0/versions.md`.

From the repository root:

```sh
nix-shell   # NixOS only

# cycle counts, both configurations. Executes only, takes seconds.
cargo run --release -p lez-probe

# vary the signer counts
cargo run --release -p lez-probe -- --signers 1,3,5,10,20

# also prove, as LEZ's private path does. Minutes per proof.
cargo run --release -p lez-probe -- --prove --repeat 2

# restrict to one configuration: mixed | accelerated
cargo run --release -p lez-probe -- --only mixed
```

The guardrail tests are the fastest way to confirm the environment reproduces the
published figures:

```sh
cargo test --release -p lez-probe               # under a second
cargo test --release -p lez-probe -- --ignored  # proving, minutes
```

## Two execution modes, two different costs

**Public execution is capped in cycles and not proven.** LEZ has no separate
compute-unit currency:

```rust
// lee/state_machine/src/program/mod.rs
const MAX_NUM_CYCLES_PUBLIC_EXECUTION: u64 = 1024 * 1024 * 32; // 32M cycles
env_builder.session_limit(Some(MAX_NUM_CYCLES_PUBLIC_EXECUTION));
// Execute the program (without proving)
```

So cycles are directly comparable to the budget of **33,554,432**, and to the
baseline's figures, with no conversion and no calibration.

**Private execution is proven, and uncapped.** The privacy-preserving path proves
in two stages:

```rust
// lee/state_machine/src/privacy_preserving_transaction/circuit/mod.rs
fn execute_and_prove_program(..) {
    Program::write_inputs(..)          // same protocol as public execution
    prover.prove(env, program.elf())   // stage 1: no session_limit is set
}
// ..then stage 2 over the collected outputs:
prover.prove_with_opts(env, PRIVACY_PRESERVING_CIRCUIT_ELF, &ProverOpts::succinct())
```

Note stage one sets **no session limit**, so the 32M cap is public-only. In private
mode cycles are not the binding constraint; proving cost is.

| mode    | proven | cycle cap | binding cost |
| ------- | ------ | --------- | ------------ |
| public  | no     | 32M       | cycles       |
| private | yes    | none      | proving      |

Because stage one uses the same input protocol, `--prove` here measures exactly
what a private-mode consumer pays for *this* program. Stage two is LEZ's own fixed
circuit, essentially constant with respect to the adaptor, and driving it would
need account identities, commitments, nullifiers and key material, so it is out of
scope.

## Method

`Program::execute` upstream is `pub(crate)` and discards cycle counts, so the host
reproduces its input-writing order instead: program id, caller program id,
pre-states, instruction words, under the same 32M session limit. **That order must
stay in step with `Program::write_inputs` upstream**; if it drifts, the guest will
fail to deserialize rather than silently mismeasure.

Two programs, differing only in whether they verify:

- `lez_noop`: reads LEE inputs and echoes the account as its post state. Its cycle
  count is the floor a LEZ program pays before any application logic: account
  deserialization, instruction decode, and serialization of the proposed state
  diff.
- `lez_verify`: the same, plus one keccak256 and one secp256k1 recovery per data
  package, with the checksum written into account data so it cannot be optimized
  away. Recovery success is asserted, so a run that took the cheap error path
  cannot be misread as a measurement.

Both are built in two guest configurations, with byte-identical source, differing
only in their `[patch.crates-io]` section:

- `methods-mixed/guest`: secp256k1 accelerated, keccak256 in software. The
  configuration the baseline recommends.
- `methods-accelerated/guest`: both accelerators, including the keccak256
  coprocessor.

Carrying both matters here rather than only in the baseline, because private
execution is the proven path, so a LEZ program is where the keccak coprocessor's
cost is actually paid.

## Results

| measurement                    | cycles    | share of the 32M budget |
| ------------------------------ | --------- | ----------------------- |
| LEZ framework floor            | 39,103    | 0.12%                   |
| 1 signer, total                | 664,117   | 1.98%                   |
| **3 signers (RFP default)**    | 1,906,737 | **5.68%**               |
| 5 signers, total               | 3,150,337 | 9.39%                   |

Verification alone, with the framework floor subtracted: 625,014 at 1 signer,
1,867,634 at 3, 3,111,234 at 5. That is **about 621,555 cycles per additional
signer**, linear, consistent with the baseline's finding that nothing is shared
between signers.

### Private mode: proving the same program

`--prove` mirrors stage one of the private path, with no session limit, as upstream
does:

| measurement         | cycles    | prove time | proof size |
| ------------------- | --------- | ---------- | ---------- |
| LEZ framework floor | 39,103    | 8.57 s     | 244,430 B  |
| 3 signers           | 1,906,737 | 137.45 s   | 564,637 B  |

Figures are the fastest of 2 proving runs, spread 1.4% to 2.9%.

### Reproducibility across machines

Built from scratch and run on three machines across two architectures. Every cycle
count matched **byte for byte**, and the guardrail tests passed on each:

| 3-of-N, mixed  | NixOS x86_64 | Ubuntu 24.04 x86_64 | macOS ARM64 |
| -------------- | ------------ | ------------------- | ----------- |
| framework floor | 39,103      | 39,103              | 39,103      |
| 1 signer        | 664,117     | 664,117             | 664,117     |
| 3 signers       | 1,906,737   | 1,906,737           | 1,906,737   |
| 5 signers       | 3,150,337   | 3,150,337           | 3,150,337   |

That also exercises the dependency arrangement: each host resolved `lee_core` from
the pinned https git revision with no credentials and no local checkout, which is
the portability the *Setup* section claims.

### The two accelerators, compared inside a LEZ program

Both guest configurations, 3-of-N, best of 2:

| configuration | cycles    | share of budget | prove time | proof size |
| ------------- | --------- | --------------- | ---------- | ---------- |
| mixed         | 1,906,737 | 5.68%           | **137.45 s** | **564,637 B** |
| accelerated   | 1,862,527 | **5.55%**       | 166.75 s   | 787,749 B  |

The keccak coprocessor is worth **44,210 fewer cycles** and costs **29.30 s more
proving (+21.3%) and 223,112 more proof bytes (+39.5%)**. That reproduces the
baseline's bare-RISC-Zero finding almost exactly, which measured 44,212 fewer
cycles, +26.90 s, and +223,133 more bytes: the cycle delta agrees to 2 cycles and
the proof-size delta to 21 bytes. The trade is a property of the coprocessor, not
of the surrounding context.

Which side wins therefore depends only on execution mode, and both are visible in
this one table: in cycles, which is what public execution is charged, accelerated
is 0.13 percentage points better; in proving, which is what private execution
pays, it is 21% and 40% worse.

Set against the baseline's bare-RISC-Zero figures for the same configuration
(1,813,434 cycles, 137.67 s, 562,764 B), **the LEZ framework is close to free on
every axis**: proving time is identical within noise, and the proof grows by 1,873
bytes, 0.3%. Both sides are on risc0-zkvm 3.0.6, so the cycle difference of 93,303
is not confounded by a version gap; it does span a slightly different guest, so it
is an upper bound on the framework's share rather than a clean attribution.

The floor is worth noting on its own: any LEZ program costs at least 8.81 s and
244 KB to prove, against 2.23 s and 209 KB for an empty risc0 guest. Verification
is still the overwhelming majority, roughly 127 s of the 136 s.

### What this settles

**The framework is nearly free.** 39,103 cycles is 0.12% of the budget, so LEZ's
account handling and state-diff serialization are not a cost factor. Verification
is 98% of the program.

**A 3-of-N update fits comfortably in one transaction**, using 5.68% of the budget
and leaving 94.3% free. This resolves the open risk the baseline flagged: the
single-transaction verify-and-publish requirement (Performance 1) is not in
danger, and a secp256k1 precompile stays an optimisation rather than becoming a
prerequisite.

**The threshold could go far wider than needed.** At 621,555 cycles per signer,
the budget accommodates roughly **53 signers** in one transaction. The linear cost
the baseline documented is a real constraint on price, not on feasibility.

**The software configuration remains infeasible.** At 33,347,017 cycles on bare
RISC Zero it is 99.4% of the budget before the framework's 39,103 and before
payload decode, median, or the canonical account write. It does not fit. That is a
harder argument for the secp256k1 accelerator than the proving-time figures were.

## In-program versus precompile: first delta sketch

RFP-020 asks M0 for a first sketch of the in-program-versus-precompile delta; the
full per-mode report is M3. The measured part is what a native secp256k1 recovery
precompile would remove; the estimated part is what would remain.

Decomposing the 3-of-N LEZ program in the recommended configuration:

| component                       | cycles    | share   |
| ------------------------------- | --------- | ------- |
| LEZ framework floor             | 39,103    | 2.05%   |
| 3 x keccak256 over 77 B, software | 51,956  | 2.72%   |
| 3 x recovery and signature parse | 1,815,678 | **95.22%** |
| **total**                       | 1,906,737 | 100%    |

**A recovery precompile addresses 95.2% of the program.** At roughly 605,226 cycles
per recovery, that share is measured, not modelled.

What would remain is the framework, the hashing, and whatever guest-side
marshalling a precompile syscall costs. Taking that residual at 1,000 to 10,000
cycles per call, which spans the range an accelerator-style syscall plausibly
occupies:

| assumption            | program total | share of budget | reduction |
| --------------------- | ------------- | --------------- | --------- |
| 1,000 cycles per call | 94,059        | 0.28%           | **20.3x** |
| 10,000 cycles per call | 121,059      | 0.36%           | 15.8x     |

So the sketch is a **15x to 20x reduction**, taking budget use from 5.68% to under
0.4%. The residual is an assumption: no such precompile exists to measure, and the
figure should be replaced with a measurement if one is built.

### Why the ratio is so different from EVM

RFP-020 cites RedStone's EVM path at 50K to 100K gas end to end, where a native
`ecrecover` is about 3,000 gas. Three recoveries are therefore roughly 9,000 gas,
some 9% to 18% of the EVM total. In LEZ without a native primitive the same three
recoveries are **95%** of the program.

That is the sketch's substantive point: the missing primitive does not make
verification somewhat dearer, it moves recovery from a minor line item to
essentially the entire cost. It is also why a keccak256 precompile is not worth
pairing with it, at 2.72% of the program.

### What the delta is, and is not, worth

**It is not needed for feasibility.** At 5.68% of the budget with roughly 94% free,
a 3-of-N update already fits comfortably in one public transaction. Nothing in the
scope depends on the precompile existing.

**Its value is economic, and conditional on the fee model.** The 32M cap carries
`TODO: Make this variable when fees are implemented` upstream, so pricing is
undecided. If fees end up proportional to cycles, a 15x to 20x cycle reduction is a
15x to 20x reduction in the per-update fee, and that compounds over an operating
period: five feeds on a one-hour heartbeat is about **43,800 updates a year**,
which is 83.5 billion cycles annually as measured against roughly 4.1 billion with
a precompile.

So the recommendation is to propose the precompile on cost-of-operation grounds
rather than feasibility grounds, and to treat its priority as contingent on how LEZ
prices execution.

## Recommendations

RFP-020's M0 done gate asks for four things to be agreed. Each is answered below
from the measurements above and in `m0/cost-baseline/`.

### 1. In-program path: accelerate secp256k1, leave keccak256 in software

The secp256k1 accelerator is not optional: without it a 3-of-N update is
33,347,017 cycles, **99.4% of the budget** before the framework, decode, median or
account write. It does not fit. With it the update is 5.68%.

The keccak256 coprocessor is the closer call, and the table above shows why it goes
the other way. It buys 44,210 cycles, worth 0.13 percentage points of a budget with
94% headroom, and costs 29.30 s of proving and 223,112 proof bytes. Public
execution pays only the cycles, so there it is marginally ahead; private execution
pays the proving, so there it is 21% and 40% behind. Since LEZ runs the **same
program** in either mode, and a consumer holding private accounts has this code
proven, software keccak is the configuration that is never badly wrong. It also
proved on a 6 GiB GPU where the accelerated one exhausted VRAM.

Selecting per mode, accelerated for public and software for private, is not
available. A LEZ program's identity **is** its ELF hash: `Program::new` derives
`ProgramId` from `compute_image_id()`, and accounts carry `program_owner:
ProgramId`. Two binaries would therefore be two programs with two IDs, so the
canonical RFP-019 price account could belong to only one of them, giving two feeds
rather than one feed with two flavours.

Runtime dispatch inside a single ELF would be technically possible, since a
coprocessor proof is only needed if the accelerated path actually runs. But a
program cannot detect its own execution mode, because public execution and the
private path's stage one run the identical ELF, so it would have to branch on a
caller-supplied hint. That buys 44,210 cycles at the price of two cryptographic
code paths to audit and cost behaviour driven by untrusted input, against RFP-020's
own principle that verification lives in one place with one audited implementation.
The asymmetry decides it: software keccak costs 0.13 percentage points in public
mode and saves 29.30 s plus 223 KB in private mode, so one choice serves both.

### 2. Heartbeat width: keep the RFP defaults, revisit when fees land

Execution cost does not constrain the cadence. Each update is a fixed ≈1.9M cycles
regardless of price volatility, and effectively independent of message size at
RedStone's 77 bytes, so a tighter heartbeat costs linearly more rather than
risking the budget. At the proposed 1-hour heartbeat across five feeds that is
about **43,800 updates a year**; halving the interval doubles it.

So the 0.5% deviation and 1-hour heartbeat in the proposal should stand. The
binding input is the fee model, which LEZ has not published: the cap carries
`TODO: Make this variable when fees are implemented`. Cadence should be revisited
then, not now, and the monthly-report mechanism the proposal already commits to is
the right place to propose a widened band for any feed that proves uneconomic.

### 3. Push versus pull: default to push, and price pull rather than discourage it

Both modes run the same verifier over the same payload, so a pull read costs the
same ≈1.9M cycles as a push update. Two consequences, and the second is a
correction to how the baseline framed this.

Push should be the recommended integration: it pays once per update and every
reader shares the result, whereas pull makes each consumer pay in full on every
read, and also carry 440 payload bytes each time. The multi-feed batching soft
requirement amortizes across feeds within one update, which benefits push only.

But pull is **comfortably affordable**, not prohibitive. A pull consumer spends
5.68% of its own transaction budget on verification and keeps roughly 94% for its
own logic. The argument for push is fee amortization, not budget pressure, and the
documentation should put the per-read cost in front of the decision rather than
steer consumers away from a mode the RFP requires.

### 4. Precompile follow-on: recommend it, for recovery only, on cost grounds

Recommend a **secp256k1 recovery precompile**, scoped to recovery alone. It
addresses 95.2% of the program, for a 15x to 20x reduction; keccak256 is 2.72% and
should not be paired into the request. See *In-program versus precompile* above.

Recommend it on **cost of operation, not feasibility**. Nothing in the scope needs
it: verify-and-publish already fits one transaction with 94% of the budget unused,
so its priority should be treated as contingent on whether LEZ ends up pricing
execution per cycle.

### Still outstanding for M0

- **Done: the configuration is now machine-checked, in CI.** See *Guardrails* below.
  The companion suite in `m0/cost-baseline/host/tests/guardrails.rs` pins the
  bare-RISC-Zero figures the same way, and `.github/workflows/guardrails.yml` runs
  both on x86_64 and, for pushes to `main`, on ARM64.
- **Done: every version requirement is an exact pin.** A caret range would let the
  software arm drift to a different patch release than the fork tags the other arms
  use, which is worth about 0.01% on the recovery figures and confounds the
  comparison. Recorded in `m0/versions.md`, which is also where M0-01's pin list lives.
- **Deferred to the toolchain task in M1: the standalone sequencer and `lgs`.** `lgs`
  is the standalone sequencer path, not an alternative to it, and it would not distort
  a measurement. It is simply not on the path from a guest ELF to a cycle count, so no
  figure here depends on one. A spike stood it up to confirm that and to settle the
  version question below; the findings are in `m0/versions.md` and the setup is not
  carried. What genuinely needs a live sequencer is M1-05's integration tests and
  M2-19's end-to-end verify-and-publish.
- **Open, and the most consequential item: which LEZ.** These figures are built
  against `lee_core` at v0.2.1 (`15144ddb`). The `lgs` toolchain pins v0.1.2
  (`cf3639d8`), 784 commits and three months earlier, and upstream has since tagged up
  to v0.2.4. The cost figures are unaffected, since they are a property of the guest
  ELF and no sequencer takes part in producing them. What is affected is M1-05, which
  cannot be written until the two agree.

  Pointing `lgs` at v0.2.1 gets partway: the sequencer **compiles** (so this is not a
  deep API break) but `test-node` **will not start** it, because scaffold generates a
  v0.1.2-shaped
  `sequencer_config.json` and v0.2.1 wants a numeric `genesis_id`. SPEL also stays
  pinned to v0.1.2 regardless. Options and evidence are in *The two LEZ versions* in
  `m0/versions.md`. This is a decision for M0-13.

## Guardrails

RFP-020 asks for cost numbers "reproducible from a benchmark target". These are
**tests, not a `cargo bench` target**, and that was a deliberate choice worth
recording.

Cycle counts are deterministic and execute in under a second, which suits CI
exactly. Proving takes minutes per sample, which criterion-style benchmarking, with
its many iterations and statistical machinery, handles badly. More to the point,
what the figures actually need is not a timing harness but an assertion that fails
a build when they move. A `cargo bench` target would have satisfied the wording
while checking nothing.

```sh
cargo test --release -p lez-probe               # fast, under a second
cargo test --release -p lez-probe -- --ignored  # proving, minutes
```

The fast tests run in CI on both architectures, which is what makes them a
regression gate; the proving tests are excluded there because they are neither fast
nor hardware independent. See `.github/workflows/guardrails.yml`.

Five fast assertions plus one slow ignored one, in
`m0/lez-probe/host/tests/guardrails.rs`:

| guardrail | catches |
| --------- | ------- |
| framework floor is unchanged | drift in LEZ's own input/output handling |
| mixed cycles unchanged at 1, 3, 5 signers | either accelerator changing, in either direction |
| accelerated cycles unchanged at 3 signers | drift in the comparison arm |
| 3-of-N leaves ample budget headroom | losing secp256k1, semantically rather than exactly |
| per-signer cost is linear | the workload no longer being the one measured |
| proof sizes distinguish the configurations (ignored) | gaining the keccak coprocessor, which *lowers* cycles |

Two design points. **Exact equality is intentional**: cycle counts are a
deterministic function of the guest ELF and its input, reproduced byte-identically
across x86_64 and ARM64, so pinning them exactly means any toolchain bump or
dependency change fails here and forces the published figures to be re-tagged
deliberately. A failure is a prompt to re-measure, not necessarily a defect.

**Two dimensions are needed because the accelerators fail in opposite directions.**
Losing secp256k1 raises cycles roughly 20x. Gaining keccak256 *lowers* cycles by
about 44,000 while adding 223 KB of proof. A cycle assertion catches the first and,
by exact equality, the second; the proof-size assertion is what makes the second
unmistakable in the dimension where it actually hurts.

## Caveats

- **risc0 version: this project and the baseline are on the same one.** LEZ's own
  workspace pins risc0-zkvm **3.0.5**, which is a floor rather than a ceiling:
  `lee_core` accepts a newer patch release, so the guest ELFs here are built on
  **3.0.6**, the same version the baseline uses, and the two sets of figures are
  directly comparable. Both sides are `=3.0.6` pins, so the resolved version and the
  documented version cannot part company.

  Worth being precise about what a crate pin fixes and what it does not: the crate
  version fixes the guest ELF, while the executor is a separate `r0vm` binary,
  resolved by rzup to the version matching the crate, or falling back to whatever
  `r0vm` is on `PATH`. Cycle counts were measured under **both r0vm 3.0.5 and 3.0.6
  and came out identical**, so these figures are insensitive to the executor's patch
  version.
- **The precompile residual is assumed, not measured.** The 95.2% a precompile
  would address is measured; what remains after it is an estimate spanning 1,000 to
  10,000 cycles per call, because no such precompile exists to measure.
- **Not the full transaction.** This measures program execution only. Payload
  decode, the median across signers, the canonical RFP-019 account write, and
  admin checks are M1 and M2 work and add to the 5.68%.
- **Stage two of private execution is not measured.** The wrapper circuit's cost
  is additional to the figures above. It is LEZ's own and roughly fixed with
  respect to the adaptor, but a full private-mode transaction cost would include
  it.
- **Upstream is early, and moving.** Pinned at `15144ddb` (2026-08-02), which was
  the tip of `main` at the time and carries the tag **v0.2.1**; the repo's default
  branch is `dev`, though `main` was 9 commits ahead of it when pinned. Upstream has
  since tagged v0.2.2, v0.2.3 and v0.2.4, so this pin is already behind, and the
  `lgs` toolchain pins further back still, at v0.1.2. The 32M constant is marked
  `TODO: Make this variable when fees are implemented` upstream, so it should be
  expected to change.
