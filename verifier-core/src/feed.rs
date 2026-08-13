//! One feed's configuration, and the M-of-N rule over it.
//!
//! The configuration is the *caller's*, never the payload's. SEC2 requires that
//! in pull mode the signer set come from the consumer and never from the data,
//! and nothing here reads a signer, a threshold or a feed id out of a payload.

use crate::{
    backend::{SignerAddress, VerifierBackend},
    decode::{Payload, FEED_ID_BYTES},
    error::{ConfigError, VerifyError},
    value::{median, Value},
};

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

/// A price that met its feed's threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedFeed {
    /// The median across the signers that reported.
    pub value: Value,
    /// How many distinct authorised signers reported. Always at least the
    /// threshold.
    pub signers: u8,
}

/// Verifies one feed out of a payload.
///
/// Walks the payload once, recovering one signer per package, and counts the
/// distinct authorised signers that reported a non-zero value for
/// `config.feed_id()`. Returns the median across them once the threshold is met.
///
/// Unknown signers and unrequested feeds are **skipped, not fatal**. That is
/// RedStone's own rule, and it is what lets one published payload serve consumers
/// whose signer sets and feed interests differ. Failing the payload on any
/// unknown signer would hand a denial-of-service primitive to anyone able to
/// append a package.
///
/// # Errors
///
/// [`VerifyError`] for a malformed payload, an unusable signature, one signer
/// supplying the feed twice, a value too wide to represent, or a threshold that
/// was not reached.
pub fn verify_feed<B: VerifierBackend>(
    payload: &Payload<'_>,
    config: &FeedConfig<'_>,
    backend: &B,
) -> Result<VerifiedFeed, VerifyError> {
    // One slot per configured signer: the slot is both the collection point and
    // the duplicate check, exactly as RedStone's (feed, signer) matrix cell is.
    let mut reported: [Option<Value>; MAX_SIGNERS] = [None; MAX_SIGNERS];
    // Distinct unknown signers. Counting packages instead would let three
    // packages from one unknown address read as three missing signers.
    let mut unknown: [SignerAddress; MAX_SIGNERS] =
        [SignerAddress([0; SignerAddress::LEN]); MAX_SIGNERS];
    let mut unknown_count = 0usize;

    let walked = payload.for_each_package(|package| {
        let digest = backend.keccak256(package.signable());
        let signer = backend
            .recover_signer(&digest, &package.signature)
            .map_err(VerifyError::InvalidSignature)?;

        let Some(index) = config.signers().iter().position(|s| *s == signer) else {
            if unknown_count < MAX_SIGNERS
                && !unknown
                    .get(..unknown_count)
                    .is_some_and(|seen| seen.contains(&signer))
            {
                if let Some(slot) = unknown.get_mut(unknown_count) {
                    *slot = signer;
                    unknown_count += 1;
                }
            }
            return Ok(());
        };

        for point in package.data_points() {
            if point.feed_id != config.feed_id() {
                continue;
            }
            // Unrepresentable values are skipped like zero values, not fatal.
            // Erroring here would let one configured signer deny the feed,
            // which is the thing M-of-N exists to prevent. RedStone sanitises
            // instead of rejecting, so this also keeps us aligned with it.
            let Some(value) = Value::from_be_slice(point.value) else {
                continue;
            };
            if value.is_zero() {
                continue;
            }
            match reported.get_mut(index) {
                Some(slot) if slot.is_some() => return Err(VerifyError::ReoccurringSigner),
                Some(slot) => *slot = Some(value),
                // Unreachable: `index` came from `config.signers()`, whose length
                // `try_new` capped at MAX_SIGNERS.
                None => return Err(VerifyError::ReoccurringSigner),
            }
        }
        Ok(())
    });

    match walked {
        Err(err) => return Err(VerifyError::Malformed(err)),
        Ok(Err(err)) => return Err(err),
        Ok(Ok(())) => {}
    }

    let mut collected = [Value::default(); MAX_SIGNERS];
    let mut met = 0usize;
    for value in reported.iter().flatten() {
        if let Some(slot) = collected.get_mut(met) {
            *slot = *value;
            met += 1;
        }
    }

    let met_count = u8::try_from(met).unwrap_or(u8::MAX);
    if met_count < config.threshold() {
        // Were the skipped packages enough to have reached the threshold? If so
        // the configured signer set is what failed, not the data.
        let reachable = met.saturating_add(unknown_count) >= usize::from(config.threshold());
        return Err(if reachable {
            VerifyError::UnauthorisedSigner
        } else {
            VerifyError::ThresholdNotMet {
                met: met_count,
                required: config.threshold(),
            }
        });
    }

    let value = match collected.get_mut(..met).and_then(median) {
        Some(value) => value,
        // Unreachable: met >= threshold >= 1. Reported rather than panicked,
        // because a panic in a guest aborts the transaction.
        None => {
            return Err(VerifyError::ThresholdNotMet {
                met: met_count,
                required: config.threshold(),
            })
        }
    };

    Ok(VerifiedFeed {
        value,
        signers: met_count,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;

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

    use crate::{
        backend::InProgramBackend,
        decode::Payload,
        test_support::{address_of, signing_key, PayloadBuilder},
    };

    /// `n` distinct keys, and the signer set they form.
    fn keys(n: u8) -> std::vec::Vec<k256::ecdsa::SigningKey> {
        (1..=n).map(signing_key).collect()
    }

    fn signer_set(keys: &[k256::ecdsa::SigningKey]) -> std::vec::Vec<SignerAddress> {
        keys.iter().map(address_of).collect()
    }

    #[test]
    fn three_of_three_signers_reporting_the_same_price_verifies() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], 1_770_000_000_000);
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.signers, 3);
        assert_eq!(verified.value, Value::from_be_slice(&[100]).expect("fits"));
    }

    #[test]
    fn three_of_five_is_enough_and_the_median_is_over_the_signers_that_reported() {
        // Five configured, three sign, with different prices. The median is over
        // the three that reported, not over the configured set.
        let keys = keys(5);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1_770_000_000_000)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 30])], 1_770_000_000_000)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 20])], 1_770_000_000_000)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.signers, 3);
        assert_eq!(verified.value, Value::from_be_slice(&[20]).expect("fits"));
    }

    #[test]
    fn an_even_number_of_reporting_signers_averages_the_middle_two() {
        let keys = keys(4);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], 1)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 40])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.signers, 4);
        assert_eq!(verified.value, Value::from_be_slice(&[25]).expect("fits"));
    }

    #[test]
    fn a_package_carrying_other_feeds_contributes_only_the_one_requested() {
        // A RedStone payload is multi-feed even though an update is single-feed,
        // so unrequested feeds arrive on every real call.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(
                key,
                &[(b"ETH", &[0, 0, 0, 7]), (b"BTC", &[0, 0, 0, 50])],
                1,
            );
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.value, Value::from_be_slice(&[50]).expect("fits"));
    }

    #[test]
    fn an_unknown_signer_is_skipped_and_the_payload_still_verifies() {
        // Three authorised signers plus a stranger. RedStone publishes payloads
        // carrying more signers than any one consumer configures, so this is the
        // healthy case rather than an attack.
        let keys = keys(3);
        let set = signer_set(&keys);
        let stranger = signing_key(0x99);

        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 60])], 1);
        }
        let bytes = builder
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0x27, 0x0F])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.signers, 3, "the stranger is not counted");
        assert_eq!(
            verified.value,
            Value::from_be_slice(&[60]).expect("fits"),
            "and cannot move the median"
        );
    }

    #[test]
    fn one_signer_cannot_reach_the_threshold_alone_by_repeating_the_feed() {
        // The anti-inflation rule. Without it a single signer occupies as many
        // slots as it sends packages.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 20])], 2)
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 30])], 3)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ReoccurringSigner)
        );
    }

    #[test]
    fn a_zero_value_does_not_count_toward_the_threshold() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ThresholdNotMet {
                met: 2,
                required: 3
            }),
            "two non-zero reports out of three"
        );
    }

    #[test]
    fn a_payload_with_no_packages_for_the_requested_feed_is_a_threshold_failure() {
        // Not a decode error: the payload is well formed, it just says nothing
        // about the feed we asked about.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"ETH", &[0, 0, 0, 5])], 1);
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            })
        );
    }

    #[test]
    fn unauthorised_is_reported_when_the_skipped_signers_would_have_made_quorum() {
        // met = 2, one distinct unknown, threshold 3. Authorising that signer
        // would have reached quorum, so the signer set is what failed. Reporting
        // "2 of 3" here would send an operator to look for missing data.
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&configured[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&configured[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::UnauthorisedSigner)
        );
    }

    #[test]
    fn a_threshold_that_was_unreachable_anyway_is_not_blamed_on_the_signer_set() {
        // met = 0, one distinct unknown, threshold 3. Even authorising the
        // stranger leaves one of three, so the honest answer is that too few
        // signers signed. The naive "nothing matched, so blame the set" rule gets
        // this wrong.
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            })
        );
    }

    #[test]
    fn repeated_packages_from_one_unknown_signer_count_as_one() {
        // met = 1, three packages from the *same* stranger, threshold 3.
        // Counting packages would make this look reachable and report
        // UnauthorisedSigner; counting distinct signers gets it right.
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&configured[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], 1)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 31])], 2)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 32])], 3)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ThresholdNotMet {
                met: 1,
                required: 3
            })
        );
    }

    #[test]
    fn a_malformed_payload_is_reported_as_malformed_not_as_a_threshold_failure() {
        // The distinction decode.rs exists to preserve, carried through the
        // threshold: "not well formed" and "not authorised" stay separate.
        let keys = keys(1);
        let set = signer_set(&keys);
        let valid = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .build();
        let mut bytes = std::vec![0xDEu8; 16];
        bytes.extend_from_slice(&valid);

        let payload = Payload::decode(&bytes).expect("framing decodes");
        let config = FeedConfig::try_new(b"BTC", &set, 1).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::Malformed(
                crate::decode::DecodeError::TrailingBytes(16)
            ))
        );
    }

    #[test]
    fn an_unrepresentable_value_costs_only_that_signer_not_the_whole_feed() {
        // The reason this is a skip and not an error: erroring would let a single
        // configured signer deny the feed to everyone, which is exactly what an
        // M-of-N threshold exists to prevent. Three good signers plus one sending
        // an unrepresentable width must still produce a price.
        let keys = keys(4);
        let set = signer_set(&keys);
        let wide = [0x01u8; 33];
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &wide)], 1)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
        let verified = verify_feed(&payload, &config, &InProgramBackend::new()).expect("verifies");

        assert_eq!(verified.signers, 3, "the oversized signer is not counted");
        assert_eq!(verified.value, Value::from_be_slice(&[20]).expect("fits"));
    }

    #[test]
    fn an_unrepresentable_value_can_still_leave_the_threshold_unmet() {
        // Skipping is not silence: with too few good signers left, the caller
        // still gets a rejection, just one that names the threshold rather than
        // blaming the payload's shape.
        let keys = keys(3);
        let set = signer_set(&keys);
        let wide = [0x01u8; 33];
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &wide)], 1)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");

        assert_eq!(
            verify_feed(&payload, &config, &InProgramBackend::new()),
            Err(VerifyError::ThresholdNotMet {
                met: 1,
                required: 3
            })
        );
    }

    #[test]
    fn the_threshold_boundary_accepts_at_m_and_rejects_at_m_minus_one() {
        let keys = keys(5);
        let set = signer_set(&keys);

        for reporting in 1..=5usize {
            let mut builder = PayloadBuilder::default();
            for key in keys.iter().take(reporting) {
                builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 10])], 1);
            }
            let bytes = builder.build();
            let payload = Payload::decode(&bytes).expect("well formed");
            let config = FeedConfig::try_new(b"BTC", &set, 3).expect("valid config");
            let outcome = verify_feed(&payload, &config, &InProgramBackend::new());

            if reporting >= 3 {
                assert_eq!(
                    outcome.map(|v| v.signers),
                    Ok(u8::try_from(reporting).expect("at most five")),
                    "{reporting} signers must meet a threshold of 3"
                );
            } else {
                assert_eq!(
                    outcome,
                    Err(VerifyError::ThresholdNotMet {
                        met: u8::try_from(reporting).expect("at most five"),
                        required: 3
                    }),
                    "{reporting} signers must not meet a threshold of 3"
                );
            }
        }
    }
}
