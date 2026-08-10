# Architecture decision records

The decisions behind this repository, one file each: what the situation was, what was
decided, and what it costs. Written after the fact for M0 and M1, and kept current from
here.

These are records, not documentation. For *what* the code does, the crate doc comments
and `TRACEABILITY.md` are the sources. An ADR answers **why**, and is the thing to read
before undoing one.

An ADR is not amended once it is accepted, except to change its status. If a decision is
reversed, the new ADR supersedes it and says so, and the old one stays where it is —
a record of a decision that was later reversed is more useful than no record of it.

| | decision | status |
| --- | --- | --- |
| [1](0001-dual-licence-enforced-by-a-machine-checked-allowlist.md) | Dual licence, enforced by a machine-checked allowlist | accepted |
| [2](0002-one-verification-core-shared-by-both-modes.md) | One verification core, shared by both modes, in a crate layout created up front | accepted |
| [3](0003-cryptographic-primitives-behind-a-verifier-backend-trait.md) | Cryptographic primitives behind a `VerifierBackend` trait | accepted |
| [4](0004-guest-reachable-code-is-no-std-unsafe-free-and-panic-free.md) | Guest-reachable code is `no_std`, unsafe-free, and must not panic | accepted |
| [5](0005-a-first-party-redstone-decoder-separate-from-verification.md) | A first-party RedStone decoder, kept separate from verification | accepted |
| [6](0006-mixed-accelerator-configuration.md) | The mixed accelerator configuration: accelerate secp256k1, leave keccak256 in software | accepted |
| [7](0007-two-workspaces-and-a-lockfile-per-resolution-domain.md) | Two workspaces, and a lockfile per resolution domain | accepted |
| [8](0008-exact-pins-and-tracking-the-estate.md) | Exact version pins, and the product tracks the estate | accepted |
| [9](0009-re-export-the-canonical-price-account-and-vendor-its-idl.md) | Re-export the canonical price account, and vendor its IDL as a conformance oracle | accepted |
| [10](0010-cycle-counts-as-exact-equality-guardrails-in-a-separate-workflow.md) | Cycle counts asserted by exact equality, in a workflow of their own | accepted |
| [11](0011-a-machine-checked-traceability-matrix.md) | A traceability matrix that is generated and machine-checked | accepted |
| [12](0012-a-standalone-lez-sequencer-without-lgs-run-from-a-prebuilt-image.md) | A standalone LEZ sequencer without `lgs`, run in CI from a prebuilt image | accepted |
| [13](0013-staleness-is-measured-against-the-lez-clock-program.md) | Staleness measured against the LEZ clock program's every-block account | accepted |
| [14](0014-build-admin-gating-against-the-unmerged-spel-admin-authority.md) | Admin gating built against the unmerged SPEL admin-authority, shim in reserve | accepted, contingency live |

## How they fit together

Four of these are one argument in four parts. ADR 7 splits the workspaces so measurement
and product can move independently; ADR 8 then moves the product onto the estate's pins,
which is what made ADR 9's canonical price account linkable; ADR 10 is what made that
move provably safe rather than hopeful, by asserting every published figure exactly.

Three more are the shape of the verification path: ADR 2 puts one implementation behind
both modes, ADR 3 puts the expensive primitives behind one swappable trait, and ADR 4
states what the guest environment demands of anything either of them touches. ADR 5 and
ADR 13 are the two places where an external format and an external clock enter that
path, and both are decided the same way — take the untrusted thing, bound it, and refuse
to let a caller choose it.

The remaining four exist to keep claims verifiable: the licence gate (1), the guardrails
(10), the traceability checker (11), and a sequencer that cannot drift from the code it
tests (12).

## Open questions carried by these decisions

Four are with Logos, and all four are stated in full in `m0/versions.md`, *Open:
questions outstanding with Logos*:

- **Which LEZ pin the estate is standardising on**, and the timeline for scaffold
  PR #246 including whether `test-node` is in scope (ADR 8, ADR 12).
- **Is `#212` merging, and when** (ADR 14).
- **Is LEZ's transitive LGPL-3.0 dependency an accepted position for the estate**
  (ADR 1) — and, separately, three Logos crates ship without a `license` field, which a
  one-line manifest change would fix for every downstream consumer (ADR 9).

Two are settled locally, with a measurement in M2: which clock account an oracle should
read (ADR 13), and the CI ceilings that stop the accelerator configuration regressing
silently (ADR 6, landing with M1-25).
