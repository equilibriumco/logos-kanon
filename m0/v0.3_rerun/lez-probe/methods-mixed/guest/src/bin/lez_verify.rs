//! The same LEZ program as `lez_noop`, plus a full M-of-N RedStone verification:
//! per data package, keccak256 over that signer's signed byte range, then
//! secp256k1 public-key recovery.
//!
//! Subtracting `lez_noop` gives the verification cost inside a real LEZ program,
//! and comparing that against the bare-RISC-Zero figure isolates what the LEZ
//! framework adds around it.
//!
//! The verification is in the `plan`, which is where it has to be on v0.3: an
//! `apply` sees one effect and one shard, not the instruction.
//!
//! The crypto is duplicated here rather than shared with the frozen cost
//! baseline, deliberately: this probe is independent of it.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use lee_core::program::{Plan, PlanInput};
use tiny_keccak::{Hasher, Keccak};

/// `(signed_bytes, signature, recovery_id)` per signer.
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

                let sig = Signature::from_slice(sig_bytes).expect("malformed signature");
                let recid = RecoveryId::from_byte(*recid).expect("bad recovery id");
                if let Ok(vk) = VerifyingKey::recover_from_prehash(&digest, &sig, recid) {
                    recovered += 1;
                    checksum ^= vk.to_encoded_point(false).as_bytes()[1];
                }
            }
            assert_eq!(
                recovered as usize,
                packages.len(),
                "every recovery must succeed, or the measurement is of the cheap error path"
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
