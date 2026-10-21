//! The backend that recovers through `risc0-crypto-evm`.
//!
//! This is the implementor [`super`] was shaped for: "replacing the in-program
//! implementation with a precompile means writing one new implementor and
//! changing one type parameter". The precompile arrived, from Boundless rather
//! than from LEZ, and this is that implementor.
//!
//! `secp256k1_ecrecover` is backed by the same `risc0-bigint2` accelerator the
//! patched `k256` already calls into, but it does the whole recovery in one
//! call instead of driving the accelerator from a loop in the guest. Measured on
//! the product guest: a 3-of-N update falls from 1,766,937 gas to 481,474, which
//! is 3.67x.
//!
//! # zkVM only
//!
//! `risc0-crypto-evm` links only for `riscv32im-risc0-zkvm-elf`, so this module
//! is gated and the host keeps using [`super::InProgramBackend`]. That means the
//! two implementations must agree exactly, which is not an assumption:
//! `methods/guest/src/bin/backend_equivalence.rs` runs both over the committed
//! RedStone capture inside the zkVM and asserts identical addresses.
//!
//! # What this file has to do that the precompile does not
//!
//! `secp256k1_ecrecover` applies EIP-2 low-`s` normalisation, so it *accepts*
//! malleable signatures and returns the same address for both forms.
//! [`super::VerifierBackend`]'s contract refuses them, and that refusal is load
//! bearing: every ECDSA signature has a second valid encoding with `s` replaced
//! by `n - s`, so accepting both gives one package two encodings and defeats any
//! replay defence keyed on signature bytes.
//!
//! So the range and malleability checks `k256`'s `from_slice` and
//! `normalize_s` perform are done here explicitly, before the call. They are
//! byte comparisons against two constants and cost almost nothing, and without
//! them this backend would quietly accept what the other one rejects.

use risc0_crypto_evm::secp256k1_ecrecover;
use tiny_keccak::{Hasher, Keccak};

use super::{BackendError, Signature, SignerAddress, VerifierBackend};

/// The secp256k1 group order `n`, big-endian.
const ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

/// `n / 2`, big-endian. A signature is canonical when `s <= n/2`.
const HALF_ORDER: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
];

/// secp256k1 recovery through `risc0-crypto-evm`, keccak256 in the guest.
///
/// Zero-sized, like [`super::InProgramBackend`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct R0CryptoBackend;

impl R0CryptoBackend {
    /// A new backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Normalises the wire recovery byte to the 0/1 form.
///
/// Identical to [`super::in_program`]'s, and deliberately duplicated rather than
/// shared: the two backends must be able to diverge without one silently
/// dragging the other, and this is four lines.
const fn normalise_recovery_id(v: u8) -> Option<u8> {
    match v {
        0 | 1 => Some(v),
        27 | 28 => Some(v - 27),
        _ => None,
    }
}

/// `a < b` over big-endian 32-byte scalars.
fn lt(a: &[u8], b: &[u8; 32]) -> bool {
    for (x, y) in a.iter().zip(b.iter()) {
        if x != y {
            return x < y;
        }
    }
    false
}

/// Whether every byte is zero.
fn is_zero(a: &[u8]) -> bool {
    a.iter().all(|b| *b == 0)
}

impl VerifierBackend for R0CryptoBackend {
    fn keccak256(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = Keccak::v256();
        hasher.update(data);
        let mut out = [0u8; 32];
        hasher.finalize(&mut out);
        out
    }

    fn recover_signer(
        &self,
        message_hash: &[u8; 32],
        signature: &Signature,
    ) -> Result<SignerAddress, BackendError> {
        let recovery_id =
            normalise_recovery_id(signature.v()).ok_or(BackendError::InvalidRecoveryId)?;

        let r = signature.r();
        let s = signature.s();

        // What `K256Signature::from_slice` enforces: both scalars non-zero and
        // below the group order.
        if is_zero(r) || is_zero(s) || !lt(r, &ORDER) || !lt(s, &ORDER) {
            return Err(BackendError::InvalidSignature);
        }

        // What `normalize_s().is_some()` catches: the malleable upper-half form.
        // `secp256k1_ecrecover` would normalise and accept it, so refusing it is
        // this file's responsibility rather than the precompile's.
        if !lt(s, &HALF_ORDER) && s[..] != HALF_ORDER[..] {
            return Err(BackendError::InvalidSignature);
        }

        let mut sig = [0u8; 64];
        sig.copy_from_slice(&signature.0[..64]);

        let address = secp256k1_ecrecover(&sig, recovery_id, message_hash)
            .ok_or(BackendError::RecoveryFailed)?;

        let mut out = [0u8; SignerAddress::LEN];
        out.copy_from_slice(address.as_slice());
        Ok(SignerAddress(out))
    }
}
