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

Right now: **1 verified, 13 partial, 22 planned**, of 36 requirements.

## Functionality

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **F1** | partial | `M1‑10`, `M1‑11`, `M2‑01`, `M2‑02` | `verifier‑core`, `aggregator‑program`, `methods/guest` | `the_trait_is_object_safe`, `a_signature_round_trips_to_the_signing_key_address`, `keccak256_matches_the_known_digest_of_the_empty_input` |
| **F2** | partial | `M1‑01`, `M1‑10`, `M1‑19`, `M1‑22`, `M3‑01` | `verifier‑core`, `pull‑lib` | `exactly_the_threshold_verifies_and_reports_how_many_signers_met_it`, `more_signers_than_the_threshold_all_count_toward_the_median`, `one_unusable_value_rejects_even_when_the_other_signers_reach_threshold`, `one_stranger_rejects_even_when_the_authorised_signers_reach_threshold`, `a_package_at_the_far_edge_of_the_window_is_still_current`, `an_accepted_price_carries_both_scales_so_a_caller_need_not_convert`, `no_two_failure_modes_answer_with_the_same_variant` |
| **F3** | verified | `M1‑13`, `M1‑23` | `verifier‑core` | `three_of_three_signers_reporting_the_same_price_verifies`, `the_threshold_boundary_accepts_at_m_and_rejects_at_m_minus_one`, `one_signer_cannot_reach_the_threshold_alone_by_repeating_the_feed`, `a_single_package_carrying_the_requested_feed_twice_still_counts_once`, `a_copy_of_a_package_already_in_the_payload_cannot_deny_the_feed`, `copies_of_one_package_cannot_outvote_the_round_the_signers_agree_on`, `one_package_cannot_crowd_the_other_signers_out_of_the_report_buffer`, `a_package_from_another_round_cannot_take_the_round_from_the_signers`, `two_rounds_with_equal_support_are_settled_by_the_newer`, `an_older_package_from_a_signer_that_already_reported_cannot_deny_the_feed`, `a_package_duplicated_by_anyone_does_not_deny_the_feed_to_the_rest`, `a_threshold_of_one_is_met_by_one_signer`, `a_threshold_equal_to_the_signer_count_needs_every_one_of_them`, `one_package_past_the_ceiling_is_refused_rather_than_answered`, `packages_for_other_feeds_are_free_and_do_not_count_against_the_ceiling` |
| **F4** | partial | `M1‑08`, `M1‑12`, `M1‑14`, `M1‑15`, `M1‑16`, `M1‑17`, `M1‑21` | `verifier‑core`, `kanon‑clock` | `the_signed_span_covers_the_points_and_the_three_trailing_fields`, `truncation_at_every_length_is_rejected_rather_than_panicking`, `trailing_bytes_before_the_packages_are_rejected`, `a_feed_registered_against_another_pair_is_refused`, `a_pair_matching_only_on_the_base_is_still_a_mismatch`, `a_negative_value_rejects_even_when_quorum_is_present`, `every_repeated_point_for_the_requested_feed_is_validated`, `one_stale_package_rejects_even_when_quorum_is_present`, `one_future_package_rejects_even_when_quorum_is_present`, `signers_that_all_reported_something_unusable_are_named_as_the_fault`, `the_staleness_boundary_admits_a_package_exactly_max_age_old`, `a_package_dated_beyond_clock_skew_is_a_different_failure_from_a_stale_one`, `a_replayed_payload_is_refused_however_well_signed_it_is`, `any_other_account_is_refused_before_its_contents_are_read`, `every_published_signature_recovers_to_the_signer_redstone_named`, `a_published_payload_verifies_end_to_end`, `a_wrong_signed_span_recovers_to_nobody` |
| **F5** | partial | `M1‑09`, `M1‑16`, `M1‑17`, `M1‑20`, `M1‑26`, `M2‑02` | `aggregator‑program/src/publish.rs`, `verifier‑core`, `kanon‑idl` | `the_verified_price_carries_both_scales`, `an_agreed_price_too_large_for_the_account_is_reported_not_wrapped`, `a_fresh_account_takes_every_field_from_the_feed_and_its_configuration`, `the_source_id_is_the_bytes_a_consumer_can_write_down`, `rebuilding_the_adaptor_does_not_change_what_it_writes`, `a_newer_observation_moves_the_price_and_the_timestamp_and_nothing_else`, `an_account_for_another_pair_is_refused_rather_than_repointed`, `an_account_another_source_populated_is_not_this_adaptors_to_write`, `an_older_observation_inside_the_window_cannot_move_the_price_backwards`, `the_same_observation_twice_is_not_an_update`, `the_timestamp_is_the_round_every_counted_package_shares`, `the_vendored_idl_declares_the_fields_we_expect`, `field_order_is_part_of_the_standard`, `the_reexported_type_is_constructible_with_the_documented_shape`, `the_account_encodes_to_the_bytes_a_foreign_decoder_expects`, `the_encoded_length_is_what_the_idl_field_types_add_up_to`, `a_round_trip_through_the_on_chain_container_recovers_every_field`, `a_seventh_field_would_be_a_break_rather_than_something_consumers_ignore` |
| **F6** | planned | `M1‑07`, `M2‑06`, `M2‑07`, `M2‑08`, `M2‑09`, `M2‑10` | `aggregator‑program` | — |
| **F7** | planned | `M2‑00`, `M2‑11`, `M2‑12` | `aggregator‑program` | — |
| **F8** | planned | `M4‑01`, `M4‑02`, `M4‑03`, `M4‑04`, `M4‑05`, `M4‑06`, `M4‑07`, `M4‑08` | `kanon‑relayer` | — |
| **F9** | planned | `M3‑01`, `M3‑02`, `M3‑03`, `M3‑04` | `pull‑lib` | — |

