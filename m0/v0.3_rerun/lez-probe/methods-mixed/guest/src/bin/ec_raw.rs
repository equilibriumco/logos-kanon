//! The irreducible floor: two secp256k1 scalar multiplications and one addition,
//! called straight into `risc0-bigint2`'s circuits, with no `k256` around them.
//!
//! This is the narrowest possible statement of what a recovery *has* to cost.
//! `u1·G + u2·R` is work a native `ecrecover` still has to perform, however it is
//! packaged, so whatever this guest spends is a lower bound on a recovery under
//! any precompile.
//!
//! Between this and `ec_lincomb` lies `k256`'s own overhead around the same
//! circuits — conversions, `Scalar`/`FieldElement` wrapping, projective
//! bookkeeping — which a dedicated circuit could also avoid. Between
//! `ec_lincomb` and `ec_recover` lies the rest of ECDSA recovery. Three guests,
//! so the two kinds of removable work can be told apart instead of lumped.

use core::hint::black_box;
use risc0_bigint2::ec::{AffinePoint, Secp256k1Curve, EC_256_WIDTH_WORDS};

/// Must match `ec_lincomb.rs` and `ec_recover.rs`.
const ROUNDS: u32 = 4;

/// secp256k1's generator, as little-endian u32 digits.
const GX: [u32; EC_256_WIDTH_WORDS] = [
    0x16F81798, 0x59F2815B, 0x2DCE28D9, 0x029BFCDB, 0xCE870B07, 0x55A06295, 0xF9DCBBAC, 0x79BE667E,
];
const GY: [u32; EC_256_WIDTH_WORDS] = [
    0xFB10D4B8, 0x9C47D08F, 0xA6855419, 0xFD17B448, 0x0E1108A8, 0x5DA4FBFC, 0x26A3C465, 0x483ADA77,
];

type P = AffinePoint<EC_256_WIDTH_WORDS, Secp256k1Curve>;

fn main() {
    let mut checksum = 0u32;

    let g = P::new_unchecked(GX, GY);

    // The second base point, as recovery obtains R from the signature's r.
    // Derived once, outside the measured loop: a recovery performs exactly two
    // scalar multiplications and one addition, and charging a third here would
    // overstate the curve cost by half again.
    let mut r = P::new_unchecked([0u32; 8], [0u32; 8]);
    g.mul(&scalar(3), &mut r);

    for i in 0..ROUNDS {
        let mut a = P::new_unchecked([0u32; 8], [0u32; 8]);
        let mut b = P::new_unchecked([0u32; 8], [0u32; 8]);
        let mut sum = P::new_unchecked([0u32; 8], [0u32; 8]);

        black_box(&g).mul(black_box(&scalar(i as u8 + 1)), &mut a);
        black_box(&r).mul(black_box(&scalar(i as u8 + 9)), &mut b);
        a.add(&b, &mut sum);

        if let Some(words) = sum.as_u32s() {
            checksum ^= words[0][0];
        }
    }

    risc0_zkvm::guest::env::commit(&checksum);
}

/// A full-width scalar, differing per round, as little-endian u32 digits.
///
/// Full width matters: `AffinePoint::mul` is a double-and-add loop over
/// `bits(scalar)`, so an undersized scalar does proportionally fewer accelerator
/// round trips and understates the cost. An earlier version of this guest set
/// only the low 160 bits and measured about half of what a real ECDSA scalar
/// costs. Kept below the curve order, which `mul` requires.
fn scalar(seed: u8) -> [u32; EC_256_WIDTH_WORDS] {
    let mut s = [0u32; EC_256_WIDTH_WORDS];
    for (i, word) in s.iter_mut().enumerate() {
        *word = 0x3333_3333u32
            .wrapping_mul(u32::from(seed).wrapping_add(1))
            .wrapping_add(i as u32);
    }
    // Top word set so the scalar is genuinely 256-bit, but well under the
    // secp256k1 order (which begins 0xFFFFFFFF...).
    s[EC_256_WIDTH_WORDS - 1] = 0x7FFF_0000 | u32::from(seed);
    s
}
