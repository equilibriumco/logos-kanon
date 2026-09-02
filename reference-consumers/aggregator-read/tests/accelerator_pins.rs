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

use std::path::Path;

fn manifest(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("reference-consumers")
        .parent()
        .expect("the repository root");
    std::fs::read_to_string(root.join(relative)).unwrap_or_else(|_| panic!("{relative} is on disk"))
}

/// The crate names under a `[patch.crates-io]` heading, up to the next section.
fn pinned_crates(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[patch.crates-io]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(|line| line.split('#').next().unwrap_or("").trim().to_owned())
        .filter(|line| !line.is_empty())
        .map(|line| line.split('=').next().unwrap_or_default().trim().to_owned())
        .collect()
}

#[test]
fn the_reading_consumers_guest_pins_the_hash_it_computes() {
    let pins = pinned_crates(&manifest(
        "reference-consumers/aggregator-read/guest/Cargo.toml",
    ));

    assert_eq!(
        pins,
        ["sha2"],
        "this consumer derives three multi-seed PDAs per settlement and verifies \
         nothing, so `sha2` is the one accelerator it has a use for. A signature \
         pin here buys no cycles and a missing hash pin spends some, and either \
         makes M3-09's per-mode comparison a comparison of manifests: {pins:?}"
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
        let pins = pinned_crates(&manifest(verifying));
        for expected in ["k256", "crypto-bigint"] {
            assert!(
                pins.contains(&expected.to_owned()),
                "{verifying} recovers signatures and is expected to pin {expected}: {pins:?}"
            );
        }
    }
}