**F1** — Public-mode push aggregator accepting signed RedStone data packages, with the signature path structured so swapping in a future host primitive (precompile or syscall) stays a localised change

Two halves. The verification half has landed: the `VerifierBackend` trait (M1-10) is the localised-swap structure, and `InProgramBackend` (M1-11) implements it with the same k256 and tiny-keccak M0 measured. The guest patches in the *mixed* accelerator configuration -- recovery accelerated, keccak in software -- which is M0's recommendation rather than a default. The aggregator half is M2.

**F2** — Verification packaged as a library callable both from the aggregator and from third-party consumer programs, so one audited implementation backs both modes

Structural: `verifier-core` depends on neither mode, and both modes depend on it. M3-02 asserts the pull side carries no aggregator dependency. M1-22 supplies the behavioural half: `verifier-core/src/accept_and_reject.rs` reads the accept and reject contract off the crate's public API, which is the surface both modes call. It is tested once rather than once per mode because `verify_feed` takes no argument naming its caller, so there is no per-mode behaviour to reach -- ADR 23 records that, and what the two callers add on top is M2-02 and M3-01, with M3-04 asserting they agree.

**F3** — M-of-N signer threshold per feed, configurable at registration, default 3-of-N; reject any package that does not meet it

`verify_feed` enforces M-of-N over distinct authorised signers: one slot per configured signer, so a signer counts once however many packages it sends (M1-13). A second package from a slotted signer is skipped rather than fatal -- ADR 24 records why, and the tests cover both ways an attacker can produce a duplicate without a key. Which of a signer's packages fills its slot is settled by ADR 28 rather than by ADR 24's freshest rule: the price comes from one round, chosen as the timestamp the most distinct configured signers agree on, and one package files one report however many times it repeats the feed. M1-23 pins the boundaries where an off-by-one would live -- M, M-1, one, and unanimity. ADR 15 records the contract's strict package checks before the threshold is evaluated.

**F4** — Decode the RedStone data-package format (asset identifier, value, timestamp, signer set) and reject any package whose timestamp is older than a configurable `maxAge`, whose value is zero, negative or otherwise invalid, or whose asset identifier does not match the registered feed

Four rejection classes in one requirement. The malformed-wire-format one has landed: M1-12 decodes zero-copy, bounds-checks every length, and is tested against truncation at every offset, since a panic in a guest aborts the transaction rather than rejecting the package. Conformance against published vectors is M1-21. Asset identity has landed (M1-16), and it is a comparison between the caller's expectation and the registration rather than a property of the payload, because a RedStone package names a feed and nothing else -- ADR 16 records what that check does and does not establish. Value sanity has landed too (M1-17): zero, negative and unrepresentable values reject the payload immediately, even when other packages meet the threshold. Staleness has landed as well (M1-14, M1-15): every package for the requested feed must be inside the two-sided window, read from LEZ's pinned every-block clock account. ADR 15 records the strict package-level contract. M1-21 conforms the decoder against payloads captured from RedStone's gateway, using their own signatures as the oracle: a signature recovers to the address they published only if the signed span matches theirs byte for byte, and a negative control shows seven plausible wrong spans recovering nobody. There are no published vectors to test against -- the Rust SDK builds its test data in code and the EVM connector is BUSL -- which is what ADR 19 records.

**F5** — Publish the verified price into a canonical RFP-019 price account, populating base and quote asset, price, timestamp, source identifier (a constant identifying RedStone) and a zero confidence interval (RedStone publishes none)

