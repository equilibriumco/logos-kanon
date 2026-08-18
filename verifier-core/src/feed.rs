//! One feed's configuration, and the M-of-N rule over it.
//!
//! The configuration is the *caller's*, never the payload's. In pull mode the
//! signer set comes from the consumer and never from the data, so nothing here
//! reads a signer, a threshold or a feed id out of a payload.

use crate::{
    backend::{SignerAddress, VerifierBackend},
    decode::{Payload, FEED_ID_BYTES},
    error::{ConfigError, VerifyError},
    time::TimeSource,
    value::{median, Value, MAX_DECIMALS},
};

/// How far ahead of the chain's clock a package may be dated.
///
/// Allowance for skew between a RedStone signer's clock and the sequencer's,
/// not a policy knob: no feed operator knows that skew better than this crate
/// does. RedStone's own `MAX_TIMESTAMP_AHEAD_MS`, so a package RedStone would
/// accept is never one Kanon rejects.
pub const MAX_AHEAD_MS: u64 = 3 * 60 * 1000;

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
    max_age_ms: u64,
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
        max_age_ms: u64,
        signers: &'a [SignerAddress],
        threshold: u8,
    ) -> Result<Self, ConfigError> {
        if max_age_ms == 0 {
            return Err(ConfigError::MaxAgeZero);
        }
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
            max_age_ms,
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

    /// How old a package may be, in milliseconds.
    #[must_use]
    pub const fn max_age_ms(&self) -> u64 {
        self.max_age_ms
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
    /// When the oldest package behind `value` was signed, in milliseconds.
    ///
    /// The oldest rather than the newest, because a median is only as current
    /// as the stalest report that shaped it. A consumer applying its own
    /// `maxAge` to this is therefore never told a price is fresher than every
    /// signer behind it supports.
    pub timestamp_ms: u64,
}

/// Where a package's timestamp sits relative to the chain's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freshness {
    Current,
    Stale,
    Future,
}

