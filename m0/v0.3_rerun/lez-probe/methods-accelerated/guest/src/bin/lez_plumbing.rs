//! `lez_noop` carrying the *same* instruction as `lez_verify`, and doing
//! everything that program does except the cryptography.
//!
//! `lez_noop` is measured on an empty instruction, so its cycle count is the
//! floor a LEZ program pays with nothing to carry. The part of the framework's
//! own work that scales with the signer count — borsh-decoding the instruction
//! into `Instruction` — is not in that floor, so subtracting the floor from
//! `lez_verify` would fold it into what reads as the cost of recovery.
//!
//! This program isolates it. `lez_verify - lez_plumbing` is the cryptography
//! alone; `lez_plumbing - lez_noop` is the framework's per-package handling.

use lee_core::program::{Plan, PlanInput};

/// `(signed_bytes, signature, recovery_id)` per signer. Must stay identical to
/// `lez_verify`'s, or the two are not decoding the same thing.
type Package = (Vec<u8>, Vec<u8>, u8);
type Instruction = Vec<Package>;

fn main() {
    lee_core::program::run_program(
        |input: &PlanInput, packages: Instruction| {
            // The decoded instruction lives in memory, so an unused result could
            // be optimized away and the measurement would understate the
            // handling cost. Folding the field lengths keeps it live at a cost
            // of a few cycles per package, and mirrors `lez_verify` carrying its
            // checksum into the effect for exactly the same reason.
            let mut checksum = 0u8;
            for (msg, sig_bytes, recid) in &packages {
                checksum ^= (msg.len() as u8) ^ (sig_bytes.len() as u8) ^ recid;
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
