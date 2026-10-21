//! The plumbing program plus an M-of-N BIP-340 approval check, through the
//! GLV + wNAF backend.
//!
//! The verification is in the `plan`, which is where a multisig has to put it
//! on v0.3: an `apply` sees one effect and one shard, not the instruction, and
//! a plan has no access to account contents. So the member keys arrive with
//! the approvals rather than being read from the configuration, and what this
//! measures is the signature checking, not the configuration checking that
//! follows it.

use lee_core::program::{Plan, PlanInput};
use verifier_core::backend::bip340;

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
                let mut key = [0u8; 32];
                key.copy_from_slice(public_key);
                let mut raw = [0u8; 64];
                raw.copy_from_slice(signature);

                if bip340::verify(&key, &message, &raw).is_ok() {
                    approved += 1;
                    checksum ^= key[0];
                }
            }
            assert_eq!(
                approved as usize,
                approvals.len(),
                "every approval must verify, or the measurement is of the cheap refusal path"
            );

            // Carry the checksum into the effect so the verification cannot be
            // optimized away as dead code.
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
