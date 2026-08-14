//! Every way verification can fail, as one enum.
//!
//! Every failure mode gets its own variant, because a caller that cannot tell
//! them apart cannot act on any of them. A value out of scale is separate from a
//! value that is zero or negative, since they point at different faults upstream.
//! `ReoccurringSigner` is separate again: the payload is well formed and its
//! signer is authorised, so neither `Malformed` nor `UnauthorisedSigner`
//! describes it.
//!
//! [`DecodeError`] and [`BackendError`] are wrapped rather than flattened.
//! `decode` exists to keep "not well formed" and "not authorised" apart, and
//! collapsing them here would undo that.

use crate::{backend::BackendError, decode::DecodeError, time::TimeError};

/// Why a payload did not yield a verified price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// The payload is not well formed. Carries the decoder's reason.
    Malformed(DecodeError),
    /// A signature was malformed, malleable, or yielded no key.
    /// Nothing constructs this today: an unrecoverable signature is skipped
    /// rather than rejected (ADR 15), so one malformed package — needing no
    /// key and no valid signature — cannot deny the feed to everyone else it
    /// serves. A future single-package API, where the caller names one
    /// package and expects it to verify, is where this becomes reportable.
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
    /// Enough authorised signers reported for this feed that the threshold
    /// would have been met, but their packages were older than the feed's
    /// `maxAge`.
    ///
    /// The one failure in this enum that resolves on its own: a fresher payload
    /// fixes it, where every other cause needs someone to change something.
    StalePackage,
    /// As [`Self::StalePackage`], but the packages were dated further ahead
    /// than clock skew allows.
    ///
    /// Separate because the two send an operator to different machines. Stale
    /// means the relayer is behind or a payload was replayed; future-dated
    /// means a signer's clock is wrong, or this node's is.
    FuturePackage,
    /// The feed is registered against a different asset pair than the caller
    /// expects.
    ///
    /// A comparison between the caller's expectation and the registration, not
    /// a property of the payload: a RedStone package names a feed and nothing
    /// else, so no signer attests to which assets that feed prices.
    AssetMismatch,
    /// Enough authorised signers reported for this feed that the threshold
    /// would have been met, but every value they supplied was unusable — zero,
    /// negative, or wider than a price can represent.
    ///
    /// Distinct from [`Self::ThresholdNotMet`] for the same reason
    /// [`Self::UnauthorisedSigner`] is: signers that did report and signers
    /// that never signed call for different responses. A single signer cannot
    /// force this, since it can only spoil its own slot.
    ValueOutOfRange,
    /// The agreed price cannot be represented on the scale the price account
    /// uses.
    ScalingOutOfRange,
    /// The caller's feed configuration is itself invalid.
    InvalidConfig(ConfigError),
    /// No trustworthy clock was available, so staleness could not be decided.
    ///
    /// Reported rather than skipped past. Verifying without a clock would mean
    /// accepting a package of any age, which is the replay this check exists to
    /// stop.
    NoClock(TimeError),
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

impl From<TimeError> for VerifyError {
    fn from(err: TimeError) -> Self {
        Self::NoClock(err)
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
    /// A decimal exponent the price conversion cannot divide by.
    DecimalsOutOfRange { decimals: u8, max: u8 },
    /// A `maxAge` of zero, which no package can ever be young enough to satisfy.
    MaxAgeZero,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::BackendError, decode::DecodeError, time::TimeError};

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
    fn a_missing_clock_is_not_a_stale_package() {
        // The distinction that matters operationally: one says the data is old,
        // the other says we could not find out. Answering the second with the
        // first would have an operator chasing a relayer over a wiring mistake.
        let no_clock: VerifyError = TimeError::WrongAccount.into();
        assert_eq!(no_clock, VerifyError::NoClock(TimeError::WrongAccount));
        assert_ne!(no_clock, VerifyError::StalePackage);
        assert_ne!(
            VerifyError::NoClock(TimeError::Missing),
            VerifyError::NoClock(TimeError::WrongAccount)
        );
    }

    #[test]
    fn a_stale_package_and_a_future_one_stay_distinguishable() {
        // They send an operator to different machines: the relayer for one, a
        // signer's or this node's clock for the other.
        assert_ne!(VerifyError::StalePackage, VerifyError::FuturePackage);
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
        // "not enough signers" without the counts gives an operator nothing to
        // act on: one short and three short call for different responses.
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
