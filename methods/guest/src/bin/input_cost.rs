//! What a transaction pays before its body is entered (M3-10).
//!
//! Every other figure in `COSTS.md` is a program *body*. LEZ reads a transaction's
//! inputs first, and those cycles are spent whatever the program then does. ADR 26
//! put the read at "about 113 cycles" from one end-to-end observation — a
//! 127,814-byte payload whose transaction overran the budget, with 14.5M of the
//! overrun attributed to the read by subtraction. This measures it directly.
//!
//! # It reads what LEZ reads, in the order LEZ reads it
//!
//! `read_lee_inputs` does four `env::read()` calls: the program's own id, the
//! caller's, the pre-state accounts, then the instruction words. This guest does
//! the same four and no other work, so what varies between runs is only what those
//! reads were given.
//!
//! A run is therefore not the read alone: it also carries zkVM startup and the
//! journal commit below, as any guest run does. That floor is the same in every
//! run, so differencing two input sizes removes it and the marginal rates are
//! clean — while a single run quoted by itself is a gross figure, and the host
//! test says so where it publishes one.
//!
//! It matters that the four reads are the real four. A model that multiplied a
//! word count by a rate would have nowhere to put what four framed reads cost
//! before any account or instruction word does, and nowhere to put an account
//! costing more than its width.
//!
//! # The stages
//!
//! There are none: the *input* is the variable. The host runs this over inputs of
//! different shapes and differences them, which is the same discipline `cost.rs`
//! uses with a stage number instead.
//!
//! # Panics
//!
//! None. The journal carries back what was read so the host can assert the guest
//! read what it was given — a read that was skipped would report as a very cheap
//! one rather than as a failure.

use nssa_core::{account::AccountWithMetadata, program::ProgramId};
use risc0_zkvm::guest::env;

fn main() {
    // The order and the types `read_lee_inputs` uses. Anything else would be a
    // measurement of a different transaction shape.
    let self_program_id: ProgramId = env::read();
    let caller_program_id: Option<ProgramId> = env::read();
    let pre_states: Vec<AccountWithMetadata> = env::read();
    let instruction_words: Vec<u32> = env::read();

    // Kept alive, because values nothing observes are values the optimiser may
    // decline to materialise -- and this guest does nothing else, so an elision
    // would leave a figure that still looked like a measurement. `COSTS.md`
    // records the two earlier times that happened here.
    core::hint::black_box(&self_program_id);
    core::hint::black_box(&caller_program_id);
    core::hint::black_box(&pre_states);
    core::hint::black_box(&instruction_words);

    env::commit(&(pre_states.len() as u32, instruction_words.len() as u32));
}
