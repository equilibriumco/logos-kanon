//! Per-verification cycles for BIP-340, per implementation.
//!
//! The ordering is pinned, not the figures: a cycle count is a property of the
//! guest ELF and moves with any dependency bump, but a backend that stopped
//! beating the patched `k256` path would have lost its reason to exist, and
//! that should fail here rather than surface as a drifted table.

mod support;

use risc0_zkvm::{default_executor, ExecutorEnv};

/// Enough cases to average out the scalar-dependent wobble of a
/// double-and-add.
const CASES: usize = 10;

#[test]
fn the_backend_beats_the_patched_k256_path() {
    let cases = support::valid_cases(CASES);
    let env = ExecutorEnv::builder()
        .write(&cases)
        .expect("the cases serialise")
        .build()
        .expect("an executor environment");

    let session = default_executor()
        .execute(env, bip340_bench::BIP340_COST_ELF)
        .expect("the guest runs to completion");

    let (count, reference, backend): (u64, u64, u64) = session
        .journal
        .decode()
        .expect("the guest commits the figures");
    assert_eq!(count, cases.len() as u64);

    println!("\n| implementation | cycles / verification |");
    println!("| --- | ---: |");
    println!("| patched k256 schnorr | {} |", reference / count);
    println!("| fast (GLV + wNAF + fixed base) | {} |", backend / count);

    assert!(
        backend < reference,
        "the backend costs {} per verification against the patched k256 path's {}",
        backend / count,
        reference / count
    );
}
