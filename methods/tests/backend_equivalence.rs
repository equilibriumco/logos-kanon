//! The three backends agree, checked inside the zkVM.
//!
//! `R0CryptoBackend` and `FastRecoveryBackend` link only for the guest target,
//! so this is the only place the comparison can be made. The guest runs all
//! three over every case here and asserts the whole `Result` matches — the
//! same address on success, the same `BackendError` on refusal.
//!
//! The malformed and adversarial cases matter more than the real ones.
//! `secp256k1_ecrecover` normalises high-`s` signatures and would accept what
//! `k256` rejects, so `R0CryptoBackend` re-implements those checks by hand.
//! `FastRecoveryBackend` owns the entire recovery — GLV split, wNAF digits,
//! fixed-base tables — which is exactly the code where a sign error produces a
//! plausible wrong address rather than a loud failure. These cases are what
//! say they all got it right.
//!
//! Cases are `(message_hash, signature)` rather than `(message, signature)`:
//! the hash is an input of recovery, and several cases below — a zero digest,
//! a digest at the group order — cannot be produced by hashing anything. The
//! guest still compares the backends' keccak256 over every case's bytes, so
//! hashing stays held to agreement too.

use risc0_zkvm::{default_executor, ExecutorEnv};

/// secp256k1's group order `n`, big-endian.
const N: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

/// `n / 2`, big-endian: the canonical-`s` boundary, inclusive.
const HALF_N: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
];

/// secp256k1's generator x-coordinate, big-endian. Below `n`, so it is also a
/// valid `r` — the one whose nonce point is `G` itself.
const G_X: [u8; 32] = [
    0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87, 0x0B, 0x07,
    0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B, 0x16, 0xF8, 0x17, 0x98,
];

/// `n - value` over big-endian 32-byte integers.
fn order_minus(value: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let d = i16::from(N[i]) - i16::from(value[i]) - borrow;
        let (d, b) = if d < 0 { (d + 256, 1) } else { (d, 0) };
        out[i] = d as u8;
        borrow = b;
    }
    out
}

/// A 65-byte signature from its parts.
fn sig(r: &[u8; 32], s: &[u8; 32], v: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(65);
    out.extend_from_slice(r);
    out.extend_from_slice(s);
    out.push(v);
    out
}

/// A 32-byte big-endian integer holding `value` in its last byte.
fn small(value: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[31] = value;
    out
}

