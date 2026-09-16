# 4. Guest-reachable code is `no_std`, unsafe-free, and must not panic

- **Status**: accepted
- **Milestone**: M1 (`M1-01`, `M1-10`, `M1-12`)
- **Requirements**: F1, F4, U6
- **Artefacts**: `verifier-core/src/lib.rs`, `pull-lib/src/lib.rs`, `methods/guest/`, `ci.yml` jobs `no-std` and `guest`

## Context

Verification runs inside a zkVM guest. That environment changes what a defect costs.

A **panic in a guest aborts the whole program**, which on LEZ means the transaction
fails rather than one package being rejected. So the difference between `return
Err(Truncated)` and an index-out-of-bounds is not a matter of style: one rejects a bad
package and lets the transaction continue, the other destroys the transaction. An
attacker who can get a malformed payload in front of the verifier and cause a panic has
a denial-of-service primitive.

The guest also cross-compiles to `riscv32im-risc0-zkvm-elf` with RISC Zero's own Rust
toolchain, under its own resolution. A dependency that is not guest-compatible is
discovered at link time, and the later that happens the more work has been built on it.

## Decision

Three rules, each with a CI gate behind it.

**`#![no_std]` on every crate that can be reached from a guest** — `verifier-core` and
`pull-lib` today. Enforced by the `no-std` job, which builds them for
`riscv32im-unknown-none-elf`: a bare target with no std at all, so an accidental `std`
dependency fails the build rather than being tolerated because the risc0 target happens
to provide one.

**`#![forbid(unsafe_code)]` everywhere**, including the host-side crates. Nothing in
this deliverable needs unsafe, and `forbid` rather than `deny` means it cannot be
locally re-enabled.

**Panic-freedom is a documented contract, not an aspiration.** `VerifierBackend`'s
contract says an implementation must be deterministic and must not panic on any input;
every failure is a `BackendError`. The decoder bounds-checks every length field and is
tested against truncation *at every offset* of a valid payload
(`truncation_at_every_length_is_rejected_rather_than_panicking`), which is the
systematic version of the property rather than a handful of chosen cases.

The `guest` CI job builds product code into a real guest ELF with the pinned rzup
toolchain, from `methods/guest/src/bin/verifier_link.rs`. That binary exists for no
other reason: it links `verifier-core` and `kanon-idl` and commits a version string, so
the riscv32 cross-compile of shipping code is exercised from M1-01 rather than first
attempted when the aggregator lands. It touches `size_of::<OraclePriceAccount>()`
explicitly, because an unused dependency can be dropped by the linker and a dependency
that is never linked never proves anything.

## Consequences

- A malformed payload is always a rejection and never a failed transaction, which is
  what makes F4's "reject" verbs mean what they say.
- Adding a dependency to `verifier-core` that is not `no_std` or not guest-compatible
  fails on that commit, naming the crate.
- Convenience is genuinely lost: no `String`, no `Vec` in the shared path, no
  `std::error::Error`. The decoder is zero-copy partly for that reason (ADR 5).
- The `no-std` job's bare target is stricter than the actual delivery target. That is
  intentional and occasionally means working around something the real target would
  have allowed.

## Alternatives considered

- **`std` with a `no_std` feature flag.** Rejected: a feature-gated `no_std` is only as
  good as the CI matrix that exercises it, and the default configuration is the one
  people test.
- **Catching panics.** Not available — a guest panic is not unwindable into a
  recoverable error.
- **Testing panic-freedom by fuzzing only.** Kept as a complement, not a substitute:
  the exhaustive truncation test is deterministic and runs on every push.
