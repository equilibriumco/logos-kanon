# 7. Two workspaces, and a lockfile per resolution domain

- **Status**: accepted
- **Milestone**: M1
- **Requirements**: P2, S2
- **Artefacts**: `Cargo.toml`, `m0/Cargo.toml`, `methods/guest/Cargo.toml`, `README.md` (*Workspaces*)

## Context

M0 is delivered. Its figures are published in `m0/M0-report.pdf` and they are a
property of an exact toolchain and an exact dependency graph — cycle counts are a
deterministic function of the guest ELF and its input.

The product is the opposite. It tracks a live LEZ and a live SPEL, and its pins have to
move as those move.

With one shared lockfile these two constrain each other in the worst possible way: an
ordinary product upgrade could move a number in a delivered report, silently, in a
green build; and freezing M0's pins would put a ceiling on the product. The M0 report
would become a document about a state of the world that no longer exists, with nothing
to say so.

Separately, the guest workspaces exist to differ. `m0/`'s three configurations —
software, accelerated, mixed — *are* three different `[patch.crates-io]` sections over
the same source (ADR 6). That distinction is the measurement.

## Decision

Three resolution domains, each with its own lockfile:

| domain | lockfile | pins |
| --- | --- | --- |
| repository root | `Cargo.lock` | the product crates. Pins move here, as ordinary upgrades |
| `m0/` | `m0/Cargo.lock` | frozen to the toolchain the published figures were measured on |
| each `*/guest/` | its own | cross-compiles to `riscv32im-risc0-zkvm-elf`, own `[patch.crates-io]` |

`m0` is in the root workspace's `exclude` list, not its `members`. Guest crates are
deliberately absent from `members` too: they are built by `risc0-build` from the
`methods*` build scripts, under their own resolution, and folding them in would
collapse the three configurations into one.

Guest lockfiles are **committed** and CI builds with `RISC0_BUILD_LOCKED=1`, because
the guest ELF is what every cycle figure is a property of.

## Consequences

- An M0 figure cannot move because of a product upgrade. That is the whole point.
- The product and `m0/` can — and now do — sit on different versions of the same
  dependency without either being wrong. See ADR 8.
- **`cargo test --workspace` no longer runs everything.** CI covers this by splitting:
  `ci.yml` for the product, `guardrails.yml` for `m0/` (ADR 10). A contributor running
  the root workspace's tests locally has not run M0's.
- The licence gate has to be run per domain, against six guest lockfiles plus two
  workspaces (ADR 1). Adding a domain means adding it to that job.
- `scripts/lez-sequencer.sh` reads the LEZ revision from the **product** lockfiles only,
  and skips `m0/`'s; scanning both would now be a permanent false alarm (ADR 12).
- Dependabot-style automation, when it arrives, needs to know about all three domains.

## Alternatives considered

- **One workspace, one lockfile.** Rejected: it makes every product upgrade a
  re-measurement risk on a delivered report.
- **Archive `m0/` outside the repository.** Rejected: P2 makes cost measurement a
  primary deliverable that must be "reproducible from the test suite", so the harnesses
  have to live where CI can run them.
- **Vendor M0's dependencies.** Rejected as a heavier way to achieve exactly what a
  second lockfile already achieves.