/// Real signatures plus hand-built malformed and adversarial ones, as
/// `(message_hash, signature)` pairs.
fn cases() -> Vec<(Vec<u8>, Vec<u8>)> {
    use k256::ecdsa::SigningKey;
    use tiny_keccak::{Hasher, Keccak};

    let mut out = Vec::new();

    // Real signatures, signed here rather than taken from a payload: the
    // backends are given `(hash, signature)` and know nothing about RedStone
    // framing, so the framing would add nothing to what this checks. Twenty-four
    // keys and messages rather than a handful, because the fast backend's GLV
    // split has four sign combinations and its wNAF four digit widths — a wide
    // spread of scalars is what exercises them all.
    for seed in 1..=24u8 {
        let mut scalar = [0u8; 32];
        scalar[31] = seed;
        scalar[0] = seed.wrapping_mul(11); // spread the keys across the field
        let key = SigningKey::from_bytes(&scalar.into()).expect("a valid scalar");
        let message = format!("equivalence case {seed}: both parities, all sign splits");

        let mut hasher = Keccak::v256();
        hasher.update(message.as_bytes());
        let mut digest = [0u8; 32];
        hasher.finalize(&mut digest);

        let (signature, recid) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing succeeds");
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recid.to_byte());
        out.push((digest.to_vec(), raw));
    }

    let hash = {
        let mut hasher = Keccak::v256();
        hasher.update(b"a message every backend must treat identically");
        let mut digest = [0u8; 32];
        hasher.finalize(&mut digest);
        digest
    };
    let real = out[0].1.clone();
    let real_r: [u8; 32] = real[..32].try_into().expect("32 bytes");
    let real_s: [u8; 32] = real[32..64].try_into().expect("32 bytes");

    // High-`s`: the malleable form. `k256` refuses it; `secp256k1_ecrecover`
    // would normalise and accept. This is the case the hand-written checks
    // exist for, and the one that would silently change verification semantics.
    out.push((
        hash.to_vec(),
        sig(&real_r, &order_minus(&real_s), real[64] ^ 1),
    ));

    // Zero scalars, scalars at and just past the order, and `r` just below it
    // — the decompression edge, where whether a point exists at `x = n - 1`
    // is the curve's business and the backends must simply agree about it.
    out.push((hash.to_vec(), sig(&[0u8; 32], &real_s, real[64])));
    out.push((hash.to_vec(), sig(&real_r, &[0u8; 32], real[64])));
    out.push((hash.to_vec(), sig(&N, &real_s, real[64])));
    out.push((hash.to_vec(), sig(&real_r, &N, real[64])));
    for v in [0u8, 1] {
        out.push((hash.to_vec(), sig(&order_minus(&small(1)), &small(1), v)));
        out.push((hash.to_vec(), sig(&order_minus(&small(2)), &small(1), v)));
    }

    // `s` exactly at the half-order boundary (canonical, accepted) and one
    // past it (malleable, refused). The boundary itself is the off-by-one a
    // hand-written comparison gets wrong.
    out.push((hash.to_vec(), sig(&real_r, &HALF_N, real[64])));
    let mut half_plus_one = HALF_N;
    half_plus_one[31] += 1;
    out.push((hash.to_vec(), sig(&real_r, &half_plus_one, real[64])));

    // Recovery bytes nobody uses.
    for v in [2u8, 3, 4, 26, 29, 255] {
        out.push((hash.to_vec(), sig(&real_r, &real_s, v)));
    }

    // Both accepted recovery-id conventions, which must behave the same.
    for v in [0u8, 1, 27, 28] {
        out.push((hash.to_vec(), sig(&real_r, &real_s, v)));
    }

    // z = 0: a digest that is zero, which no message hashes to but the wire
    // can claim. Recovery degenerates to `Q = r⁻¹s·R` — the whole generator
    // half of the fast backend's loop goes silent, and the answers must not.
    out.push((vec![0u8; 32], sig(&real_r, &real_s, real[64])));

    // z ≡ 0 the other way: a digest at the group order, which reduces to zero.
    // Exercises the reduction path rather than the zero fast path.
    out.push((N.to_vec(), sig(&real_r, &real_s, real[64])));

    // A digest past the order, reduced rather than refused.
    out.push((vec![0xFF; 32], sig(&real_r, &real_s, real[64])));

    // A signature that recovers to the point at infinity, built rather than
    // found: with `r = G.x` the nonce point is `±G`, and choosing `z = s·1`
    // makes `s·R - z·G` vanish exactly. `G.y` is even, so `v = 0` selects `R
    // = G` and hits the identity; `v = 1` selects `-G` and must recover a real
    // (if meaningless) key. Every backend must refuse the first and agree on
    // the second — an accumulator that is the identity at the end is the one
    // result a wNAF loop cannot hash into an address.
    out.push((small(1).to_vec(), sig(&G_X, &small(1), 0)));
    out.push((small(1).to_vec(), sig(&G_X, &small(1), 1)));

    out
}

#[test]
fn the_three_backends_agree_on_every_case() {
    let cases = cases();
    let count = u32::try_from(cases.len()).expect("fits");

    let env = ExecutorEnv::builder()
        .write(&cases)
        .expect("the cases serialise")
        .build()
        .expect("an executor environment");

    let session = default_executor()
        .execute(env, kanon_methods::BACKEND_EQUIVALENCE_ELF)
        .expect("the guest runs to completion: a disagreement is an assertion inside it");

    let agreed: u32 = session.journal.decode().expect("the guest commits a count");
    assert_eq!(
        agreed, count,
        "the guest agreed on {agreed} of {count} cases"
    );
    println!("all backends agreed on all {count} cases");
}
