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

/// The `[patch.crates-io]` entries of a manifest, as normalised lines.
///
/// Whole lines rather than crate names, so a tag is compared as well as a
/// crate. The set alone would let this guest's `sha2` drift to another revision
/// while still reading as `["sha2"]`, which is exactly the manifest difference
/// the header says must not reach M3-09's per-mode delta.
fn pins(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[patch.crates-io]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(|line| line.split('#').next().unwrap_or("").trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// The one entry naming `crate`, or `None`.
fn pin_for(manifest: &str, crate_name: &str) -> Option<String> {
    pins(manifest)
        .into_iter()
        .find(|line| line.starts_with(crate_name))
}

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
