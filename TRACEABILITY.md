# Requirement traceability

Every RFP-020 requirement, what implements it, and what verifies it.

**Generated. Do not edit.** The sources are `traceability/requirements.toml`
(the inventory, transcribed from RFP-020) and `traceability/matrix.toml`
(the mapping). Regenerate with:

```sh
cargo run -p traceability --bin render-traceability
```

`cargo test -p traceability` fails if this file and those sources disagree,
if any evidence below names a test, CI job or document that does not exist,
or if a task id written anywhere in the repository appears in no row. That
check is the deliverable, not the table: an unverified matrix drifts into
fiction by the third milestone.

Statuses are `planned` (nothing implemented yet), `partial` (partly
satisfied, or satisfied by M0 evidence that later tasks extend) and
`verified` (satisfied now, with a test or a CI gate behind it, never a
document alone). M5-06 is the task that has to leave every row `verified`.

Not tracked here: the Servicing obligations from the proposal's *Servicing
and SLA* section. They are contractual rather than code and have no test to
name; monthly operating reports are their evidence.

Right now: **0 verified, 11 partial, 25 planned**, of 36 requirements.

## Functionality

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **F1** | partial | `M1‑10`, `M1‑11`, `M2‑01`, `M2‑02` | `verifier‑core`, `aggregator‑program`, `methods/guest` | `the_trait_is_object_safe`, `a_signature_round_trips_to_the_signing_key_address`, `keccak256_matches_the_known_digest_of_the_empty_input` |
| **F2** | planned | `M1‑01`, `M1‑10`, `M1‑19`, `M3‑01` | `verifier‑core`, `pull‑lib` | — |
| **F3** | partial | `M1‑13`, `M1‑23` | `verifier‑core` | `three_of_three_signers_reporting_the_same_price_verifies`, `the_threshold_boundary_accepts_at_m_and_rejects_at_m_minus_one`, `one_signer_cannot_reach_the_threshold_alone_by_repeating_the_feed` |
| **F4** | partial | `M1‑08`, `M1‑12`, `M1‑14`, `M1‑15`, `M1‑16`, `M1‑17`, `M1‑21` | `verifier‑core` | `the_signed_span_covers_the_points_and_the_three_trailing_fields`, `truncation_at_every_length_is_rejected_rather_than_panicking`, `trailing_bytes_before_the_packages_are_rejected` |
| **F5** | planned | `M1‑09`, `M1‑20`, `M1‑26`, `M2‑02` | `aggregator‑program` | — |
| **F6** | planned | `M1‑07`, `M2‑06`, `M2‑07`, `M2‑08`, `M2‑09`, `M2‑10` | `aggregator‑program` | — |
| **F7** | planned | `M2‑00`, `M2‑11`, `M2‑12` | `aggregator‑program` | — |
| **F8** | planned | `M4‑01`, `M4‑02`, `M4‑03`, `M4‑04`, `M4‑05`, `M4‑06`, `M4‑07`, `M4‑08` | `kanon‑relayer` | — |
| **F9** | planned | `M3‑01`, `M3‑02`, `M3‑03`, `M3‑04` | `pull‑lib` | — |

**F1** — Public-mode push aggregator accepting signed RedStone data packages, with the signature path structured so swapping in a host precompile stays a localised change

Two halves. The verification half has landed: the `VerifierBackend` trait (M1-10) is the localised-swap structure, and `InProgramBackend` (M1-11) implements it with the same k256 and tiny-keccak M0 measured. The guest patches in the *mixed* accelerator configuration -- recovery accelerated, keccak in software -- which is M0's recommendation rather than a default. The aggregator half is M2.

**F2** — Verification packaged as a library callable both from the aggregator and from third-party consumer programs, so one audited implementation backs both modes

Structural: `verifier-core` depends on neither mode, and both modes depend on it. M3-02 asserts the pull side carries no aggregator dependency.

**F3** — M-of-N signer threshold per feed, configurable at registration, default 3-of-N; reject any package that does not meet it

M1-13 has landed: `verify_feed` enforces M-of-N over distinct authorised signers and rejects a signer occupying two slots. M1-23 covers the boundaries, which is where an off-by-one in a threshold check lives.

**F4** — Decode the RedStone data-package format and reject stale (`maxAge`), zero, negative or otherwise invalid values, and asset identifiers that do not match the registered feed

Four rejection classes in one requirement. The malformed-wire-format one has landed: M1-12 decodes zero-copy, bounds-checks every length, and is tested against truncation at every offset, since a panic in a guest aborts the transaction rather than rejecting the package. Conformance against published vectors is M1-21. Still to come: staleness (M1-15), asset identity (M1-16), value sanity and scaling (M1-17). Staleness needs a clock before it needs a rule: M1-08 is answered, and the answer is LEZ's clock program rather than the Bedrock time service. M1-14 wraps the every-block clock account and M1-15 rejects any other, because a caller-supplied clock makes a stale price look current.

