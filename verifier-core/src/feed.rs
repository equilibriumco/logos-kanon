//! One feed's configuration, and the M-of-N rule over it.
//!
//! The configuration is the *caller's*, never the payload's. In pull mode the
//! signer set comes from the consumer and never from the data, so nothing here
//! reads a signer, a threshold or a feed id out of a payload.

use crate::{
    backend::{SignerAddress, VerifierBackend},
    decode::{DataPackage, Payload, FEED_ID_BYTES},
    error::{ConfigError, VerifyError},
    time::TimeSource,
    value::{median, Value, MAX_DECIMALS},
};

/// How far ahead of the chain's clock a package may be dated.
///
/// Allowance for skew between a RedStone signer's clock and the sequencer's,
/// not a policy knob: no feed operator knows that skew better than this crate
/// does. RedStone's own `MAX_TIMESTAMP_AHEAD_MS`, so Kanon is never stricter at
/// this end than the upstream default.
pub const MAX_AHEAD_MS: u64 = 3 * 60 * 1000;

/// The oldest a package may be allowed to be, whatever a feed configures.
///
/// RedStone's own `MAX_TIMESTAMP_DELAY_MS`, which is the default their validator
/// applies rather than a bound every integration of theirs is held to. Kanon
/// adopts it as policy: a feed is free to be stricter and most should be, and no
/// feed may be looser than the upstream default. The ceiling is enforced rather
/// than recommended because the failure is silent: `freshness` saturates, so a
/// `maxAge` near `u64::MAX` puts the lower edge of the window at zero and admits
/// every past timestamp while still looking configured.
pub const MAX_MAX_AGE_MS: u64 = 15 * 60 * 1000;

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
/// `verify_feed` walks with — `reported` (`[Option<Value>; 32]`, 1,056 bytes)
/// and `collected` (`[Value; 32]`, 1,024 bytes), about 2 KB in one frame — which
/// is how the threshold runs
/// without an allocator. `collected` exists only because `median` takes
/// `&mut [Value]` while `reported` holds `Option<Value>`; sorting `reported` in
/// place would remove it and save a kilobyte of guest stack, and is deliberately
/// left for later rather than folded in here. RedStone caps at 255 and live
/// feeds run ten to twenty, so this leaves headroom. Raising it costs stack and
/// nothing else.
pub const MAX_SIGNERS: usize = 32;

