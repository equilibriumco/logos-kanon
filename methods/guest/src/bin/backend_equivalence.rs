//! Every backend, over the same signatures, inside the zkVM.
//!
//! The accelerated backends link only for the guest target, so no host test
//! can compare them against `InProgramBackend`. This guest can: it runs all
//! three over every `(message, signature)` pair it is given and asserts they
//! agree on the whole `Result`, byte for byte.
//!
//! That matters because the three are not the same code. `secp256k1_ecrecover`
//! applies EIP-2 low-`s` normalisation and would accept malleable signatures
//! `k256` rejects, so `R0CryptoBackend` re-implements the range and
//! malleability checks itself. `FastRecoveryBackend` goes further and owns the
//! whole recovery: GLV decomposition, wNAF digits, precomputed tables — all
//! the code where a sign error produces a *plausible* wrong address. This is
//! what says they got it right — over real RedStone-shaped signatures, and
//! over the malformed and adversarial ones `tests/backend_equivalence.rs`
//! constructs.

#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

use verifier_core::backend::{
    FastRecoveryBackend, InProgramBackend, R0CryptoBackend, Signature, VerifierBackend,
};

fn main() {
    // `(message_hash, signature)` pairs: the hash arrives precomputed because
    // several cases — a zero digest, one at the group order — cannot be
    // produced by hashing anything. `[u8; 65]` has no serde impl, so both
    // arrive as `Vec<u8>`.
    let cases: Vec<(Vec<u8>, Vec<u8>)> = risc0_zkvm::guest::env::read();

    let k256 = InProgramBackend::new();
    let r0 = R0CryptoBackend::new();
    let fast = FastRecoveryBackend::new();

    let mut agreed = 0u32;
    for (hash_bytes, sig_bytes) in &cases {
        let mut raw = [0u8; Signature::LEN];
        raw.copy_from_slice(sig_bytes);
        let signature = Signature(raw);

        let mut hash = [0u8; 32];
        hash.copy_from_slice(hash_bytes);

        // Hashing stays held to agreement too: every backend digests the
        // case's own bytes and the digests must match.
        let keccak_sample = k256.keccak256(sig_bytes);
        assert_eq!(
            keccak_sample,
            r0.keccak256(sig_bytes),
            "r0crypto hashed differently"
        );
        assert_eq!(
            keccak_sample,
            fast.keccak256(sig_bytes),
            "fast hashed differently"
        );

        let reference = k256.recover_signer(&hash, &signature);

        // Equality over the whole `Result`: the same address on success, and
        // the same `BackendError` on refusal. A backend that accepted what
        // another refused would be the defect worth catching, and it would not
        // show up in a comparison of successes alone.
        assert_eq!(
            reference,
            r0.recover_signer(&hash, &signature),
            "r0crypto disagreed with k256"
        );
        assert_eq!(
            reference,
            fast.recover_signer(&hash, &signature),
            "fast recovery disagreed with k256"
        );
        agreed += 1;
    }

    risc0_zkvm::guest::env::commit(&agreed);
}