**F5** — Publish the verified price into a canonical RFP-019 price account, populating base and quote asset, price, timestamp, source identifier and a zero confidence interval

M1-09 spikes whether the canonical RFP-019 struct is available at all; M1-20 writes it or a forward-compatible fallback, and M1-26 conforms the layout.

**F6** — Admin authority (RFP-001 via SPEL) can register feeds, update a feed's signer set on roster changes, and deregister feeds

Gated on RFP-001 having merged an admin-authority interface; M1-07 establishes what to build against, M2-06 is the shim if nothing has.

**F7** — BTC/USD, ETH/USD, SOL/USD, XMR/USD and ZEC/USD registered on LEZ devnet/testnet as part of the deliverable

M2-00 first confirms RedStone publishes XMR/USD and ZEC/USD on the chosen data service and captures the authorised signer sets; the two privacy-asset feeds are the ones that cannot be assumed.

**F8** — Relayer as a Logos module with a Logos Core headless daemon: configurable data service and feeds, heartbeat and deviation triggers, parallel gateways with first-success, retry and back-off, structured logging, wallet-balance monitoring, clean shutdown

Equilibrium operates this under the SLA, so its logging, wallet monitoring and shutdown paths are deliverables rather than conveniences.

**F9** — Public-mode pull verification library returning a verified price or a typed error, with no dependency on the aggregator's price account

"Must not depend on the aggregator's price account" is asserted mechanically by M3-02, not left to review.

## Usability

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **U1** | planned | `M4‑11` | `kanon‑sdk` | — |
| **U2** | planned | `M4‑18`, `M4‑19`, `M4‑20`, `M4‑21` | `kanon‑app` | — |
| **U3** | planned | `M4‑13`, `M4‑14`, `M4‑15`, `M4‑16` | — | — |
| **U4** | planned | `M2‑01`, `M4‑12` | `kanon‑idl` | — |
| **U5** | planned | `M4‑17` | — | — |
| **U6** | partial | `M1‑18`, `M1‑22`, `M3‑04` | `verifier‑core`, `pull‑lib` | `a_decode_failure_and_a_signature_failure_stay_distinguishable`, `the_threshold_failure_carries_what_was_reached_and_what_was_needed` |
| **U7** | planned | `M3‑05`, `M3‑06`, `M3‑07` | `reference‑consumers/aggregator‑read`, `reference‑consumers/pull` | — |

**U1** — SDK for building Logos modules in both modes, exposing helpers ergonomic enough that switching modes leaves payload handling unchanged

The mode-switch ergonomics are the testable part: the same payload handling has to serve both paths.

**U2** — Basecamp mini-app dashboard: live prices, signer set, threshold, latest timestamp and staleness per feed, plus a public-mode pull dry-run panel

Built from the M4-18 designs rather than ahead of them; M4-20 is the pull dry-run panel, M4-21 the Basecamp load and flow tests.

**U3** — CLI covering both modes: submit, query the canonical account, register and deregister feeds, update signer sets, and an off-chain pull dry-run reporting the same typed errors as the on-chain library

The dry-run has to report the same typed error codes the on-chain pull library returns, which is what ties M4-16 to M3-04.

**U4** — IDL for the adaptor program and the RFP-019 price-account standard, re-exported rather than forked, via SPEL

Re-exported, not forked: the RFP-019 standard's definition stays RFP-019's.

**U5** — SPEL enhanced so a new program can hook in pull and push feed modes easily

A change to SPEL, exercised from a fresh scaffold program so the hook is shown to work outside this repository.

**U6** — Clear, actionable errors for every failure mode: stale package, threshold not met, unauthorised signer, asset mismatch, malformed package, invalid signature, zero or negative price

One error enum in `verifier-core` is what makes push and pull report the same failure the same way; M3-04 asserts the parity. M1-18 defines all ten `VerifyError` variants now -- nine payload failure modes plus `InvalidConfig` for a bad configuration. Five of the nine have no producer today: `StalePackage`, `AssetMismatch`, `ValueOutOfRange` and `ScalingOutOfRange` await M1-15 through M1-17, and `InvalidSignature` has none by design -- an unrecoverable signature is skipped, not rejected (ADR 15), so a future single-package API is where it becomes reportable.

**U7** — Two reference consumer programs, one per mode, each showing asset-pair verification, staleness handling, typed-error handling and refuse-on-unavailable

Refuse-on-unavailable is the pattern being demonstrated: a consumer must never fall back to an unsafe default.