/// How many packages carrying the requested feed will be recovered before the
/// payload is refused.
///
/// Separate from [`MAX_SIGNERS`] because it is bounded by a different thing.
/// `MAX_SIGNERS` costs guest stack; this costs the transaction. A recovery is
/// about 585,000 cycles and LEZ allows 33,554,432 in a public transaction
/// (`COSTS.md`), so without a ceiling the payload decides how much of the budget
/// verification spends -- and the package count sits in the envelope, outside
/// every signature, where anyone handling the payload can raise it.
///
/// At 32 the ceiling measures 59% of the budget, which
/// `the_most_a_payload_can_cost_still_fits_in_one_transaction` asserts, and no
/// honest payload approaches it: one package per signer per feed, on a service
/// running ten to twenty.
pub const MAX_RECOVERIES: usize = 32;

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
    /// feed id that is empty, all-zero, or wider than the wire field, a decimal
    /// exponent above [`MAX_DECIMALS`], or a `maxAge` of zero or above
    /// [`MAX_MAX_AGE_MS`].
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
        if max_age_ms > MAX_MAX_AGE_MS {
            return Err(ConfigError::MaxAgeTooLarge {
                max_age_ms,
                max: MAX_MAX_AGE_MS,
            });
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

/// Whether this package says anything at all about the requested feed.
///
/// The one test cheap enough to run before recovery, and the one that decides
/// whether recovery is worth about 585,000 cycles. A package carrying no data
/// point for this feed cannot fill a slot or affect this feed's package checks,
/// so nothing about it changes the answer.
fn carries_feed(package: &DataPackage<'_>, config: &FeedConfig<'_>) -> bool {
    package
        .data_points()
        .any(|point| point.feed_id == config.feed_id())
}

/// Verifies one feed out of a payload.
///
/// Walks the payload once, recovering one signer per package, and counts the
/// distinct authorised signers that reported a usable value for
/// `config.feed_id()`. Returns the median across them, on both RedStone's scale
/// and the price account's, once the threshold is met.
///
/// Packages that do not carry the requested feed are irrelevant and skipped
/// before recovery. Every package that does carry it is verified strictly: a
/// moment other than the payload's, an invalid signature, an unauthorised
/// signer, an out-of-window timestamp, or an unusable value rejects the payload.
/// This is the contract promised by F4, U6, and SEC1.
///
/// `expected` is the asset pair the caller believes this feed prices, compared
/// against the pair the feed was registered with. It is a parameter rather than
/// a separate method so that a caller cannot reach a price without stating what
/// it thought it was asking for.
///
/// # Errors
///
/// [`VerifyError`] for a malformed payload, packages describing more than one
/// moment, more packages for this feed than verification will pay to recover, an
/// invalid signature, an unauthorised signer, a package outside the timestamp
/// window, an asset pair that is not the caller's, a threshold that was not
/// reached, an unusable value, or a price the account's scale cannot hold. A
/// second package from a signer whose slot is filled remains non-counting rather
/// than fatal; ADR 24 records why.
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

    // One slot per configured signer, filled as the walk reaches it. A signer
    // that reports twice fills one slot: the first value the walk sees holds it,
    // and a second package needs the same key to exist at all, so no ordering
    // rule could make one of two equally attested values the right one (ADR 24).
    let mut reported: [Option<Value>; MAX_SIGNERS] = [None; MAX_SIGNERS];
    // Packages recovered so far. Payloads are multi-feed and multi-consumer, so
    // most of what arrives is somebody else's; this counts only what was worth
    // paying for.
    let mut recovered = 0usize;
    // The one observation this payload describes for the requested feed, taken
    // from the first package carrying it and required of every one after.
    let mut payload_ms: Option<u64> = None;

    let walked = payload.for_each_package(|package| {
        // Before the hash, and the only thing that is. Everything else the walk
        // decides needs the signer, and the signer costs a recovery.
        if !carries_feed(&package, config) {
            return Ok(());
        }

        // A price is a statement about one moment, and a payload carrying two
        // moments for this feed is one whose author picked which moment answers.
        // Scoped to the requested feed, because SEC1 and Reliability 3 are both
        // written per feed: another feed's trouble must not reach this one. A
        // field comparison, so a mismatch is refused without paying a recovery.
        match payload_ms {
            None => payload_ms = Some(package.timestamp_ms),
            Some(expected) if expected != package.timestamp_ms => {
                return Err(VerifyError::TimestampMismatch {
                    expected,
                    found: package.timestamp_ms,
                })
            }
            Some(_) => {}
        }

        recovered += 1;
        if recovered > MAX_RECOVERIES {
            return Err(VerifyError::TooManyPackages {
                max: MAX_RECOVERIES,
            });
        }

        let digest = backend.keccak256(package.signable());
        let signer = backend
            .recover_signer(&digest, &package.signature)
            .map_err(VerifyError::InvalidSignature)?;

        let Some(index) = config.signers().iter().position(|s| *s == signer) else {
            return Err(VerifyError::UnauthorisedSigner);
        };

        match freshness(package.timestamp_ms, now_ms, config.max_age_ms()) {
            Freshness::Stale => return Err(VerifyError::StalePackage),
            Freshness::Future => return Err(VerifyError::FuturePackage),
            Freshness::Current => {}
        }

        let mut package_value = None;
        for point in package.data_points() {
            if point.feed_id != config.feed_id() {
                continue;
            }
            let value = Value::from_be_slice(point.value)
                .filter(usable)
                .ok_or(VerifyError::ValueOutOfRange)?;
            if package_value.is_none() {
                package_value = Some(value);
            }
        }

        // One report per package, whatever the package repeats. Every repeated
        // point was still validated above; only the first valid value is filed.
        if let Some(value) = package_value {
            if let Some(slot) = reported.get_mut(index) {
                if slot.is_none() {
                    *slot = Some(value);
                }
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
        // Every counted package carried it; the walk refused the payload otherwise.
        timestamp_ms: payload_ms.unwrap_or_default(),
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

    /// The widest bound a feed may configure, so a test that is not about
    /// timestamps is never refused for one. Staleness has its own tests.
    const NO_MAX_AGE: u64 = MAX_MAX_AGE_MS;

    /// Three ordered moments for the tests about which round a package belongs
    /// to. Relative to `NOW_MS`, because a bounded `maxAge` means a round label
    /// has to be a time the test clock would call current.
    const ROUND_A: u64 = NOW_MS - 4_000;
    const ROUND_B: u64 = NOW_MS - 2_000;
    const ROUND_C: u64 = NOW_MS;

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
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], NOW_MS);
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
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
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
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 40])], NOW_MS)
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
                NOW_MS,
            );
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.value, Value::from_be_slice(&[50]).expect("fits"));
    }

    #[test]
    fn an_unknown_signer_rejects_the_payload_even_when_quorum_is_present() {
        // SEC1 applies to every package for the requested feed, independently
        // of whether the authorised packages already meet the threshold.
        let keys = keys(3);
        let set = signer_set(&keys);
        let stranger = signing_key(0x99);

        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 60])], NOW_MS);
        }
        let bytes = builder
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0x27, 0x0F])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner)
        );
    }

    #[test]
    fn one_signer_cannot_reach_the_threshold_alone_by_repeating_the_feed() {
        // The anti-inflation rule. Without it a single signer occupies as many
        // slots as it sends packages. Reported as the threshold failure it is:
        // one signer of three, whatever it sent.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
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
    fn a_copy_of_a_package_already_in_the_payload_cannot_deny_the_feed() {
        // The denial of service the skip exists to close. Appending a package
        // takes no key and no valid signature -- copy one that is already
        // there -- so failing the payload over it would be the cheapest way to
        // deny a verified price to everyone the payload serves.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 60])], NOW_MS);
        }
        // Byte for byte what the first package already is.
        let bytes = builder
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 60])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.signers, 3, "the copy fills no second slot");
    }

    #[test]
    fn an_older_package_from_a_signer_that_already_reported_refuses_the_payload() {
        // The same attack with a package that is not a copy: every package
        // inside `maxAge` is public and validly signed, so whoever adds bytes to
        // a payload has around eighteen of each signer's to choose from. Any one
        // of them describes another moment, so the payload stops describing one
        // and is refused. That is a denial available without a key, which is why
        // a submitter has to own the bytes it submits (ADR 27).
        //
        // Run at both ends of the package list, because `for_each_package`
        // walks from the tail: an attacker chooses where the bytes go, so the
        // answer has to be the same wherever they put them.
        let keys = keys(3);
        let set = signer_set(&keys);
        let round = |replay_first: bool| {
            let mut builder = PayloadBuilder::default();
            if replay_first {
                builder =
                    builder.signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 99])], NOW_MS - 60_000);
            }
            builder = builder
                .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
                .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
                .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], NOW_MS);
            if !replay_first {
                builder =
                    builder.signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 99])], NOW_MS - 60_000);
            }
            builder.build()
        };

        for replay_first in [false, true] {
            let bytes = round(replay_first);
            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 3);

            assert!(
                matches!(
                    verify(&payload, &config),
                    Err(VerifyError::TimestampMismatch { .. })
                ),
                "replay_first = {replay_first}"
            );
        }
    }

    #[test]
    fn a_zero_value_is_rejected_before_threshold_is_evaluated() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        // The package error is reported directly, even though the other two
        // signers would leave the payload short of its threshold.
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
            builder = builder.signed_package(key, &[(b"ETH", &[0, 0, 0, 5])], NOW_MS);
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
    fn an_unauthorised_signer_is_reported_directly() {
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&configured[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&configured[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner)
        );
    }

    #[test]
    fn one_unknown_signer_is_rejected_before_threshold_is_evaluated() {
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner)
        );
    }

    #[test]
    fn repeated_packages_from_one_unknown_signer_are_still_unauthorised() {
        let configured = keys(3);
        let set = signer_set(&configured);
        let stranger = signing_key(0x99);

        let bytes = PayloadBuilder::default()
            .signed_package(&configured[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 31])], NOW_MS + 1)
            .signed_package(&stranger, &[(b"BTC", &[0, 0, 0, 32])], NOW_MS + 2)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner)
        );
    }

    #[test]
    fn a_malformed_payload_is_reported_as_malformed_not_as_a_threshold_failure() {
        // The distinction decode.rs exists to preserve, carried through the
        // threshold: "not well formed" and "not authorised" stay separate.
        let keys = keys(1);
        let set = signer_set(&keys);
        let valid = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
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
    fn one_unrepresentable_value_rejects_even_when_quorum_is_present() {
        let keys = keys(4);
        let set = signer_set(&keys);
        let wide = [0x01u8; 33];
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &wide)], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }

    #[test]
    fn an_unrepresentable_value_is_reported_before_threshold_failure() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let wide = [0x01u8; 33];
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &wide)], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }

    #[test]
    fn the_threshold_boundary_accepts_at_m_and_rejects_at_m_minus_one() {
        let keys = keys(5);
        let set = signer_set(&keys);

        for reporting in 1..=5usize {
            let mut builder = PayloadBuilder::default();
            for key in keys.iter().take(reporting) {
                builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 10])], NOW_MS);
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
    fn a_garbage_signature_rejects_even_when_quorum_is_present() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], NOW_MS);
        }
        let bytes = builder
            .opaque_package(&[(b"BTC", &[0, 0, 0, 100])], 1, 0xFF)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::InvalidSignature(
                crate::backend::BackendError::InvalidRecoveryId
            ))
        );
    }

    #[test]
    fn strangers_reporting_only_another_feed_do_not_report_unauthorised_signer() {
        // Strict package checks are scoped to the requested feed. Packages that
        // only carry ETH are irrelevant to a BTC verification and are skipped
        // before recovery, whatever signer produced them.
        let stranger_keys = [signing_key(0x99), signing_key(0x98), signing_key(0x97)];
        let configured = keys(3);
        let set = signer_set(&configured);

        let mut builder = PayloadBuilder::default();
        for key in &stranger_keys {
            builder = builder.signed_package(key, &[(b"ETH", &[0, 0, 0, 7])], NOW_MS);
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
    fn a_single_package_carrying_the_requested_feed_twice_still_counts_once() {
        // The same rule reached from the inner point loop rather than across
        // packages: two data points for one feed in one signature.
        let keys = keys(2);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(
                &keys[0],
                &[(b"BTC", &[0, 0, 0, 10]), (b"BTC", &[0, 0, 0, 20])],
                NOW_MS,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 2);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::ThresholdNotMet {
                met: 1,
                required: 2
            }),
            "the second point does not fill a second slot"
        );
    }

    #[test]
    fn every_repeated_point_for_the_requested_feed_is_validated() {
        // Filing one report per signature must not mean stopping validation at
        // the first usable point. Otherwise a valid point could hide a zero
        // later in the same signed package.
        let keys = keys(1);
        let set = signer_set(&keys);
        let valid = (b"BTC".as_slice(), [0, 0, 0, 10].as_slice());
        let invalid = (b"BTC".as_slice(), [0, 0, 0, 0].as_slice());

        for points in [[valid, invalid], [invalid, valid]] {
            let bytes = PayloadBuilder::default()
                .signed_package(&keys[0], &points, NOW_MS)
                .build();
            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 1);

            assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
        }
    }

    /// A payload three configured signers agree on, for the tests that only
    /// vary one thing about the caller or the values.
    fn agreed(keys: &[k256::ecdsa::SigningKey], value: &[u8]) -> std::vec::Vec<u8> {
        let mut builder = PayloadBuilder::default();
        for key in keys {
            builder = builder.signed_package(key, &[(b"BTC", value)], NOW_MS);
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
    fn a_negative_value_rejects_even_when_quorum_is_present() {
        // The top half of the range is interpreted as the negative half of an
        // int256-like price field and rejected directly under F4.
        let keys = keys(4);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys[..3] {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 100])], NOW_MS);
        }
        let bytes = builder
            .signed_package(&keys[3], &[(b"BTC", &[0xFF; 32])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
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
    fn one_bad_value_is_reported_before_threshold_is_evaluated() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 0])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }

    #[test]
    fn the_first_bad_package_in_the_wire_walk_is_reported() {
        // Package iteration is tail-first. Both errors are strict; when a
        // payload contains more than one, the first one encountered is returned.
        let keys = keys(4);
        let set = signer_set(&keys[..3]);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], NOW_MS)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], NOW_MS)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 30])], NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::UnauthorisedSigner)
        );
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
    fn one_bad_package_from_a_signer_rejects_in_either_order() {
        let keys = keys(2);
        let set = signer_set(&keys);
        let good = (b"BTC".as_slice(), [0, 0, 0, 10].as_slice());
        let useless = (b"BTC".as_slice(), [0, 0, 0, 0].as_slice());

        for (first, second) in [(good, useless), (useless, good)] {
            let bytes = PayloadBuilder::default()
                .signed_package(&keys[0], &[first], NOW_MS)
                .signed_package(&keys[0], &[second], NOW_MS)
                .build();

            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 2);

            assert_eq!(
                verify(&payload, &config),
                Err(VerifyError::ValueOutOfRange),
                "the bad package is rejected whichever package is walked first"
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
    fn one_package_past_the_ceiling_is_refused_rather_than_answered() {
        // The ceiling is what stops a payload deciding how much of the
        // transaction's cycle budget verification spends. Refused, not
        // truncated: answering from the packages that fit would let whoever
        // assembled the payload choose which of a signer's packages counts.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for i in 0..=MAX_RECOVERIES {
            builder = builder.signed_package(&keys[i % 3], &[(b"BTC", &[0, 0, 0, 50])], NOW_MS);
        }
        let bytes = builder.build();
        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::TooManyPackages {
                max: MAX_RECOVERIES
            }),
            "three signers' worth of price, presented {} times",
            MAX_RECOVERIES + 1
        );
    }

    #[test]
    fn packages_for_other_feeds_are_free_and_do_not_count_against_the_ceiling() {
        // What makes the ceiling affordable rather than a limit on payload size.
        // A RedStone payload is multi-feed by design, so most of what arrives is
        // somebody else's; none of it is recovered, and none of it uses up the
        // budget this feed is allowed.
        //
        // Run at both ends of the package list. `for_each_package` walks from
        // the tail, so putting the other feed's packages last would let a
        // ceiling that counted them still pass -- the three that matter would
        // already have been read.
        let keys = keys(3);
        let set = signer_set(&keys);
        let others = |builder: PayloadBuilder| {
            (0..MAX_RECOVERIES * 2).fold(builder, |b, i| {
                b.signed_package(&keys[i % 3], &[(b"ETH", &[0, 0, 0, 7])], NOW_MS)
            })
        };
        let ours = |builder: PayloadBuilder| {
            keys.iter().fold(builder, |b, key| {
                b.signed_package(key, &[(b"BTC", &[0, 0, 0, 50])], NOW_MS)
            })
        };

        for ours_first in [false, true] {
            let builder = PayloadBuilder::default();
            let bytes = if ours_first {
                others(ours(builder)).build()
            } else {
                ours(others(builder)).build()
            };
            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 3);

            assert_eq!(
                verify(&payload, &config).expect("verifies").signers,
                3,
                "sixty-four packages for another feed change nothing \
                 (ours_first = {ours_first})"
            );
        }
    }

    /// One round: every signer dated `at`, each with its own value.
    fn agreed_at_value(
        keys: &[k256::ecdsa::SigningKey],
        at: u64,
        values: &[u8],
    ) -> std::vec::Vec<u8> {
        let mut builder = PayloadBuilder::default();
        for (key, value) in keys.iter().zip(values) {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, *value])], at);
        }
        builder.build()
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
    fn a_signers_stale_package_about_another_feed_does_not_make_this_one_stale() {
        // Signers on `redstone-primary-prod` subscribe to different feed
        // subsets (ADR 19), so a configured signer whose package is stale and
        // about ETH is ordinary. It reported nothing for BTC, so BTC's slot
        // stayed empty for want of a signer, not for want of a fresh one --
        // and `StalePackage` would send an operator to look at a relayer that
        // is doing its job.
        let keys = keys(3);
        let set = signer_set(&keys);
        let now = NOW_MS;

        for (label, at) in [
            ("stale", now - 10 * ONE_MINUTE),
            ("future dated", now + 10 * ONE_MINUTE),
        ] {
            let bytes = PayloadBuilder::default()
                .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 100])], now)
                .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 100])], now)
                .signed_package(&keys[2], &[(b"ETH", &[0, 0, 0, 100])], at)
                .build();

            let payload = Payload::decode(&bytes).expect("well formed");
            let config = timed_config(&set, 3, ONE_MINUTE);

            assert_eq!(
                verify_at(&payload, &config, now),
                Err(VerifyError::ThresholdNotMet {
                    met: 2,
                    required: 3
                }),
                "{label}"
            );
        }
    }

    #[test]
    fn a_signers_stale_package_about_this_feed_still_makes_it_stale() {
        // The other half of the gate, and the one that has to keep working: the
        // signer did report this feed with a value the threshold could have
        // used, so age is the only thing in the way and age is what resolves on
        // its own. Paired with the test above so a gate that suppressed too
        // much would fail here.
        let keys = keys(3);
        let set = signer_set(&keys);
        let now = NOW_MS;
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 100])], now)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 100])], now)
            .signed_package(
                &keys[2],
                &[(b"BTC", &[0, 0, 0, 100])],
                now - 10 * ONE_MINUTE,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, now),
            Err(VerifyError::StalePackage),
            "two of three reported and the third is only late"
        );
    }

    #[test]
    fn package_age_is_checked_before_its_value() {
        // Both conditions reject. Age is checked first so the result is stable
        // and no untrusted value is parsed from an already-stale package.
        let keys = keys(3);
        let set = signer_set(&keys);
        let now = NOW_MS;
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 100])], now)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 100])], now)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], now - 10 * ONE_MINUTE)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, now),
            Err(VerifyError::StalePackage)
        );
    }

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
    fn one_stale_package_rejects_even_when_quorum_is_present() {
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
            verify_at(&payload, &config, NOW_MS),
            Err(VerifyError::StalePackage)
        );
    }

    #[test]
    fn one_future_package_rejects_even_when_quorum_is_present() {
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
                NOW_MS + MAX_AHEAD_MS + 1,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = timed_config(&set, 3, ONE_MINUTE);

        assert_eq!(
            verify_at(&payload, &config, NOW_MS),
            Err(VerifyError::FuturePackage)
        );
    }

    #[test]
    fn signer_authority_is_checked_before_package_age() {
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
            Err(VerifyError::UnauthorisedSigner)
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
    fn a_max_age_wider_than_redstones_own_bound_is_rejected() {
        // The reason the bound is enforced and not merely recommended: at the
        // top of the range `freshness` saturates, so the window's lower edge
        // sits at zero and every past timestamp is current. A feed configured
        // that way looks configured and checks nothing.
        let signers = [signer(1)];
        let boundless =
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, u64::MAX, &signers, 1).unwrap_err();
        assert_eq!(
            boundless,
            ConfigError::MaxAgeTooLarge {
                max_age_ms: u64::MAX,
                max: MAX_MAX_AGE_MS
            }
        );

        assert!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, MAX_MAX_AGE_MS, &signers, 1).is_ok(),
            "the bound itself is a legal configuration"
        );
        assert!(
            FeedConfig::try_new(b"BTC", pair(), DECIMALS, MAX_MAX_AGE_MS + 1, &signers, 1).is_err(),
            "one millisecond past it is not"
        );
    }

    #[test]
    fn the_ceiling_admits_nothing_redstone_would_refuse() {
        // The bound is RedStone's `MAX_TIMESTAMP_DELAY_MS`, so a package Kanon
        // accepts at the widest legal `maxAge` is one RedStone accepts too.
        // Stated as an assertion, because the two constants drifting apart is
        // exactly the change that would go unnoticed.
        assert_eq!(MAX_MAX_AGE_MS, 15 * 60 * 1000);

        let signers = [signer(1)];
        let widest = FeedConfig::try_new(b"BTC", pair(), DECIMALS, MAX_MAX_AGE_MS, &signers, 1)
            .expect("valid config");
        let oldest_admitted = NOW_MS - widest.max_age_ms();

        assert_eq!(
            freshness(oldest_admitted, NOW_MS, widest.max_age_ms()),
            Freshness::Current
        );
        assert_eq!(
            freshness(oldest_admitted - 1, NOW_MS, widest.max_age_ms()),
            Freshness::Stale
        );
    }

    #[test]
    fn the_first_strict_rejection_precedes_threshold_for_any_configuration() {
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
            Err(VerifyError::ValueOutOfRange)
        );

        let strict = timed_config(&set, 3, ONE_MINUTE);
        assert_eq!(
            verify_at(&payload, &strict, NOW_MS),
            Err(VerifyError::ValueOutOfRange)
        );
    }

    #[test]
    fn the_first_strict_rejection_is_not_ranked_against_later_failures() {
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
            Err(VerifyError::StalePackage)
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
    fn the_timestamp_is_the_round_every_counted_package_shares() {
        // There is one timestamp to report because there is one round behind
        // the price. RedStone signs a round's packages under a common
        // timestamp, and the median of a set that does not share one is a
        // number nobody published.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = agreed_at_value(&keys, ROUND_A, &[10, 20, 30]);

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.timestamp_ms, ROUND_A);
        assert_eq!(verified.value, Value::from_be_slice(&[20]).expect("fits"));
    }

    #[test]
    fn values_spliced_from_three_rounds_do_not_become_a_price() {
        // Each of these is validly signed and inside the window, so nothing but
        // their disagreement about when marks them out -- and a median across
        // them is an observation nobody made. The payload is refused rather
        // than answered from part of itself.
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], ROUND_C)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], ROUND_A)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], ROUND_B)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert_eq!(
            verify(&payload, &config),
            Err(VerifyError::TimestampMismatch {
                expected: ROUND_B,
                found: ROUND_A
            }),
            "three signers describing three moments is not one observation"
        );
    }

    #[test]
    fn a_package_from_another_round_refuses_the_payload() {
        // One package describing another moment is enough, in either direction:
        // the payload no longer describes one observation, and which of the two
        // answers is not the caller's to be handed silently.
        let keys = keys(3);
        let set = signer_set(&keys);
        for (label, appended_at) in [("older", ROUND_A), ("newer", ROUND_C)] {
            let mut builder = PayloadBuilder::default();
            for key in &keys {
                builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 60])], ROUND_B);
            }
            let bytes = builder
                .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 99])], appended_at)
                .build();

            let payload = Payload::decode(&bytes).expect("well formed");
            let config = feed_config(&set, 3);

            assert!(
                matches!(
                    verify(&payload, &config),
                    Err(VerifyError::TimestampMismatch { .. })
                ),
                "{label} package from another round should refuse the payload"
            );
        }
    }

    #[test]
    fn one_package_cannot_crowd_the_other_signers_out() {
        // A package files one value however many times it repeats the feed, so
        // packing data points into one signature buys nothing. Worth asserting
        // because the alternative -- one report per point -- costs one recovery
        // and would let a single signature answer for everybody.
        let keys = keys(3);
        let set = signer_set(&keys);
        let crowd: std::vec::Vec<(&[u8], &[u8])> = (0..MAX_RECOVERIES)
            .map(|_| (b"BTC".as_slice(), [0, 0, 0, 99].as_slice()))
            .collect();

        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 50])], NOW_MS)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 50])], NOW_MS)
            .signed_package(&keys[2], &crowd, NOW_MS)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);
        let verified = verify(&payload, &config).expect("verifies");

        assert_eq!(verified.signers, 3, "all three signers still fit");
        assert_eq!(
            verified.value,
            Value::from_be_slice(&[50]).expect("fits"),
            "and the packed package still counts once"
        );
    }

    #[test]
    fn copies_of_a_package_from_another_round_refuse_the_payload() {
        // Copying a package needs no key, so the copies cannot be allowed to
        // decide anything. They do not: the payload describes two moments and
        // is refused, whatever the tally would have been.
        let keys = keys(3);
        let set = signer_set(&keys);
        let mut builder = PayloadBuilder::default();
        for key in &keys {
            builder = builder.signed_package(key, &[(b"BTC", &[0, 0, 0, 60])], ROUND_B);
        }
        for _ in 0..5 {
            builder = builder.signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 99])], ROUND_A);
        }
        let bytes = builder.build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 3);

        assert!(matches!(
            verify(&payload, &config),
            Err(VerifyError::TimestampMismatch { .. })
        ));
    }

    #[test]
    fn a_payload_carrying_two_whole_rounds_is_refused() {
        // What a relayer bundling two fetches produces. Both rounds are
        // complete and either would verify alone, which is exactly why picking
        // one here would be this code choosing the price on the caller's
        // behalf. The relayer submits one round or the other.
        let keys = keys(4);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], ROUND_A)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 10])], ROUND_A)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 30])], ROUND_B)
            .signed_package(&keys[3], &[(b"BTC", &[0, 0, 0, 30])], ROUND_B)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 2);

        assert!(matches!(
            verify(&payload, &config),
            Err(VerifyError::TimestampMismatch { .. })
        ));
    }

    #[test]
    fn a_bad_value_rejects_before_the_threshold_is_evaluated() {
        let keys = keys(3);
        let set = signer_set(&keys);
        let bytes = PayloadBuilder::default()
            .signed_package(&keys[0], &[(b"BTC", &[0, 0, 0, 10])], ROUND_B)
            .signed_package(&keys[1], &[(b"BTC", &[0, 0, 0, 20])], ROUND_B)
            .signed_package(&keys[2], &[(b"BTC", &[0, 0, 0, 0])], ROUND_B)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let config = feed_config(&set, 2);
        assert_eq!(verify(&payload, &config), Err(VerifyError::ValueOutOfRange));
    }
}
