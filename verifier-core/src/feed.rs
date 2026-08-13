//! One feed's configuration, and the M-of-N rule over it.
//!
//! The configuration is the *caller's*, never the payload's. In pull mode the
//! signer set comes from the consumer and never from the data, so nothing here
//! reads a signer, a threshold or a feed id out of a payload.

use crate::{
    backend::{SignerAddress, VerifierBackend},
    decode::{Payload, FEED_ID_BYTES},
    error::{ConfigError, VerifyError},
    value::{median, Value, MAX_DECIMALS},
};

/// The two assets a price relates: how much quote one unit of base is worth.
///
/// Opaque bytes here rather than a LEZ `AccountId`, because this crate carries
/// no chain dependency. The aggregator and the pull library map the real
/// identifier in at their boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetPair {
    pub base: [u8; Self::ID_LEN],
    pub quote: [u8; Self::ID_LEN],
}

impl AssetPair {
    /// Width of one asset identifier, matching the LEZ account id.
    pub const ID_LEN: usize = 32;

    #[must_use]
    pub const fn new(base: [u8; Self::ID_LEN], quote: [u8; Self::ID_LEN]) -> Self {
        Self { base, quote }
    }
}

/// The most signers one feed may configure.
///
/// A real limit rather than a guard: it sizes three fixed stack buffers
/// `verify_feed` walks with — `reported` (`[Option<Value>; 32]`, 1,056 bytes),
/// `unknown` (`[SignerAddress; 32]`, 640 bytes) and `collected` (`[Value; 32]`,
/// 1,024 bytes), about 2.7 KB in one frame — which is how the threshold runs
/// without an allocator. `collected` exists only because `median` takes
/// `&mut [Value]` while `reported` holds `Option<Value>`; sorting `reported` in
/// place would remove it and save a kilobyte of guest stack, and is deliberately
/// left for later rather than folded in here. RedStone caps at 255 and live
/// feeds run ten to twenty, so this leaves headroom. Raising it costs stack and
/// nothing else.
pub const MAX_SIGNERS: usize = 32;

/// A feed, its authorised signers, and how many of them must agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedConfig<'a> {
    feed_id: [u8; FEED_ID_BYTES],
    assets: AssetPair,
    decimals: u8,
    signers: &'a [SignerAddress],
    threshold: u8,
}

impl<'a> FeedConfig<'a> {
    /// Checks a configuration and pads its feed id to the wire width.
    ///
    /// Every invariant here is one a payload cannot repair, so they are settled
    /// once at configuration rather than re-tested per package.
    ///
    /// `assets` is the pair this feed is registered against, and `decimals` the
    /// power of ten the signers scale their values by.
    ///
    /// The pair is stored as given. An all-zero identifier is a legitimate LEZ
    /// account id, and whether a pair is meaningful is the registering
    /// authority's judgement to make, not this crate's.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] for an empty or oversized signer list, a threshold of zero
    /// or one no signer set can reach, a repeated or zero signer address, a
    /// feed id that is empty, all-zero, or wider than the wire field, or a
    /// decimal exponent above [`MAX_DECIMALS`].
    pub fn try_new(
        feed_id: &[u8],
        assets: AssetPair,
        decimals: u8,
        signers: &'a [SignerAddress],
        threshold: u8,
    ) -> Result<Self, ConfigError> {
        if decimals > MAX_DECIMALS {
            return Err(ConfigError::DecimalsOutOfRange {
                decimals,
                max: MAX_DECIMALS,
            });
        }
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
            assets,
            decimals,
            signers,
            threshold,
        })
    }

    /// The asset pair this feed is registered against.
    #[must_use]
    pub const fn assets(&self) -> &AssetPair {
        &self.assets
    }

    /// The power of ten the feed's signers scale their values by.
    #[must_use]
    pub const fn decimals(&self) -> u8 {
        self.decimals
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
    /// The median across the signers that reported, on RedStone's scale.
    ///
    /// Kept alongside `price` because conformance against published RedStone
    /// vectors has to compare a number this crate did not derive.
    pub value: Value,
    /// `value` converted to the price account's `Q64.64` scale.
    pub price: u128,
    /// How many distinct authorised signers reported. Always at least the
    /// threshold.
    pub signers: u8,
}

