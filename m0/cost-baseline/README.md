# Kanon M0: in-zkVM cost baseline

RFP-020 makes the in-zkVM cost of RedStone signature verification the primary
deliverable. This is the M0 baseline that establishes it.

The measurement deliberately has **no dependency on LEZ, SPEL, a sequencer, or a
live RedStone feed** (`m0/lez-probe/`, its sibling, is where LEZ enters; the two share
the root workspace but not a dependency graph). It measures the two cryptographic
primitives on bare RISC Zero,
because those are what the cost question actually turns on and because anything
else in the way would only add noise and schedule risk.

The headline result: a 3-of-N update costs **1,813,434 cycles** in the recommended
configuration, which is **5.4% of LEZ's 32M per-transaction budget**. Recovery is
97% of that and effectively all of the cost. Of RISC Zero's two relevant
accelerators, the secp256k1 one is essential; the keccak256 one is declined, for
reasons that depend on execution mode and are set out in *Recommendations*.

Cycles are the figure that matters because LEZ caps public execution in cycles and
does not prove it. Proving time and proof size are also reported (137.67 s and
562,764 bytes for that update) because they are what distinguishes the two
accelerators, and because the same program is proven when a consumer uses private
accounts.

All paths below are relative to the repository root; `m0/cost-baseline/` is this
directory.

## Setup

Two toolchains are involved and they are not the same thing:

- The **host** toolchain builds the benchmark driver and the prover. Any recent
  stable Rust.
- The **guest** toolchain cross-compiles to `riscv32im-risc0-zkvm-elf`. This is
  RISC Zero's own Rust fork, managed by `rzup`, installed under `~/.risc0`. Its
  version is pinned by RISC Zero rather than selected here.

### NixOS

`shell.nix` at the repository root provides the host toolchain and puts
`~/.cargo/bin` on `PATH`.

NixOS needs one system-level prerequisite, because `rzup` distributes prebuilt
dynamically-linked binaries that will not otherwise run:

```nix
# /etc/nixos/configuration.nix
programs.nix-ld.enable = true;
```

Then, from the repository root:

```sh
nix-shell                                    # host toolchain
cargo install rzup                           # once, if rzup is absent
rzup install                                 # guest toolchain, cargo-risczero, r0vm
cargo run --release -p kanon-cost-baseline
```

### Non-NixOS (Linux, macOS)

Install the host toolchain via rustup, then the RISC Zero toolchain via rzup:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # host Rust
curl -L https://risczero.com/install | bash                      # installs rzup
rzup install                                 # guest toolchain, cargo-risczero, r0vm
```

`rzup` places `r0vm` and `cargo-risczero` in `~/.cargo/bin`, which must be on
`PATH`. Then, from the repository root:

```sh
cargo run --release -p kanon-cost-baseline
```

Build prerequisites are the usual ones for Rust crypto crates: a C toolchain,
`pkg-config`, and OpenSSL headers (`libssl-dev` on Debian/Ubuntu,
`openssl-devel` on Fedora, `brew install openssl pkg-config` on macOS). The
`shell.nix` `buildInputs` list is the authoritative inventory of what is needed.

## Running it

From the repository root:

```sh
# cycle counts, all three configurations -- executes only, takes seconds
cargo run --release -p kanon-cost-baseline

# real proofs, for proving time and proof size -- takes minutes to hours
cargo run --release -p kanon-cost-baseline -- --prove

# restrict to one configuration: software | accelerated | mixed
cargo run --release -p kanon-cost-baseline -- --only mixed

# restrict to one workload: noop | keccak | recover | verify
cargo run --release -p kanon-cost-baseline -- --workload verify

# prove several times and report the fastest plus the spread; proving wall-clock
# is noisy enough that a single run can order two configurations wrongly
cargo run --release -p kanon-cost-baseline -- \
  --workload verify --prove --repeat 3
```

The two filters compose, which is how the headline comparison is reproduced
without proving the workloads that are not part of it:

```sh
cargo run --release -p kanon-cost-baseline -- \
  --workload verify --prove --only accelerated
cargo run --release -p kanon-cost-baseline -- \
  --workload verify --prove --only mixed
```

Proving the software configuration end to end is impractical on CPU (34 segments
per 3-of-N update), and unnecessary: it is ruled out on cycles alone.

GPU proving needs `--features cuda`, and on NixOS `nix-shell --arg withCuda true`.
That produces a different binary linked against `libcuda`, so switching between CPU
and GPU measurement is a rebuild rather than a flag.

On NixOS, prefix with `nix-shell --run '...'` or run inside `nix-shell`.

Every figure this file publishes is also asserted by a test, described under
*Guardrails* below:

```sh
cargo test --release -p kanon-cost-baseline               # cycles, seconds
cargo test --release -p kanon-cost-baseline -- --ignored  # proof sizes, minutes
```

## What is being measured and why

Verifying a RedStone price update on-chain means two things:

1. **keccak256** over the signed byte range of each data package, to reproduce
   the digest its signer signed.
2. **secp256k1 ECDSA public-key recovery**, once per signer, to learn who signed
   and test them against the authorized set. At RFP-020's default 3-of-N
   threshold, that is at least three recoveries per update.

On Ethereum both are native chain primitives (`ecrecover`, `KECCAK256`) at a
fixed, small gas cost. LEZ has neither. Every step of the computation runs as
RISC-V instructions in a zkVM, so these primitives cost whatever their software
implementation costs. That cost, and how much of it a native precompile would
recover, is what RFP-020 asks this baseline to establish.

The unit is the **cycle**: one RISC-V instruction step. It is the unit LEZ itself
charges in, since it caps a public execution at 32M cycles. Public executions are
not proven, so proving cost is a separate axis rather than a component of the
on-chain price, and the two can point different ways: see *Recommendations* item 7.

### The message size that matters: 77 bytes

Taken from the wire format in RedStone's own Rust SDK, which is Boost Software
License 1.0 and so licence-compatible with an MIT + Apache-2.0 deliverable,
unlike the BUSL-1.1 EVM connector. Per `crates/redstone/src/protocol/constants.rs` and
`payload_decoder.rs`, the signed range of one data package is:

```
data_point_count * (value_size + DATA_FEED_ID_BS)
  + DATA_POINT_VALUE_BYTE_SIZE_BS + TIMESTAMP_BS + DATA_POINTS_COUNT_BS
