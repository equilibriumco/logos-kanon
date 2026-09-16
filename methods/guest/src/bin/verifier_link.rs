//! Links `verifier-core` into a real guest ELF and commits its version.
//!
//! It exists so the `riscv32` cross-compile of *product* code is exercised
//! continuously. The harnesses in `m0/` proved the toolchain on measurement code;
//! this proves it on the crate that ships, and it fails at the point a
//! dependency added to `verifier-core` turns out not to be guest-compatible
//! rather than once a milestone's worth of work has been built on it.
//!
//! It is not a benchmark and not the aggregator. Real guest work arrives with
//! the verification path, and the SPEL program that carries it comes later.

use risc0_zkvm::guest::env;

fn main() {
    // Touching the price account type is what keeps it linked: an unused
    // dependency can be dropped by the linker, and a dependency that is never
    // linked never proves it cross-compiles.
    let _ = core::mem::size_of::<kanon_idl::OraclePriceAccount>();
    env::commit_slice(verifier_core::VERSION.as_bytes());
}