/// Places `timestamp_ms` in the window `[now - max_age, now + MAX_AHEAD]`.
///
/// Saturating on both sides: a chain clock below either bound is possible on a
/// fresh devnet, and a panic in a guest aborts the transaction.
fn freshness(timestamp_ms: u64, now_ms: u64, max_age_ms: u64) -> Freshness {
    if timestamp_ms < now_ms.saturating_sub(max_age_ms) {
        Freshness::Stale
    } else if timestamp_ms > now_ms.saturating_add(MAX_AHEAD_MS) {
        Freshness::Future
    } else {
        Freshness::Current
    }
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
pub fn verify_feed<B: VerifierBackend, T: TimeSource>(
    payload: &Payload<'_>,
    config: &FeedConfig<'_>,
    expected: &AssetPair,
    backend: &B,
    clock: &T,
) -> Result<VerifiedFeed, VerifyError> {
    // Before the walk: a mismatch is a 64-byte comparison, where reaching the
    // same answer afterwards would have cost one signature recovery per signer.
    if config.assets() != expected {
        return Err(VerifyError::AssetMismatch);
    }

    // Read once, before the walk: a clock that moved mid-payload would let two
    // packages of the same age fall on opposite sides of the same window.
    let now_ms = clock.now_ms()?;

    // One slot per configured signer: the slot is both the collection point and
    // the duplicate check, exactly as RedStone's (feed, signer) matrix cell is.
    let mut reported: [Option<Value>; MAX_SIGNERS] = [None; MAX_SIGNERS];
    // Distinct unknown signers. Counting packages instead would let three
    // packages from one unknown address read as three missing signers.
    let mut unknown: [SignerAddress; MAX_SIGNERS] =
        [SignerAddress([0; SignerAddress::LEN]); MAX_SIGNERS];
    let mut unknown_count = 0usize;
    // Configured signers that supplied this feed at all, usable value or not.
    // Whether that cost anything is decided against `reported` after the walk,
    // because a signer can send one package this reads and another it does not,
    // in either order.
    let mut supplied_feed = [false; MAX_SIGNERS];
    // Why a slot stayed empty, when the reason was the package's age. Resolved
    // against `reported` after the walk, like `supplied_feed`.
    let mut stale = [false; MAX_SIGNERS];
    let mut future = [false; MAX_SIGNERS];
    // The oldest package that filled a slot. A running minimum rather than a
    // timestamp per slot, which would cost 256 bytes of the guest frame to hold
    // 31 values nothing reads: slots are only ever filled, never cleared, so
    // every update here is a package that ends up behind the median.
    let mut oldest_ms = u64::MAX;

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
            // Also requires the package to be in the window: authorising a
            // stranger whose package is stale would not have produced a price
            // either, so it is not a signer set the caller should be told to fix.
            let contributes = freshness(package.timestamp_ms, now_ms, config.max_age_ms())
                == Freshness::Current
                && package.data_points().any(|point| {
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
        // After recovery, so the package is known to be a configured signer's.
        // Checking it first would be cheaper -- two comparisons against 565,497
        // cycles -- but recovery would then be skipped for a package outside the
        // window, and an outsider could append three unsigned stale packages to
        // turn a threshold failure into a staleness one. A fresh payload has
        // nothing to skip, so the saving only ever arrived in the two cases
        // where naming the right cause matters most.
        match freshness(package.timestamp_ms, now_ms, config.max_age_ms()) {
            Freshness::Stale => {
                if let Some(flag) = stale.get_mut(index) {
                    *flag = true;
                }
                return Ok(());
            }
            Freshness::Future => {
                if let Some(flag) = future.get_mut(index) {
                    *flag = true;
                }
                return Ok(());
            }
            Freshness::Current => {}
        }

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
            oldest_ms = oldest_ms.min(package.timestamp_ms);
        }

        if supplied {
            if let Some(flag) = supplied_feed.get_mut(index) {
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
        // Each tally counts only signers whose slot stayed empty. A signer that
        // filled its slot is already in `met`, and counting it again would let
        // its own second package close the gap a silent neighbour left.
        let blocked = |flags: &[bool; MAX_SIGNERS]| {
            flags
                .iter()
                .zip(reported.iter())
                .filter(|(flagged, slot)| **flagged && slot.is_none())
                .count()
        };

        // Were the configured signers there at all? A signer whose slot stayed
        // empty for any of these reasons still signed, and answering "the
        // threshold was not met" would send an operator looking for signers that
        // are already in the payload. Counted over signers rather than over
        // reasons, because one signer can arrive stale in one package and
        // useless in another and must not close two gaps by itself.
        let present = stale
            .iter()
            .zip(future.iter())
            .zip(supplied_feed.iter())
            .zip(reported.iter())
            .filter(|(((s, f), v), slot)| (**s || **f || **v) && slot.is_none())
            .count();

        if met.saturating_add(present) >= required {
            // Which reason to name, when the signers are present but several
            // things are wrong. The largest cause, because it is the one whose
            // fixing moves the count furthest; ties go to age, then to skew,
            // then to values, which is the order in which a cause resolves
            // without anybody acting. A fresher payload fixes staleness, where
            // a bad value needs someone to change something, and sending an
            // operator to reconfigure a feed that will be fine in thirty
            // seconds is the wrong answer even when it is also a true one.
            let stale_count = blocked(&stale);
            let future_count = blocked(&future);
            let spoiled_count = blocked(&supplied_feed);

            return Err(
                if stale_count >= future_count && stale_count >= spoiled_count {
                    VerifyError::StalePackage
                } else if future_count >= spoiled_count {
                    VerifyError::FuturePackage
                } else {
                    VerifyError::ValueOutOfRange
                },
            );
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
        timestamp_ms: oldest_ms,
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
        FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, signers, threshold)
            .expect("valid config")
    }

    /// A clock the test chooses, standing in for the pinned LEZ account.
    struct FixedClock(Result<u64, TimeError>);

    impl TimeSource for FixedClock {
        fn now_ms(&self) -> Result<u64, TimeError> {
            self.0
        }
    }

    const NOW_MS: u64 = 1_770_000_000_000;

    /// No staleness bound, so every test that is not about timestamps keeps
    /// using whatever timestamp reads clearly. Staleness has its own tests.
    const NO_MAX_AGE: u64 = u64::MAX;

    fn verify(payload: &Payload<'_>, config: &FeedConfig<'_>) -> Result<VerifiedFeed, VerifyError> {
        verify_feed(
            payload,
            config,
            &pair(),
            &InProgramBackend::new(),
            &FixedClock(Ok(NOW_MS)),
        )
    }

    #[test]
    fn a_feed_id_is_right_padded_the_way_the_wire_pads_it() {
        // `FeedConfig<'a>` borrows `signers`, so the array needs a name: an
        // inline `&[signer(1)]` is a temporary that doesn't outlive `config`,
        // which is used again below.
        let signers = [signer(1)];
        let config =
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, &signers, 1).expect("valid");
        let mut expected = [0u8; FEED_ID_BYTES];
        expected[..3].copy_from_slice(b"BTC");
        assert_eq!(config.feed_id(), &expected);
    }

    #[test]
    fn an_empty_signer_list_makes_every_threshold_unreachable() {
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, &[], 1).unwrap_err(),
            ConfigError::NoSigners
        );
    }

    #[test]
    fn a_zero_threshold_is_rejected() {
        // A threshold of zero is satisfied by a payload nobody signed.
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, &[signer(1)], 0).unwrap_err(),
            ConfigError::ThresholdZero
        );
    }

    #[test]
    fn a_threshold_no_signer_set_can_reach_is_rejected_at_configuration_time() {
        assert_eq!(
            FeedConfig::try_new(
                b"BTC",
                pair(),
                DECIMALS,
                NO_MAX_AGE,
                &[signer(1), signer(2)],
                3
            )
            .unwrap_err(),
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
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, &many, 3).unwrap_err(),
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
        assert!(FeedConfig::try_new(b"BTC", pair(), DECIMALS, NO_MAX_AGE, &many, 3).is_ok());
    }

    #[test]
    fn a_repeated_signer_is_rejected() {
        // Two slots for one address would let it reach any threshold alone.
        assert_eq!(
            FeedConfig::try_new(
                b"BTC",
                pair(),
                DECIMALS,
                NO_MAX_AGE,
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
            FeedConfig::try_new(
                b"BTC",
                pair(),
                DECIMALS,
                NO_MAX_AGE,
                &[signer(1), signer(0)],
                1
            )
            .unwrap_err(),
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
                NO_MAX_AGE,
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
            FeedConfig::try_new(&long, pair(), DECIMALS, NO_MAX_AGE, &[signer(1)], 1).unwrap_err(),
            ConfigError::FeedIdTooLong {
                len: FEED_ID_BYTES + 1
            }
        );
    }

    #[test]
    fn an_empty_or_zero_feed_id_is_rejected() {
        // It would match the padding of any feed id in any payload.
        assert_eq!(
            FeedConfig::try_new(b"", pair(), DECIMALS, NO_MAX_AGE, &[signer(1)], 1).unwrap_err(),
            ConfigError::ZeroFeedId
        );
        assert_eq!(
            FeedConfig::try_new(&[0u8; 4], pair(), DECIMALS, NO_MAX_AGE, &[signer(1)], 1)
                .unwrap_err(),
            ConfigError::ZeroFeedId
        );
    }

    #[test]
    fn a_threshold_equal_to_the_signer_count_is_allowed() {
        assert!(FeedConfig::try_new(
            b"BTC",
            pair(),
            DECIMALS,
            NO_MAX_AGE,
            &[signer(1), signer(2)],
            2
        )
        .is_ok());
    }

    #[test]
    fn an_exponent_the_conversion_cannot_divide_by_is_a_configuration_error() {
        let signers = [signer(1)];
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), MAX_DECIMALS + 1, NO_MAX_AGE, &signers, 1)
                .unwrap_err(),
            ConfigError::DecimalsOutOfRange {
                decimals: MAX_DECIMALS + 1,
                max: MAX_DECIMALS,
            }
        );
        assert!(FeedConfig::try_new(b"BTC", pair(), MAX_DECIMALS, NO_MAX_AGE, &signers, 1).is_ok());
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
            NO_MAX_AGE,
            &signers,
            1
        )
        .is_ok());
        assert!(FeedConfig::try_new(
            b"BTC",
            AssetPair::new(same, same),
            DECIMALS,
            NO_MAX_AGE,
            &signers,
            1
        )
        .is_ok());
    }

    use crate::{
        backend::InProgramBackend,
        decode::Payload,
        test_support::{address_of, signing_key, PayloadBuilder},
        time::TimeError,
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
            verify_feed(
                &payload,
                &config,
                &elsewhere,
                &InProgramBackend::new(),
                &FixedClock(Ok(NOW_MS))
            ),
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
            verify_feed(
                &payload,
                &config,
                &other_quote,
                &InProgramBackend::new(),
                &FixedClock(Ok(NOW_MS))
            ),
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

    #[test]
    fn a_signer_that_reported_is_not_also_counted_among_the_spoiled() {
        // One signer sending a good package and a useless one must not close
        // the gap its silent neighbour left. Both orderings, because packages
        // are walked last-first and the answer must not depend on which of the
        // two the walk reaches first.
        let keys = keys(2);
        let set = signer_set(&keys);
        let good = (b"BTC".as_slice(), [0, 0, 0, 10].as_slice());
        let useless = (b"BTC".as_slice(), [0, 0, 0, 0].as_slice());

        for (first, second) in [(good, useless), (useless, good)] {
            let bytes = PayloadBuilder::default()
                .signed_package(&keys[0], &[first], 1)
                .signed_package(&keys[0], &[second], 2)
                .build();

            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 2);

            assert_eq!(
                verify(&payload, &config),
                Err(VerifyError::ThresholdNotMet {
                    met: 1,
                    required: 2
                }),
                "one of two signers signed, whichever package came first"
            );
        }
    }

    #[test]
    fn a_threshold_of_one_is_met_by_one_signer() {
        // The lower boundary. Nothing about the walk should special-case it, but
        // an off-by-one in the comparison would show up here first.
        let keys = keys(1);
        let set = signer_set(&keys);
        let bytes = agreed(&keys, &[0, 0, 0, 42]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 1);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.signers, 1);
        assert_eq!(verified.value, Value::from_be_slice(&[42]).expect("fits"));
    }

    #[test]
    fn a_threshold_equal_to_the_signer_count_needs_every_one_of_them() {
        // The upper boundary: unanimity, where one silent signer is the whole
        // difference between a price and a refusal.
        let keys = keys(5);
        let set = signer_set(&keys);

        // Named, because `Payload<'a>` borrows the bytes: an inline call is a
        // temporary that does not outlive the payload built from it.
        let complete = agreed(&keys, &[0, 0, 0, 77]);
        let all = Payload::decode(&complete).expect("well formed");
        let config = feed_config(&set, 5);
        assert_eq!(verify(&all, &config).expect("verifies").signers, 5);

        let short = agreed(&keys[..4], &[0, 0, 0, 77]);
        let short = Payload::decode(&short).expect("well formed");
        assert_eq!(
            verify(&short, &config),
            Err(VerifyError::ThresholdNotMet {
                met: 4,
                required: 5
            })
        );
    }

    #[test]
    fn more_unknown_signers_than_the_buffer_holds_neither_panics_nor_overruns() {
        // The buffer is sized for the configured signers, and the unknown ones
        // are whoever else happens to be in a payload — a number no consumer
        // controls. A panic here would abort the transaction rather than refuse
        // the price.
        let keys = keys(40);
        let set = signer_set(&keys[..3]);

        let mut builder = PayloadBuilder::default();
        for key in &keys[3..] {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 50])], 1);
        }
        let bytes = builder.build();
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert!(
            keys.len() - 3 > MAX_SIGNERS,
            "the test is only meaningful if the buffer actually fills"
        );
        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner),
            "thirty-seven strangers and none of the three configured signers"
        );
    }

    /// A payload three configured signers agree on, dated `at`.
    fn agreed_at(keys: &[k256::ecdsa::SigningKey], at: u64) -> std::vec::Vec<u8> {
        let mut builder = PayloadBuilder::default();
        for key in keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], at);
        }
        builder.build()
    }

    fn timed_config(signers: &[SignerAddress], threshold: u8, max_age_ms: u64) -> FeedConfig<'_> {
        FeedConfig::try_new(b"BTC", pair(), DECIMALS, max_age_ms, signers, threshold)
            .expect("valid config")
    }

    fn verify_at(
        payload: &Payload<'_>,
        config: &FeedConfig<'_>,
        now_ms: u64,
    ) -> Result<VerifiedFeed, VerifyError> {
        verify_feed(
            payload,
            config,
            &pair(),
            &InProgramBackend::new(),
            &FixedClock(Ok(now_ms)),
        )
    }

    const ONE_MINUTE: u64 = 60 * 1000;

    #[test]
    fn a_package_inside_the_window_verifies_and_one_past_it_does_not() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at(&keys, NOW_MS - ONE_MINUTE);
        let payload = Payload::decode(&bytes).expect("well formed");

        let generous = timed_config(&set, 3, 5 * ONE_MINUTE);
        assert_eq!(
            verify_at(&payload, &generous, NOW_MS)
                .expect("fresh enough")
                .signers,
            3
        );

        let strict = timed_config(&set, 3, 30 * 1000);
        assert_eq!(
            verify_at(&payload, &strict, NOW_MS),
            Err(VerifyError::StalePackage),
            "a minute old against a thirty-second maxAge"
        );
    }

    #[test]
    fn the_staleness_boundary_admits_a_package_exactly_max_age_old() {
        // The window is inclusive at the far edge, matching RedStone's
        // `is_same_or_after`. An exclusive bound here would reject a package
        // RedStone accepts, for one millisecond of difference.
        let keys = keys(3);
        let set = signer_set(&keys);
        let config = timed_config(&set, 3, ONE_MINUTE);

        let exactly = agreed_at(&keys, NOW_MS - ONE_MINUTE);
        let exactly = Payload::decode(&exactly).expect("well formed");
        assert!(
            verify_at(&exactly, &config, NOW_MS).is_ok(),
            "exactly maxAge old"
        );

        let older = agreed_at(&keys, NOW_MS - ONE_MINUTE - 1);
        let older = Payload::decode(&older).expect("well formed");
        assert_eq!(
            verify_at(&older, &config, NOW_MS),
            Err(VerifyError::StalePackage),
            "one millisecond older"
        );
    }

    #[test]
    fn a_package_dated_beyond_clock_skew_is_a_different_failure_from_a_stale_one() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let config = timed_config(&set, 3, ONE_MINUTE);

        let allowed = agreed_at(&keys, NOW_MS + MAX_AHEAD_MS);
        let allowed = Payload::decode(&allowed).expect("well formed");
        assert!(
            verify_at(&allowed, &config, NOW_MS).is_ok(),
            "skew up to the tolerance is not an error"
        );

        let beyond = agreed_at(&keys, NOW_MS + MAX_AHEAD_MS + 1);
        let beyond = Payload::decode(&beyond).expect("well formed");
        assert_eq!(
            verify_at(&beyond, &config, NOW_MS),
            Err(VerifyError::FuturePackage),
            "not StalePackage: the relayer is not the machine to look at"
        );
    }

    #[test]
    fn a_replayed_payload_is_refused_however_well_signed_it_is() {
        // Every signature still verifies and every signer is authorised. Age is
        // the only thing wrong with it, which is the whole point of the check.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at(&keys, NOW_MS);
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, 5 * ONE_MINUTE);

        assert!(
            verify_at(&payload, &config, NOW_MS).is_ok(),
            "valid when fresh"
        );
        assert_eq!(
            verify_at(&payload, &config, NOW_MS + 60 * ONE_MINUTE),
            Err(VerifyError::StalePackage),
            "the same bytes an hour later"
        );
    }

    #[test]
    fn one_stale_package_costs_only_its_own_signer() {
        // The rule that has held for every other bad package: a threshold
        // exists to survive one signer, so one stale report must not deny the
        // feed to the consumers the payload also serves.
        let keys = keys(4);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys[..3] {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], NOW_MS);
        }
        let bytes = builder
            .signed_package(
                &keys[3],
                &[(b"BTC", &[0, 0, 0, 100])],
                NOW_MS - 60 * ONE_MINUTE,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, NOW_MS)
                .expect("three fresh signers agree")
                .signers,
            3
        );
    }

    #[test]
    fn strangers_sending_stale_packages_cannot_rename_a_threshold_failure() {
        // The reason the timestamp is checked after recovery. If it were checked
        // first, these packages would never be attributed to anyone and three of
        // them would turn this into StalePackage -- telling an operator to
        // refetch when the signer set is what does not match.
        let keys = keys(6);
        let set = signer_set(&keys[..3]);
        let mut builder = PayloadBuilder::default();
        for key in &keys[3..] {
            builder =
                builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], NOW_MS - 60 * ONE_MINUTE);
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, NOW_MS),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            }),
            "none of the configured signers signed, and no stranger's age changes that"
        );
    }

    #[test]
    fn a_clock_that_cannot_be_read_refuses_rather_than_guesses() {
        // Verifying without a clock would accept a package of any age, which is
        // the replay this check exists to stop.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at(&keys, NOW_MS);
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        for reason in [
            TimeError::Missing,
            TimeError::WrongAccount,
            TimeError::Undecodable,
            TimeError::Unavailable,
        ] {
            assert_eq!(
                verify_feed(
                    &payload,
                    &config,
                    &pair(),
                    &InProgramBackend::new(),
                    &FixedClock(Err(reason))
                ),
                Err(VerifyError::NoClock(reason))
            );
        }
    }

    #[test]
    fn a_max_age_of_zero_is_rejected_at_configuration_time() {
        // No package can be young enough, so the feed would never produce a
        // price. Caught where the caller can fix it.
        let signers = [signer(1)];
        assert_eq!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, 0, &signers, 1).unwrap_err(),
            ConfigError::MaxAgeZero
        );
    }

    #[test]
    fn signers_blocked_for_different_reasons_still_count_as_present() {
        // Two configured signers reported: one package too old, one carrying a
        // zero. Neither cause reaches a threshold of two on its own, and
        // checking them one at a time would answer "met: 0" -- sending an
        // operator to look for signers that are both already in the payload.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(
                &keys[0],
                &[(b"BTC", &[0, 0, 0, 10])],
                NOW_MS - 60 * ONE_MINUTE,
            )
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 0])], NOW_MS)
            .build();
        let payload = Payload::decode(&bytes).expect("well formed");

        let config = timed_config(&set, 2, ONE_MINUTE);
        assert_eq!(
            verify_at(&payload, &config, NOW_MS),
            Err(VerifyError::StalePackage),
            "both signers are present; age is named because it resolves on its own"
        );

        // At a threshold of three the two of them could not have been enough
        // even if nothing were wrong with either, so the count is the honest
        // answer after all.
        let strict = timed_config(&set, 3, ONE_MINUTE);
        assert_eq!(
            verify_at(&payload, &strict, NOW_MS),
            Err(VerifyError::ThresholdNotMet {
                met: 0,
                required: 3
            })
        );
    }

    #[test]
    fn the_largest_cause_is_named_when_several_block_the_threshold() {
        // Three signers report zeros and one is stale, against a threshold of
        // four. Age wins ties but does not win outright: naming staleness here
        // would have an operator refetch a payload whose real problem is that
        // three of its signers sent nothing usable.
        let keys = keys(4);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys[..3] {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 0])], NOW_MS);
        }
        let bytes = builder
            .signed_package(
                &keys[3],
                &[(b"BTC", &[0, 0, 0, 10])],
                NOW_MS - 60 * ONE_MINUTE,
            )
            .build();
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 4, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, NOW_MS),
            Err(VerifyError::ValueOutOfRange)
        );
    }

    #[test]
    fn a_clock_below_the_max_age_does_not_underflow_into_rejecting_everything() {
        // A fresh devnet has a clock smaller than any sensible maxAge, and
        // `now - max_age` would wrap to nearly u64::MAX -- rejecting every
        // package ever signed. Saturating, so it does not.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at(&keys, 5);
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(verify_at(&payload, &config, 10).expect("fresh").signers, 3);
    }

    #[test]
    fn a_clock_near_the_top_of_its_range_neither_overflows_nor_panics() {
        // The mirror of the underflow case. `now + MAX_AHEAD` would wrap to a
        // small number and let every future-dated package through, so it
        // saturates.
        //
        // The wire timestamp is six bytes, so no package can be dated past
        // 2^48 - 1 ms and the wrap is unreachable from a payload — every
        // encodable timestamp is ancient against a clock this size. The
        // assertion is therefore that it answers rather than panics, and
        // answers the true thing.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at(&keys, (1u64 << 48) - 1);
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, u64::MAX),
            Err(VerifyError::StalePackage)
        );
        assert_eq!(
            verify_at(&payload, &config, u64::MAX - 1),
            Err(VerifyError::StalePackage)
        );
    }
    #[test]
    fn the_timestamp_is_the_oldest_package_behind_the_median() {
        // The median is only as current as the stalest report that shaped it,
        // so the newest package must not speak for the ones beside it.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 9_000)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 5_000)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], 7_000)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.timestamp_ms, 5_000);
    }

    #[test]
    fn a_package_that_did_not_count_does_not_age_the_timestamp() {
        // A skipped value costs its own signer's slot and nothing else. Letting
        // its timestamp through would publish a price as older than any report
        // behind it, on the strength of a report that is not behind it.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], 5_000)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], 6_000)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], 1_000)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 2);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.signers, 2);
        assert_eq!(verified.timestamp_ms, 5_000);
    }
}
