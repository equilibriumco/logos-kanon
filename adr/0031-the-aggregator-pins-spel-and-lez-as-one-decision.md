# 31. The aggregator pins SPEL and LEZ as one decision, spelled the way the graph already resolves them

- **Status**: accepted
- **Milestone**: M2 (`M2-01`)
- **Requirements**: F1
- **Artefacts**: `Cargo.toml`, `aggregator-program/Cargo.toml`, `methods/guest/Cargo.toml`, `scaffold.toml`

## Where the answers came from

`Cargo.lock`, which already resolved `spel-framework-core` at
`tag=v0.6.0#0cb7e098` through `twap_oracle_core`, and `spel-framework`'s own manifest,
which takes LEZ at `tag = "v0.2.0"` under the name `nssa_core`.

## Context

`m0/versions.md` carried the SPEL version as M2's open question, and framed it as a
choice between the v0.1.2 the default scaffold vendors and the v0.2.0-rc3 a newer one
does. That framing was a year stale. SPEL is at v0.6.0, `scaffold.toml` already pins
`0cb7e098`, and — the part that removes the choice — `Cargo.lock` already resolves
`spel-framework-core` at that same commit, because `twap_oracle_core` uses the
framework's `#[account_type]` macro. The canonical price account brought SPEL into the
graph before the aggregator existed.

So the aggregator was never choosing a SPEL version. It was choosing whether to agree
with the one already there.

Disagreeing is worse than it looks, and for a reason `Cargo.toml` already documents
about LEZ. `spel-framework` depends on LEZ as
`nssa_core = { git = "...logos-execution-zone.git", tag = "v0.2.0", package = "lee_core" }`
— the same crate this repository calls `lee_core`, renamed. Cargo unifies the two only
when source and tag match character for character, and `AccountId` is defined there. A
SPEL revision built against a different LEZ tag would put two `AccountId` types in the
guest, and the price account would stop being the account consumers read.

That coupling runs the other way too. LEZ cannot be bumped for the product without a
SPEL that agrees with it, because SPEL names its LEZ tag inside its own manifest and
nothing here can override it.

## Decision

**SPEL is pinned by `tag = "v0.6.0"`, spelled identically everywhere it appears**, which
is what `twap_oracle_core` already resolves and what `scaffold.toml` already pins.

**A SPEL bump and a LEZ bump are one decision.** Moving either means checking that the
SPEL revision names the LEZ tag the product uses, and moving both together when it does
not. ADR 8's "the product tracks the estate" is what says when to move; this records
that the two pins cannot move independently.

**The question `m0/versions.md` carried is closed, not answered.** There was no fork to
resolve — the graph had already decided, and the alternative was incoherence rather than
a trade-off.

## Consequences

- **The instruction set is declared twice, and SPEL joins the halves positionally.**
  `spel-cli` encodes an instruction's discriminant as its index in `idl.instructions`,
  which comes from the guest function order; the guest decodes it as the declaration index
  of `Instruction`. Nothing in the framework ties the two, so two variants of the same
  shape could be swapped in one place only, and a caller's `deregister_feed` would arrive
  at `pause_feed`. `tests/instruction_parity.rs` asserts both the discriminant and the
  per-variant argument order against `risc0_zkvm::serde`, which is the codec
  `read_lee_inputs` uses, so the assertion is about the transaction rather than about a
  restatement of it.
- **`Instruction` derives serde and not Borsh**, because serde is what the wire uses. A
  Borsh derive would offer a second encoding of the same type that nothing reads, and whose
  discriminant is a `u8` where the wire's is a `u32` word.
- **`aggregator-program` depends on the framework, for the IDL generator rather than for
  the program.** `#[account_type]` is a no-op the generator reads and the instruction enum
  needs no runtime, so the macro crate alone would do; `generate_idl!` expands to a `main`
  that names `spel_framework`, and a binary cannot take that as a dev-dependency. The cost
  is small because the framework's own dependency, LEZ, is in the graph already through
  `kanon-idl`. A binary and not an example, so `tests/idl.rs` can reach it through
  `CARGO_BIN_EXE_generate-idl` and keep the check in `cargo test`.
- **`spel-framework` forces LEZ's `host` feature on, and the guest builds anyway.**
  Its `nssa_core` dependency carries `features = ["host"]` unconditionally, which pulls
  `chacha20`, `ml-kem` and `crypto-common` — and through them `getrandom`, which has no
  backend for `riscv32im-risc0-zkvm-elf`. Built through `risc0-build`, as `kanon-methods`
  does and as CI does, the guest compiles: the toolchain supplies the backend. Built by
  invoking `cargo` against the target by hand it does not, which is a property of the
  build path rather than of the code, and worth knowing before diagnosing it as breakage.
- **The licence gate needed a third clarification, and a root-only run does not show it.**
  `spel-framework` declares no `license` field, like the two SPEL crates `deny.toml`
  already clarifies. Against the root workspace `cargo deny` resolves it from the
  checkout's licence files and passes; against the guest lockfile it reports
  `unlicensed` and fails. Running the gate over every workspace, as `licenses.yml` does,
  is what makes the difference visible, and it is why a green local run of the root
  workspace is not evidence the gate passes.
- **`scaffold.toml`'s SPEL pin is now load-bearing rather than decorative.** ADR 28 has
  `lgs-build.yml` checking the scaffold pins against `Cargo.lock`; the SPEL line is now one
  the aggregator depends on, so a pin bump that misses either place fails there.

## Alternatives considered

- **Take the newest SPEL and let cargo resolve.** Rejected: `twap_oracle_core` pins its
  own, so a newer tag adds a second SPEL and a second LEZ rather than replacing the first.
  The failure is a type mismatch in the guest, not a version warning.
- **Vendor the parts of SPEL the aggregator uses.** Removes the coupling and keeps the
  guest's dependency graph small. Rejected because the framework's value is that Logos
  maintains it against LEZ: a vendored copy would have to be re-checked against every LEZ
  bump by hand, which is the work the pin does mechanically.
- **Wait for the SPEL question to be settled with Logos before building.** The question
  was recorded as outstanding, so this was the procedurally correct-looking option.
  Rejected because the graph had already answered it, and holding M2-01 for confirmation
  of something `Cargo.lock` states would have stalled the milestone's critical path.
