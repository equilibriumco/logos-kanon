//! The cases the BIP-340 backend and `k256::schnorr` are held to agree on.
//!
//! Valid signatures are the easy half. The cases that matter are the ones a
//! sign error or a missing range check would let through: scalars at and past
//! their modulus, an x-coordinate no point has, a zero `s`, and signatures one
//! bit away from valid.
//!
//! Each case carries the answer it was built to have, so the guest asserts
//! three ways rather than two. Two implementations agreeing on a forgery would
//! agree perfectly.

mod support;

use k256::elliptic_curve::bigint::ArrayEncoding;
use k256::elliptic_curve::Curve;
use k256::schnorr::VerifyingKey;
use k256::Secp256k1;
use risc0_zkvm::{default_executor, ExecutorEnv};
use verifier_core::backend::bip340_params::FIELD_ORDER;

/// `(public_key, message, signature, expected)`, as the guest reads it.
type Case = (Vec<u8>, Vec<u8>, Vec<u8>, bool);

/// The secp256k1 group order `n`, from `k256` rather than a second copy of it.
fn group_order() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(Secp256k1::ORDER.to_be_byte_array().as_slice());
    bytes
}

/// An x-coordinate no curve point has, found by trying candidates against the
/// reference implementation rather than by asserting one.
fn x_with_no_point() -> [u8; 32] {
    for candidate in 1u8..=255 {
        let mut x = [0u8; 32];
        x[31] = candidate;
        if VerifyingKey::from_bytes(&x).is_err() {
            return x;
        }
    }
    panic!("every small x had a point, which secp256k1 does not do");
}

/// The vectors published with BIP-340, as committed in `support/`.
///
/// Only the rows with a 32-byte message: the backend verifies over a digest,
/// and BIP-340's four variable-length cases are outside what it is given. What
/// remains is every adversarial row, which is the half worth having — an
/// infinite `sG − eP`, an odd `R.y`, `r` at the field size, `s` at the curve
/// order, and keys that are not x-coordinates.
fn official_vectors() -> Vec<Case> {
    const VECTORS: &str = include_str!("support/bip340-test-vectors.csv");

    let parsed: Vec<Case> = VECTORS
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            // `splitn`, so a comma inside the trailing comment stays there.
            let fields: Vec<&str> = line.splitn(8, ',').collect();
            let public_key = hex::decode(fields[2]).expect("the key column is hex");
            let message = hex::decode(fields[4]).expect("the message column is hex");
            let signature = hex::decode(fields[5]).expect("the signature column is hex");

            if message.len() != 32 {
                return None;
            }
            assert_eq!(public_key.len(), 32, "row {} has a short key", fields[0]);
            assert_eq!(
                signature.len(),
                64,
                "row {} has a short signature",
                fields[0]
            );

            Some((
                public_key,
                message,
                signature,
                fields[6].eq_ignore_ascii_case("TRUE"),
            ))
        })
        .collect();

    assert!(
        parsed.len() >= 15,
        "expected BIP-340's 32-byte-message rows, found {}",
        parsed.len()
    );
    parsed
}

/// Four cases per key: the valid signature, and three ways of being one step
/// away from it.
fn tampered_around(seed: u8) -> Vec<Case> {
    let (key, digest, signature) = support::signed(seed);
    let key_bytes = key.to_bytes().to_vec();

    // One bit off in `s`: the challenge still forms, and the sum lands
    // somewhere other than the committed `R`.
    let mut moved_s = signature;
    moved_s[63] ^= 1;

    // One bit off in `r`: the x-coordinate comparison is what refuses it.
    let mut moved_r = signature;
    moved_r[0] ^= 1;

    vec![
        (key_bytes.clone(), digest.to_vec(), signature.to_vec(), true),
        (key_bytes.clone(), digest.to_vec(), moved_s.to_vec(), false),
        (key_bytes.clone(), digest.to_vec(), moved_r.to_vec(), false),
        // The same signature over a different message.
        (
            key_bytes,
            support::message(seed.wrapping_add(64)).to_vec(),
            signature.to_vec(),
            false,
        ),
    ]
}

/// Signatures and keys that are structurally out of range, rather than merely
/// wrong.
fn out_of_range() -> Vec<Case> {
    let (key, digest, signature) = support::signed(11);
    let key_bytes = key.to_bytes().to_vec();

    let mut s_at_order = signature;
    s_at_order[32..].copy_from_slice(&group_order());

    let mut r_at_field_order = signature;
    r_at_field_order[..32].copy_from_slice(&FIELD_ORDER);

    // BIP-340 does not refuse a zero scalar the way ECDSA does, so these have
    // to fail verification rather than fail a range check.
    let mut zero_s = signature;
    zero_s[32..].fill(0);
    let mut zero_r = signature;
    zero_r[..32].fill(0);

    vec![
        (
            key_bytes.clone(),
            digest.to_vec(),
            s_at_order.to_vec(),
            false,
        ),
        (
            key_bytes.clone(),
            digest.to_vec(),
            r_at_field_order.to_vec(),
            false,
        ),
        (key_bytes.clone(), digest.to_vec(), zero_s.to_vec(), false),
        (key_bytes.clone(), digest.to_vec(), zero_r.to_vec(), false),
        (key_bytes, digest.to_vec(), vec![0u8; 64], false),
        // A key at the field characteristic, and one whose x has no point.
        (
            FIELD_ORDER.to_vec(),
            digest.to_vec(),
            signature.to_vec(),
            false,
        ),
        (
            x_with_no_point().to_vec(),
            digest.to_vec(),
            signature.to_vec(),
            false,
        ),
    ]
}

fn cases() -> Vec<Case> {
    let mut cases = official_vectors();
    cases.extend((1..=8u8).flat_map(tampered_around));

    // A valid signature under the wrong key.
    let (key, digest, signature) = support::signed(9);
    let (other_key, _, _) = support::signed(10);
    assert_ne!(key.to_bytes(), other_key.to_bytes());
    cases.push((
        other_key.to_bytes().to_vec(),
        digest.to_vec(),
        signature.to_vec(),
        false,
    ));

    cases.extend(out_of_range());
    cases
}

#[test]
fn the_backend_agrees_with_k256_and_with_the_expected_answer() {
    let cases = cases();
    assert!(
        cases.iter().any(|(_, _, _, expected)| *expected),
        "a suite of refusals would pass against a backend that refuses everything"
    );

    let env = ExecutorEnv::builder()
        .write(&cases)
        .expect("the cases serialise")
        .build()
        .expect("an executor environment");

    let session = default_executor()
        .execute(env, bip340_bench::BIP340_EQUIVALENCE_ELF)
        .expect("the guest runs to completion");

    let agreed: u32 = session.journal.decode().expect("the guest commits a count");
    assert_eq!(agreed as usize, cases.len());
}
