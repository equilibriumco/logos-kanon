//! The BIP-340 verification equation, spelled out over `k256`'s host group ops.
//!
//! The guest backend drives the EC circuits, which a host test cannot reach.
//! This runs the same equation through `k256` instead, so a disagreement
//! between the backend and the reference can be attributed: if this passes,
//! the equation and the challenge encoding are right and the fault is in how
//! the circuits are driven.

mod support;

use k256::elliptic_curve::ops::Reduce;
use k256::elliptic_curve::point::DecompressPoint;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::elliptic_curve::subtle::Choice;
use k256::elliptic_curve::PrimeField;
use k256::{AffinePoint, FieldBytes, ProjectivePoint, Scalar};
use sha2::{Digest, Sha256};
use verifier_core::backend::bip340_params::CHALLENGE_TAG;

/// `int(tagged_hash("BIP0340/challenge", r ‖ P ‖ m)) mod n`.
///
/// Hashes the tag rather than taking the precomputed hash, so the two are
/// still independent here.
fn challenge(r_bytes: &[u8], public_key: &[u8], message: &[u8]) -> Scalar {
    let tag = Sha256::digest(CHALLENGE_TAG);

    let mut hasher = Sha256::new();
    hasher.update(tag);
    hasher.update(tag);
    hasher.update(r_bytes);
    hasher.update(public_key);
    hasher.update(message);

    <Scalar as Reduce<k256::U256>>::reduce_bytes(&hasher.finalize())
}

#[test]
fn the_equation_verifies_a_signature_k256_made() {
    let (key, message, raw) = support::signed(7);
    let public_key = key.to_bytes();
    let (r_bytes, s_bytes) = raw.split_at(32);

    // P = lift_x(pk): the point with this x and an even y.
    let point = AffinePoint::decompress(FieldBytes::from_slice(&public_key), Choice::from(0));
    let point = Option::<AffinePoint>::from(point).expect("the key is a valid x-coordinate");

    let s = Option::<Scalar>::from(Scalar::from_repr(*FieldBytes::from_slice(s_bytes)))
        .expect("s is a scalar");
    let e = challenge(r_bytes, &public_key, &message);

    let computed = ProjectivePoint::GENERATOR * s - ProjectivePoint::from(point) * e;
    let computed = computed.to_affine().to_encoded_point(false);

    assert_eq!(
        computed.x().expect("R is finite").as_slice(),
        r_bytes,
        "x(R) should equal the signature's r"
    );

    let y = computed.y().expect("R is finite");
    assert_eq!(y[31] & 1, 0, "R should have even y");
}