The account is RFP-019's, re-exported rather than forked (M1-09), and `publish` decides what goes in its six fields. `source_id` is the constant F5 names: `RedStone`, ASCII right-padded, so a consumer checking where a price came from needs nothing from this repository to construct it. ADR 22 records the decision, including why the writer's own identity was the wrong answer: `source_id` is one of the three identifiers `publish` refuses to overwrite, so an identifier that moved with the program id would leave a rebuilt adaptor unable to update the accounts it created. The pair comes from the registration claim (M1-16), the price is the median on the account's Q64.64 scale (M1-17), the timestamp is the observation's and not the write's, and the confidence interval is zero because RedStone publishes none. M1-26 conforms the layout in both directions. What remains is M2-02: the instruction that calls `publish` inside a real transaction.

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
| **U6** | partial | `M1‑15`, `M1‑16`, `M1‑17`, `M1‑18`, `M1‑22`, `M3‑04` | `verifier‑core`, `pull‑lib` | `a_decode_failure_and_a_signature_failure_stay_distinguishable`, `the_threshold_failure_carries_what_was_reached_and_what_was_needed`, `a_feed_registered_against_another_pair_is_refused`, `signers_that_all_reported_something_unusable_are_named_as_the_fault`, `one_bad_value_is_reported_before_threshold_is_evaluated`, `the_first_bad_package_in_the_wire_walk_is_reported`, `an_exponent_the_conversion_cannot_divide_by_is_a_configuration_error`, `a_missing_clock_is_not_a_stale_package`, `a_stale_package_and_a_future_one_stay_distinguishable`, `a_signers_stale_package_about_another_feed_does_not_make_this_one_stale`, `a_signers_stale_package_about_this_feed_still_makes_it_stale`, `values_spliced_from_three_rounds_do_not_become_a_price`, `values_from_different_rounds_are_not_made_into_one_price`, `the_timestamp_is_the_round_every_counted_package_shares`, `more_packages_for_this_feed_than_verification_will_pay_for_is_refused`, `package_age_is_checked_before_its_value`, `a_clock_that_cannot_be_read_refuses_rather_than_guesses`, `no_two_failure_modes_answer_with_the_same_variant`, `a_malformed_payload_is_named_as_malformed_and_carries_the_decoder_reason`, `an_envelope_that_is_not_redstones_is_refused_before_any_verification`, `too_few_signers_reports_how_many_arrived_against_how_many_were_needed`, `signers_nobody_configured_are_named_as_the_fault_rather_than_the_count`, `a_pair_that_is_not_the_callers_is_refused_before_a_single_recovery`, `a_payload_older_than_the_window_is_stale_rather_than_short_of_signers`, `a_payload_dated_ahead_of_the_clock_is_its_own_failure_not_a_stale_one`, `zero_and_negative_prices_are_rejected_directly`, `one_signer_supplying_the_feed_twice_counts_once_and_is_short_of_the_threshold`, `a_price_the_accounts_scale_cannot_hold_is_reported_not_wrapped`, `a_clock_that_cannot_be_read_fails_the_verification_rather_than_defaulting`, `an_unusable_configuration_is_refused_before_a_payload_is_ever_seen`, `a_signature_that_recovers_to_nobody_is_reported_directly` |
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

One error enum in `verifier-core` is what makes push and pull report the same failure the same way; M3-04 asserts the parity. M1-18 defines the payload and configuration failures, while later hardening added `TooManyPackages` and `MixedRounds`. ADR 15 requires strict package rejection: `InvalidSignature`, `UnauthorisedSigner`, `StalePackage`, `FuturePackage`, and `ValueOutOfRange` return immediately for any package carrying the requested feed, before threshold evaluation. `ReoccurringSigner` remains the sole variant without a producer because duplicate packages cannot fill another signer slot and ADR 24 keeps them non-fatal. M1-22 provides the contract-shaped suite and asserts that no two causes collapse onto the same variant.

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
| **P1** | partial | `M2‑02`, `M2‑19`, `M3‑06` | `m0/lez‑probe` | `a_3_of_n_update_leaves_ample_budget_headroom`, `the_most_a_payload_can_cost_still_fits_in_one_transaction`, `the_largest_payload_the_decoder_accepts_is_read_and_verified_inside_the_budget`, `a_payload_past_the_size_limit_is_refused_before_it_is_parsed`, `m0/M0‑report.pdf` |
| **P2** | partial | `M1‑25`, `M2‑18`, `M3‑08`, `M3‑10`, `M5‑07` | `m0/cost‑baseline`, `m0/lez‑probe`, `methods/tests/cost.rs`, `COSTS.md` | `recovery_cycles_are_unchanged`, `keccak_cycles_per_hash_are_unchanged`, `full_update_cycles_are_unchanged`, `accelerated_configuration_cycles_are_unchanged`, `proof_sizes_distinguish_the_configurations`, `instruction_handling_cost_is_unchanged`, `per_signer_cost_is_linear`, `per_component_cycles_are_unchanged`, `the_dominant_cost_is_linear_in_the_signer_count`, `recovery_is_still_the_accelerated_implementation`, `the_keccak_accelerator_is_still_declined`, CI job `guardrails`, CI job `product‑costs`, `m0/M0‑report.pdf`, `COSTS.md` |
| **P3** | partial | `M3‑09`, `M5‑07` | `m0/cost‑baseline` | `m0/M0‑report.pdf` |

