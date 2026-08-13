//! Every way verification can fail, as one enum.
//!
//! U6 asks for clear, actionable errors for every failure mode and names seven.
//! Two more are here. `ScalingOutOfRange` comes from the proposal and the M1
//! milestone, which both separate a value out of scale from a value that is zero
//! or negative. `ReoccurringSigner` is named by neither and is required by the
//! counting rule: the payload is well formed and its signer is authorised, so
//! neither `Malformed` nor `UnauthorisedSigner` describes it.
//!
//! [`DecodeError`] and [`BackendError`] are wrapped rather than flattened.
//! `decode` exists to keep "not well formed" and "not authorised" apart, and
//! collapsing them here would undo that.

use crate::{backend::BackendError, decode::DecodeError};

/// Why a payload did not yield a verified price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// The payload is not well formed. Carries the decoder's reason.
    Malformed(DecodeError),
    /// A signature was malformed, malleable, or yielded no key.
    InvalidSignature(BackendError),
    /// No configured signer signed, and the packages that were skipped as
    /// unauthorised would have been enough to reach the threshold.
    ///
    /// The distinction from [`Self::ThresholdNotMet`] is the actionable part: this
    /// says the configured signer set is wrong for this payload, rather than that
    /// too few signers signed at all.
    UnauthorisedSigner,
    /// Fewer distinct authorised signers than the feed requires, and authorising
    /// the skipped ones would not have changed that.
    ThresholdNotMet { met: u8, required: u8 },
    /// One signer supplied the requested feed twice.
    ///
    /// Rejected rather than counted once, because a signer that can occupy two
    /// slots reaches any threshold alone.
    ReoccurringSigner,
    /// The package is older than the feed's `maxAge`. Returned by M1-15.
    StalePackage,
    /// The package does not carry the asset the caller asked for. M1-16.
    AssetMismatch,
    /// The value is zero, negative, or wider than a price can represent.
    /// Nothing constructs this today: an unrepresentable value is skipped
    /// rather than rejected (ADR 15), so one configured signer cannot deny a
    /// feed by sending one. M1-17 adds value-sanity bounds and gives this a
    /// producer.
    ValueOutOfRange,
    /// The value's scale or exponent is outside the feed's configured bounds.
    /// M1-17.
    ScalingOutOfRange,
    /// The caller's feed configuration is itself invalid.
    InvalidConfig(ConfigError),
}

impl From<DecodeError> for VerifyError {
    fn from(err: DecodeError) -> Self {
        Self::Malformed(err)
    }
}

impl From<BackendError> for VerifyError {
    fn from(err: BackendError) -> Self {
        Self::InvalidSignature(err)
    }
}

impl From<ConfigError> for VerifyError {
    fn from(err: ConfigError) -> Self {
        Self::InvalidConfig(err)
    }
}

/// Why a feed configuration cannot be used.
///
/// These are registration mistakes rather than payload faults, which is why they
/// are a separate type: a caller can fix one and cannot fix the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// The signer list is empty, so every threshold is unreachable.
    NoSigners,
    /// A threshold of zero would accept an empty signer set vacuously.
    ThresholdZero,
    /// More signers are required than are configured.
    ThresholdExceedsSigners { threshold: u8, signers: u8 },
    /// More signers than the fixed buffers hold.
    TooManySigners { signers: usize, max: usize },
    /// The same address appears twice, which would let one signer fill two slots.
    DuplicateSigner,
    /// The all-zero address is not a signer; recovery never produces it, so it
    /// can only be a placeholder left in by mistake.
    ZeroSignerAddress,
    /// A feed id wider than the wire format's field.
    FeedIdTooLong { len: usize },
    /// An empty or all-zero feed id would match a padded field in any payload.
    ZeroFeedId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::BackendError, decode::DecodeError};

    #[test]
    fn a_decode_failure_and_a_signature_failure_stay_distinguishable() {
        // The whole reason the two existing enums are wrapped rather than
        // flattened: "this payload is not well formed" and "this payload is not
        // authorised" are different answers with different causes.
        let malformed: VerifyError = DecodeError::Truncated.into();
        let unusable: VerifyError = BackendError::InvalidSignature.into();

        assert_eq!(malformed, VerifyError::Malformed(DecodeError::Truncated));
        assert_eq!(
            unusable,
            VerifyError::InvalidSignature(BackendError::InvalidSignature)
        );
        assert_ne!(malformed, unusable);
    }

    #[test]
    fn a_config_mistake_is_not_a_payload_fault() {
        // A caller can fix a bad config and cannot fix a bad payload, so the two
        // are separate types with one wrapping the other.
        let bad: VerifyError = ConfigError::ThresholdZero.into();
        assert_eq!(bad, VerifyError::InvalidConfig(ConfigError::ThresholdZero));
    }

    #[test]
    fn the_threshold_failure_carries_what_was_reached_and_what_was_needed() {
        // U6 asks for actionable errors. "not enough signers" without the counts
        // is not actionable.
        let err = VerifyError::ThresholdNotMet {
            met: 2,
            required: 3,
        };
        assert_eq!(
            err,
            VerifyError::ThresholdNotMet {
                met: 2,
                required: 3
            }
        );
        assert_ne!(
            err,
            VerifyError::ThresholdNotMet {
                met: 1,
                required: 3
            }
        );
    }
}
