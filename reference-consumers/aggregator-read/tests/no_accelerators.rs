//! This consumer's guest carries no accelerator patches, and that is asserted
//! rather than left to whoever copies the manifest next.
//!
//! The aggregator and reference consumer B both pin risc0's `k256`,
//! `crypto-bigint` and `sha2` forks, because both recover secp256k1 signatures
//! and that is where their cycles go. This consumer recovers nothing: it reads
//! an account somebody else already verified.
//!
//! Two things would go unnoticed without this file. A copied manifest would
//! carry pins that buy nothing here, which is the kind of thing nobody reads
//! twice. And the difference between the two modes is exactly what M3-09's
//! per-mode precompile delta is about, so an accidental pin would make the two
//! sides of that report incomparable.

use std::path::Path;

fn manifest(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("reference-consumers")
        .parent()
        .expect("the repository root");
    std::fs::read_to_string(root.join(relative)).unwrap_or_else(|_| panic!("{relative} is on disk"))
}

/// Everything under a `[patch.crates-io]` heading, up to the next section.
fn patch_section(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[patch.crates-io]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(|line| line.split('#').next().unwrap_or("").trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn the_reading_consumers_guest_pins_no_accelerators() {
    let pins = patch_section(&manifest(
        "reference-consumers/aggregator-read/guest/Cargo.toml",
    ));

    assert!(
        pins.is_empty(),
        "this consumer verifies nothing, so an accelerator pin here buys no cycles \
         and makes M3-09's per-mode comparison meaningless: {pins:?}"
    );
}

/// The other half, so the test above is a statement about a difference rather
/// than an absence nobody would notice being wrong.
#[test]
fn the_verifying_guests_do_pin_them() {
    for verifying in [
        "methods/guest/Cargo.toml",
        "reference-consumers/pull/guest/Cargo.toml",
    ] {
        assert!(
            !patch_section(&manifest(verifying)).is_empty(),
            "{verifying} recovers signatures and is expected to pin the accelerators"
        );
    }
}
