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
ELF, which changes the image id. Diffing two such ELFs shows it exactly: the only
differences are 178 symbol names whose crate disambiguators moved — for `aggregator`,
`aggregator_program`, `kanon_clock` and `verifier_core`, our four path crates, and for
nothing else. The two happened to be the same size, which is not a property of the
mechanism and should not be read as one: symbol name lengths shift when the index digits
in `.Lanon.<hash>.N` do, and another reviewer's pair differed by eight bytes. Every registry crate is stable, and so are the git
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

### The pin is by digest, and it is the load-bearing half

`risc0-build` defaults to `r0.1.88.0`. Taking the default would swap the guest compiler
under every figure in `COSTS.md`, all measured on guest rustc 1.97.0, and turn a change
about *where* a guest is compiled into a change about *what* compiles it. Built on
the image carrying that compiler, `r0.1.97.0`, all 31 cost assertions pass unchanged.

**A tag is not a pin.** It is a mutable pointer, so two builds of this commit far enough
apart could resolve one to different image contents and produce different program ids —
the failure this decision exists to prevent, arriving slowly rather than immediately. The
image is therefore pinned by digest, spelled `r0.1.97.0@sha256:7ae0a27f…`: Docker accepts
`repo:tag@digest` and resolves the digest, so the tag survives for legibility and decides
nothing.

**And a pin `risc0-build` will ignore is not a pin either.** `RISC0_DOCKER_CONTAINER_TAG`
is consulted *ahead* of the configured image, so the build script refuses to run when that
variable is set to anything else, rather than quietly building a different program and
publishing its ids as these. Both it and the escape hatch are declared
`rerun-if-env-changed`, so changing either rebuilds instead of serving a stale ELF.

The digest and ADR 8's pins say the same thing and have to move together. A toolchain bump
is now two edits, and a mismatch between them is a re-measurement nobody asked for.

### `KANON_GUEST_BUILD=host` remains

The builder image is x86_64 linux, so without an escape a developer on another
architecture could not build at all. ADR 34 kept `KANON_SEQUENCER_RUNTIME=host` for the
same reason, and this follows it. What a host build must not be used for is anything
published, and `the_guests_were_built_in_a_container` is what says so, because the build
warning this started with is lost in a CI log and easier to lose locally.

**The two escapes are aimed at the same machines, and together they are a position worth
stating once.** ADR 34's exists because the published sequencer image is x86_64 glibc and
will not run on NixOS or a non-x86_64 host; this one exists because the guest builder
image is x86_64 linux. So for a developer on such a machine the combined answer is:
Docker is required to build a guest reproducibly, the published sequencer image is not
usable, and both escapes are theirs to take — a host guest build whose ids are their
own, and a host sequencer. Neither is a degraded mode for CI, which is x86_64 and takes
neither. Written down here rather than left to be discovered by whoever is next on one
of those machines.

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
  that digest produces from this source, and reproducible by anybody who pulls it:

  | program | id |
  | --- | --- |
  | `AGGREGATOR_ID` | `[971849553, 3084464373, 3244420584, 96452387, 1372981235, 4041411305, 576478209, 3732546978]` |
  | `PULL_CONSUMER_ID` | `[2448652319, 1874296164, 1455065431, 1890407805, 3407326031, 2980391474, 692379260, 3920251675]` |
  | `AGGREGATOR_READ_CONSUMER_ID` | `[1881734432, 4067271232, 2539781409, 540757059, 4239726867, 2496739987, 1054066166, 1376984117]` |

- **`[M3-08:01]`'s rule loses its qualification and keeps its precondition.** "Compare
  within one build directory" is obsolete: comparing across directories is now the point.
  "Wipe `target/riscv-guest` first" still holds, because a stale ELF from another branch
  is a false pass whatever compiles it.
- **Docker with buildx becomes a requirement for building this repository**, as docker
  already is for the sequencer harness (ADR 34). The plugin is not incidental:
  `risc0-build` runs `docker build --output`, which only BuildKit provides, and the
  legacy builder's refusal names the flag rather than the cause. CI cannot catch that --
  GitHub's runners ship buildx, so the requirement is invisible exactly where it is
  exercised and visible only outside it, which is where it was found. `build.rs` checks
  for it before anything else and fails naming it.
- **Both lockfiles move.** `e2e/` resolves `kanon-methods` in its own workspace (ADR 7,
  `[M2-19:01]`), so the `toml` edge lands in `e2e/Cargo.lock` as well, and a `--locked`
  build fails on whichever of the two is forgotten.
- **CI needs no new step, and one that looks redundant is not.** The guest jobs already
  install the rzup guest toolchain, and it is still required: `risc0-build` reads the
  installed toolchain version to compute the guest rustflags, on the docker path as much
  as the host one, and panics if it is absent. Removing that install because "the
  container has a compiler" would break the build.
- **A clean containerised build is slower**, measured at 5m 42s including the image pull
  and 3m 53s after. The runner cache covers `target/`, and the ELFs land under it, so the
  cost falls on cold caches rather than on every run.
- **A container inherits no environment, and the guests read some.** Each program takes
  its genesis authority through `option_env!`, which resolves against the compiler's
  environment — and the compiler is now inside a container that inherits nothing. Left
  alone, every guest would have built without one, compiled cleanly, and refused every
  transaction at execution time with `this build configured no genesis authority`, which
  is how CI found it. The build script forwards what the guests read, discovered by
  scanning them for the call rather than by keeping a list, for the same reason the guest
  packages are derived: an authority reaching one guest and not another is a difference
  nothing would announce. Only variables actually set are forwarded, so an unset one
  still takes `option_env!`'s `None` branch exactly as on the host, and the ids above are
  the ones an unconfigured build produces.
- **The guest list is derived, not restated.** `risc0-build` applies default — host —
  options to any guest absent from the map it is handed, so a list of guests beside
  `[package.metadata.risc0]` would be a second copy whose disagreement is silent: a fourth
  guest added there and not here would compile outside the container and have a directory
  in its id again, with nothing to say so. The build script reads that metadata instead,
  which costs one dependency edge on `toml`, already in the graph, and no new package.
- **It does not fit on a stock runner, and the failure is not legible.** The builder image
  is 1.73 GB compressed, and the container resolves each guest workspace's dependencies
  inside itself, on top of whatever the job already needs. The first CI run failed twice
  on that one cause and only one of them said so: `no space left on device` unpacking the
  image in one job, and `ld terminated with signal 7 [Bus error]` in another, which is
  what a truncated mmap looks like when the disk fills under a linker. Every job that
  builds `kanon-methods` now reclaims the runner's preinstalled SDKs first — the guest
  build, the end-to-end suite and the cost table in `ci.yml` and `guardrails.yml`, and
  `lgs build`, which reaches the same image through a release build of `methods/`.
  Anybody adding another needs that step, and will otherwise be debugging a linker
  crash.
- **`m0/versions.md` question 6 is answered, affirmatively and by measurement.** That
  question asked whether two machines at the same pins produce the same guest ELF, and
  said nothing in this repository established it. A review of this branch built it on a
  second machine and got all three product ids identical to the ones recorded above,
  which were produced here — so a release can be independently verified by rebuilding it,
  which is the property the deployment story had been assuming without evidence. Both
  halves are measured: the answer was *no* before this decision and *yes* after it. What
  it does not extend to is a third architecture, since the builder image is x86_64.
