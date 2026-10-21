//! Per-recovery cycles, per backend, measured in isolation.
//!
//! `recovery_cost.rs` (the guest) brackets `recover_signer` alone over the
//! same signatures for every backend; this runs it and prints the table. The
//! figures are not pinned exactly — they are a property of that guest's ELF
//! and move with any dependency bump — but the *ordering* is pinned, because
//! the ordering is the reason `FastRecoveryBackend` exists: a change that
//! makes it slower than `secp256k1_ecrecover` has removed its reason to be
//! wired into the product guests, and should fail loudly here rather than
//! surface as a drifted phase table.

use risc0_zkvm::{default_executor, ExecutorEnv};

/// Valid `(message_hash, signature)` pairs, enough of them to average out the
/// scalar-dependent wobble of a double-and-add.
fn cases() -> Vec<(Vec<u8>, Vec<u8>)> {
    use k256::ecdsa::SigningKey;
    use tiny_keccak::{Hasher, Keccak};

    (1..=10u8)
        .map(|seed| {
            let mut scalar = [0u8; 32];
            scalar[31] = seed;
            scalar[7] = seed.wrapping_mul(29);
            let key = SigningKey::from_bytes(&scalar.into()).expect("a valid scalar");

            let mut hasher = Keccak::v256();
            hasher.update(format!("recovery cost case {seed}").as_bytes());
            let mut digest = [0u8; 32];
            hasher.finalize(&mut digest);

            let (signature, recid) = key
                .sign_prehash_recoverable(&digest)
                .expect("signing succeeds");
            let mut raw = signature.to_bytes().to_vec();
            raw.push(recid.to_byte());
            (digest.to_vec(), raw)
        })
        .collect()
}

#[test]
fn the_fast_backend_beats_what_it_replaced() {
    let cases = cases();
    let env = ExecutorEnv::builder()
        .write(&cases)
        .expect("the cases serialise")
        .build()
        .expect("an executor environment");

    let session = default_executor()
        .execute(env, kanon_methods::RECOVERY_COST_ELF)
        .expect("the guest runs to completion");

    let (count, in_program, r0crypto, fast, _checksum): (u64, u64, u64, u64, u32) = session
        .journal
        .decode()
        .expect("the guest commits the figures");
    assert_eq!(count, cases.len() as u64);

    println!("\n| backend | cycles / recovery |");
    println!("| --- | ---: |");
    println!("| in-program (patched k256) | {} |", in_program / count);
    println!("| risc0-crypto-evm | {} |", r0crypto / count);
    println!("| fast (GLV + wNAF + fixed base) | {} |", fast / count);

    assert!(
        fast < r0crypto,
        "the fast backend costs {} per recovery against risc0-crypto-evm's {} — \
         it has lost its reason to be wired in",
        fast / count,
        r0crypto / count
    );
    assert!(
        r0crypto < in_program,
        "risc0-crypto-evm ({}) should beat the patched k256 loop ({})",
        r0crypto / count,
        in_program / count
    );
}