```

For a single data point with a 32-byte value: `1 * (32 + 32) + 4 + 6 + 3` =
**77 bytes**. The signature covers that range only, so each signer's digest is a
77-byte hash, *not* a hash of the whole payload. Adding the 65-byte signature
gives a **142-byte** data package on the wire, and RedStone's own sample payload
(`sample-data/payload.hex`, 2,144 bytes) is exactly 15 such packages plus a
14-byte trailer (9-byte marker, 3-byte unsigned-metadata length, 2-byte package
count), which confirms the arithmetic.

### Payload size at 3-of-N: 440 bytes

Each signer gets its own complete data package, so the 64-byte data point is
repeated per signer rather than shared under three signatures. A 3-of-N
single-feed update is therefore:

| part                                                    | bytes   |
| ------------------------------------------------------- | ------- |
| 3 data packages x 142 (77 signed + 65 signature)        | 426     |
| RedStone marker (`REDSTONE_MARKER_BS`)                  | 9       |
| unsigned-metadata length field (`UNSIGNED_METADATA_BYTE_SIZE_BS`) | 3 |
| data-packages count (`DATA_PACKAGES_COUNT_BS`)          | 2       |
| **total**                                               | **440** |

Exact, given empty unsigned-metadata content, which the sample payload confirms
is the normal case. Any unsigned metadata a deployment adds is on top.

**Payload size scales linearly with M, for the same reason recovery cost does:
nothing is shared between signers.** The verification work is three 77-byte
hashes plus three recoveries, not one hash of 440 bytes.

## Method

Three guest workspaces with **byte-identical source**, differing only in their
`[patch.crates-io]` section:

- `m0/cost-baseline/methods-software/guest`: no patches. Upstream `tiny-keccak` and
  `k256` executed as plain RISC-V. The baseline.
- `m0/cost-baseline/methods-accelerated/guest`: both of RISC Zero's accelerated
  forks, for keccak256 and for the secp256k1 field arithmetic.
- `m0/cost-baseline/methods-mixed/guest`: the secp256k1 accelerator only, with
  keccak256 left in software. The two accelerators turn out not to be the same
  kind of thing, and this variant separates them.

The patch section being the only difference is what makes the numbers comparable.
The shared workload lives in `m0/cost-baseline/bench-lib` and has no risc0
dependency, so all three workspaces compile the same code.

Each workload runs **twice on identical input**: once with the cryptographic
work enabled, once with it skipped (`do_work: false`). Subtracting the cycle
counts cancels out zkVM startup, input deserialization, and journal commit, so
what remains is the cost of the primitive alone. Dividing by the iteration count
gives a marginal per-operation cost carrying no fixed overhead.

The recovery workload asserts that every recovery actually succeeded, so a run
that silently took the cheap error path cannot be misread as a valid
measurement. Signing keys are fixed, so runs are reproducible.

Cycle counts are deterministic and need one run. Proving wall-clock is not, so
`--repeat N` proves each measurement N times and reports the **fastest** sample
with the observed spread: contention only ever adds time, so the minimum is the
cleanest estimate of the underlying cost, and the spread shows whether a claimed
difference clears the noise. The four `verify` workload figures come from the real
update shape, three data packages each hashed over its own 77 signed bytes and then
recovered, with only the hashing toggled so the difference isolates keccak256.

## Environment

### Software

| Component     | Version                                                        |
| ------------- | -------------------------------------------------------------- |
| risc0 / r0vm  | 3.0.6 (latest stable, released 2026-07-17)                     |
| host rustc    | 1.97.1 (8bab26f4f, 2026-07-14), latest stable, identical on all machines |
| guest rustc   | 1.97.0-dev (e638c6cfe, 2026-07-15), risc0's fork, pinned by rzup |
| guest target  | `riscv32im-risc0-zkvm-elf`                                     |
| CUDA (GPU run only) | 12.8, nvcc 12.8.93                                       |

The two Rust toolchains are independent and only one affects the numbers. The
**guest** toolchain is risc0's own fork, its version chosen by RISC Zero and
pinned by rzup, and it produces the guest ELF, so it determines the cycle counts.
The **host** toolchain compiles only the benchmark driver, and the prover is the
prebuilt `r0vm` binary rather than anything the host compiler builds, so the host
version cannot move a measurement. That was confirmed rather than assumed: a run
on host rustc 1.93.0 produced cycle counts byte-identical to 1.97.1. All machines
were nonetheless brought to 1.97.1 so the figures below describe one environment.

A `v5.0.0-rc.1` pre-release exists upstream but has not advanced since
2026-01-15 while the 3.0.x line continues to ship, so 3.0.6 is the right choice
today. The version that ultimately governs is whichever risc0 LEZ itself pins,
which is **3.0.5** (`logos-execution-zone` workspace at commit `15144ddb`). That is
a floor rather than a ceiling: `lee_core` accepts a newer patch release, so both
this project and `m0/lez-probe/` resolve and are pinned to **3.0.6**, and the two sets
of figures are directly comparable. Every risc0 requirement here is an `=` pin for
that reason; see `m0/versions.md`. Numbers must be re-tagged on any toolchain bump.

**Which prover produced these numbers.** The host crate does not enable risc0's
`prove` feature, so `default_prover()` falls through to `ExternalProver`, and all
CPU proving ran in an **`r0vm` 3.0.6 subprocess** rather than in-process. This is
what makes the cross-machine comparison meaningful: the same r0vm version does the
work on each host. Enabling `--features cuda` switches to the in-process CUDA
prover, which is the only configuration where the host crate itself proves.

**What is and is not machine-dependent.** Cycle counts, segment counts, and proof
sizes are determined by the guest ELF and its input, so they are identical across
machines given the same guest toolchain and the same resolved crate versions.
Only proving wall-clock time varies with hardware. The six `Cargo.lock` files (the
root workspace plus one per guest workspace) and the rzup-pinned guest toolchain are
jointly what make that guarantee hold; matching one without the other is not
sufficient.

### Hardware

Four configurations were measured, across three machines and two architectures.

| | NixOS (primary) | Ubuntu | macOS | GPU |
| --- | --- | --- | --- | --- |
| CPU | AMD Ryzen 9 7900, 12 cores / 24 threads | AMD EPYC 9454P, 48 cores | Apple M3 Pro, 11 cores | host as NixOS |
| Memory | 93 GiB | 63 GiB | 18 GiB | 6 GiB VRAM |
| OS | NixOS 25.05, kernel 6.12.30, x86_64 | Ubuntu 24.04.4 LTS, kernel 6.8.0, x86_64 | macOS 26.5.2 (25F84), Darwin 25.5.0, arm64 | as NixOS |
| ISA notes | avx2, avx512f, bmi2, adx, sha_ni | avx2, avx512f, bmi2, adx, sha_ni | ARMv8 | compute capability 7.5 |
| Device | | | | NVIDIA GTX 1660 SUPER, driver 570.144 |

Unless stated otherwise, every figure below is from the NixOS host with CPU
proving. Two practical constraints found while setting the others up: a transitive
dependency (`ruint 1.20`) requires **rustc 1.90 or newer** on the host, so 1.89
fails to build outright; and the `cuda` feature produces a separate binary that
links against `libcuda`, so switching between CPU and GPU measurement means a
rebuild, not just a flag.

## Results

Empty-guest floor: **2,940 cycles**, 2.23 s to prove, 209,570-byte proof.
Identical on both paths.

### keccak256, cycles per hash (mean over 16 hashes per run)

Each run hashes 16 times and the figures below are the total divided by 16. A
single hash is small enough that loop overhead would be a visible fraction of it,
so averaging makes the primitive dominate.

The loop carries two guards that stop the compiler from optimizing the benchmark
away: one byte store per iteration so the input differs and the hash cannot be
hoisted out as loop-invariant, and a 32-byte XOR fold of each digest into an
accumulator that is returned and committed, so no hash result is dead code. Both
sit inside the measured region, so they are priced separately rather than assumed
negligible. `keccak_loop_overhead` runs the same loop with the hash replaced by a
stand-in, and measures **94 cycles per iteration**, flat across message sizes as
expected for a fixed 32-byte fold. `net` below is gross minus that.

| message bytes     | software gross | software net | accelerated gross | accelerated net | speed-up (net) |
| ----------------- | -------------- | ------------ | ----------------- | --------------- | -------------- |
| 32                | 17,277         | 17,182       | 2,212             | 2,118           | 8.1x           |
| **77** (RedStone) | 17,592         | 17,498       | 2,527             | 2,433           | 7.2x           |
| 141               | 34,510         | 34,416       | 4,071             | 3,977           | 8.7x           |
| 512               | 70,037         | 69,942       | 8,850             | 8,756           | 8.0x           |

The 94 cycles are negligible on the software path (0.5% at 32 bytes) but not on
the accelerated one, where they reach **4.3% at 32 bytes and 3.7% at the 77-byte
RedStone size**. Since the accelerated path is the one that ships, the net column
is the one to quote.

Note the step, not a slope: 32 and 77 bytes cost nearly the same, and 141 bytes
costs almost exactly double. keccak256 absorbs input in 136-byte blocks, so cost
is per block. **The 77-byte RedStone digest fits in a single permutation, the
cheapest case the primitive has.** There is nothing to win by shrinking it.

### secp256k1 ECDSA public-key recovery, in cycles

One recovery per signer: the guest recovers exactly once per signature in the
list, so 3 signers means 3 recoveries. Unlike the keccak table, the `1` row is a
direct single-operation measurement rather than an average, which is what makes
the agreement across 1/3/5 meaningful.

Two things are being reported here and they behave differently: what a whole
M-signer check costs, and what each individual recovery inside it costs. Each
per-recovery figure also includes parsing and validating that signature
(`Signature::from_slice`, `RecoveryId::from_byte`), which a real verifier has to
do anyway.

| signers | software total | software per recovery | accelerated total | accelerated per recovery | speed-up |
| ------- | -------------- | --------------------- | ----------------- | ------------------------ | -------- |
| 1       | 11,074,701     | 11,074,701            | 565,551           | 565,551                  | 19.6x    |
| 3       | 33,228,606     | 11,076,202            | 1,686,893         | 562,298                  | 19.7x    |
| 5       | 55,375,167     | 11,075,033            | 2,817,096         | 563,419                  | 19.7x    |

**Total cost grows linearly**: 3 signers cost 3x what 1 signer costs.
**Per-recovery cost is constant**, and that is exactly why the total is linear:
each recovery is an independent scalar multiplication on the curve with nothing
shared between them, so `total = M x constant`. The flat per-recovery column and
the linear total column are the same fact viewed two ways, not a contradiction.

What makes this worth stating is the absence of the alternative: if recoveries
could share work, per-recovery cost would *fall* as the signer count rose and the
total would grow sublinearly. It doesn't, so there is no free amortization to
exploit. The residual variation under 0.3% is the scalar-dependent
double-and-add path, plus loop overhead.

The flat column is also evidence the `do_work` subtraction is working: had fixed
overhead leaked into the per-recovery figure, it would have been divided across
more recoveries and the column would have drifted downward.

All six figures in this table are pinned by `recovery_cycles_are_unchanged` in
`m0/cost-baseline/host/tests/guardrails.rs`, so the table and the build cannot part
company without a test failure. They are sensitive at the 0.01% level to which patch
release of `k256` the guest resolves, which is why `bench-lib` pins it exactly; see
that file's comment and *Why every requirement is an `=` pin* in `m0/versions.md`.

### Segments, and why proof size is predictable

A zkVM circuit proves a bounded number of cycles, so execution longer than that
is split into **segments**, each proven separately and then recursively combined
into one receipt. Segments are what let a program exceed a single circuit's
capacity at all, and they can be proven in parallel on separate machines.

The segment counts here imply a capacity near 2^20 cycles (1,048,576): 565,551
cycles fit in one segment, 1.69M take two, 2.82M take three, and the 33.3M-cycle
software update takes 34. Proof size then follows the segment count almost
exactly, at roughly **281 KB per segment**:

| workload      | cycles    | segments | proof size | per segment |
| ------------- | --------- | -------- | ---------- | ----------- |
| 1 recovery    | 565,551   | 1        | 281,450 B  | 281,450 B   |
| 3 recoveries  | 1,686,893 | 2        | 562,764 B  | 281,382 B   |
| 5 recoveries  | 2,817,096 | 3        | 844,078 B  | 281,359 B   |

Two consequences. First, proof size is predictable from cycles alone, so a cycle
budget is also a proof-size budget. Second, it locates the keccak accelerator's
penalty precisely: the accelerated 3-of-N update is 785,897 bytes across the same
2 segments the mixed one uses, so its extra 223 KB is not additional segments but
a separate coprocessor proof attached alongside them.

Note also that a 3-of-N update at 1.81M cycles sits just past the one-segment
boundary. Even 2-of-N would cross it, at roughly 1.13M cycles.

### Proving the primitives

Recovery, accelerated (single runs):

| workload     | cycles    | prove time | proof size | segments |
| ------------ | --------- | ---------- | ---------- | -------- |
| empty guest  | 2,940     | 2.23 s     | 209,570 B  | 1        |
| 1 recovery   | 565,551   | 68.50 s    | 281,450 B  | 1        |
| 3 recoveries | 1,686,893 | 136.11 s   | 562,764 B  | 2        |
| 5 recoveries | 2,817,096 | 207.67 s   | 844,078 B  | 3        |

keccak256 at 16 hashes per run, both paths, same guest source, best of 2:

| msg bytes | accelerated time | software time | accelerated proof | software proof |
| --------- | ---------------- | ------------- | ----------------- | -------------- |
| 32        | 30.50 s          | 36.19 s       | 444,599 B         | 268,442 B      |
| **77**    | **34.94 s**      | **34.82 s**   | 467,639 B         | 268,442 B      |
| 141       | 34.55 s          | 68.21 s       | 467,639 B         | 281,690 B      |
| 512       | 43.49 s          | 86.51 s       | 479,351 B         | 537,532 B      |

At the 77-byte RedStone size and 16 hashes the two paths are **level on proving
time**, while software still yields a proof 199 KB smaller. That crossing point is
what fixes the amortization question below. At 512 bytes the software path finally
loses on size too, because 1,120,585 cycles spills into a second segment.

### The full 3-of-N update, and the two accelerators compared

The `verify` workload runs the real shape: three data packages, each hashed over
its own 77 signed bytes and then recovered. Toggling only the hashing isolates
what keccak256 contributes to a genuine update.

Proving figures are the fastest of 3 runs, with the observed spread shown, because
wall-clock noise on a shared machine is large enough to invert a single-run
comparison (see *Caveats*).

| configuration             | cycles     | prove time (best of 3) | proof size    | segments |
| ------------------------- | ---------- | ---------------------- | ------------- | -------- |
| software                  | 33,347,017 | not run                | not run       | 34       |
| accelerated (both)        | 1,769,222  | 164.02 s (spread 1.0%) | 785,897 B     | 2        |
| **mixed** (recovery only) | 1,813,434  | **137.67 s** (spread 5.2%) | **562,764 B** | 2    |

Choosing mixed over accelerated is worth **26.35 s and 223 KB per update**, for
2.5% more cycles.

The software configuration is ruled out on cycles alone at 19x the others, and
proving 34 segments on CPU was not worth the hours it would take. Receipts are
`composite` with 0 assumptions in every configuration, so the choice does not
change what an on-chain verifier must handle.

What the three hashes cost, per configuration. Each figure is a *difference*
within one configuration, `hash + recover` minus `recover only`:

| configuration | 3 hashes, cycles | share of cycles | added prove time | added proof bytes |
| ------------- | ---------------- | --------------- | ---------------- | ----------------- |
| accelerated   | 7,744            | 0.44%           | **+26.90 s**     | **+223,133**      |
| mixed         | 51,956           | 2.87%           | -1.53 s (noise)  | **0**             |

Two entries need reading carefully. Software keccak's `-1.53 s` is not a speedup
from doing more work; it is below the noise floor, and the honest reading across
all three machines (+0.45 s, -0.82 s, -1.92 s) is that software keccak costs
approximately nothing to prove at this scale.

Its **0** added proof bytes is exact, not rounded, and it follows from proof size
being a step function quantised by segment. The extra 51,956 cycles stay inside
the same 2 segments, which hold up to 2,097,152 cycles, so the proof is
byte-identical. Adding cycles is free for proof size until a segment boundary,
then costs a whole segment at once; there are about 283,718 cycles of headroom
left here, roughly 16 more software hashes. The accelerated row is the contrast
that matters: **+223,133 bytes while still using 2 segments**, so that is not
extra segments but a separate coprocessor proof attached alongside them.

**The two accelerators are different in kind, and only one of them is worth
using.** The secp256k1 accelerator is pure cycle reduction: 20x fewer cycles,
proportionally less proving work, no side effects. The keccak accelerator instead
routes hashing to a coprocessor carrying a large fixed cost that the cycle count
cannot see. It makes the three hashes look 6.7x cheaper in cycles while making the
update 26.72 s slower to prove and 223 KB larger.

**The coprocessor cost is paid per proof, not per hash.** Measuring the same
accelerated path at two hash counts, each against its own baseline, shows almost
all of it is fixed:

| accelerated keccak                       | hashes | added prove time | added proof bytes |
| ---------------------------------------- | ------ | ---------------- | ----------------- |
| in `verify`, against recover-only        | 3      | +26.90 s         | +223,133          |
| in `keccak`, against the empty guest     | 16     | +28.27 s         | +235,029          |

A 5.3x increase in hash count added 1.37 s and 11,896 bytes, so the marginal rate
is roughly **0.105 s and 915 bytes per additional hash** on top of a fixed charge
near 27 s and 223 KB. That is what a separate circuit costs: its proof has to be
constructed and recursively verified once, whether it hashed three times or
sixteen.

So the accelerator amortizes, and faster than a fixed cost that large suggests.
The 16-hash measurement above locates the crossing directly: at the 77-byte
RedStone size, **the two paths are level on proving time at about 16 hashes**,
which is 5 to 6 feeds at a 3-of-N threshold. Below that, software keccak is
cheaper; above it, the accelerator pulls ahead and keeps pulling.

Proof size crosses much later. Software keccak adds no bytes until it spills into
another segment, which at 77 bytes takes roughly 60 hashes, so between about 16 and
60 hashes the accelerator is faster while still producing the larger proof. Which
configuration wins in that band depends on whether proving time or proof size is
the binding cost, and that is a LEZ question rather than a RISC Zero one.

For the deliverable as scoped, none of this is close: a single-feed 3-of-N update
is **three** hashes, where software keccak is free on both axes and the accelerator
costs 26.90 s and 223 KB.

### Reproducibility across machines

The 3-of-N workload was run on three machines and two instruction sets. Every
deterministic quantity matched **byte for byte**:

| 3-of-N, mixed        | NixOS x86_64 | Ubuntu x86_64 | macOS ARM64 |
| -------------------- | ------------ | ------------- | ----------- |
| cycles, recover only | 1,761,478    | 1,761,478     | 1,761,478   |
| cycles, hash+recover | 1,813,434    | 1,813,434     | 1,813,434   |
| segments             | 2            | 2             | 2           |
| proof size           | 562,764 B    | 562,764 B     | 562,764 B   |
| cycles per recovery  | 565,551      | 565,551       | 565,551     |

Two conditions make that hold, and both are necessary. The six `Cargo.lock` files
pin crate resolution, and rzup pins the guest toolchain; the macOS machine
initially had r0vm 3.0.3 with guest rust 1.88.0 and had to be brought to parity
before its numbers meant anything. Matching lockfiles without matching the guest
toolchain is not sufficient.

The **host** compiler is not one of those conditions. Running the Ubuntu machine at
host rustc 1.93.0 and again at 1.97.1, with everything else fixed, produced
identical cycle counts, which is what confirms the host compiler builds only the
driver while the guest toolchain and the prebuilt `r0vm` do the work that gets
measured.

### GPU proving, and a third argument against the keccak accelerator

GPU proving needs `--features cuda`, which produces a separate binary linked
against `libcuda`; switching between CPU and GPU measurement is a rebuild, not a
flag. On NixOS it also needs `nix-shell --arg withCuda true`.

The card used here (6 GiB, with a desktop session already holding 2.4 GiB) cannot
fit a full-size segment, so these runs cap segments at 2^18 via `--segment-po2 18`.
That raises the segment count from 2 to 9 and the proof from 563 KB to 2.29 MB,
so GPU figures are only comparable against CPU figures taken at the same cap.

| 3-of-N, mixed, segment po2 18 | prove time | proof size | segments |
| ----------------------------- | ---------- | ---------- | -------- |
| CPU (Ryzen 9 7900)            | 155.55 s   | 2,302,714 B | 9       |
| GPU (GTX 1660 SUPER)          | **9.98 s** | 2,302,714 B | 9       |

**15.6x faster on GPU, with byte-identical proof size**, which is independent
confirmation that proof size is a property of the computation rather than of the
machine.

The more consequential result is what would not run at all. **The accelerated
configuration exhausted VRAM at every segment cap tried (2^20, 2^19, 2^18), while
the mixed configuration completed at 2^18.** At 2^18 the only difference between
them is the keccak coprocessor, so on this card that coprocessor is the difference
between proving and not proving. Its peak request at the default cap was a single
3.3 GiB allocation.

Treat this as directional rather than exact: the card is a consumer part sharing
memory with a display, and free VRAM moved between runs. But it points the same
way as the proving-time and proof-size results, and it is the one that turns a
cost question into a feasibility one on constrained proving hardware.

## What these numbers say

**Recovery is the entire cost.** One ECDSA recovery costs ≈232x the 77-byte
keccak256 hash it verifies on the accelerated path, and ≈633x on the software
path. The ratio differs because the two accelerators are not equally effective:
keccak gains ≈7x, recovery ≈20x. Either way the conclusion is the same. All
optimization pressure and the whole precompile argument belong on the signature
path; the hash is noise.

**An M-of-N threshold costs M times a single recovery, and the signatures are
over distinct messages.** Each RedStone node signs its own value and its own
millisecond timestamp, so of the 77 signed bytes only 39 (feed id, value byte
size, data point count) are common across signers. The SDK's own aggregator
builds a feed-by-signer matrix and takes the median across signers, which is only
meaningful because the values differ. So M-of-N carries M independent
observations, not one observation signed M times.

That rules out the obvious optimization. There is nothing shared between the
recoveries to amortize, because there is no shared message. Widening the
threshold is linearly priced in both payload bytes and verification cycles, and
no batching of ECDSA closes that: ECDSA has no practical batch-verification
construction the way Schnorr and BLS do, and verifying against a known signer key
instead of recovering saves only the recovery-specific work, not the scalar
multiplication that dominates.

The lever that does collapse it is a **different signature scheme**: a threshold
or aggregate signature (FROST/BIP-340, BLS) turns M verifications into one,
taking the cost from linear in the signer count to constant. The price is that
signers must agree on one exact message, so aggregation and the median move
off-chain and become a coordination point. That is a change to the trust model
rather than to the encoding, and it is out of scope here: RedStone signs
per-signer ECDSA today, and this baseline measures what that costs.

**The accelerated path is mandatory, not an optimization.** At 3-of-N, software
recovery is ≈33.2M cycles per update against ≈1.7M accelerated. The accelerated
path has to be the one that ships, so the M0 cost table must be taken against
it. A baseline measured on the software path would overstate real cost by ≈20x
and mislead the precompile decision.

**Cycle counts materially understate accelerated keccak, and the proving data
proves it.** 35,389 cycles of accelerated keccak take 32.41 s to prove, while
565,551 cycles of recovery (16x the cycles) take only 68.50 s, roughly 2x the
time. The keccak accelerator moves work into a coprocessor whose cost is
invisible to the main cycle count but very much present in proving time and
proof size (a 444 KB keccak proof against a 281 KB recovery proof). **Cycles are
not a valid cross-path cost metric once an accelerator is in play.** Proof time
and proof size are the sound basis for comparing an accelerated path against a
software one.

**No unstable feature flag was needed** for the accelerated keccak path to build,
execute, *and* prove under risc0 3.0.6; the `[patch.crates-io]` entry was
sufficient. Earlier risc0 releases did gate it behind one, so guidance to that
effect no longer applies at 3.0.6.

## Recommendations

A 3-of-N single-feed verification costs **1,813,434 cycles and 137.67 s to prove**
in the recommended configuration, producing a 562,764-byte proof. Recovery is
97.1% of those cycles and effectively all of the proving cost. Every
recommendation below follows from that. The figure covers verification only, not
payload decode, median, the canonical price-account write, or SPEL overhead,
which M1 and M2 add.

**1. Ship the mixed configuration: accelerate recovery, leave keccak256 in
software.** The secp256k1 accelerator is a 20x cycle reduction with no side
effects and is not optional.

The keccak accelerator is the finer call, and it turns on which cost LEZ actually
charges. LEZ executes public transactions **without proving** and caps them in
cycles, so in public mode proof time and proof size are not paid at all, and the
keccak accelerator is marginally *better*: 1,769,222 cycles against 1,813,434, a
2.4% saving. On that basis alone it would win.

It should still be declined, because LEZ runs the **same program** publicly or
privately, and a consumer holding private accounts has this code proven. In that
case the accelerator costs 26.90 s of proving and 223 KB of proof per update, from
a coprocessor charge paid once per proof that needs roughly 16 hashes to amortize
against an update's three. It also failed to fit in 6 GiB of VRAM where the mixed
configuration proved successfully.

So the trade is: negligibly worse in public mode, where the gap is 0.13
percentage points of a budget with 94% headroom, against decisively better in
private mode. Software keccak is the configuration that is never badly wrong.

**2. Guard the configuration in CI, on two metrics.** A `[patch.crates-io]`
section fails silently: a crate bump, a moved tag, or a dropped section restores
software cost with a green build. Because the two accelerators fail in opposite
directions, one assertion cannot catch both. A cycle ceiling on a single recovery
catches losing the secp256k1 accelerator, and a proof-size ceiling on a 3-of-N
update catches gaining the keccak one, since 223 KB of coprocessor proof is
unmistakable and invisible to cycles.

**3. Recommend a secp256k1 recovery precompile. Do not pair it with keccak256.**
RFP-020 frames the follow-on as "secp256k1 + keccak256", and the measurement
separates them cleanly. Recovery is 97% of the cycles and essentially all of the
proving cost, so a native `ecrecover` would remove nearly all of it. Software
keccak256 over 77 bytes is a single permutation costing 17,498 cycles and 0.15 s
of proving, 2.9% of the update. A keccak precompile would be a rounding error,
and the accelerator that already exists for it is a net loss here. The follow-on
should be scoped to recovery alone.

**4. Treat M as a priced security parameter, and keep the default at 3.** Cost is
strictly linear in the signer count, in cycles and in payload bytes alike, and
there is no batching remedy for the recoveries themselves: the signatures are over
distinct messages, and ECDSA
has no practical batch-verification construction. Moving 3-of-N to 5-of-N is a
67% cost increase with no engineering mitigation available. Anything beyond a
threshold change, meaning a threshold or aggregate signature scheme, alters the
trust model rather than the encoding and belongs outside this deliverable.

**5. Make push the recommended integration path and price pull explicitly.** Both
modes run the same verifier over the same payload, so a pull read costs the same
1.81M cycles as a push update. The difference is who pays and how often: push
pays once per update and every reader shares the result, while pull makes each
consumer pay in full on every read. With recovery unbatchable, that asymmetry
cannot be engineered away, and it compounds in pull, where the consumer also
carries the 440 payload bytes per read. The multi-feed batching soft requirement
amortizes across feeds within one update, which benefits push only, and it is also
the one setting where the keccak accelerator becomes worth reconsidering: batching
past roughly 5 or 6 feeds per proof crosses its amortization point. Pull remains
the right choice where a consumer needs a price fresher than the heartbeat inside
its own transaction, and the documentation should put the per-read cost in front
of that decision rather than behind it.

**6. The LEZ budget question is settled, and there is ample headroom.** No unit
conversion turned out to be necessary: LEZ has no separate compute-unit currency
and caps a public execution directly in risc0 cycles, at
`MAX_NUM_CYCLES_PUBLIC_EXECUTION = 32M` (33,554,432), enforced through the same
`ExecutorEnv` session limit used here. Cycles measured above are therefore already
in the budget's units.

A 3-of-N update at 1,813,434 cycles is **5.4% of that budget**, or 5.68% once the
LEZ program framework's own 39,103 cycles are included (measured separately in
`m0/lez-probe/`). The risk this recommendation previously flagged does not
materialize: verify-and-publish fits in one transaction with roughly 94% of the
budget unused, so Performance 1 is safe and a recovery precompile stays an
optimization rather than becoming a prerequisite. The budget would in fact hold
around 53 signers.

The software configuration is the one that does not fit: 33,347,017 cycles is
99.4% of the budget before the framework, the payload decode, the median, or the
account write. That is a harder argument for the secp256k1 accelerator than any
proving figure above.

**7. State which cost is being compared, because cycles and proving diverge.**
Judged on cycles the keccak accelerator looks like a 6.7x win; judged on proving it
is a 19% loss and 40% more proof bytes. A coprocessor's cost does not appear in the
cycle count of the program that invokes it, so the two metrics genuinely disagree
and neither is universally correct.

Which one binds depends on the execution mode. **Public execution on LEZ is
unproven and capped in cycles, so cycles are the operative cost** and the figure to
quote for the push aggregator and public-mode pull. Proving time and proof size
become real only on the private path, and they remain the sound basis for any
accelerated-versus-software or in-program-versus-precompile comparison, since a
cycles-only comparison there would be actively misleading.

## Guardrails

M0-09 asks for figures reproducible from a benchmark target, and M0-10 for
CI-pinned assertions that catch later regressions. Both are met by tests rather
than a `cargo bench` target, which was a deliberate choice: cycle counts are
deterministic and execute in under two seconds, which suits CI exactly, while
proving takes minutes per sample and is hardware dependent, which criterion-style
benchmarking handles badly. What these figures need is not a timing harness but an
assertion that fails a build when they move. A `cargo bench` target would have
satisfied the wording while checking nothing.

The CLI and the tests call the same functions in `m0/cost-baseline/host/src/lib.rs`, so
a published figure and an asserted figure cannot drift apart. Seven fast assertions
plus one slow ignored one, in `m0/cost-baseline/host/tests/guardrails.rs`:

| guardrail | catches |
| --------- | ------- |
| zkVM floor unchanged, all three paths | drift in the baseline every other figure is relative to |
| keccak256 cycles per hash unchanged | movement in either keccak implementation, at all four message sizes |
| loop bookkeeping flat and unchanged | the 94-cycle correction the net per-hash figures depend on |
| recovery cycles unchanged at 1, 3, 5 signers | movement in either secp256k1 implementation |
| per-recovery cost does not amortise | recoveries starting to share work, which would invalidate the linear-pricing finding |
| full 3-of-N update cycles and segments unchanged | the headline figure, on all three configurations |
| mixed is the combination it claims | a `[patch.crates-io]` section going missing on either side |
| proof sizes distinguish the configurations (ignored) | gaining the keccak coprocessor, which *lowers* cycles |

Three design points.

**Exact equality is intentional.** Cycle counts are a deterministic function of the
guest ELF and its input, reproduced byte-identically across x86_64 and ARM64, so
pinning them exactly means any toolchain bump or dependency change fails here and
forces the published figures to be re-tagged deliberately. A failure is a prompt to
re-measure, not necessarily a defect.

**The configurations are pinned against each other, not just absolutely.** Each is
defined by a `[patch.crates-io]` section, and such a section fails *silently*: drop
it and a different cost profile ships with a green build. Absolute values would
catch that only as an unexplained number change, whereas asserting that `mixed`
matches `software` on hashing and `accelerated` on recovery states the claim the
recommendation actually rests on. It is also what makes a slipped version pin legible
rather than mysterious: if the arms stop agreeing where they should, the pin moved.

**Two dimensions are needed because the accelerators fail in opposite directions.**
Losing secp256k1 raises cycles roughly 20x. Gaining keccak256 *lowers* cycles by
about 44,000 while adding 223 KB of proof. Exact equality catches both, but only the
proof-size assertion makes the second unmistakable in the dimension where it hurts.

Both suites run in CI on x86_64 and, for pushes to `main`, on ARM64
(`.github/workflows/guardrails.yml`). The two architectures assert the same
constants, so the cross-machine reproducibility claim above is a standing check
rather than a one-off observation. CI builds with `--locked` and
`RISC0_BUILD_LOCKED=1`, since the guest workspaces carry their own lockfiles and the
guest ELF is what the counts measure.

## Caveats

- **These are bare-RISC-Zero numbers.** They need no conversion, because LEZ caps
  public execution in risc0 cycles rather than a separate currency, but they do
  exclude what the LEZ program framework adds around verification. That is
  measured separately in `m0/lez-probe/`, at 39,103 cycles, and it does not change any
  conclusion here.
- **Proving figures apply to the private path only.** LEZ executes public
  transactions without proving, so for the adaptor as scoped the proving times and
  proof sizes above are not costs paid on-chain. They are reported because the same
  program is proven when a consumer uses private accounts, and because they are
  what makes the two accelerators distinguishable at all.
- **Proving wall-clock is noisy; treat single runs as unreliable.** Repeated runs
  on the same machine vary by 1 to 7%, which is enough to invert a comparison: one
  single-run measurement showed a strictly larger workload proving *faster* than a
  smaller one, an impossibility. Every proving figure quoted here is therefore the
  fastest of several runs (`--repeat N`), reported with its spread, on the basis
  that contention only ever adds time. Any delta smaller than the stated spread
  should be read as zero, which is why software keccak's cost is described as
  approximately nothing rather than as a negative number.
- **Proving hardware differs from production.** Absolute times come from consumer
  and general-purpose server CPUs plus one 6 GiB consumer GPU, none of which is a
  proving host. Cycle counts, segment counts, and proof sizes are
  hardware-independent and reproduced byte-identically across three machines, so
  those transfer; times do not. The keccak-accelerator conclusion is safe against
  this because its decisive evidence, the 223 KB proof-size penalty, is
  deterministic, and because the percentage penalty held at 15 to 16% on two
  unrelated CPUs.
- **GPU results are directional.** The card shares memory with a desktop session,
  free VRAM moved between runs, and full-size segments do not fit at all. The
  accelerated-configuration OOM is consistent with the CPU findings but is not a
  controlled measurement.
- **The software configuration was not proved end to end.** At 34 segments per
  3-of-N update it is ruled out on cycles alone, so only its cycle counts appear
  above.
- **Derived rather than decoded.** The 77-byte figure comes from the published
  wire format, cross-checked against RedStone's sample payload. It has not yet
  been confirmed by a decoder running against a live package.
