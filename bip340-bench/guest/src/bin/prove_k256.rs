//! Verifies every case through `k256`'s Schnorr verifier, and nothing else.
//!
//! The counterpart to `prove_fast`: same program, same inputs, same commit,
//! different verifier. One implementation per ELF, so a proving time is one
//! implementation's and not a blend.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use k256::schnorr::signature::hazmat::PrehashVerifier;
use k256::schnorr::{Signature, VerifyingKey};
use risc0_zkvm::guest::env;

fn main() {
    let raw: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = env::read();

    let mut verified = 0u32;
    for (public_key, message, signature) in &raw {
        let key = VerifyingKey::from_bytes(public_key).expect("a valid key");
        let parsed = Signature::try_from(&signature[..]).expect("a valid signature");
        verified += u32::from(key.verify_prehash(message, &parsed).is_ok());
    }

    assert_eq!(
        verified as usize,
        raw.len(),
        "a case did not verify, so the proof is of a refusal path"
    );
    env::commit(&verified);
}
