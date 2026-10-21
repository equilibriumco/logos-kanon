//! The floor carrying the *same* instruction as the two verifying programs,
//! and doing everything they do except the cryptography.
//!
//! The floor guest decodes an empty instruction, so the part of the
//! framework's work that scales with the approval count is absent from it.
//! Subtracting the floor alone would fold borsh decoding into what reads as
//! the cost of verification. This program isolates it: a verifying program
//! minus this one is the cryptography, and this one minus the floor is the
//! per-approval handling.

use lee_core::program::{Plan, PlanInput};

/// `(public_key, signature)` per approver, over one proposal digest. Must stay
/// identical in all three programs, or they are not decoding the same thing.
type Approval = (Vec<u8>, Vec<u8>);
type Instruction = (Vec<u8>, Vec<Approval>);

fn main() {
    lee_core::program::run_program(
        |input: &PlanInput, (digest, approvals): Instruction| {
            // The decoded instruction lives in memory, so an unused result
            // could be optimized away and the measurement would understate the
            // handling cost. Folding the field lengths keeps it live at a cost
            // of a few cycles per approval.
            let mut checksum = digest.len() as u8;
            for (public_key, signature) in &approvals {
                checksum ^= (public_key.len() as u8) ^ (signature.len() as u8);
            }

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