## Reliability

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **R1** | planned | `M2‑04` | `aggregator‑program` | — |
| **R2** | planned | `M2‑07` | `aggregator‑program` | — |
| **R3** | planned | `M2‑05` | `aggregator‑program` | — |
| **R4** | planned | `M4‑05`, `M4‑08` | `kanon‑relayer` | — |

**R1** — A price read is read-only and never modifies adaptor state

**R2** — Feed registration is atomic: partial failure leaves existing registrations intact

Atomicity is the assertion: a partial registration must leave existing feeds untouched.

**R3** — An upstream error on one feed does not affect push mode for other feeds handled by the same node

**R4** — Temporary upstream errors leave the daemon recoverable, able to push again once feeds return

Fault injection and restart, not just a retry loop: the requirement is about the state the daemon is left in.

## Performance

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **P1** | partial | `M2‑02`, `M2‑19`, `M3‑06` | `m0/lez‑probe` | `a_3_of_n_update_leaves_ample_budget_headroom`, `m0/M0‑report.pdf` |
| **P2** | partial | `M1‑25`, `M2‑18`, `M3‑08`, `M3‑10`, `M5‑07` | `m0/cost‑baseline`, `m0/lez‑probe` | `recovery_cycles_are_unchanged`, `keccak_cycles_per_hash_are_unchanged`, `full_update_cycles_are_unchanged`, `accelerated_configuration_cycles_are_unchanged`, `proof_sizes_distinguish_the_configurations`, `instruction_handling_cost_is_unchanged`, `per_signer_cost_is_linear`, CI job `guardrails`, `m0/M0‑report.pdf` |
| **P3** | partial | `M3‑09`, `M5‑07` | `m0/cost‑baseline` | `m0/M0‑report.pdf` |

**P1** — A single 3-of-N verify-and-publish completes within one LEZ public transaction at the budget in force at delivery, in both modes

M0 measured a 3-of-N update at 1,906,737 cycles against LEZ's 32M per-transaction budget, 5.68%, and pinned it with a guardrail test. What is still open is the shipping path in one real transaction, per mode: M2-02 and M2-19 for push, M3-06 for pull.

**P2** — Cost measurement as a primary deliverable: per-signer recovery, keccak256, decode, membership check and (push) account write and registration, per mode, reproducible from the test suite

"Reproducible from the test suite" is satisfied the strict way: every published figure is asserted by exact equality and runs in CI on x86_64. The per-component split is measured rather than inferred: `lez_plumbing` is a third guest identical to `lez_verify` with the cryptography removed, so the framework's per-package instruction handling is subtracted directly instead of being left in a residual, and `instruction_handling_cost_is_unchanged` pins it. Without that separation, drift in the framework's cost is silently reattributed to recovery. The cross-architecture finding (M0-11) stands in the report, measured across three machines; ARM64 is not a delivery target and is no longer re-checked on every push. Still to come are the per-component table on product code (M1-25), the push write (M2-18), the pull per-read (M3-08) and byte accounting (M3-10).

**P3** — Documented cost delta between the in-program path and a hypothetical native ECDSA and keccak256 precompile, reported separately per mode

M0 delivered the first delta sketch against per-chain reference points. M3-09 splits it per mode, which is the form the requirement asks for, because a per-read pull path and a per-update push path amortise a precompile differently.

## Supportability

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **S1** | planned | `M2‑11`, `M5‑01`, `M5‑02` | — | — |
| **S2** | partial | `M1‑03`, `M1‑04a`, `M1‑05`, `M2‑19`, `M3‑07` | `scripts/lez‑sequencer.sh` | CI job `sequencer` |
| **S3** | partial | `M1‑06`, `M1‑22`, `M1‑23`, `M1‑24`, `M2‑13`, `M2‑14`, `M2‑15`, `M2‑16`, `M5‑06` | `traceability` | CI job `traceability` |
| **S4** | planned | `M5‑03` | `README.md` | — |
| **S5** | planned | `M5‑04` | — | — |
| **S6** | planned | `M5‑05` | — | — |
| **S7** | planned | `M4‑18` | — | — |

**S1** — The adaptor program is deployed and tested on LEZ devnet/testnet

Devnet in M2 with the day-one feeds, current testnet in M5.

**S2** — End-to-end integration tests run against a standalone LEZ sequencer in CI, and CI is green on the default branch

The harness is standing: CI boots a Bedrock node and a sequencer built from the same LEZ revision `lee_core` resolves to, and asserts it serves. What is still missing is adaptor traffic across it, which arrives with M2-19 and M3-07. M1-03 is the other half of the requirement, the workflow that has to be green on the default branch.

**S3** — At least one test per hard requirement, including per mode: valid-signature acceptance, invalid-signature rejection, threshold boundaries, staleness, asset mismatch, invalid value, and (push) registration and signer-set transitions

