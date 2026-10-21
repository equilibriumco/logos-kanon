//! `lez_verify`, with the recovery done by `risc0-crypto-evm` instead of `k256`.
//!
//! Identical in every other respect: the same LEZ v0.3 `plan`/`apply` shape, the
//! same software keccak256 over the same 77-byte signed ranges, the same
//! per-package loop, the same effect written. Only the recovery call differs.
//!
//! This is what turns the substitution arithmetic into a measurement. Comparing
//! it against `lez_verify` gives the real program cost of adopting
//! `risc0-crypto-evm`, with the framework and the hashing in both figures rather
//! than assumed to cancel.
//!
//! One genuine difference in work done, and it favours this guest only slightly:
//! `secp256k1_ecrecover` returns a 20-byte address, having hashed the recovered
//! key to get there, where `k256` returns the key and the caller hashes it. A
//! RedStone verifier needs the address, so this is the shape the product would
//! actually use.

use lee_core::program::{Plan, PlanInput};
use tiny_keccak::{Hasher, Keccak};

/// `(signed_bytes, signature, recovery_id)` per signer. Must stay identical to
/// `lez_verify`'s.
type Package = (Vec<u8>, Vec<u8>, u8);
type Instruction = Vec<Package>;

fn main() {
    lee_core::program::run_program(
        |input: &PlanInput, packages: Instruction| {
            let mut recovered = 0u32;
            let mut checksum = 0u8;
            for (msg, sig_bytes, recid) in &packages {
                let mut hasher = Keccak::v256();
                hasher.update(msg);
                let mut digest = [0u8; 32];
                hasher.finalize(&mut digest);

                let mut sig = [0u8; 64];
                sig.copy_from_slice(sig_bytes);
                if let Some(address) =
                    risc0_crypto_evm::secp256k1_ecrecover(&sig, *recid, &digest)
                {
                    recovered += 1;
                    checksum ^= address.as_slice()[0];
                }
            }
            assert_eq!(
                recovered as usize,
                packages.len(),
                "every recovery must succeed, or the measurement is of the cheap error path"
            );

            let mut plan = Plan::new(input);
            let account = input
                .accounts
                .first()
                .expect("expected exactly one input account");
            plan.effect(account, &checksum);
            plan
        },
        |checksum: u8, shard: &[u8]| {
            let mut bytes = shard.to_vec();
            bytes.push(checksum);
            Some(bytes)
        },
    );
}
