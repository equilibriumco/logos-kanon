//! A LEZ v0.3 program that does no work beyond the framework's own call
//! handling: decode the call, name one shard, and write it back.
//!
//! Its cycle count is the floor a v0.3 program pays before any application
//! logic. v0.3 splits that floor in two: a `plan` run that decodes the
//! instruction and emits effects, and one `apply` run per shard the plan named.
//! Both are billed, so both are measured.

use lee_core::program::{Plan, PlanInput};

type Instruction = Vec<u8>;

fn main() {
    lee_core::program::run_program(
        |input: &PlanInput, _instruction: Instruction| {
            let mut plan = Plan::new(input);
            let account = input
                .accounts
                .first()
                .expect("expected exactly one input account");
            plan.effect(account, &0u8);
            plan
        },
        |_effect: u8, shard: &[u8]| Some(shard.to_vec()),
    );
}
