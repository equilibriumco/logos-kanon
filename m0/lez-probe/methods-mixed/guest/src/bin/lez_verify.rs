//! The same LEZ program as `lez_noop`, plus a full M-of-N RedStone verification:
//! per data package, keccak256 over that signer's signed byte range, then
//! secp256k1 public-key recovery.
//!
//! Subtracting `lez_noop` gives the verification cost inside a real LEZ program,
//! and comparing that against the bare-RISC-Zero figure isolates what the LEZ
//! framework adds around it.
//!
//! The crypto is duplicated here rather than shared with the frozen cost
//! baseline, deliberately: this probe is independent of it. M1 unifies both onto
//! a single `verifier-core`, at which point this copy goes away.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use lee_core::program::{read_lee_inputs, AccountPostState, Claim, ProgramInput, ProgramOutput};
use tiny_keccak::{Hasher, Keccak};

/// `(signed_bytes, signature, recovery_id)` per signer.
type Package = (Vec<u8>, Vec<u8>, u8);
type Instruction = Vec<Package>;

fn main() {
    let (
        ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction: packages,
        },
        instruction_data,
    ) = read_lee_inputs::<Instruction>();

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

    // Write the checksum into account data so the verification cannot be
    // optimized away as dead code.
    let [pre_state] = pre_states
        .try_into()
        .unwrap_or_else(|_| panic!("expected exactly one input account"));
    let post_account = {
        let mut account = pre_state.account.clone();
        let mut bytes = account.data.into_inner();
        bytes.push(checksum);
        account.data = bytes.try_into().expect("data should fit the account limit");
        account
    };

    let post_state = AccountPostState::new_claimed_if_default(post_account, Claim::Authorized);
    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_data,
        vec![pre_state],
        vec![post_state],
    )
    .write();
}
