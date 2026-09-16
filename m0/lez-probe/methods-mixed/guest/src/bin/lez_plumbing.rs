//! `lez_noop` carrying the *same* instruction as `lez_verify`, and doing
//! everything that program does except the cryptography.
//!
//! `lez_noop` is measured on an empty instruction, so its cycle count is the
//! floor a LEZ program pays with nothing to carry. Two parts of the framework's
//! own work scale with the instruction, and therefore with the signer count:
//! `read_lee_inputs` deserializes the instruction words into `Instruction`, and
//! `ProgramOutput` echoes `instruction_data` straight back out. Neither is in the
//! floor, so subtracting the floor from `lez_verify` leaves both folded into what
//! would otherwise read as the cost of recovery.
//!
//! This program isolates them. `lez_verify - lez_plumbing` is the cryptography
//! alone; `lez_plumbing - lez_noop` is the framework's per-package handling.

use lee_core::program::{read_lee_inputs, AccountPostState, Claim, ProgramInput, ProgramOutput};

/// `(signed_bytes, signature, recovery_id)` per signer. Must stay identical to
/// `lez_verify`'s, or the two are not deserializing the same thing.
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

    // The deserialized instruction is read from an in-memory slice, so an unused
    // result could be optimized away and the measurement would understate the
    // handling cost. Folding the field lengths keeps it live at a cost of a few
    // cycles per package, and mirrors `lez_verify` writing its checksum into
    // account data for exactly the same reason.
    let mut checksum = 0u8;
    for (msg, sig_bytes, recid) in &packages {
        checksum ^= (msg.len() as u8) ^ (sig_bytes.len() as u8) ^ recid;
    }

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
