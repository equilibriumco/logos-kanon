# 8. Exact version pins, and the product tracks the estate rather than the newest release

- **Status**: accepted
- **Milestone**: M0 (exact pins), M1 (product realignment to risc0 3.0.5 / LEZ v0.2.0)
- **Requirements**: P1, P2
- **Artefacts**: `Cargo.toml`, `m0/Cargo.toml`, `m0/versions.md`, `methods/guest/Cargo.toml`

## Context

Two related problems.

**Ranges confound measurement.** A caret range lets the software arm of M0's comparison
resolve a different patch release than the fork tags the accelerated arms use. That
moves every recovery figure by about 0.01% and confounds the comparison, with nothing
visible in a green build.

**Cargo does not unify a `tag` source with a `rev` source, even for the same commit.**
This was discovered the expensive way. Taking a dependency on `twap_oracle_core` (ADR 9)
meant two crates could pin `lee_core` — one by `rev = "15144ddb…"`, one by
`tag = "v0.2.1"`, *which is that commit* — and get two `lee_core` packages, and
therefore two incompatible `AccountId` types. `[patch]` cannot bridge it either:
patching a git source with the same URL fails with *"patches must point to different
sources"*. Matching requires a byte-identical source spec.

And `twap_oracle_core` pins `risc0-zkvm = "=3.0.5"`, stricter than `lee_core` itself.
Against the `=3.0.6` pin here that was unsatisfiable, and resolution failed before `lee_core` was
even considered.

## Decision

**Every version requirement is `=`, not a caret range**, including in the guest
manifests and the `[patch.crates-io]` tags.

**The product is pinned to risc0 3.0.5 and LEZ v0.2.0. `m0/` stays at 3.0.6 and
v0.2.1.** The workspace split (ADR 7) is what lets those be different answers, and they
answer different questions: `m0/` is frozen to the toolchain its published figures were
measured on; the product tracks what the estate ships, because that is the constraint
that actually binds. LEZ, `lez-programs` and `twap_oracle_core` are all on 3.0.5, and
M0 took 3.0.6 only because `lee_core` treats 3.0.5 as a floor rather than a ceiling.
Sitting one patch release above the estate bought nothing and cost the ability to link
the canonical price account.

**Comparability was measured, not argued.** The risc0 crate demonstrably changes the
ELF: rebuilding `methods/guest` at 3.0.5 moved `VERIFIER_LINK_ID` from `[3246365612,
3942306992, …]` to `[882320099, 3252084823, …]`. So `m0/` was temporarily flipped to
3.0.5, every guest rebuilt, and both guardrail suites run: **all twelve assertions
that existed then passed unchanged**. A different ELF, identical cycle counts. `m0/` was then reverted.

Toolchain versions that no lockfile can express are recorded in `m0/versions.md`:
r0vm 3.0.6, guest Rust 1.97.0, cargo-risczero 3.0.6, all rzup-managed. Both the r0vm
version and the guest rustc version are necessary conditions — one machine gave
different counts until it came off r0vm 3.0.3 with guest rustc 1.88.0. The **host**
compiler is not a condition: 1.93.0 and 1.97.1 gave identical counts, because the host
builds only the driver.

`ci.yml` installs rzup 3.0.5 for the product guest; `guardrails.yml` keeps 3.0.6 for m0.

## Consequences

- The canonical price account is linkable, which is what unblocked ADR 9.
- Doing this in M1 was the cheapest it will ever be: no product figure had been measured
  yet. M1-25 is the first, and it will be taken at 3.0.5.
- `ruint` is pinned to 1.17.0 in the product graph because `twap_oracle_core` pins it
  exactly — the last release before an MSRV bump the risc0 guest toolchain cannot meet.
  It satisfies risc0's own `^1.15.0`, so nothing else moved.
- **The sequencer's CLI changed across the pin move.** v0.2.1's `sequencer_service` takes
  `--listen-address` and `--home`; v0.2.0 takes neither and rejects an unknown flag
  outright, which is how CI found out. `scripts/lez-sequencer.sh` now asks the binary
  what it accepts via `--help` rather than assuming (ADR 12).
- Upstream LEZ is already at v0.2.4, so the product will move again — to wherever
  `lez-programs` goes. `m0/versions.md` carries the open question to Logos about which
  pin the estate is standardising on.
- Exact pins mean no automatic patch upgrades, including security ones. Bumps are
  deliberate, and `m0/versions.md`'s *Changing any of this* states the procedure: bump,
  run both guardrail suites, update the constants, the affected report tables and that
  file in one commit.

## Alternatives considered

- **Caret ranges with a lockfile.** Rejected for `m0/`: `--locked` protects CI, but a
  contributor who runs `cargo update` gets a different measurement with no warning.
- **Keep the product on 3.0.6 and vendor a copy of `OraclePriceAccount`.** Rejected: it
  forks a standard the RFP explicitly says to re-export (ADR 9).
- **`[patch]` to unify the two `lee_core` sources.** Not possible — cargo rejects a
  patch pointing at the same source.
