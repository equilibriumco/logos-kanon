# 12. A standalone LEZ sequencer without `lgs`, run in CI from a prebuilt image

- **Status**: accepted
- **Milestone**: M1 (`M1-04a`, `M1-05`)
- **Requirements**: S2
- **Artefacts**: `scripts/lez-sequencer.sh`, `.github/workflows/lez-sequencer-image.yml`, `ci.yml` job `sequencer`

## Context

RFP-020 Supportability 2 asks for end-to-end integration tests against a LEZ sequencer
in **standalone mode, in CI**.

The obvious route is the `lgs` toolchain's `test-node`, and it does not work at the
revision this repository is built against. A spike established why, reproducibly from
`logos-co/scaffold` at `9fcc3766`:

- `lgs` pins LEZ v0.1.2 and scaffold generates a **v0.1.2-shaped**
  `sequencer_config.json`.
- v0.2.x replaced `genesis_id`, `is_genesis_random`, `initial_accounts` and
  `initial_commitments` with a single tagged `genesis` array plus
  `bedrock_config.funding_key`. Scaffold requires a numeric `genesis_id`, and its state
  seeding writes `initial_public_accounts` / `initial_private_accounts`, which no longer
  exist.
- `lgs test-node prepare --lez-ref 15144ddb…` **builds** a v0.2.1 sequencer in 2m02s, so
  this is not a deep API break. `lgs test-node start` will not start it.
- `localnet` is unaffected — `prepare_sequencer_config` discards `genesis_id`.
- Upstream scaffold #240 / PR #246 covers `setup`, `run` and `doctor`, but not
  `test-node`.

So `lgs` forced a choice between the revision the code is built against and the revision
the toolchain pins. That is a version-skew hazard, and it was blocking M1-05.

There was a second, independent problem: **CI cannot build LEZ.** A cold build ran past
90 minutes on a standard runner, and because a timeout cancels the job before
`actions/cache` saves, the cache could never warm up. Every run was cold and every run
was killed.

## Decision

**Skip scaffold.** `scripts/lez-sequencer.sh` runs the two services LEZ itself runs: a
Bedrock node from the image LEZ's own `bedrock/docker-compose.yml` names, and
`sequencer_service` built from the LEZ checkout, started against the debug config that
ships in that checkout. This is the shape of LEZ's own integration-test harness, and the
approach `logos-co/eth-lez-atomic-swaps` arrived at downstream in
`canary/lib/localnet.sh` — not something invented here.

**The revision comes from the committed lockfiles, not from configuration.** The script
scans every *product* `Cargo.lock` and refuses to run if they disagree, so the sequencer
is always built from the commit `lee_core` resolves to. That is the reconciliation
M1-04a asks for: the mismatch that made `lgs` unusable now has no place to enter.
`m0/`'s lockfile is deliberately skipped — it pins the LEZ its published figures were
measured against, which is no longer the one the product tracks (ADR 7, ADR 8), so
scanning it would be a permanent false alarm.

**CI pulls an image rather than building.** `lez-sequencer-image.yml` builds the
sequencer once per LEZ revision and publishes
`ghcr.io/equilibriumco/kanon-lez-sequencer:<rev>`; `fetch` unpacks it into the same
layout `build` would have produced, so `start`, `smoke` and `stop` do not care which was
used. Bedrock was already an image; the sequencer is now treated the same way. **That
workflow must be run after a pin bump** — CI fails naming it rather than starting an
hour-long build.

**The script asks the binary what it accepts.** v0.2.1's `sequencer_service` takes
`--listen-address` and `--home`; v0.2.0 takes neither and rejects an unknown flag
outright, which is how CI found out about the pin move in ADR 8. Rather than assume,
`start` feature-detects both flags via `--help`, and treats a failure to run `--help` as
an error rather than as "no flags", so a broken binary cannot silently produce a
misconfigured sequencer.

## Consequences

- **M1-05 is not blocked on the LEZ pin decision.** Nothing here reads a scaffold pin.
  The open version question reverts to what it always was — SPEL and program deployment
  — which is M2's problem.
- **Bedrock is not optional.** The sequencer blocks in `SequencerCore::start_from_config`
  on Bedrock's `/time/info` and never opens its RPC port without it. That is a startup
  dependency and nothing more. It is not a `maxAge` time source; that is the clock
  program (ADR 13).
- CI's `sequencer` job asserts the sequencer serves RPC and produces blocks. Adaptor
  traffic across it arrives with M2-19 and M3-07; the harness standing is the M1 half.
- A pin bump now has a manual step: publish the image, then CI passes. The failure is
  explicit and names the workflow.
- The build cache lives outside the repository, keyed by revision, so a pin bump does not
  overwrite the previous multi-gigabyte checkout.
- A 400-line bash harness is now maintained here, duplicating part of what `lgs` is meant
  to do. If scaffold's `test-node` gains v0.2.x support, this decision should be revisited.

## Alternatives considered

- **Use `lgs test-node`.** Blocked; documented above.
- **Downgrade to LEZ v0.1.2 to match `lgs`.** Rejected: it would invalidate M0's figures
  and move the product away from the estate it interoperates with.
- **Build LEZ in CI with a cache.** Rejected empirically — the timeout cancels the job
  before the cache is saved, so it can never warm.
