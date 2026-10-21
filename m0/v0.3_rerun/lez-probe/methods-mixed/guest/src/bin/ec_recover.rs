//! A full ECDSA public-key recovery, over the same inputs and iteration count as
//! `ec_lincomb.rs`.
//!
//! `ec_recover - ec_lincomb` is what a native `ecrecover` could remove: the
//! scalar decomposition and lookup tables, the projective-to-affine conversions
//! and word marshalling, signature parsing, point decompression, the modular
//! inversions, and the surplus coprocessor round trips.
//!
//! `ec_lincomb` itself is the floor — the curve arithmetic, already in a
//! `risc0-bigint2` circuit, which no precompile can make cheaper because the
//! work still has to happen.

use core::hint::black_box;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

/// Must match `ec_lincomb.rs`.
const ROUNDS: u32 = 4;

fn main() {
    // One signature, read from the host so nothing is constant-folded, and so
    // the figure is over a real signature rather than a synthetic scalar.
    let (digest, sig_bytes, recid): ([u8; 32], Vec<u8>, u8) = risc0_zkvm::guest::env::read();

    let mut checksum = 0u8;
    for _ in 0..ROUNDS {
        let sig = Signature::from_slice(black_box(&sig_bytes)).expect("malformed signature");
        let recid = RecoveryId::from_byte(recid).expect("bad recovery id");
        let vk = VerifyingKey::recover_from_prehash(black_box(&digest), &sig, recid)
            .expect("recovery must succeed");
        checksum ^= vk.to_encoded_point(false).as_bytes()[1];
    }

    risc0_zkvm::guest::env::commit(&checksum);
}
