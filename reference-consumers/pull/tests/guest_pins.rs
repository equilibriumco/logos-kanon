//! The two verifying guests accelerate the same cryptography.
//!
//! Two of the three product guests recover secp256k1 signatures: the
//! aggregator's and this one's. Reference consumer A's does not, and
//! `reference-consumers/aggregator-read/tests/accelerator_pins.rs` is where its
//! narrower pin list is asserted. This file is about the two that do.
//!
//! Each guest is its own workspace, so each carries its own
//! `[patch.crates-io]`: cargo has no way to share one, and `[M3-06:01]` records
//! why the consumer's guest cannot live beside the aggregator's. The pins are
//! therefore written twice, and nothing about writing them twice keeps them
//! equal.
//!
//! What that would cost is the reason this is a test rather than a comment. The
//! two modes call one verifier, which is ADR 2's "one audit covers both modes" —
//! and if one guest's `k256` is bumped and the other's is not, they stop calling
//! the same *implementation* of it. Push and pull would verify identical payloads
//! with different code, nothing would fail, and the cost figures would diverge
//! with no explanation. `m0/cost-baseline/host/tests/guardrails.rs`'s
//! `mixed_configuration_is_the_combination_it_claims` guards a duplicated patch
//! section for the same reason, and says so in nearly the same words.
//!
//! It lives in this crate because the consumer's guest is what introduced the
//! second copy, and because these tests run in the ordinary workspace job;
//! `kanon-methods` builds all three guests but its tests are excluded from it.

#[path = "support/pins.rs"]
mod pins;

use pins::{manifest, pins};

#[test]
fn both_verifying_guests_pin_the_same_accelerators() {
    let aggregator = pins(&manifest("methods/guest/Cargo.toml"));
    let consumer = pins(&manifest("reference-consumers/pull/guest/Cargo.toml"));

    // The section is found at all, in both. A rename or a deletion would
    // otherwise leave two empty lists comparing equal.
    assert_eq!(
        aggregator.len(),
        3,
        "the aggregator guest should pin k256, crypto-bigint and sha2; found {aggregator:#?}"
    );

    assert_eq!(
        consumer, aggregator,
        "the two verifying guests have drifted apart on which cryptography they \
         accelerate. Both verify with the same `verifier-core`, so a difference \
         here means push and pull run different implementations of the same audit \
         (ADR 2), and their cycle figures stop being comparable. Bump both or \
         neither."
    );
}
