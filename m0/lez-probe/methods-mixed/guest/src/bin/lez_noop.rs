//! A LEZ program that does no work beyond the framework's own input/output
//! handling: read the LEE inputs, echo the account back as its post state.
//!
//! Its cycle count is therefore the floor a LEZ program pays before any
//! application logic: account deserialization, instruction decode, and the
//! serialization of the proposed state diff.

use lee_core::program::{read_lee_inputs, AccountPostState, Claim, ProgramInput, ProgramOutput};

type Instruction = Vec<u8>;

fn main() {
    let (
        ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction: _,
        },
        instruction_data,
    ) = read_lee_inputs::<Instruction>();

    let [pre_state] = pre_states
        .try_into()
        .unwrap_or_else(|_| panic!("expected exactly one input account"));

    let post_state =
        AccountPostState::new_claimed_if_default(pre_state.account.clone(), Claim::Authorized);

    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_data,
        vec![pre_state],
        vec![post_state],
    )
    .write();
}
