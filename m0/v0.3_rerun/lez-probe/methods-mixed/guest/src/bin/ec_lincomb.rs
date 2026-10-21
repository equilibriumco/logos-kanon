//! The elliptic-curve half of an ECDSA recovery, on its own.
//!
//! ECDSA public-key recovery is, in outline:
//!
//! 1. parse `r` and `s`, and recover the point `R` from `r` by decompression;
//! 2. compute `u1 = -z/r` and `u2 = s/r`, two modular inversions;
//! 3. compute `Q = u1·G + u2·R` — two scalar multiplications and one addition.
//!
//! Step 3 is the part `risc0-bigint2` accelerates with dedicated circuits
//! (`ec_add_256`, `ec_double_256`, and scalar multiplication built on them), and
//! it is the part a native `ecrecover` could not make cheaper, because the curve
//! arithmetic still has to happen.
//!
//! This guest does *only* step 3. Subtracting it from a full recovery leaves
//! everything a native `ecrecover` would actually remove: decomposition and
//! table bookkeeping, conversions and marshalling, parsing, decompression, and
//! the surplus coprocessor round trips.
//!
//! Paired with `ec_recover.rs`, which runs the whole thing over the same inputs
//! and the same iteration count.

use core::hint::black_box;
use k256::elliptic_curve::group::GroupEncoding;
use k256::elliptic_curve::PrimeField;
use k256::{ProjectivePoint, Scalar};

/// Iterations, so the zkVM floor and the input setup are amortised rather than
/// charged to the measurement. Must match `ec_recover.rs`.
const ROUNDS: u32 = 4;

fn main() {
    let mut checksum = 0u8;

    for i in 0..ROUNDS {
        // Two scalars and a second point, varied per round so nothing folds.
        let u1 = Scalar::from_repr(scalar_bytes(i as u8 + 1).into()).unwrap();
        let u2 = Scalar::from_repr(scalar_bytes(i as u8 + 9).into()).unwrap();
        let r_point = ProjectivePoint::GENERATOR * Scalar::from(7u64 + u64::from(i));

        let q = black_box(
            ProjectivePoint::GENERATOR * black_box(u1) + black_box(r_point) * black_box(u2),
        );

        checksum ^= q.to_affine().to_bytes()[1];
    }

    risc0_zkvm::guest::env::commit(&checksum);
}

/// A valid, non-trivial scalar that differs per round.
fn scalar_bytes(seed: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[31] = seed;
    bytes[15] = seed.wrapping_mul(31).wrapping_add(7);
    bytes[0] = 1;
    bytes
}
