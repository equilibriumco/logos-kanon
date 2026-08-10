//! The floor: what a guest costs before it does anything. Every other number is
//! only meaningful relative to this.

use risc0_zkvm::guest::env;

fn main() {
    env::commit(&0u32);
}
