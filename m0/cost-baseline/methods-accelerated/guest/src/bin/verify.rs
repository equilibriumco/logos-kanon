use risc0_zkvm::guest::env;

type Package = (Vec<u8>, Vec<u8>, Vec<u8>, u8);

fn main() {
    let (hash, packages): (bool, Vec<Package>) = env::read();
    env::commit(&bench_lib::verify_bench(hash, &packages));
}
