//! The deterministic keys, messages and signatures every measurement here
//! runs over.
//!
//! One source, so the equivalence suite and the two cost tests are measuring
//! the same inputs. Signature verification is scalar-dependent, so two tests
//! generating their own cases would be comparing different work.

#![allow(dead_code)]

use k256::schnorr::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::schnorr::{Signature, SigningKey, VerifyingKey};

/// A signing key, varied by `seed`.
pub fn signing_key(seed: u8) -> SigningKey {
    let mut scalar = [0u8; 32];
    scalar[31] = seed;
    scalar[11] = seed.wrapping_mul(37).wrapping_add(3);
    SigningKey::from_bytes(&scalar).expect("a valid scalar")
}

/// A 32-byte message, varied by `seed`.
pub fn message(seed: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = seed.wrapping_mul(i as u8).wrapping_add(seed);
    }
    bytes
}

/// A valid signature and the key that made it.
///
/// `sign_prehash` rather than `sign`: the `Signer` impl digests its argument
/// with SHA-256 first, so `sign` over 32 bytes signs something else.
pub fn signed(seed: u8) -> (VerifyingKey, [u8; 32], [u8; 64]) {
    let key = signing_key(seed);
    let digest = message(seed);
    let signature: Signature = key.sign_prehash(&digest).expect("signing succeeds");

    let verifying = *key.verifying_key();
    verifying
        .verify_prehash(&digest, &signature)
        .expect("a freshly made signature verifies");

    (verifying, digest, signature.to_bytes())
}

/// `count` valid cases as the guests read them, seeded from 1.
pub fn valid_cases(count: usize) -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    (1..=count)
        .map(|seed| {
            let (key, digest, signature) = signed(seed as u8);
            (key.to_bytes().to_vec(), digest.to_vec(), signature.to_vec())
        })
        .collect()
}
