//! The backend that does the work in the guest.
//!
//! `k256` for recovery, `tiny-keccak` for hashing — the same two crates the cost
//! baseline in `m0/` measured, so the cycle figures in `m0/M0-report.pdf`
//! describe this code rather than something adjacent to it. In a guest workspace
//! that patches in RISC Zero's accelerated forks, the same source compiles
//! against those instead; that substitution is a `[patch.crates-io]` in the
//! guest manifest and is invisible here, which is exactly what made the
//! three-way accelerator comparison in `m0/` possible.
//!
//! This is the implementation a host precompile would replace. Everything about
//! it is local to this file.

use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
use tiny_keccak::{Hasher, Keccak};

use super::{BackendError, Signature, SignerAddress, VerifierBackend};

/// secp256k1 recovery and keccak256, computed inside the program.
///
/// Zero-sized: it carries no state and exists to name an implementation, so
/// holding one costs nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InProgramBackend;

impl InProgramBackend {
    /// A new backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Normalises the wire recovery byte to the 0/1 form `k256` expects.
///
/// RedStone signers are Ethereum tooling, and Ethereum has used both
/// conventions: `0/1` as the raw recovery id, and `27/28` from the original
/// Bitcoin-derived encoding. Accepting both is interoperability rather than
/// laxity — the value carries no authority, since a wrong recovery id yields a
/// different address, which then fails the signer-set check.
const fn normalise_recovery_id(v: u8) -> Option<u8> {
    match v {
        0 | 1 => Some(v),
        27 | 28 => Some(v - 27),
        _ => None,
    }
}

impl VerifierBackend for InProgramBackend {
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
        let recovery_id =
            RecoveryId::from_byte(recovery_id).ok_or(BackendError::InvalidRecoveryId)?;

        // `from_slice` validates that `r` and `s` are non-zero scalars in
        // range. It does *not* enforce a canonical low `s`, so malleability has
        // to be refused explicitly.
        let signature = K256Signature::from_slice(&signature.0[..64])
            .map_err(|_| BackendError::InvalidSignature)?;

        // `normalize_s` returns `Some` exactly when `s` was in the upper half of
        // the curve order, which is the malleable form: every signature has a
        // second, equally valid encoding with `s` replaced by `n - s`. Recovery
        // would reject it anyway, but as `RecoveryFailed`, which says "no key
        // came back" when the truth is "this signature is malleable". Rejecting
        // it here keeps the error honest and the reason greppable.
        if signature.normalize_s().is_some() {
            return Err(BackendError::InvalidSignature);
        }

        let key = VerifyingKey::recover_from_prehash(message_hash, &signature, recovery_id)
            .map_err(|_| BackendError::RecoveryFailed)?;

        Ok(address_of(&key))
    }
}

/// The Ethereum address of a public key: the last 20 bytes of the keccak256 of
/// its uncompressed encoding, minus the `0x04` prefix byte.
fn address_of(key: &VerifyingKey) -> SignerAddress {
    let point = key.to_encoded_point(false);
    // 65 bytes: a `0x04` tag then x and y. The tag is not part of what is
    // hashed, which is the detail that silently produces wrong-but-plausible
    // addresses when it is missed.
    let uncompressed = &point.as_bytes()[1..];

    let mut hasher = Keccak::v256();
    hasher.update(uncompressed);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    let mut address = [0u8; SignerAddress::LEN];
    address.copy_from_slice(&digest[12..]);
    SignerAddress(address)
}

#[cfg(test)]
mod tests {
    use k256::ecdsa::SigningKey;

    use super::*;

