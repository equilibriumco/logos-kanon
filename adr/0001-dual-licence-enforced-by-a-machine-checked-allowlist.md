# 1. Dual licence, enforced by a machine-checked allowlist

- **Status**: accepted
- **Milestone**: M1 (`M1-02`, `M1-04`)
- **Requirements**: OS1
- **Artefacts**: `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `deny.toml`, `ci.yml` job `licenses`

## Context

RFP-020 requires the deliverable to be open source under a permissive licence, with no
copyleft or BUSL dependency, transitive ones included. That is a delivery condition
rather than a preference, and the failure mode is specific: a copyleft crate arrives
transitively, in a dependency that was never selected directly, and nothing in a green
build says so.

Two hazards are concrete rather than hypothetical.

RedStone's Rust SDK is Boost licensed and permissive, but it lives in a monorepo
alongside BUSL-1.1 packages, including the EVM connector. "Only the permissive part is
a dependency" is a claim that needs re-checking on every build, not once at review.

LEZ's host graph can carry LGPL-3.0. `lee_core` → `risc0-zkvm` →
`risc0-circuit-rv32im` pulls in `malachite` and its family **when risc0's proving
feature is enabled**, which is the measurement harness's configuration and not the
product's: the five LGPL crates are in `m0/Cargo.lock` and absent from the product
lockfile. LEZ itself ships MIT-or-Apache-2.0 while carrying them, so this is an
inherited position rather than a chosen one — but it is a narrower one than a graph
listing suggests, and the difference is the feature flag.

## Decision

Ship dual MIT and Apache-2.0, and enforce it with `cargo deny` rather than assert it.

`deny.toml` is an **allowlist**, not a denylist. A licence that is not named fails the
build. BUSL-1.1, the GPL family and SSPL are therefore denied by omission and by
intent, and a licence not yet considered stops a build and is reviewed instead of being
absorbed silently. The fuzzy-match confidence threshold is raised
from cargo-deny's default 0.8 to 0.93, because a low-confidence text match is how a
copyleft licence gets read as something permissive.

The gate runs in CI over **every** resolution domain: the product workspace, `m0/`,
and each guest workspace's own lockfile. It is evaluated against both delivery
targets, `x86_64-unknown-linux-gnu` and `riscv32im-risc0-zkvm-elf`, because dependency
graphs are platform-dependent even though cycle counts are not, and a platform-gated
crate would otherwise slip past on the target it does not appear on.

Every exception is **per-crate**, named and commented. An exception grants a licence
to one crate, so it cannot widen into a general allowance by accident. The three
Logos crates that ship without a `license` field — `twap_oracle_core`,
`spel-framework-core`, `spel-framework-macros` — get `[[licenses.clarify]]` entries
rather than a widened allowlist, for the same reason: a clarification names one crate
and can be checked.

The RedStone boundary is stated in `NOTICE` and backed by the gate: the SDK may be a
declared dependency; the BUSL EVM connector is not copied, ported, or read for the
purpose of writing the decoder here, and is used only as a black-box oracle against
published vectors where byte-level conformance has to be checked.

## Consequences

- The gate is enforced on every pull request over the whole graph rather than declared,
  which is what the exceptions below have to be read against.
- **OS1 is not `verified`, and the gate is the reason why.** The requirement asks for no
  copyleft dependency, transitive ones included; the gate passes because copyleft is
  admitted by named exception rather than absent. Passing CI therefore proves the gate
  ran, not that the requirement is met, so the row stays `partial` until Logos rules on
  it. The exceptions are narrower than they look — the LGPL crates are confined to the
  measurement workspace's proving path, and the MPL one is build-time only — and that
  distinction is the substance of the question, so it belongs in the row rather than in
  a reviewer's head.
- A new transitive dependency with an unlisted licence breaks the build. That is the
  intended cost; the alternative is finding out at delivery.
- The LGPL-3.0 position is written down and raised with Logos rather than waved
  through (`m0/versions.md`, *Open: the LGPL-3.0 dependency*). It is not something
  this repository can fix — it is upstream in `risc0-circuit-rv32im` — and it affects
  every LEZ program's host tooling, not only this one.
- Exception licences are spelled in the deprecated bare form (`LGPL-3.0`) because
  cargo-deny's parser rejects `LGPL-3.0-only` in an exception. That is deliberate, not
  stale.
- `cargo deny` must be re-run against each new guest workspace. Adding a guest without
  adding it to the `licenses` job creates an unchecked graph.

## Alternatives considered

- **A denylist of known-bad licences.** Rejected: it only catches what has already
  been thought of, which is the opposite of the property wanted.
- **Review discipline.** Rejected: the hazard is transitive and arrives without a
  reviewer seeing a diff.
- **A wider allowlist covering unlicensed crates.** Rejected in favour of per-crate
  clarifications, which cannot silently cover a future unlicensed dependency.
