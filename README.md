# Kanon: RedStone oracle adaptor for LEZ

RFP-020. Built by Equilibrium for Logos. Dual licensed MIT and Apache-2.0.

**Start with the M0 report**, `m0/M0-report.pdf`. It is the M0 deliverable: what a
RedStone price update costs inside a LEZ program, how that was measured, what we
recommend, and the decisions we need from Logos.

The headline: a 3-of-N single-feed update costs **1,906,737 cycles**, 5.68% of LEZ's
32M per-transaction budget, and secp256k1 recovery is 95% of it.

## Layout

| | |
|---|---|
| `m0/` | M0's measurement harnesses, report and pinned versions. Delivered; see below |
| `shell.nix` | dev shell: host Rust toolchain and the build prerequisites |

M1 adds the product crates at this level: `verifier-core`, `methods/guest`,
`aggregator-program`, `pull-lib`, `kanon-idl`, `kanon-sdk`, `kanon-relayer`,
`kanon-app`, `reference-consumers` and `examples`. `m0/` is separate so the
measurement harnesses do not read as product code once those arrive.

### `m0/`

| | |
|---|---|
| `M0-report.pdf` | the report. Read this first |
| `versions.md` | what to install for the figures to reproduce, and the LEZ version question |
| `cost-baseline/` | the two primitives on bare RISC Zero, no LEZ dependency at all |
| `lez-probe/` | the same workload inside a real LEZ program, via `lee_core` |

Two harnesses rather than one, deliberately. `cost-baseline/` isolates the primitives
from any framework, which is where the accelerator comparison is made. `lez-probe/`
puts them in a real program, which is what makes the figures comparable to LEZ's
budget. Each README covers its own method in full.

## Reproducing the figures

From this directory, inside `nix-shell` on NixOS:

```sh
cargo test --release --locked --workspace              # every published figure, seconds
cargo test --release --locked --workspace -- --ignored # the proving figures, minutes
```

Every number in the report is asserted by exact equality in one of the two guardrail
suites, and both run in CI on x86_64 and ARM64. A guardrail failure is a prompt to
re-measure and re-tag deliberately, not necessarily a defect: cycle counts are a
deterministic function of the guest ELF and its input, so any toolchain or dependency
change is meant to fail there rather than pass quietly.

Guest code cross-compiles to `riscv32im-risc0-zkvm-elf` with RISC Zero's own Rust
toolchain, managed by `rzup` outside Nix. On NixOS that needs
`programs.nix-ld.enable = true`, because rzup ships prebuilt dynamically linked
binaries. `m0/cost-baseline/README.md` has the full setup for both NixOS and
non-NixOS.