**P1** — A single 3-of-N verify-and-publish completes within one LEZ public transaction at the budget in force at delivery, in both modes

M0 measured a 3-of-N update at 1,906,737 cycles against LEZ's 32M per-transaction budget, 5.68%, and pinned it with a guardrail test. That is the typical case; the ceiling is the other half, because the package count sits in the payload's envelope and a hash plus recovery costs about 602,700 cycles per package. Unbounded, 56 of them overran the budget and the transaction aborted instead of publishing. ADR 26 bounds it and `the_most_a_payload_can_cost_still_fits_in_one_transaction` asserts the bound holds. That covers verification and not the read: LEZ reads a program's whole instruction data before the program's first instruction, at about 113 cycles a byte, so a 127,814-byte payload overran the budget on the read alone with verification still correctly bounded. ADR 27 sets `MAX_PAYLOAD_BYTES` at 32 KiB and the largest payload the decoder accepts reads and verifies in 23,042,390 cycles, 69% of the budget. Between the two this row is now a statement about every payload rather than about the captured one. Bounding `InstructionData` itself is LEZ's, not this adaptor's -- no program can refuse bytes it has already been charged for -- so the limit is also a number for the relayer and the aggregator to enforce before submitting. What is still open is the shipping path in one real transaction, per mode: M2-02 and M2-19 for push, M3-06 for pull.

**P2** — Cost measurement as a primary deliverable: per-signer recovery, keccak256, decode, membership check and (push) account write and registration, per mode, reproducible from the test suite

"Reproducible from the test suite" is satisfied the strict way: every published figure is asserted by exact equality and runs in CI on x86_64. M0 measured the two primitives in isolation, and its per-component split comes from a subtraction rather than a residual: `lez_plumbing` is a third guest identical to `lez_verify` with the cryptography removed, so the framework's per-package instruction handling is taken out directly and `instruction_handling_cost_is_unchanged` pins it. Without that separation, drift in the framework's cost is silently reattributed to recovery. M1-25 does the same for the shipped path: `COSTS.md` breaks a real update into decode, keccak256, recovery, the signer-set check and a named remainder, by differencing prefixes of `verify_feed` in the product guest over payloads RedStone published. Recovery is 95.81% of a three-signer update and nothing amortises, so a wider threshold is linearly priced. The table is printed by the code that asserts it, so a published figure and an asserted figure cannot drift apart, and the two rows for keccak256 and recovery are what finally give ADR 6 its CI guards in cycles alone. The cross-architecture finding (M0-11) stands in the report, measured across three machines; ARM64 is not a delivery target and is no longer re-checked on every push. Still to come are the push write (M2-18), the pull per-read (M3-08) and byte accounting (M3-10).

**P3** — Documented cost delta between the in-program path and a hypothetical native ECDSA and keccak256 precompile, reported separately per mode

M0 delivered the first delta sketch against per-chain reference points. M3-09 splits it per mode, which is the form the requirement asks for, because a per-read pull path and a per-update push path amortise a precompile differently.

## Supportability

