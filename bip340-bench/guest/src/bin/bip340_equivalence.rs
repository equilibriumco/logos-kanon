//! BIP-340 verification against `k256::schnorr`, inside the zkVM.
//!
//! The backend links only for the guest target, so no host test can run it.
//! This guest can: it verifies every case both ways and requires the two to
//! agree, and requires both to match the result the case was built to have.
//!
//! Agreement alone would be too weak. Two implementations that both accepted a
//! forged signature would agree perfectly, so each case carries its own
//! expected answer and the assertion is three-way.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use k256::schnorr::signature::hazmat::PrehashVerifier;
use k256::schnorr::{Signature as ReferenceSignature, VerifyingKey};
use verifier_core::backend::bip340;

/// What `k256::schnorr` makes of the case. A key or signature it will not even
/// parse is a refusal, which is what the backend answers too.
///
/// `verify_prehash` rather than `verify`: the latter digests its argument with
/// SHA-256 first, which would make the two implementations verify over
/// different messages.
fn reference(public_key: &[u8; 32], message: &[u8; 32], signature: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(parsed) = ReferenceSignature::try_from(&signature[..]) else {
        return false;
    };
    key.verify_prehash(message, &parsed).is_ok()
}

fn main() {
    // `(public_key, message, signature, expected)`. Fixed-size arrays have no
    // serde impl at these lengths, so the three byte strings arrive as vectors.
    let cases: Vec<(Vec<u8>, Vec<u8>, Vec<u8>, bool)> = risc0_zkvm::guest::env::read();

    let mut agreed = 0u32;
    for (public_key, message, signature, expected) in &cases {
        let mut key = [0u8; 32];
        key.copy_from_slice(public_key);
        let mut digest = [0u8; 32];
        digest.copy_from_slice(message);
        let mut raw = [0u8; 64];
        raw.copy_from_slice(signature);

        let outcome = bip340::verify(&key, &digest, &raw);
        let ours = outcome.is_ok();
        assert_eq!(
            ours,
            reference(&key, &digest, &raw),
            "the backend and k256 disagreed on case {agreed}, backend said {outcome:?}"
        );
        assert_eq!(
            ours, *expected,
            "both implementations disagreed with the expected result on case {agreed}"
        );
        agreed += 1;
    }

    risc0_zkvm::guest::env::commit(&agreed);
}
