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

- **The framework is split across three manifests, by what each needs.**
  `aggregator-program` takes `spel-framework-macros` only, because `#[account_type]` is a
  no-op the IDL generator reads and the instruction enum needs no runtime. The guest takes
  the full `spel-framework`. The IDL generator takes it as a dev-dependency, because the
  macro expands to code that names it.
- **`spel-framework` forces LEZ's `host` feature on, and the guest builds anyway.**
  Its `nssa_core` dependency carries `features = ["host"]` unconditionally, which pulls
  `chacha20`, `ml-kem` and `crypto-common` — and through them `getrandom`, which has no
  backend for `riscv32im-risc0-zkvm-elf`. Built through `risc0-build`, as `kanon-methods`
  does and as CI does, the guest compiles: the toolchain supplies the backend. Built by
  invoking `cargo` against the target by hand it does not, which is a property of the
  build path rather than of the code, and worth knowing before diagnosing it as breakage.
- **The licence gate needed no new entry.** `spel-framework` declares no `license` field,
  like the two SPEL crates already clarified in `deny.toml`, but `cargo deny` resolves it
  from the LICENSE files at the checkout root. The two existing clarifications stay
  because they are still what covers the crates that have none.
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
