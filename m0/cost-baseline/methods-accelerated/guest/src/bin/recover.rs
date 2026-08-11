use risc0_zkvm::guest::env;

fn main() {
    let (do_work, digest, sigs): (bool, Vec<u8>, Vec<(Vec<u8>, u8)>) = env::read();
    env::commit(&bench_lib::recover_bench(do_work, &digest, &sigs));
}
