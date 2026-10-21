//! The same program as `multisig_fast`, verifying through `k256`'s Schnorr
//! implementation over RISC Zero's patched curve arithmetic.
//!
//! `verify_prehash` rather than `verify`: the latter digests its argument with
//! SHA-256 first, so the two programs would be verifying over different
//! messages and their cycle counts would not be a comparison.

use k256::schnorr::signature::hazmat::PrehashVerifier;
use k256::schnorr::{Signature, VerifyingKey};
use lee_core::program::{Plan, PlanInput};

/// `(public_key, signature)` per approver, over one proposal digest.
type Approval = (Vec<u8>, Vec<u8>);
type Instruction = (Vec<u8>, Vec<Approval>);

fn main() {
    lee_core::program::run_program(
        |input: &PlanInput, (digest, approvals): Instruction| {
            let mut message = [0u8; 32];
            message.copy_from_slice(&digest);

            let mut approved = 0u32;
            let mut checksum = 0u8;
            for (public_key, signature) in &approvals {
                let mut key_bytes = [0u8; 32];
                key_bytes.copy_from_slice(public_key);

                let key = VerifyingKey::from_bytes(&key_bytes).expect("a valid key");
                let parsed = Signature::try_from(&signature[..]).expect("a valid signature");
                if key.verify_prehash(&message, &parsed).is_ok() {
                    approved += 1;
                    checksum ^= key_bytes[0];
                }
            }
            assert_eq!(
                approved as usize,
                approvals.len(),
                "every approval must verify, or the measurement is of the cheap refusal path"
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
