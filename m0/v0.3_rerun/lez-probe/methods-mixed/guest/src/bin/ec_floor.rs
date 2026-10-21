//! The zkVM floor for the EC probes: reads the same shaped input and commits,
//! doing no curve work. Subtracted from both `ec_lincomb` and `ec_recover` so
//! neither carries the harness.

fn main() {
    let _: () = risc0_zkvm::guest::env::read();
    risc0_zkvm::guest::env::commit(&0u8);
}
