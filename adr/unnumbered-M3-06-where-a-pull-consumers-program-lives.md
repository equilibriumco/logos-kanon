# [M3-06:01]. Where a pull consumer's program lives

- **Status**: accepted
- **Milestone**: M3 (`M3-06`)
- **Requirements**: F9, U7
- **Artefacts**: `reference-consumers/pull/guest/`, `methods/Cargo.toml`, `.github/workflows/ci.yml`

## Context

F9 says a consumer using only pull integrates without registering a feed against the
aggregator, and M3-02 decided that no test can hold that claim: the way it would be lost
is an addition that compiles and passes everything. So the claim is a gate on the
resolved build closure, pinned to an allow-list, and `pull-lib` has satisfied it since
M3-02.

U7 now asks for the consumer itself — a program, not a library — and a SPEL program is a
guest binary, because that is what LEZ executes. `methods/guest` is where this
repository's only guest binary lived, so the obvious place for a second one was beside
the first.

That would have put the artefact the requirement is about outside the gate. `cargo tree`
resolves a package, not a binary, and `methods/guest` depends on `aggregator-program`
and `kanon-idl` for the aggregator binary it already holds. A pull consumer there would
sit in a closure containing the push aggregator and the canonical price account, with no
walk able to distinguish "this binary does not use them" from "this binary does". F9
would be enforced on the library and unenforceable on the program.

## Decision

**The consumer's SPEL program is its own guest workspace, at
`reference-consumers/pull/guest`, and `pull-independence` walks it.**

Three walks now: `pull-lib`, the consumer's logic in the root workspace, and the program
in its own. The third is the one this decision exists for, and it comes back as
`crypto-bigint`, `k256`, `kanon-clock`, `lee_core`, `pull-lib`, `reference-consumer-pull`,
`reference-consumer-pull-guest`, `sha2`, `spel-framework`, `spel-framework-core`,
`spel-framework-macros` and `verifier-core`. Adding `aggregator-program` to that manifest
puts `aggregator-program`, `kanon-idl` and `twap_oracle_core` in the closure and fails the
diff; the same injection into the logic crate fails the second walk with the last two.
Both directions were run.

**What keeps the canonical price account out is narrower than it looks, and worth naming
because it is easy to break by accident.** `twap_oracle_core` reaches this repository
through `kanon-idl` and through nothing else — not through SPEL, not through LEZ. So a
consumer program can take `lee_core` and `spel-framework`, which no program can do
without, and still have no path to the account the push mode writes. `kanon-idl` is the
one dependency a pull consumer must refuse, and it is refused by the allow-list rather
than by anybody remembering.

**One methods crate builds every guest.** `risc0-build` resolves each entry in
`package.metadata.risc0.methods` against the methods crate's own directory, so an entry
like `"../reference-consumers/pull/guest"` reaches out of the tree and no second build
script is needed. `[M3-05:01]` adds the third entry the same way.

*Rejected: a binary under `methods/guest`.* Cheapest by every other measure — no
manifest, no lockfile, no CI line — and it costs the requirement. The gate would report
green on a program that had linked the aggregator.

*Rejected: a second methods crate.* It is the shape `m0/` uses, one methods crate per
guest workspace, and `m0/` needs it because those three build the *same* guest source
under three different `[patch.crates-io]` blocks and each has to hand out its own ELF.
Nothing here does that: a patch block belongs to a guest workspace rather than to the
methods crate above it, so guests wanting different patches is not a reason to split
one. `[M3-05:01]`'s guest is the demonstration -- it pins `sha2` alone, where the two
verifying guests pin three, and still needs no methods crate of its own.

*Rejected: shipping the logic in M3-06 and the program in M3-07.* M3-07 is the sequencer
run, and a task that has to build the program before it can run it is a task doing two
things. It would also have left the closure question open for another PR, which is the
question the whole M3-02 gate exists to keep closed.

## Consequences

- **A third lockfile, and a `[patch.crates-io]` block that is duplicated on purpose.**
  `reference-consumers/pull/guest/Cargo.lock` is committed and joins the `licenses` job's
  loop and the `fmt` step's list. The patches are the same three `methods/guest`
  declares — accelerated secp256k1 recovery, software keccak256 — and they have to be,
  or the pull-side cycle figures M3-08 publishes would not be comparable with the
  push-side ones. Guest workspaces cannot share a `[patch.crates-io]`, so the pins are
  repeated and each manifest says why.
- **The generated code is tested, which the ELF build does not do.** `#[lez_program]`'s
  dispatcher, the account constraints and the validator exist only after expansion, so
  no test in the root workspace can reach them and the cross-compile proves only that
  they compile. `cargo test --manifest-path reference-consumers/pull/guest/Cargo.toml
  --bin pull_consumer` is what runs them, and the `guest` job runs the equivalent
  for every guest.
- **The instruction enum lives in the library, as the aggregator's does.** The macro
  points at `reference_consumer_pull::Instruction`. It first generated the enum in the
  guest instead, on the argument that a reference consumer had no host-side caller in
  this repository to share a definition with -- true when this was written and no longer
  true: M3-07 drives this program across a standalone sequencer, and a test in another
  workspace cannot reach a type the macro creates inside a guest binary. Hand-encoding
  the instructions there would have been the second definition that argument was
  avoiding, arrived at from the other side.

  It costs an ordering invariant. While the macro generated the enum, a handler and its
  variant could not disagree; now the declaration order is a human's to keep, and it is
  the whole encoding, because `risc0_zkvm::serde` writes a variant's position. Swapping
  two declarations compiles on both sides and sends every caller's `RotateSigners` to
  `RegisterFeedTrust`, and so does reordering two same-typed fields inside one variant,
  which shifts values between arguments. `tests/instruction_parity.rs` is what fails
  instead, asserting both orders against the encoding a caller produces -- risc0's
  discriminant and its serialised field order -- rather than against a restatement of
  them. Its helpers are `aggregator-program`'s, which had the same two declarations to
  reconcile first.

  It also drops the guest workspace's `serde` dependency, which existed only because the
  generated enum derived it, and it adds `instruction_type` to the committed IDL -- so a
  client is now told the type it encodes against rather than inferring it.
- **The consumer's IDL sits beside the consumer.** `kanon-idl` is where the aggregator's
  goes, and it is the one crate a pull consumer must not depend on, so
  `reference-consumers/pull/pull-consumer-idl.json` is next to the program it describes.
  A JSON file needs no dependency to sit anywhere.
