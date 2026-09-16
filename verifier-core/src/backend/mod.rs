//! The two cryptographic primitives verification needs, behind one trait.
//!
//! The signature path is structured so that swapping in a host precompile stays
//! a localised change. This module is that structure, and the rule it exists to
//! enforce is narrow enough to state in one line: **no code outside this module
//! may call a signature primitive directly.** Everything reaches keccak256 and
//! public-key recovery through
//! [`VerifierBackend`], so replacing the in-program implementation with a
//! precompile means writing one new implementor and changing one type
//! parameter, rather than auditing every call site.
//!
//! That matters because of what the cost baseline measured: recovery and
//! signature parsing are 88.97% of a 3-of-N update, which costs 1,906,737 cycles
//! in total. A host precompile is the obvious lever if LEZ ever offers one, and
//! the value of being ready for it is proportional to how localised the change
//! is.
//!
//! The trait deliberately says nothing about RedStone. It is two primitives over
//! byte slices, which is what keeps it implementable by a precompile that knows
//! nothing about data packages.

pub mod in_program;

pub use in_program::InProgramBackend;

/// A 20-byte Ethereum-style address, the identity RedStone signer sets are
/// expressed in.
///
/// Derived from a recovered public key, never supplied by a payload. A verified
/// package is one whose recovered addresses are in the signer set the *consumer*
/// configured, never one the payload supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SignerAddress(pub [u8; SignerAddress::LEN]);

impl SignerAddress {
    /// Length in bytes.
    pub const LEN: usize = 20;

    /// The address bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl From<[u8; SignerAddress::LEN]> for SignerAddress {
    fn from(bytes: [u8; SignerAddress::LEN]) -> Self {
        Self(bytes)
    }
}

/// A 65-byte recoverable ECDSA signature, `r || s || v`.
///
/// The layout RedStone puts on the wire. `v` is the recovery identifier in the
/// final byte; both the `0/1` and `27/28` conventions appear in the wild, and
/// normalising between them is the backend's job rather than the caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; Signature::LEN]);

impl Signature {
    /// Length in bytes: 32 for `r`, 32 for `s`, 1 for `v`.
    pub const LEN: usize = 65;

    /// The `r` component.
    #[must_use]
    pub fn r(&self) -> &[u8] {
        &self.0[..32]
    }

    /// The `s` component.
    #[must_use]
    pub fn s(&self) -> &[u8] {
        &self.0[32..64]
    }

    /// The recovery identifier, as it appears on the wire.
    #[must_use]
    pub const fn v(&self) -> u8 {
        self.0[64]
    }
}

impl From<[u8; Signature::LEN]> for Signature {
    fn from(bytes: [u8; Signature::LEN]) -> Self {
        Self(bytes)
    }
}

/// Why a recovery failed.
///
/// Deliberately coarse. A caller cannot do anything different for a malformed
/// `r` than for a malformed `s`, and a verifier that reports precisely which
/// component of a signature was wrong tells an attacker more than it tells an
/// operator. The typed error enum the *whole* verification path reports through
/// is a separate, later type; this is only what a backend can distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendError {
    /// The recovery identifier was not one this backend accepts.
    InvalidRecoveryId,
    /// `r` or `s` was not a valid scalar, or `s` was not in the lower half of
    /// the curve order.
    ///
    /// Rejecting high-`s` signatures is not pedantry: every ECDSA signature has
    /// a second, equally valid form with `s` replaced by `n - s`, so accepting
    /// both makes a signature malleable and gives the same package two distinct
    /// encodings. Replay defences that key on signature bytes are worthless
    /// against that.
    InvalidSignature,
    /// The signature was well-formed but no public key could be recovered.
    RecoveryFailed,
}

/// keccak256 and secp256k1 public-key recovery, and nothing else.
///
/// Implementors: [`InProgramBackend`] computes both in the guest. A future
/// precompile-backed implementor would satisfy the same contract at a fraction
/// of the cycle cost, and nothing above this trait would change.
///
/// # Contract
///
/// An implementation must be deterministic and must not panic on any input:
/// every failure is a [`BackendError`]. Verification runs inside a zkVM guest,
/// where a panic aborts the whole program rather than rejecting one package, so
/// a backend that panics on malformed input turns a rejectable package into a
/// failed transaction.
pub trait VerifierBackend {
    /// The keccak256 digest of `data`.
    ///
    /// Plain keccak256, not the Ethereum `personal_sign` construction: RedStone
    /// signs the digest of the package's signable bytes directly, with no
    /// message prefix.
    fn keccak256(&self, data: &[u8]) -> [u8; 32];

    /// Recovers the signer address from a signature over `message_hash`.
    ///
    /// `message_hash` is already a digest — the output of [`Self::keccak256`]
    /// over the signable bytes — because the caller needs the hash separately
    /// anyway, so hashing again here would be a second keccak256 pass for
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`BackendError`] when the signature is malformed, malleable, or yields no
    /// public key.
    fn recover_signer(
        &self,
        message_hash: &[u8; 32],
        signature: &Signature,
    ) -> Result<SignerAddress, BackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that records what it was asked, so the tests below can assert
    /// on the shape of the contract without needing real cryptography.
    struct StubBackend;

    impl VerifierBackend for StubBackend {
        fn keccak256(&self, data: &[u8]) -> [u8; 32] {
            let mut out = [0u8; 32];
            out[0] = data.len() as u8;
            out
        }

        fn recover_signer(
            &self,
            message_hash: &[u8; 32],
            signature: &Signature,
        ) -> Result<SignerAddress, BackendError> {
            match signature.v() {
                0 | 1 | 27 | 28 => {
                    let mut address = [0u8; SignerAddress::LEN];
                    address[0] = message_hash[0];
                    Ok(SignerAddress(address))
                }
                _ => Err(BackendError::InvalidRecoveryId),
            }
        }
    }

    #[test]
    fn the_trait_is_object_safe() {
        // Object safety is the property that lets a caller hold a backend it did
        // not choose at compile time, which is what a runtime precompile switch
        // would need. Cheap to keep, expensive to regain once a generic method
        // has been added.
        let backend: &dyn VerifierBackend = &StubBackend;
        assert_eq!(backend.keccak256(b"abc")[0], 3);
    }

    #[test]
    fn signature_components_split_at_the_documented_offsets() {
        let mut bytes = [0u8; Signature::LEN];
        bytes[0] = 0xAA;
        bytes[31] = 0xBB;
        bytes[32] = 0xCC;
        bytes[63] = 0xDD;
        bytes[64] = 27;
        let signature = Signature::from(bytes);

        assert_eq!(signature.r().len(), 32);
        assert_eq!(signature.s().len(), 32);
        assert_eq!(signature.r()[0], 0xAA);
        assert_eq!(signature.r()[31], 0xBB);
        assert_eq!(signature.s()[0], 0xCC);
        assert_eq!(signature.s()[31], 0xDD);
        assert_eq!(signature.v(), 27);
    }

    #[test]
    fn a_backend_reports_an_unusable_recovery_id_rather_than_panicking() {
        let backend = StubBackend;
        let signature = Signature::from([0xFF; Signature::LEN]);

        assert_eq!(
            backend.recover_signer(&[0u8; 32], &signature),
            Err(BackendError::InvalidRecoveryId)
        );
    }
}
