//! What one BIP-340 verification costs, per implementation, in isolation.
//!
//! Both run over the same signatures in the same ELF, so the difference is the
//! scalar multiplication and nothing else: `k256` through RISC Zero's patched
//! curve arithmetic, and the backend through GLV, wNAF and a fixed-base table
//! over the same circuits.
//!
//! Each measured window spans raw bytes to verdict. `k256` parses the key and
//! the signature into its own types first and the backend takes the bytes as
//! they arrive, so the parsing sits inside whichever window needs it and
//! neither is charged for the other's shape.
//!
//! Each verification is counted and the count is committed and asserted, so
//! the optimiser cannot drop one whose result nothing reads.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use k256::schnorr::signature::hazmat::PrehashVerifier;
use k256::schnorr::{Signature as ReferenceSignature, VerifyingKey};
use risc0_zkvm::guest::env;
use verifier_core::backend::bip340;

/// A case, already in the shapes both implementations take.
struct Case {
    public_key: [u8; 32],
    message: [u8; 32],
    signature: [u8; 64],
}

/// Cycles spent verifying every case through `k256`, and a count of the ones
/// that verified.
fn measure_reference(cases: &[Case]) -> (u64, u32) {
    let mut verified = 0u32;
    let before = env::cycle_count();
    for case in cases {
        let key = VerifyingKey::from_bytes(&case.public_key).expect("a valid key");
        let signature =
            ReferenceSignature::try_from(&case.signature[..]).expect("a valid signature");
        verified += u32::from(key.verify_prehash(&case.message, &signature).is_ok());
    }
    (env::cycle_count() - before, verified)
}

/// The same, through the backend.
fn measure_backend(cases: &[Case]) -> (u64, u32) {
    let mut verified = 0u32;
    let before = env::cycle_count();
    for case in cases {
        verified +=
            u32::from(bip340::verify(&case.public_key, &case.message, &case.signature).is_ok());
    }
    (env::cycle_count() - before, verified)
}

fn main() {
    let raw: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = env::read();
    let cases: Vec<Case> = raw
        .iter()
        .map(|(public_key, message, signature)| {
            let mut case = Case {
                public_key: [0u8; 32],
                message: [0u8; 32],
                signature: [0u8; 64],
            };
            case.public_key.copy_from_slice(public_key);
            case.message.copy_from_slice(message);
            case.signature.copy_from_slice(signature);
            case
        })
        .collect();

    let (reference, verified_by_reference) = measure_reference(&cases);
    let (backend, verified_by_backend) = measure_backend(&cases);

    let count = cases.len() as u32;
    assert_eq!(
        verified_by_reference, count,
        "a cost case did not verify through k256, so the figure is of a refusal path"
    );
    assert_eq!(
        verified_by_backend, count,
        "a cost case did not verify through the backend"
    );

    env::commit(&(u64::from(count), reference, backend));
}
