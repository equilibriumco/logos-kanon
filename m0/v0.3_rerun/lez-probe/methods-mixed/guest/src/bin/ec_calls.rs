//! What one `risc0-bigint2` accelerator round trip costs.
//!
//! `AffinePoint::mul` is not a single accelerated operation. It is a
//! double-and-add loop running in ordinary guest code, calling the
//! `ec_double_256` and `ec_add_256` blobs once per scalar bit — roughly 256
//! doubles and 128 adds for a 256-bit scalar.
//!
//! So the cost of a scalar multiplication is dominated by the number of round
//! trips rather than by the arithmetic in any one of them. This guest performs
//! `ROUNDS` doublings and nothing else, so dividing out gives the cost of a
//! single call — which is what says whether a blob that looped internally would
//! be worth asking for.

use core::hint::black_box;
use risc0_bigint2::ec::{AffinePoint, Secp256k1Curve, EC_256_WIDTH_WORDS};

/// Enough that the floor and the setup are noise.
const ROUNDS: u32 = 256;

const GX: [u32; EC_256_WIDTH_WORDS] = [
    0x16F81798, 0x59F2815B, 0x2DCE28D9, 0x029BFCDB, 0xCE870B07, 0x55A06295, 0xF9DCBBAC, 0x79BE667E,
];
const GY: [u32; EC_256_WIDTH_WORDS] = [
    0xFB10D4B8, 0x9C47D08F, 0xA6855419, 0xFD17B448, 0x0E1108A8, 0x5DA4FBFC, 0x26A3C465, 0x483ADA77,
];

type P = AffinePoint<EC_256_WIDTH_WORDS, Secp256k1Curve>;

fn main() {
    let mut a = P::new_unchecked(GX, GY);
    let mut b = P::new_unchecked([0u32; 8], [0u32; 8]);

    for _ in 0..ROUNDS {
        black_box(&a).double(&mut b);
        core::mem::swap(&mut a, &mut b);
    }

    let checksum = a.as_u32s().map_or(0u32, |w| w[0][0]);
    risc0_zkvm::guest::env::commit(&checksum);
}
