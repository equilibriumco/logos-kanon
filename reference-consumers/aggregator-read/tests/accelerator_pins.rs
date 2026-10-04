//! This consumer's guest accelerates the hash it computes and not the
//! signatures it never checks, and the split is asserted rather than left to
//! whoever copies a manifest next.
//!
//! The aggregator and reference consumer B pin risc0's `k256`, `crypto-bigint`
//! and `sha2` forks, because both recover secp256k1 signatures and that is
//! where their cycles go. This consumer recovers nothing: it reads an account
//! somebody else already verified. It does hash, though. `compute_pda` combines
//! a multi-seed PDA with SHA-256, and a settlement runs three of those.
//!
//! Both halves matter for the same reason. A copied manifest would carry
//! signature pins that buy nothing here, and a missing `sha2` would spend
//! cycles this mode has no reason to spend. Either way the difference would
//! land inside M3-09's per-mode precompile delta, which is supposed to be a
//! comparison of modes rather than of manifests.

#[path = "../../pull/tests/support/pins.rs"]
mod pins;

use pins::{manifest, pin_for, pins};

#[test]
fn the_reading_consumers_guest_pins_the_hash_it_computes() {
    const GUEST: &str = "reference-consumers/aggregator-read/guest/Cargo.toml";
    let mine = pins(&manifest(GUEST));

    assert_eq!(
        mine.len(),
        1,
        "this consumer derives three multi-seed PDAs per settlement and verifies \
         nothing, so `sha2` is the one accelerator it has a use for. A signature \
         pin here buys no cycles and a missing hash pin spends some, and either \
         makes M3-09's per-mode comparison a comparison of manifests: {mine:?}"
    );

    // The same line, not merely the same crate. `guest_pins.rs` locks the two
    // verifying guests to each other this way; without this, the one pin here
    // would be compared to nothing and could drift to another revision on its
    // own.
    let theirs = pin_for(&manifest("methods/guest/Cargo.toml"), "sha2")
        .expect("the aggregator's guest pins sha2");
    assert_eq!(
        mine[0], theirs,
        "the reading guest's `sha2` has to be the revision the verifying guests \
         use, or the two sides of M3-09's delta hash with different code"
    );
}

/// The other half, so the test above is a statement about a difference rather
/// than a list nobody would notice being wrong.
#[test]
fn the_verifying_guests_pin_the_signature_crates_too() {
    for verifying in [
        "methods/guest/Cargo.toml",
        "reference-consumers/pull/guest/Cargo.toml",
    ] {
        let pins = pins(&manifest(verifying));
        for expected in ["k256", "crypto-bigint"] {
            assert!(
                pins.iter().any(|line| line.starts_with(expected)),
                "{verifying} recovers signatures and is expected to pin {expected}: {pins:?}"
            );
        }
    }
}
