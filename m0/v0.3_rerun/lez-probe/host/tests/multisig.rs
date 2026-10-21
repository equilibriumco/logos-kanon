//! What an M-of-N BIP-340 approval check costs inside a LEZ v0.3 program.
//!
//! The figure a multisig is bounded by is gas, and gas is cycles summed over
//! every metered run: the `plan` that verifies the approvals, plus one `apply`
//! per shard the plan names. A transaction may declare at most
//! `LEZ_GAS_LIMIT`, so the share of that limit is what says whether a
//! threshold is viable.
//!
//! Two subtractions make the figure attributable. A verifying program minus
//! the plumbing program is the cryptography alone; the plumbing program minus
//! the floor is what the framework spends carrying the approvals. Both terms
//! come out of the same dependency graph, which is what makes them
//! subtractable.
//!
//! This is public execution inside a LEZ program. It is not a private
//! transaction over private accounts, where the privacy circuit runs on top of
//! everything measured here.
//!
//! The proving tests are ignored by default and take minutes. Each covers one
//! implementation, so running them one at a time attributes peak memory:
//!
//! ```sh
//! cargo test --release -p lez-probe --test multisig -- --nocapture
//! /usr/bin/time -l cargo test --release -p lez-probe --test multisig \
//!   -- --ignored --nocapture proving_through_the_fast_backend
//! ```

use k256::schnorr::signature::hazmat::PrehashSigner;
use k256::schnorr::{Signature, SigningKey};
use lez_probe::{measure, multisig, LEZ_GAS_LIMIT};

/// The unit, the RFP's required threshold, and twice it.
const THRESHOLDS: &[usize] = &[1, 5, 10];

/// `(public_key, signature)` per approver, matching the guests.
type Approval = (Vec<u8>, Vec<u8>);

/// The proposal digest every approver signs.
const DIGEST: [u8; 32] = {
    let mut bytes = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        bytes[i] = (i as u8).wrapping_mul(7).wrapping_add(11);
        i += 1;
    }
    bytes
};

/// `count` approvals over [`DIGEST`], from deterministic member keys.
fn approvals(count: usize) -> (Vec<u8>, Vec<Approval>) {
    let signed = (1..=count)
        .map(|seed| {
            let seed = seed as u8;
            let mut scalar = [0u8; 32];
            scalar[31] = seed;
            scalar[11] = seed.wrapping_mul(37).wrapping_add(3);
            let key = SigningKey::from_bytes(&scalar).expect("a valid scalar");

            // `sign_prehash` rather than `sign`: the `Signer` impl digests its
            // argument with SHA-256 first, so the guests would be verifying
            // over a different message than the one signed.
            let signature: Signature = key.sign_prehash(&DIGEST).expect("signing succeeds");
            (
                key.verifying_key().to_bytes().to_vec(),
                signature.to_bytes().to_vec(),
            )
        })
        .collect();

    (DIGEST.to_vec(), signed)
}

/// Billed gas for one program over `count` approvals.
fn gas_of(elf: &[u8], count: usize) -> u64 {
    measure(elf, &approvals(count), false, 1)
        .expect("execution")
        .cycles()
}

#[test]
fn an_approval_check_inside_a_lez_program() {
    let guests = multisig();

    let floor = measure(guests.noop, &Vec::<u8>::new(), false, 1)
        .expect("execution")
        .cycles();
    println!("\nframework floor, no instruction: {floor} gas");

    println!("\n| M | program | plan | apply | billed | % of cap | crypto | per approval |");
    println!("| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |");

    for &count in THRESHOLDS {
        let plumbing = gas_of(guests.plumbing, count);

        for (name, elf) in [("k256", guests.k256), ("fast", guests.fast)] {
            let run = measure(elf, &approvals(count), false, 1).expect("execution");
            let billed = run.cycles();
            let crypto = billed - plumbing;
            println!(
                "| {count} | {name} | {} | {} | {billed} | {:.2}% | {crypto} | {} |",
                run.plan,
                run.applies.iter().sum::<u64>(),
                run.gas_pct(),
                crypto / count as u64
            );

            assert!(
                billed < LEZ_GAS_LIMIT,
                "{name} at M = {count} exceeds the gas a transaction may declare"
            );
        }

        println!(
            "| {count} | plumbing | | | {plumbing} | {:.2}% | | |",
            100.0 * plumbing as f64 / LEZ_GAS_LIMIT as f64
        );
    }
    println!();

    // The ordering is the reason the fast backend is worth carrying; the
    // figures themselves move with any dependency bump.
    let fast = gas_of(guests.fast, 5) - gas_of(guests.plumbing, 5);
    let reference = gas_of(guests.k256, 5) - gas_of(guests.plumbing, 5);
    assert!(
        fast < reference,
        "the fast backend costs {fast} gas at M = 5 against the patched k256 path's {reference}"
    );
}

/// Proves the plan at each threshold and prints seconds and receipt bytes.
fn proving_table(name: &str, elf: &[u8]) {
    // The replacement the deprecation names, `ProverOpts::is_dev_mode`, does
    // not exist in the pinned risc0, so this is the check available.
    #[allow(deprecated)]
    let dev_mode = risc0_zkvm::is_dev_mode();
    assert!(
        !dev_mode,
        "RISC0_DEV_MODE is on, so nothing here is proved and every time is meaningless"
    );

    println!("\n| M | program | billed | prove (s) | receipt (B) |");
    println!("| ---: | --- | ---: | ---: | ---: |");
    for &count in THRESHOLDS {
        let run = measure(elf, &approvals(count), true, 1).expect("proving");
        println!(
            "| {count} | {name} | {} | {:.1} | {} |",
            run.cycles(),
            run.prove_secs().expect("a proving sample"),
            run.proof_bytes.expect("a receipt size")
        );
    }
    println!();
}

#[test]
#[ignore = "real proving, minutes"]
fn proving_through_the_fast_backend() {
    proving_table("fast", multisig().fast);
}

#[test]
#[ignore = "real proving, minutes"]
fn proving_through_k256() {
    proving_table("k256", multisig().k256);
}