/// Whether a value for the requested feed is one a price can be built from.
///
/// A zero is RedStone's own "no report" placeholder, and the top bit set is the
/// only thing a negative price can look like in an unsigned field.
fn usable(value: &Value) -> bool {
    !value.is_zero() && !value.is_negative()
}

/// Verifies one feed out of a payload.
///
/// Walks the payload once, recovering one signer per package, and counts the
/// distinct authorised signers that reported a usable value for
/// `config.feed_id()`. Returns the median across them, on both RedStone's scale
/// and the price account's, once the threshold is met.
///
/// Unknown signers and unrequested feeds are **skipped, not fatal**. That is
/// RedStone's own rule, and it is what lets one published payload serve consumers
/// whose signer sets and feed interests differ. Failing the payload on any
/// unknown signer would hand a denial-of-service primitive to anyone able to
/// append a package.
///
/// `expected` is the asset pair the caller believes this feed prices, compared
/// against the pair the feed was registered with. It is a parameter rather than
/// a separate method so that a caller cannot reach a price without stating what
/// it thought it was asking for.
///
/// # Errors
///
/// [`VerifyError`] for a malformed payload, an asset pair that is not the
/// caller's, one signer supplying the feed twice, a threshold that was not
/// reached, values that were unusable, or a price the account's scale cannot
/// hold. An unrecoverable signature is skipped rather than reported; see
/// [`VerifyError::InvalidSignature`].
pub fn verify_feed<B: VerifierBackend>(
    payload: &Payload<'_>,
    config: &FeedConfig<'_>,
    expected: &AssetPair,
    backend: &B,
) -> Result<VerifiedFeed, VerifyError> {
    // Before the walk: a mismatch is a 64-byte comparison, where reaching the
    // same answer afterwards would have cost one signature recovery per signer.
    if config.assets() != expected {
        return Err(VerifyError::AssetMismatch);
    }

    // One slot per configured signer: the slot is both the collection point and
    // the duplicate check, exactly as RedStone's (feed, signer) matrix cell is.
    let mut reported: [Option<Value>; MAX_SIGNERS] = [None; MAX_SIGNERS];
    // Distinct unknown signers. Counting packages instead would let three
    // packages from one unknown address read as three missing signers.
    let mut unknown: [SignerAddress; MAX_SIGNERS] =
        [SignerAddress([0; SignerAddress::LEN]); MAX_SIGNERS];
    let mut unknown_count = 0usize;
    // Configured signers that supplied this feed and whose every value for it
    // was unusable. Kept apart from the untouched slots so "your signers sent
    // nothing usable" and "your signers did not sign" stay different answers.
    let mut spoiled = [false; MAX_SIGNERS];

    let walked = payload.for_each_package(|package| {
        let digest = backend.keccak256(package.signable());
        // An unrecoverable signature (bad recovery id, r/s outside the curve
        // order, a malleable high-s) is skipped, not fatal: it needs no key and
        // no valid signature to produce, which makes propagating it the
        // cheapest denial-of-service primitive in this design. RedStone skips
        // it the same way (`Some(address) => address, _ => continue`).
        let Ok(signer) = backend.recover_signer(&digest, &package.signature) else {
            return Ok(());
        };

        let Some(index) = config.signers().iter().position(|s| *s == signer) else {
            // Recorded only if the package actually carries a usable data
            // point for the requested feed: an unknown signer that reported
            // nothing for this feed could not have moved the threshold even
            // if it had been authorised.
            let contributes = package.data_points().any(|point| {
                point.feed_id == config.feed_id()
                    && Value::from_be_slice(point.value).is_some_and(|value| usable(&value))
            });
            if contributes
                && unknown_count < MAX_SIGNERS
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

        // Skipping rather than erroring on a missing slot is deliberate:
        // skipping is this crate's right default. Unreachable in practice —
        // `index` came from `config.signers()`, whose length `try_new` capped
        // at MAX_SIGNERS — but a missing slot is not this signer's fault.
        let Some(slot) = reported.get_mut(index) else {
            return Ok(());
        };
        let mut supplied = false;
        for point in package.data_points() {
            if point.feed_id != config.feed_id() {
                continue;
            }
            supplied = true;
            // Zero, negative and unrepresentable values are skipped, not fatal.
            // Erroring here would let one configured signer deny the feed,
            // which is the thing M-of-N exists to prevent. RedStone sanitises
            // instead of rejecting, so this also keeps us aligned with it. What
            // the skips cost collectively is answered after the walk.
            let Some(value) = Value::from_be_slice(point.value).filter(usable) else {
                continue;
            };
            if slot.is_some() {
                return Err(VerifyError::ReoccurringSigner);
            }
            *slot = Some(value);
        }

        if supplied && slot.is_none() {
            if let Some(flag) = spoiled.get_mut(index) {
                *flag = true;
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
        // Which of the skips cost the threshold? Each test asks whether undoing
        // that one class of skip would have reached it; the first that would is
        // the cause worth naming. Spoiled values come before unknown signers
        // because a configured signer that did report is the nearer fault.
        let required = usize::from(config.threshold());
        let spoiled_count = spoiled.iter().filter(|flagged| **flagged).count();

        if met.saturating_add(spoiled_count) >= required {
            return Err(VerifyError::ValueOutOfRange);
        }
        if met.saturating_add(unknown_count) >= required {
            return Err(VerifyError::UnauthorisedSigner);
        }
        return Err(VerifyError::ThresholdNotMet {
            met: met_count,
            required: config.threshold(),
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

    let price = value
        .to_q64_64(config.decimals())
        .ok_or(VerifyError::ScalingOutOfRange)?;

    Ok(VerifiedFeed {
        value,
        price,
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

    /// RedStone's usual exponent, so the conversion runs on every test rather
    /// than only where it is the subject.
    const DECIMALS: u8 = 8;

    fn pair() -> AssetPair {
        AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
    }

    fn feed_config(signers: &[SignerAddress], threshold: u8) -> FeedConfig<'_> {
        FeedConfig::try_new(b"BTC", pair(), DECIMALS, signers, threshold).expect("valid config")
    }

    fn verify(payload: &Payload<'_>, config: &FeedConfig<'_>) -> Result<VerifiedFeed, VerifyError> {
        verify_feed(payload, config, &pair(), &InProgramBackend::new())
    }

    #[test]
    fn a_feed_id_is_right_padded_the_way_the_wire_pads_it() {
        // `FeedConfig<'a>` borrows `signers`, so the array needs a name: an
        // inline `&[signer(1)]` is a temporary that doesn't outlive `config`,
        // which is used again below.
        let signers = [signer(1)];
        let config = FeedConfig::try_new(b"BTC", pair(), DECIMALS, &signers, 1).expect("valid");
        let mut expected = [0u8; FEED_ID_BYTES];
        expected[..3].copy_from_slice(b"BTC");
        assert_eq!(config.feed_id(), &expected);
    }

    #[test]
    fn an_empty_signer_list_makes_every_threshold_unreachable() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, &[], 1).unwrap_err(),
            ConfigError::NoSigners
        );
    }

    #[test]
    fn a_zero_threshold_is_rejected() {
        // A threshold of zero is satisfied by a payload nobody signed.
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, &[signer(1)], 0).unwrap_err(),
            ConfigError::ThresholdZero
        );
    }

    #[test]
    fn a_threshold_no_signer_set_can_reach_is_rejected_at_configuration_time() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, &[signer(1), signer(2)], 3).unwrap_err(),
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
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, &many, 3).unwrap_err(),
            ConfigError::TooManySigners {
                signers: MAX_SIGNERS + 1,
                max: MAX_SIGNERS
            }
        );
    }

    #[test]
    fn exactly_max_signers_signers_is_accepted() {
        // Only MAX_SIGNERS + 1 is tested elsewhere, so an off-by-one in
        // `signers.len() > MAX_SIGNERS` would pass every existing test.
        let many: [SignerAddress; MAX_SIGNERS] =
            core::array::from_fn(|i| signer(u8::try_from(i + 1).unwrap_or(u8::MAX)));
        assert!(FeedConfig::try_new(b"BTC", pair(), DECIMALS, &many, 3).is_ok());
    }

    #[test]
    fn a_repeated_signer_is_rejected() {
        // Two slots for one address would let it reach any threshold alone.
        assert_eq!(
            FeedConfig::try_new(
                b"BTC",
                pair(),
                DECIMALS,
                &[signer(1), signer(2), signer(1)],
                2
            )
            .unwrap_err(),
            ConfigError::DuplicateSigner
        );
    }

    #[test]
    fn the_zero_address_is_not_a_signer() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, &[signer(1), signer(0)], 1).unwrap_err(),
            ConfigError::ZeroSignerAddress
        );
    }

    #[test]
    fn a_zero_address_paired_with_a_duplicate_reports_zero_signer_address() {
        // Pins which check wins when both apply: the zero-address check runs
        // before the duplicate check, not after.
        assert_eq!(
            FeedConfig::try_new(
                b"BTC",
                pair(),
                DECIMALS,
                &[signer(1), signer(0), signer(1)],
                1
            )
            .unwrap_err(),
            ConfigError::ZeroSignerAddress
        );
    }

    #[test]
    fn a_feed_id_wider_than_the_wire_field_is_rejected() {
        let long = [b'X'; FEED_ID_BYTES + 1];
        assert_eq!(
            FeedConfig::try_new(&long, pair(), DECIMALS, &[signer(1)], 1).unwrap_err(),
            ConfigError::FeedIdTooLong {
                len: FEED_ID_BYTES + 1
            }
        );
    }

    #[test]
    fn an_empty_or_zero_feed_id_is_rejected() {
        // It would match the padding of any feed id in any payload.
        assert_eq!(
            FeedConfig::try_new(b"", pair(), DECIMALS, &[signer(1)], 1).unwrap_err(),
            ConfigError::ZeroFeedId
        );
        assert_eq!(
            FeedConfig::try_new(&[0u8; 4], pair(), DECIMALS, &[signer(1)], 1).unwrap_err(),
            ConfigError::ZeroFeedId
        );
    }

    #[test]
    fn a_threshold_equal_to_the_signer_count_is_allowed() {
        assert!(FeedConfig::try_new(b"BTC", pair(), DECIMALS, &[signer(1), signer(2)], 2).is_ok());
    }

    #[test]
    fn an_exponent_the_conversion_cannot_divide_by_is_a_configuration_error() {
        let signers = [signer(1)];
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), MAX_DECIMALS + 1, &signers, 1).unwrap_err(),
            ConfigError::DecimalsOutOfRange {
                decimals: MAX_DECIMALS + 1,
                max: MAX_DECIMALS,
            }
        );
        assert!(FeedConfig::try_new(b"BTC", pair(), MAX_DECIMALS, &signers, 1).is_ok());
    }

    #[test]
    fn an_asset_pair_is_stored_as_given_rather_than_judged() {
        // A zero account id is legitimate on LEZ, and which pairs are meaningful
        // is the registering authority's call. Nothing here second-guesses it.
        let signers = [signer(1)];
        let same = [7u8; AssetPair::ID_LEN];
        assert!(FeedConfig::try_new(
            b"BTC",
            AssetPair::new([0; AssetPair::ID_LEN], [0; AssetPair::ID_LEN]),
            DECIMALS,
            &signers,
            1
        )
        .is_ok());
        assert!(
            FeedConfig::try_new(b"BTC", AssetPair::new(same, same), DECIMALS, &signers, 1).is_ok()
        );
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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 3);

        // The zero fills no slot, so two of three is short. All three signers
        // did report, though, which is a different problem from one of them
        // staying silent, and the caller gets told which.
        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 1);

        assert_eq!(
            verify(&payload, &config),
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
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

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
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
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
            let config = feed_config(&set, 3);
            let outcome = verify(&payload, &config);

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

    #[test]
    fn an_outsiders_garbage_signature_cannot_deny_the_feed() {
        // Change 1's regression test, and the most important of the five: no
        // key and no valid signature are needed to mount this, so if recovery
        // failure ever propagates again this is the cheapest denial-of-service
        // vector in the design.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], 1);
        }
        let bytes = builder
            .opaque_package(&[(b"BTC", &[0, 0, 0, 100])], 1, 0xFF)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(
            verified.signers, 3,
            "the garbage package is skipped, not fatal"
        );
    }

    #[test]
    fn strangers_reporting_only_another_feed_do_not_report_unauthorised_signer() {
        // Change 2's regression test. Recording these as unknown before
        // checking they carried a usable BTC point would give met = 0,
        // unknown = 3, and 0 + 3 >= 3 report UnauthorisedSigner — even though
        // authorising all three would still have produced nothing.
        let stranger_keys = [signing_key(0x99), signing_key(0x98), signing_key(0x97)];
        let configured = keys(3);
        let set = signer_set(&configured);

        let mut builder = PayloadBuilder::default();
        for key in &stranger_keys {
            builder = builder.signed_package(key, &[(b"ETH", &[0, 0, 0, 7])], 1);
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            })
        );
    }

    #[test]
    fn a_single_package_carrying_the_requested_feed_twice_is_reoccurring_signer() {
        // The cheaper attack: one signature instead of three, reaching the
        // same rule from the inner point loop rather than across packages.
        let keys = keys(1);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(
                &keys[0],
                &[(b"BTC", &[0, 0, 0, 10]), (b"BTC", &[0, 0, 0, 20])],
                1,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 1);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::ReoccurringSigner)
        );
    }

    /// A payload three configured signers agree on, for the tests that only
    /// vary one thing about the caller or the values.
    fn agreed(keys: &[k256::ecdsa::SigningKey], value: &[u8]) -> std::vec::Vec<u8> {
        let mut builder = PayloadBuilder::default();
        for key in keys {
            builder = builder.signed_package(key, &[(b"BTC", value)], 1_770_000_000_000);
        }
        builder.build()
    }

    #[test]
    fn a_feed_registered_against_another_pair_is_refused() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &[0, 0, 0, 100]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        let elsewhere = AssetPair::new([0x11; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN]);
        assert_eq!(
            verify_feed(&payload, &config, &elsewhere, &InProgramBackend::new()),
            Err(VerifyError::AssetMismatch),
            "the same payload, asked about the wrong asset"
        );
        assert!(
            verify(&payload, &config).is_ok(),
            "and it verifies when the caller expects what the feed is registered for"
        );
    }

    #[test]
    fn a_pair_matching_only_on_the_base_is_still_a_mismatch() {
        // The failure this catches in practice is a config copied between two
        // feeds that share a base and differ in what denominates them.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &[0, 0, 0, 100]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        let other_quote = AssetPair::new(pair().base, [0x99; AssetPair::ID_LEN]);
        assert_eq!(
            verify_feed(&payload, &config, &other_quote, &InProgramBackend::new()),
            Err(VerifyError::AssetMismatch)
        );
    }

    #[test]
    fn the_verified_price_carries_both_scales() {
        // $3000.12345678, as RedStone writes it at eight decimals.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &300_012_345_678u64.to_be_bytes());

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(
            verified.value,
            Value::from_be_slice(&300_012_345_678u64.to_be_bytes()).expect("fits"),
            "RedStone's scale, for conformance against its vectors"
        );
        assert_eq!(
            verified.price, 55_342_509_596_753_479_111_897,
            "the same price as the account's Q64.64"
        );
    }

    #[test]
    fn a_negative_value_costs_only_the_signer_that_sent_it() {
        // The top half of the range read as int256. Skipped like a zero, for the
        // same reason: one configured signer must not be able to deny a feed.
        let keys = keys(4);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys[..3] {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], 1);
        }
        let bytes = builder
            .signed_package(&keys[3], &[(b"BTC", &[0xFF; 32])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("three good signers still agree");

        assert_eq!(verified.signers, 3);
        assert_eq!(verified.value, Value::from_be_slice(&[100]).expect("fits"));
    }

    #[test]
    fn signers_that_all_reported_something_unusable_are_named_as_the_fault() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &[0xFF; 32]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        // Every configured signer signed and reported this feed. "Threshold not
        // met" would send an operator looking for signers that are already there.
        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }

    #[test]
    fn a_bad_value_that_could_not_have_met_the_threshold_anyway_is_not_blamed() {
        // The counterfactual, the same shape ADR 15 applies to unknown signers:
        // one spoiled report out of a threshold of three leaves the count short
        // even if the value had been perfect, so the count is the honest answer.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 0])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            })
        );
    }

    #[test]
    fn a_configured_signers_bad_value_is_named_before_a_strangers_absence() {
        // Both causal tests pass here: two good, one configured signer reporting
        // zero, one stranger reporting a price. Either would close the gap, and
        // the signer that is already authorised is the nearer thing to fix.
        let keys = keys(4);
        let set = signer_set(&keys[..3]);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 1)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 1)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], 1)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 30])], 1)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }

    #[test]
    fn an_agreed_price_too_large_for_the_account_is_reported_not_wrapped() {
        // Not reachable from a real feed, but the alternative to reporting it is
        // writing a wrapped number that looks like a price and is not one.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &[0x7F; 12]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::ScalingOutOfRange)
        );
    }
}