This matrix is the requirement's own instrument. It is `partial` by construction until M5-06 leaves every row verified; the checker is what stops that from being asserted without evidence.

**S4** — README documenting end-to-end usage for both modes: deployment, addresses, day-one feed registrations, submitting and querying, and integrating the pull library

The current README documents the M0 measurement harnesses and how to reproduce their figures. The end-to-end operator and integrator README the requirement describes is M5-03.

**S5** — SDK doc packet covering both modes, with a Recommended Consumer Pattern section and a cost-grounded push-versus-pull decision guide

Submitted through the logos-docs doc-packet template. The push-versus-pull guide has to be grounded in the P2 and P3 numbers, so it follows M5-07.

**S6** — CLI doc packet covering the operator and user journey

**S7** — Figma designs, or equivalent, for the mini-app dashboard

A deliverable in its own right, and a prerequisite for U2 rather than a by-product of it.

## Adaptor Security

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **SEC1** | partial | `M1‑13`, `M1‑22`, `M2‑13`, `M3‑03` | `verifier‑core` | `an_unknown_signer_is_skipped_and_the_payload_still_verifies`, `unauthorised_is_reported_when_the_skipped_signers_would_have_made_quorum` |
| **SEC2** | planned | `M1‑07`, `M2‑08`, `M2‑14`, `M3‑03` | `aggregator‑program`, `pull‑lib` | — |
| **SEC3** | planned | `M2‑17` | — | — |

**SEC1** — Reject any package whose signer is not in the authorised signer set for the requested feed, in both modes

Both modes, one implementation: the membership check lives in `verifier-core`, so push and pull cannot diverge on who is authorised. The rule is count-and-ignore per ADR 15, not strict rejection: an unknown signer is skipped rather than failing the payload, and only named `UnauthorisedSigner` once the skipped signers would themselves have reached quorum -- a reader expecting strict rejection would otherwise read this row as unmet.

**SEC2** — In push mode the stored signer set is updatable only by the admin authority, and the update path is tested; in pull mode the set comes from the consumer and never from the payload

Two distinct obligations. Push: only the admin authority may update the stored set, and the update path itself is tested (M2-14). Pull: the set is the consumer's, and the library must never accept one sourced from the payload (M3-03).

**SEC3** — Minimum recommended `maxAge` documented for both modes, with a manipulation analysis covering signer compromise, replay of stale packages and signer-set update delays

A document, and the one place pull's weaker position has to be stated plainly: a pull consumer keeps its own signer set fresh, because it bypasses the centrally administered one.

## Open source

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **OS1** | partial | `M1‑02`, `M1‑04` | `LICENSE‑MIT`, `LICENSE‑APACHE`, `NOTICE`, `deny.toml` | CI job `licenses`, `LICENSE‑MIT`, `LICENSE‑APACHE`, `NOTICE` |

**OS1** — The deliverable is open source under a permissive licence, dual MIT and Apache-2.0, with no copyleft or BUSL dependency, transitive ones included

The dual licence and the mechanical gate are done: `cargo deny check licenses` runs on every pull request over the whole graph, transitive dependencies included, and BUSL-1.1 is denied outright, which is also what enforces the RedStone boundary. What is not done is the requirement's second half. It asks for **no copyleft dependency, transitive ones included**, and the gate currently passes because copyleft is admitted by named exception rather than absent. Nothing copyleft is linked into a shipped artefact, which is the reassuring half and worth stating precisely rather than generously: the LGPL-3.0 family (`malachite` and its four siblings, plus `downloader`) appears in `m0/Cargo.lock` only, reached through risc0 proving, and is absent from the product lockfile entirely; `option-ext` is MPL-2.0 and reaches the product graph as a *build* dependency only, through `risc0-build` -> `dirs` -> `dirs-sys`, so it runs during a build and links into nothing. What remains is that the exceptions exist, so passing CI proves the gate is enforced rather than that the requirement is met, and a `verified` row would claim the latter on the strength of the former. It stays `partial` until Logos rules on whether build-time and measurement-only copyleft satisfies the requirement's *transitive ones included*. See *Open: the LGPL-3.0 dependency in LEZ's host graph* in `m0/versions.md`.

## Soft

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **SOFT1** *(soft)* | planned | — | — | — |
| **SOFT2** *(soft)* | planned | — | — | — |

**SOFT1** — Multi-feed batched verification, amortising calldata and recovery overhead across feeds in one instruction

Stretch, once the hard set is green. `verifier-core` is structured to allow batching; whether it is justified depends on the M0 cost result and on whether LEZ prices execution per cycle.

**SOFT2** — Multi-source integration test against the RFP-019 TWAP tier, one variant per RedStone mode, applying an example cross-source policy without the adaptor taking part in it

Stretch, and gated on RFP-019's TWAP program existing to test against.
