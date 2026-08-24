# 28. `lgs build` runs in CI, triggered by the manifests rather than by every push

- **Status**: accepted
- **Milestone**: M1 (`M1-03`)
- **Requirements**: S2
- **Artefacts**: `scaffold.toml`, `.github/workflows/lgs-build.yml`

## Context

M1's gate asks for `lgs build` green in CI. It was left open on the strength of ADR 12 —
which declined `lgs test-node`, because scaffold generates a v0.1.2-shaped
`sequencer_config.json` that cannot start the v0.2.x sequencer this repository is built
against. `build` writes no sequencer config, so that break never applied to it. Nobody
had run the command the gate names.

Run, it exits 0 at the revisions the product already resolves, needing only a
`scaffold.toml` for `lgs` to recognise a project here. What it proves is narrow:
`lgs build` ends in `cargo build --workspace` and a release build of `methods/`, both of
which `ci.yml` already runs. The addition is that the toolchain still accepts the layout.

## Decision

**Run it on the manifests, not on every push**, and without a cache. `lgs build` calls
`lgs setup` first, which builds LEZ's sequencer, wallet and SPEL from source and uses
none of them; the workflow header records why `--prebuilt` does not avoid this. Layout
compatibility can only break when a manifest, the lockfile or `scaffold.toml` changes, so
that is the trigger. Caching it would put 4.5 GB against a 10 GB per-repository budget
and evict the caches `ci.yml` and `guardrails.yml` hit on every push, to re-assert
something that cannot have changed.

**`scaffold.toml` is minimal**, and its two pins are checked against `Cargo.lock` in the
same job. A second place for a LEZ revision to be wrong is what ADR 12 removed by
deriving it from the lockfile; the check is where it cannot come back.

**`lgs test-node` stays unused.** ADR 12 is unchanged, and this decision does not reach it.

## Consequences

- The gate is met by the command it names.
- A red tick here means the layout drifted from what `lgs` expects. Nothing else this
  workflow can fail would not have failed in `ci.yml` first.
- The job is expensive when it runs and rarely runs, which is the trade. `lgs` is pinned
  at 0.3.0, so moving it is a deliberate edit here rather than an upstream release.
- If LEZ starts publishing prebuilt sequencer tags, setup collapses to a download and
  this could move into `ci.yml` on every push. That is upstream's to change.

## Alternatives considered

- **Every push, with the scaffold cache restored.** Rejected on the cache budget above.
- **Ask for the gate to be amended**, on the grounds that the sequencer harness covers
  the same ground. Rejected once the command was run: the blocker did not exist.
- **Publish a prebuilt scaffold cache per pin to ghcr**, as `lez-sequencer-image.yml`
  does for the sequencer. More machinery than this cadence needs; the answer if the job
  ever has to run on every push.
