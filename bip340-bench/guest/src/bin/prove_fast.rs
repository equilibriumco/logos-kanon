//! Verifies every case through the backend, and nothing else.
//!
//! Its own binary, so what gets proved is the backend's work plus the zkVM
//! floor, with no second implementation in the executed path. `prove_k256` is
//! the same program over `k256`, and the pair is what makes a proving-time
//! comparison a subtraction.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use risc0_zkvm::guest::env;
use verifier_core::backend::bip340;

fn main() {
    let raw: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = env::read();

    let mut verified = 0u32;
    for (public_key, message, signature) in &raw {
        let mut key = [0u8; 32];
        key.copy_from_slice(public_key);
        let mut digest = [0u8; 32];
        digest.copy_from_slice(message);
        let mut raw_signature = [0u8; 64];
        raw_signature.copy_from_slice(signature);

        verified += u32::from(bip340::verify(&key, &digest, &raw_signature).is_ok());
    }

    assert_eq!(
        verified as usize,
        raw.len(),
        "a case did not verify, so the proof is of a refusal path"
    );
    env::commit(&verified);
}
