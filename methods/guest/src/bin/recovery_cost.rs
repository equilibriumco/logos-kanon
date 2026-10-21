//! What one recovery costs, per backend, in isolation.
//!
//! The phase tables measure recovery inside the product guest, where it is
//! blended with hashing, decoding and the program around it. This guest
//! brackets `recover_signer` alone, for each backend over the same inputs, so
//! the figure a backend is chosen by is the backend's and nothing else's.
//!
//! Its own binary rather than a stage of another guest, for the reason
//! `COSTS.md` records: a cycle count is a property of an ELF, so a stage added
//! to a shared guest moves every figure that guest publishes.
//!
//! Elision is the measurement hazard here, as it was for every figure in
//! `COSTS.md`: each recovered address is folded into a checksum the guest
//! commits, so no recovery's result is a value nothing reads.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use risc0_zkvm::guest::env;
use verifier_core::backend::{
    FastRecoveryBackend, InProgramBackend, R0CryptoBackend, Signature, VerifierBackend,
};

/// Runs every case through one backend, returning spent cycles and a checksum
/// that keeps each address alive.
fn measure<B: VerifierBackend>(backend: &B, cases: &[([u8; 32], Signature)]) -> (u64, u32) {
    let mut checksum = 0u32;
    let before = env::cycle_count();
    for (hash, signature) in cases {
        let address = backend
            .recover_signer(hash, signature)
            .expect("the cost cases are valid signatures");
        checksum = address
            .as_bytes()
            .iter()
            .fold(checksum, |sum, byte| sum.wrapping_add(u32::from(*byte)));
    }
    (env::cycle_count() - before, checksum)
}

fn main() {
    let raw: Vec<(Vec<u8>, Vec<u8>)> = env::read();
    let cases: Vec<([u8; 32], Signature)> = raw
        .iter()
        .map(|(hash, sig)| {
            let mut h = [0u8; 32];
            h.copy_from_slice(hash);
            let mut s = [0u8; Signature::LEN];
            s.copy_from_slice(sig);
            (h, Signature(s))
        })
        .collect();

    let (in_program, sum_a) = measure(&InProgramBackend::new(), &cases);
    let (r0crypto, sum_b) = measure(&R0CryptoBackend::new(), &cases);
    let (fast, sum_c) = measure(&FastRecoveryBackend::new(), &cases);

    // The checksums agreeing is a weak equivalence check; the strong one is
    // `backend_equivalence`. Here it is what keeps the optimiser honest.
    assert_eq!(sum_a, sum_b, "the backends recovered different addresses");
    assert_eq!(sum_a, sum_c, "the backends recovered different addresses");

    let count = cases.len() as u64;
    env::commit(&(count, in_program, r0crypto, fast, sum_a));
}
