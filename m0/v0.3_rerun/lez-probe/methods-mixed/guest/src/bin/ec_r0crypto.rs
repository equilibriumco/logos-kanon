//! The same recovery through `risc0-crypto-evm`'s `secp256k1_ecrecover`.
//!
//! Compared against `ec_recover.rs`, which does the same work through the
//! patched `k256` this build ships. Same signature, same digest, same
//! iteration count.
//!
//! Note this guest does *more* work, not less: `secp256k1_ecrecover` recovers
//! the public key and then hashes it to a 20-byte address, which is what
//! RedStone verification needs anyway. `ec_recover.rs` stops at the public key.
//! So any saving measured here is a lower bound on the saving available.

use core::hint::black_box;

const ROUNDS: u32 = 4;

fn main() {
    let (digest, sig_bytes, recid): ([u8; 32], Vec<u8>, u8) = risc0_zkvm::guest::env::read();
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&sig_bytes);

    let mut checksum = 0u8;
    for _ in 0..ROUNDS {
        let address = risc0_crypto_evm::secp256k1_ecrecover(
            black_box(&sig),
            black_box(recid),
            black_box(&digest),
        )
        .expect("recovery must succeed");
        checksum ^= address.as_slice()[0];
    }

    risc0_zkvm::guest::env::commit(&checksum);
}