| Requirement | Status | Tasks | Implemented in | Verified by |
| --- | --- | --- | --- | --- |
| **S1** | planned | `M2‑11`, `M5‑01`, `M5‑02` | — | — |
| **S2** | partial | `M1‑03`, `M1‑04a`, `M1‑05`, `M2‑19`, `M3‑07` | `scripts/lez‑sequencer.sh` | CI job `sequencer` |
| **S3** | partial | `M1‑06`, `M1‑21`, `M1‑22`, `M1‑23`, `M1‑24`, `M2‑13`, `M2‑14`, `M2‑15`, `M2‑16`, `M5‑06` | `traceability`, `verifier‑core`, `verifier‑core/src/properties.rs` | CI job `traceability`, `every_published_signature_recovers_to_the_signer_redstone_named`, `a_garbage_signature_rejects_even_when_quorum_is_present`, `a_signer_set_that_is_not_the_published_one_reaches_no_threshold`, `lowering_the_threshold_never_changes_the_price_and_never_loses_it`, `a_payload_decodes_back_to_what_was_written_into_it`, `the_scale_conversion_is_a_shift_and_a_truncating_divide`, `the_representable_range_is_exactly_what_q64_64_can_hold`, `the_median_does_not_depend_on_the_order_the_reports_arrived_in`, `a_corrupted_payload_is_answered_rather_than_panicked_on` |
| **S4** | planned | `M5‑03` | `README.md` | — |
| **S5** | planned | `M5‑04` | — | — |
| **S6** | planned | `M5‑05` | — | — |
| **S7** | planned | `M4‑18` | — | — |

**S1** — The adaptor program is deployed and tested on LEZ devnet/testnet

Devnet in M2 with the day-one feeds, current testnet in M5.

**S2** — End-to-end integration tests run against a standalone LEZ sequencer in CI, and CI is green on the default branch

The harness is standing: CI boots a Bedrock node and a sequencer built from the same LEZ revision `lee_core` resolves to, and asserts it serves. What is still missing is adaptor traffic across it, which arrives with M2-19 and M3-07. M1-03 is the other half of the requirement, the workflow that has to be green on the default branch.

**S3** — At least one test per hard requirement, including per mode: valid-signature acceptance, invalid-signature rejection, threshold boundaries, staleness, asset mismatch, invalid value, and (push) registration and signer-set transitions

This matrix is the requirement's own instrument. It is `partial` by construction until M5-06 leaves every row verified; the checker is what stops that from being asserted without evidence. S3 asks for valid-signature acceptance and invalid-signature rejection specifically: M1-21 verifies RedStone's published signatures over their published packages, while ADR 15 adds a payload with an unrecoverable signature alongside an otherwise valid quorum and requires `InvalidSignature`. M1-24 adds the coverage an example cannot give: the threshold, the framing and the scale conversion are claims about every input, and `verifier-core/src/properties.rs` generates them (ADR 21). The last of those tests is the one that matters most for a guest, since ADR 4 makes a panic on a malformed payload an aborted transaction rather than a rejection.

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
| **SEC1** | partial | `M1‑13`, `M1‑22`, `M2‑13`, `M3‑03` | `verifier‑core` | `an_unknown_signer_rejects_the_payload_even_when_quorum_is_present`, `an_unauthorised_signer_is_reported_directly` |
| **SEC2** | planned | `M1‑07`, `M2‑08`, `M2‑14`, `M3‑03` | `aggregator‑program`, `pull‑lib` | — |
| **SEC3** | partial | `M1‑14`, `M1‑15`, `M2‑17` | `kanon‑clock`, `verifier‑core` | `any_other_account_is_refused_before_its_contents_are_read`, `a_replayed_payload_is_refused_however_well_signed_it_is`, `a_clock_that_cannot_be_read_refuses_rather_than_guesses` |

**SEC1** — Reject any package whose signer is not in the authorised signer set for the requested feed, in both modes

Both modes share one membership check in `verifier-core`. Every package carrying the requested feed is recovered and its signer must be in the configured set; the first unknown signer returns `UnauthorisedSigner` even when authorised packages already meet quorum. Packages for other feeds remain irrelevant and are skipped before recovery. ADR 15 records why this follows SEC1's literal contract and why a malicious submitter causing its own transaction to fail is not a feed-wide denial of service. Push and pull integration evidence remains M2-13 and M3-03, so the row stays partial.

**SEC2** — In push mode the stored signer set is updatable only by the admin authority, and the update path is tested; in pull mode the set comes from the consumer and never from the payload

Two distinct obligations. Push: only the admin authority may update the stored set, and the update path itself is tested (M2-14). Pull: the set is the consumer's, and the library must never accept one sourced from the payload (M3-03).

**SEC3** — Minimum recommended `maxAge` documented for both modes, with a manipulation analysis covering signer compromise, replay of stale packages and signer-set update delays

M2-17 is the document: the production `maxAge` minimum and the manipulation analysis. The mechanism it will describe exists now. Replay is refused because the clock is the pinned every-block account and no caller may substitute another, and a clock that cannot be read refuses rather than guesses, since verifying without one accepts a package of any age. RedStone's own fifteen minutes is the reference point the `maxAge` recommendation starts from. This is also the one place pull's weaker position has to be stated plainly: a pull consumer keeps its own signer set fresh, because it bypasses the centrally administered one.

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
