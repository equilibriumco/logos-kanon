use risc0_zkvm::guest::env;

fn main() {
    let (do_work, iters, msg_len): (bool, u32, u32) = env::read();
    env::commit(&bench_lib::keccak_loop_overhead(do_work, iters, msg_len));
}