    /// A fixed key, so every assertion below is reproducible rather than
    /// dependent on a generator.
    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0x42u8; 32].into()).expect("0x42… is a valid secret scalar")
    }

    #[test]
    fn keccak256_matches_the_known_digest_of_the_empty_input() {
        // The most widely published keccak256 vector there is. If this is wrong,
        // the hash function is not the one Ethereum and RedStone use, and every
        // recovered address downstream would be wrong in a way that is otherwise
        // very hard to spot.
        let expected: [u8; 32] = [
            0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7,
            0x03, 0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04,
            0x5d, 0x85, 0xa4, 0x70,
        ];
        assert_eq!(InProgramBackend::new().keccak256(b""), expected);
    }

    #[test]
    fn a_signature_round_trips_to_the_signing_key_address() {
        let backend = InProgramBackend::new();
        let key = signing_key();
        let digest = backend.keccak256(b"a redstone data package");

        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing a 32-byte prehash");

        let mut bytes = [0u8; Signature::LEN];
        bytes[..64].copy_from_slice(&signature.to_bytes());
        bytes[64] = recovery_id.to_byte();

        let recovered = backend
            .recover_signer(&digest, &Signature::from(bytes))
            .expect("a freshly produced signature recovers");

        assert_eq!(recovered, address_of(key.verifying_key()));
    }

    #[test]
    fn the_two_recovery_id_conventions_agree() {
        let backend = InProgramBackend::new();
        let key = signing_key();
        let digest = backend.keccak256(b"either convention");

        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing a 32-byte prehash");

        let mut raw = [0u8; Signature::LEN];
        raw[..64].copy_from_slice(&signature.to_bytes());
        raw[64] = recovery_id.to_byte();

        let mut eth = raw;
        eth[64] = recovery_id.to_byte() + 27;

        assert_eq!(
            backend.recover_signer(&digest, &Signature::from(raw)),
            backend.recover_signer(&digest, &Signature::from(eth)),
            "0/1 and 27/28 are the same recovery id spelled two ways"
        );
    }

    #[test]
    fn a_high_s_signature_is_rejected_as_malleable_not_as_a_recovery_failure() {
        // Every ECDSA signature has a second, equally valid form with `s`
        // replaced by `n - s`. Accepting both would give one package two
        // distinct encodings, which defeats any replay defence keyed on
        // signature bytes.
        //
        // The error code is the point of this test as much as the rejection.
        // `k256`'s `from_slice` accepts a high `s`, and recovery then fails with
        // `RecoveryFailed` -- fail-closed, but it reports "no key came back"
        // for what is actually a malleable signature. This asserts the typed
        // error the trait's contract promises.
        let backend = InProgramBackend::new();
        let key = signing_key();
        let digest = backend.keccak256(b"malleable");

        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing a 32-byte prehash");
        assert!(
            signature.normalize_s().is_none(),
            "k256 signs with a low s, so the flip below is what introduces the high one"
        );

        let high_s =
            K256Signature::from_scalars(signature.r().to_bytes(), (-*signature.s()).to_bytes())
                .expect("n - s is a valid non-zero scalar");

        let mut bytes = [0u8; Signature::LEN];
        bytes[..64].copy_from_slice(&high_s.to_bytes());
        bytes[64] = recovery_id.to_byte() ^ 1;

        assert_eq!(
            backend.recover_signer(&digest, &Signature::from(bytes)),
            Err(BackendError::InvalidSignature),
            "a high-s signature is malleable, not an unrecoverable one"
        );
    }

    #[test]
    fn a_wrong_recovery_id_yields_a_different_address_rather_than_an_error() {
        // Pinned down because it is the reason the recovery byte needs no
        // authentication: flipping it forges nothing, it recovers a different
        // address, which then fails the signer-set check.
        let backend = InProgramBackend::new();
        let key = signing_key();
        let digest = backend.keccak256(b"flip the recovery bit");

        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing a 32-byte prehash");

        let mut bytes = [0u8; Signature::LEN];
        bytes[..64].copy_from_slice(&signature.to_bytes());
        bytes[64] = recovery_id.to_byte();
        let right = backend.recover_signer(&digest, &Signature::from(bytes));

        bytes[64] = recovery_id.to_byte() ^ 1;
        let wrong = backend.recover_signer(&digest, &Signature::from(bytes));

        assert!(right.is_ok());
        if let Ok(wrong) = wrong {
            assert_ne!(right.unwrap(), wrong);
        }
    }

    #[test]
    fn a_tampered_message_does_not_recover_the_signer() {
        let backend = InProgramBackend::new();
        let key = signing_key();
        let digest = backend.keccak256(b"the original message");

        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("signing a 32-byte prehash");

        let mut bytes = [0u8; Signature::LEN];
        bytes[..64].copy_from_slice(&signature.to_bytes());
        bytes[64] = recovery_id.to_byte();

        let tampered = backend.keccak256(b"the tampered message");
        let recovered = backend.recover_signer(&tampered, &Signature::from(bytes));

        // Recovery still succeeds -- it almost always does -- but on an
        // unrelated address. This is why a package is verified by *which*
        // address came back, never by whether recovery worked.
        assert_ne!(
            recovered.ok(),
            Some(address_of(key.verifying_key())),
            "a different message must not recover the original signer"
        );
    }

    #[test]
    fn an_unusable_recovery_byte_is_rejected() {
        let backend = InProgramBackend::new();
        let signature = Signature::from([0x07; Signature::LEN]);

        assert_eq!(
            backend.recover_signer(&[0u8; 32], &signature),
            Err(BackendError::InvalidRecoveryId)
        );
    }

    #[test]
    fn a_zero_signature_is_rejected_rather_than_panicking() {
        // The contract says a backend never panics, and all-zero scalars are the
        // simplest input that would tempt one.
        let backend = InProgramBackend::new();
        let mut bytes = [0u8; Signature::LEN];
        bytes[64] = 0;

        assert_eq!(
            backend.recover_signer(&[0u8; 32], &Signature::from(bytes)),
            Err(BackendError::InvalidSignature)
        );
    }
}
