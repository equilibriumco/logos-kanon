# [M3-08:02]. The guests are compiled in a container, so a program id is a property of the source

- **Status**: accepted
- **Milestone**: M3 (`M3-08`), as the second decision that task's review produced.
- **Artefacts**: `methods/build.rs`, `README.md`
- **Replaces**: the build-directory qualification in `[M3-08:01]`, corrected there in place
  because M3 is unmerged

## Context

A SPEL program's id *is* its RISC Zero image id, and every account the program owns is a
PDA derived from it. `[M3-05:01]` records what a moved id costs: the old accounts go
dead, every open order is abandoned, and there is no migration path. `[M3-08:01]` added
that a dependency edge added to a guest workspace can move one, and set a rule about
re-measuring before such a change lands.

Reviewing that rule turned up something the rule could not survive. **A guest's image id
depends on the absolute path the repository is checked out at.** Same commit, one
machine, one toolchain, `--locked`: two checkouts at two directories produce three
different product ids.

The mechanism is ordinary once seen. Cargo derives `-C metadata` from a hash of the
package id, and a package id's source id is location-independent for registry and git
sources but *is the absolute manifest path* for a path dependency. That hash becomes
rustc's `StableCrateId`, which appears in every v0 mangled symbol name, which changes the
ELF, which changes the image id. Diffing two such ELFs shows it exactly: same size to the
byte, and the only differences are 178 symbol names whose crate disambiguators moved —
for `aggregator`, `aggregator_program`, `kanon_clock` and `verifier_core`, our four path
crates, and for nothing else. Every registry crate is stable, and so are the git
dependencies. The path never appears in the binary, only its hash, which is why grepping
for it finds nothing.

Nothing shipped was wrong. CI always builds at one runner path, so it reproduces itself.
What was broken is **independent verification**: anybody rebuilding a commit to check a
deployed program's id computes a different id, and therefore different account addresses,
unless they happen to reproduce our directory. That is the property a reader of
`m0/versions.md` question 6 was promised and it did not hold — and it fails across two
directories on one machine, which is a far weaker condition than the two-machine case
that question proposed testing.

## Decision: the guests are built in risc0's container

**`methods/build.rs` uses `embed_methods_with_options` with `use_docker` for all three
guest packages, at a pinned image tag.**

`risc0-build` already solves this and we were not using it: its docker path copies the
tree to a fixed `/src` and builds there, so manifest paths, and with them the metadata
hashes, stop depending on the host. Its existence is the strongest available statement
that the plain `embed_methods()` path is not intended to be reproducible.

Verified rather than inferred from that reasoning: two checkouts at two paths, built this
way, produce all three product ids byte-identical.

### The pinned tag is the load-bearing half

`risc0-build` defaults to `r0.1.88.0`. Taking the default would swap the guest compiler
under every figure in `COSTS.md`, all measured on guest rustc 1.97.0, and turn a change
about *where* a guest is compiled into a change about *what* compiles it. Pinned to
`r0.1.97.0` — the tag carrying that compiler — all 31 cost assertions pass unchanged.

The tag and ADR 8's pins say the same thing and have to move together. A future toolchain
bump is now two edits, and a mismatch between them is a re-measurement nobody asked for.

### `KANON_GUEST_BUILD=host` remains

The builder image is x86_64 linux, so without an escape a developer on another
architecture could not build at all. ADR 34 kept `KANON_SEQUENCER_RUNTIME=host` for the
same reason, and this follows it. The build script warns when the escape is used, because
what a host build must not be used for is anything published.

Cycle figures are unaffected either way: they are a property of the instructions, and the
difference between the two routes is confined to symbol names.

*Rejected: `--remap-path-prefix`.* The obvious first idea and it cannot work. It is a
rustc flag that rewrites paths in diagnostics and debug info; `-C metadata` is computed by
cargo before rustc is invoked. `risc0-build` also overwrites `CARGO_ENCODED_RUSTFLAGS` for
the guest build, so the only injection point would be `[package.metadata.risc0]`
`rustc-flags`, and it would not help if there were one.

*Rejected: a bind mount at a canonical path.* It fixes the metadata hash and stops there.
The guest ELF also embeds 19 `$CARGO_HOME` paths — panic locations from `k256`,
`risc0-zkvm`, `spel-framework`, `tiny-keccak` and others — so an id built that way is
still a function of whose home directory built it. Both experiments that found the path
dependence were run on one machine with one `$CARGO_HOME`, which is why this second cause
stayed invisible. A container normalises it too, because it carries its own
`/root/.cargo`.

*Rejected: pin the ids in a test and leave the build alone.* Records the symptom. A pin
would also only hold in the directory that recorded it, which is what `[M3-08:01]`
concluded and what made that guard the expensive one.

## Consequences

- **The three product ids change once, here.** They are now what a container at
  `r0.1.97.0` produces from this source, and reproducible by anybody with that image:

  | program | id |
  | --- | --- |
  | `AGGREGATOR_ID` | `[971849553, 3084464373, 3244420584, 96452387, 1372981235, 4041411305, 576478209, 3732546978]` |
  | `PULL_CONSUMER_ID` | `[2448652319, 1874296164, 1455065431, 1890407805, 3407326031, 2980391474, 692379260, 3920251675]` |
  | `AGGREGATOR_READ_CONSUMER_ID` | `[1881734432, 4067271232, 2539781409, 540757059, 4239726867, 2496739987, 1054066166, 1376984117]` |

- **`[M3-08:01]`'s rule loses its qualification and keeps its precondition.** "Compare
  within one build directory" is obsolete: comparing across directories is now the point.
  "Wipe `target/riscv-guest` first" still holds, because a stale ELF from another branch
  is a false pass whatever compiles it.
- **Docker becomes a requirement for building this repository**, as it already is for the
  sequencer harness (ADR 34). The escape hatch exists and is signposted.
- **CI needs no new step, and one that looks redundant is not.** The guest jobs already
  install the rzup guest toolchain, and it is still required: `risc0-build` reads the
  installed toolchain version to compute the guest rustflags, on the docker path as much
  as the host one, and panics if it is absent. Removing that install because "the
  container has a compiler" would break the build.
- **A clean containerised build is slower**, measured at 5m 42s including the image pull
  and 3m 53s after. The runner cache covers `target/`, and the ELFs land under it, so the
  cost falls on cold caches rather than on every run.
- **What this does not settle is the cross-machine question.** Both causes found here were
  measured on one machine, and a container plausibly closes both, but `m0/versions.md`
  question 6 asked about two machines and this evidence does not answer that. It is now a
  cheaper experiment than it was: two people, one image tag, ids compared.
