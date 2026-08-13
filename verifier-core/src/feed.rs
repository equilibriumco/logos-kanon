//! One feed's configuration, and the M-of-N rule over it.
//!
//! The configuration is the *caller's*, never the payload's. SEC2 requires that
//! in pull mode the signer set come from the consumer and never from the data,
//! and nothing here reads a signer, a threshold or a feed id out of a payload.

use crate::{backend::SignerAddress, decode::FEED_ID_BYTES, error::ConfigError};

/// The most signers one feed may configure.
///
/// A real limit rather than a guard: it sizes the two fixed buffers `verify_feed`
/// walks with, which is how the threshold runs without an allocator. RedStone
/// caps at 255 and live feeds run ten to twenty, so this leaves headroom.
/// Raising it costs stack and nothing else.
pub const MAX_SIGNERS: usize = 32;

/// A feed, its authorised signers, and how many of them must agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedConfig<'a> {
    feed_id: [u8; FEED_ID_BYTES],
    signers: &'a [SignerAddress],
    threshold: u8,
}

impl<'a> FeedConfig<'a> {
    /// Checks a configuration and pads its feed id to the wire width.
    ///
    /// Every invariant here is one a payload cannot repair, so they are settled
    /// once at configuration rather than re-tested per package.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] for an empty or oversized signer list, a threshold of zero
    /// or one no signer set can reach, a repeated or zero signer address, or a
    /// feed id that is empty, all-zero, or wider than the wire field.
    pub fn try_new(
        feed_id: &[u8],
        signers: &'a [SignerAddress],
        threshold: u8,
    ) -> Result<Self, ConfigError> {
        if signers.is_empty() {
            return Err(ConfigError::NoSigners);
        }
        if threshold == 0 {
            return Err(ConfigError::ThresholdZero);
        }
        if signers.len() > MAX_SIGNERS {
            return Err(ConfigError::TooManySigners {
                signers: signers.len(),
                max: MAX_SIGNERS,
            });
        }

        // At most MAX_SIGNERS by the check above, so this cannot truncate.
        let count = u8::try_from(signers.len()).map_err(|_| ConfigError::TooManySigners {
            signers: signers.len(),
            max: MAX_SIGNERS,
        })?;
        if count < threshold {
            return Err(ConfigError::ThresholdExceedsSigners {
                threshold,
                signers: count,
            });
        }

        if signers
            .iter()
            .any(|s| *s.as_bytes() == [0u8; SignerAddress::LEN])
        {
            return Err(ConfigError::ZeroSignerAddress);
        }
        for (index, signer) in signers.iter().enumerate() {
            if signers
                .get(index + 1..)
                .is_some_and(|rest| rest.contains(signer))
            {
                return Err(ConfigError::DuplicateSigner);
            }
        }

        if feed_id.len() > FEED_ID_BYTES {
            return Err(ConfigError::FeedIdTooLong { len: feed_id.len() });
        }
        if feed_id.iter().all(|&b| b == 0) {
            return Err(ConfigError::ZeroFeedId);
        }
        let mut padded = [0u8; FEED_ID_BYTES];
        padded
            .get_mut(..feed_id.len())
            .ok_or(ConfigError::FeedIdTooLong { len: feed_id.len() })?
            .copy_from_slice(feed_id);

        Ok(Self {
            feed_id: padded,
            signers,
            threshold,
        })
    }

    /// The feed id, right-padded to the wire width.
    #[must_use]
    pub const fn feed_id(&self) -> &[u8; FEED_ID_BYTES] {
        &self.feed_id
    }

    /// The authorised signers, in the order the caller supplied.
    #[must_use]
    pub const fn signers(&self) -> &'a [SignerAddress] {
        self.signers
    }

    /// How many distinct authorised signers a price needs.
    #[must_use]
    pub const fn threshold(&self) -> u8 {
        self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer(byte: u8) -> SignerAddress {
        SignerAddress([byte; SignerAddress::LEN])
    }

    #[test]
    fn a_feed_id_is_right_padded_the_way_the_wire_pads_it() {
        // `FeedConfig<'a>` borrows `signers`, so the array needs a name: an
        // inline `&[signer(1)]` is a temporary that doesn't outlive `config`,
        // which is used again below.
        let signers = [signer(1)];
        let config = FeedConfig::try_new(b"BTC", &signers, 1).expect("valid");
        let mut expected = [0u8; FEED_ID_BYTES];
        expected[..3].copy_from_slice(b"BTC");
        assert_eq!(config.feed_id(), &expected);
    }

    #[test]
    fn an_empty_signer_list_makes_every_threshold_unreachable() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", &[], 1).unwrap_err(),
            ConfigError::NoSigners
        );
    }

    #[test]
    fn a_zero_threshold_is_rejected() {
        // A threshold of zero is satisfied by a payload nobody signed.
        assert_eq!(
            FeedConfig::try_new(b"BTC", &[signer(1)], 0).unwrap_err(),
            ConfigError::ThresholdZero
        );
    }

    #[test]
    fn a_threshold_no_signer_set_can_reach_is_rejected_at_configuration_time() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", &[signer(1), signer(2)], 3).unwrap_err(),
            ConfigError::ThresholdExceedsSigners {
                threshold: 3,
                signers: 2
            }
        );
    }

    #[test]
    fn more_signers_than_the_buffers_hold_is_rejected() {
        let many: [SignerAddress; MAX_SIGNERS + 1] =
            core::array::from_fn(|i| signer(u8::try_from(i).unwrap_or(u8::MAX)));
        assert_eq!(
            FeedConfig::try_new(b"BTC", &many, 3).unwrap_err(),
            ConfigError::TooManySigners {
                signers: MAX_SIGNERS + 1,
                max: MAX_SIGNERS
            }
        );
    }

    #[test]
    fn a_repeated_signer_is_rejected() {
        // Two slots for one address would let it reach any threshold alone.
        assert_eq!(
            FeedConfig::try_new(b"BTC", &[signer(1), signer(2), signer(1)], 2).unwrap_err(),
            ConfigError::DuplicateSigner
        );
    }

    #[test]
    fn the_zero_address_is_not_a_signer() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", &[signer(1), signer(0)], 1).unwrap_err(),
            ConfigError::ZeroSignerAddress
        );
    }

    #[test]
    fn a_feed_id_wider_than_the_wire_field_is_rejected() {
        let long = [b'X'; FEED_ID_BYTES + 1];
        assert_eq!(
            FeedConfig::try_new(&long, &[signer(1)], 1).unwrap_err(),
            ConfigError::FeedIdTooLong {
                len: FEED_ID_BYTES + 1
            }
        );
    }

    #[test]
    fn an_empty_or_zero_feed_id_is_rejected() {
        // It would match the padding of any feed id in any payload.
        assert_eq!(
            FeedConfig::try_new(b"", &[signer(1)], 1).unwrap_err(),
            ConfigError::ZeroFeedId
        );
        assert_eq!(
            FeedConfig::try_new(&[0u8; 4], &[signer(1)], 1).unwrap_err(),
            ConfigError::ZeroFeedId
        );
    }

    #[test]
    fn a_threshold_equal_to_the_signer_count_is_allowed() {
        assert!(FeedConfig::try_new(b"BTC", &[signer(1), signer(2)], 2).is_ok());
    }
}
