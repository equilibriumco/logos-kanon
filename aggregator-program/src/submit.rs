//! The submission path: verify a payload against a registered feed, publish the
//! price, and hand LEZ the three account states that result.
//!
//! Everything the instruction decides lives here rather than in the guest, so
//! the host can test it. What the guest adds is the account layout and the two
//! constraints only the framework can express — see
//! `methods/guest/src/bin/aggregator.rs`.
//!
//! # Anyone may submit
//!
//! Submission is permissionless, and the transaction's sender is never
//! consulted. Authenticity comes from the registered RedStone signer set and
//! nowhere else, because the sender attests to nothing about the bytes it
//! carries: a payload is either signed by enough authorised keys or it is not,
//! whoever relays it.

use borsh::BorshDeserialize;
use core::fmt;

use kanon_clock::LezClock;
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use verifier_core::backend::{BackendError, InProgramBackend};
use verifier_core::decode::{DecodeError, Payload};
use verifier_core::time::TimeError;
use verifier_core::{verify_feed, AssetPair, ConfigError, FeedConfig, VerifiedFeed, VerifyError};

use crate::feed_account::FeedAccount;
use crate::publish::{self, PublishError};

/// The second seed of the price account's address, after the feed account's id.
///
/// It appears twice: here, and as a `const(...)` seed in the guest's
/// `#[account(pda = ...)]` attribute, which takes a literal and cannot read a
/// constant. `tests/idl.rs` asserts the two agree by reading the seed back out
/// of the generated IDL, which is the same remedy the literal width in
/// [`FeedAccount::signers`] uses.
///
/// Seeds are zero-padded to 32 bytes before they are combined, so what is
/// hashed is not these nineteen bytes.
pub const PRICE_ACCOUNT_SEED: &str = "KANON_PRICE_ACCOUNT";

/// Why a submission published nothing.
///
/// Every leaf cause keeps its own [`Self::code`], because a caller that cannot
/// tell two failures apart cannot act on either. `SpelError` carries a number
/// and a string, so without the dispatch below thirteen verification failures
/// would reach a relayer as one code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// The account offered as the feed is not one this program owns.
    ///
    /// The check that stops a caller supplying an account of its own whose bytes
    /// happen to decode as a [`FeedAccount`] — which would be a signer set of
    /// the caller's choosing. An unregistered account is the same refusal: it
    /// carries the default owner.
    FeedNotOurs,
    /// The feed account is this program's and empty, so the feed was retired.
    ///
    /// Its own cause and not [`Self::FeedUndecodable`]: an operator reading a
    /// failed submission needs to know the feed was deregistered rather than
    /// that its bytes were unreadable, and the two go to different people.
    /// Deregistration cannot hand the account back (rule 4), so empty is what
    /// retired looks like.
    FeedDeregistered,
    /// The feed account is owned by this program but its data is not a feed.
    FeedUndecodable,
    /// The feed is paused, so it accepts no submissions.
    FeedPaused,
    /// The price account exists but is owned by somebody else.
    ///
    /// Neither a first write nor an update this program may perform. Refused
    /// here because LEZ would refuse the resulting post-state anyway, with an
    /// error that names none of this.
    PriceAccountNotOurs,
    /// The price account is owned by this program but its data is not a price.
    PriceAccountUndecodable,
    /// No trustworthy clock, so staleness could not be decided (ADR 13).
    Clock(TimeError),
    /// The feed's stored configuration does not describe a usable feed.
    Config(ConfigError),
    /// Verification refused the payload.
    Verify(VerifyError),
    /// Verification passed and the write did not.
    Publish(PublishError),
}

impl SubmitError {
    /// A stable number per leaf cause, in blocks by origin: 100 for this layer,
    /// 200 the clock, 300 the configuration, 400 the decoder, 500 the
    /// primitives, 600 verification's own answers, 700 the write.
    ///
    /// A nested cause reached two ways gets one number, which is the intent:
    /// these name causes, not enum positions.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::FeedNotOurs => 101,
            Self::FeedUndecodable => 102,
            Self::FeedDeregistered => 106,
            Self::FeedPaused => 103,
            Self::PriceAccountNotOurs => 104,
            Self::PriceAccountUndecodable => 105,
            Self::Clock(err) => clock_code(*err),
            Self::Config(err) => config_code(*err),
            Self::Verify(err) => verify_code(*err),
            Self::Publish(err) => publish_code(*err),
        }
    }
}

const fn clock_code(err: TimeError) -> u32 {
    match err {
        TimeError::Missing => 201,
        TimeError::WrongAccount => 202,
        TimeError::Undecodable => 203,
        TimeError::Unavailable => 204,
    }
}

pub(crate) const fn config_code(err: ConfigError) -> u32 {
    match err {
        ConfigError::NoSigners => 301,
        ConfigError::ThresholdZero => 302,
        ConfigError::ThresholdExceedsSigners { .. } => 303,
        ConfigError::TooManySigners { .. } => 304,
        ConfigError::DuplicateSigner => 305,
        ConfigError::ZeroSignerAddress => 306,
        ConfigError::FeedIdTooLong { .. } => 307,
        ConfigError::ZeroFeedId => 308,
        ConfigError::DecimalsOutOfRange { .. } => 309,
        ConfigError::MaxAgeZero => 310,
        ConfigError::MaxAgeTooLarge { .. } => 311,
    }
}

const fn decode_code(err: DecodeError) -> u32 {
    match err {
        DecodeError::MissingMarker => 401,
        DecodeError::Truncated => 402,
        DecodeError::LengthOutOfRange => 403,
        DecodeError::NumberOverflow => 404,
        DecodeError::TooLong { .. } => 405,
        DecodeError::NoDataPackages => 406,
        DecodeError::NoDataPoints => 407,
        DecodeError::ZeroWidthValue => 408,
        DecodeError::TrailingBytes(_) => 409,
    }
}

const fn backend_code(err: BackendError) -> u32 {
    match err {
        BackendError::InvalidRecoveryId => 501,
        BackendError::InvalidSignature => 502,
        BackendError::RecoveryFailed => 503,
    }
}

const fn verify_code(err: VerifyError) -> u32 {
    match err {
        // The four wrapping variants dispatch to the cause rather than to
        // themselves, which is the whole reason they are wrapped.
        VerifyError::Malformed(err) => decode_code(err),
        VerifyError::InvalidSignature(err) => backend_code(err),
        VerifyError::NoClock(err) => clock_code(err),
        VerifyError::InvalidConfig(err) => config_code(err),
        VerifyError::UnauthorisedSigner => 601,
        VerifyError::ThresholdNotMet { .. } => 602,
        VerifyError::TimestampMismatch { .. } => 603,
        VerifyError::TooManyPackages { .. } => 604,
        VerifyError::StalePackage => 605,
        VerifyError::FuturePackage => 606,
        VerifyError::AssetMismatch => 607,
        VerifyError::ValueOutOfRange => 608,
        VerifyError::ScalingOutOfRange => 609,
    }
}

const fn publish_code(err: PublishError) -> u32 {
    match err {
        PublishError::AssetMismatch => 701,
        PublishError::SourceMismatch => 702,
        PublishError::NotNewer { .. } => 703,
    }
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeedNotOurs => f.write_str("the feed account is not one this program registered"),
            Self::FeedUndecodable => f.write_str("the feed account's data is not a feed"),
            Self::FeedDeregistered => {
                f.write_str("this feed was deregistered, so its account holds no feed")
            }
            Self::FeedPaused => f.write_str("the feed is paused and accepts no submissions"),
            Self::PriceAccountNotOurs => {
                f.write_str("the price account exists and this program does not own it")
            }
            Self::PriceAccountUndecodable => {
                f.write_str("the price account's data is not a price account")
            }
            Self::Clock(err) => write!(f, "no trustworthy clock: {err:?}"),
            Self::Config(err) => write!(f, "the feed's stored configuration is unusable: {err:?}"),
            Self::Verify(err) => write!(f, "the payload did not verify: {err:?}"),
            Self::Publish(err) => write!(f, "the verified price was not published: {err:?}"),
        }
    }
}

/// What the guest returns, so its handler is a `?`.
///
/// `SpelError` carries a number and a string. The number is [`SubmitError::code`]
/// offset by the framework's own custom-error base, which is what
/// `SpelError::error_code` adds; the string is for a human reading a failed
/// transaction and is not something to match on.
impl From<SubmitError> for spel_framework::error::SpelError {
    fn from(err: SubmitError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

impl From<TimeError> for SubmitError {
    fn from(err: TimeError) -> Self {
        Self::Clock(err)
    }
}

impl From<ConfigError> for SubmitError {
    fn from(err: ConfigError) -> Self {
        Self::Config(err)
    }
}

impl From<VerifyError> for SubmitError {
    fn from(err: VerifyError) -> Self {
        Self::Verify(err)
    }
}

impl From<PublishError> for SubmitError {
    fn from(err: PublishError) -> Self {
        Self::Publish(err)
    }
}

/// Verifies a payload against a registered feed and publishes the price.
///
/// The three accounts are the instruction's, in its declared order, and the
/// returned post-states are in that same order: `validate_execution` zips
/// pre-states against post-states positionally and requires equal length.
///
/// `self_program_id` comes from the execution context and never from an account.
///
/// The order of the checks is deliberate, and the claim is narrow: every
/// *reachable* rejection decidable from the accounts, the stored configuration or
/// the payload's framing happens before the first signature recovery — the clock,
/// the feed's ownership, the pause flag, the configuration itself, the price
/// account's state, and the envelope. Reachable is load-bearing there, because the
/// two identifier checks below are account checks that run afterwards.
///
/// The rest cannot be, and are not. An invalid signature and an unmet threshold
/// are answers verification exists to produce, and a replayed payload reaches
/// `NotNewer` only after it: that check compares against a timestamp
/// verification has to derive first. A caller can provoke all three.
///
/// The two identifier checks inside [`publish`] are each after the recovery for a
/// different reason. Its pair check is already done earlier and by someone else:
/// `expected` is the account's pair on the update path, so `verify_feed`'s first
/// comparison is between the same two values, and a mismatched account answers
/// `Verify(AssetMismatch)` before `publish` is entered. By the time it runs, its
/// own comparison cannot fail. Its source check is genuinely only there, and
/// stays there: with the price account derived from the feed and writable only by
/// this program, an account naming another source is a state nothing can produce,
/// so hoisting it would buy cycles on a path no caller can reach.
///
/// None of this makes an oversized payload free. LEZ has read and deserialised
/// the instruction data before this function is entered, which is the residual
/// [ADR 26] records.
///
/// # Errors
///
/// [`SubmitError`], one leaf cause per failure, which the caller sees as a
/// distinct code.
///
/// [ADR 26]: ../../adr/0026-a-maximum-payload-size-and-where-it-has-to-be-enforced.md
pub fn submit_price(
    feed: AccountWithMetadata,
    price_account: AccountWithMetadata,
    clock: AccountWithMetadata,
    payload: &[u8],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SubmitError> {
    if feed.account.program_owner != self_program_id {
        return Err(SubmitError::FeedNotOurs);
    }

    // Before the feed is read, because a caller-chosen clock is the one input
    // that would make every staleness check downstream a formality.
    let now = LezClock::from_account(clock.account_id.value(), clock.account.data.as_ref())?;

    if feed.account.data.as_ref().is_empty() {
        return Err(SubmitError::FeedDeregistered);
    }
    let stored = FeedAccount::try_from_slice(feed.account.data.as_ref())
        .map_err(|_| SubmitError::FeedUndecodable)?;
    // Administrative rather than cryptographic, so it is decided here and not in
    // `verifier-core` — and before any cryptography, because a paused feed should
    // not pay to find out it is paused.
    if stored.paused {
        return Err(SubmitError::FeedPaused);
    }

    let signers = stored.signer_addresses();
    let config = stored.config(&signers)?;

    // Three states, not two. A fully default account is a first write; an
    // account this program owns is an update; anything else is neither, and
    // `validate_execution` would refuse the post-state rather than the input.
    let create = price_account.account == Account::default();
    if !create && price_account.account.program_owner != self_program_id {
        return Err(SubmitError::PriceAccountNotOurs);
    }

    let published = if create {
        None
    } else {
        Some(
            OraclePriceAccount::try_from(&price_account.account.data)
                .map_err(|_| SubmitError::PriceAccountUndecodable)?,
        )
    };

    // ADR 16 makes the pair the caller's claim checked against the registration,
    // and in push mode the standing claim is what the account already publishes.
    // Passing the registration's own pair instead would make `verify_feed`'s
    // comparison `x != x` -- structurally dead rather than merely unreachable --
    // so the account's pair is what keeps it a real check, against the day the
    // account and the feed stop being derived from one another. On a first write
    // there is nothing published yet and the registration is the only claim
    // there is.
    let expected = published.as_ref().map_or_else(
        || *config.assets(),
        |account| {
            AssetPair::new(
                account.base_asset.into_value(),
                account.quote_asset.into_value(),
            )
        },
    );

    let decoded = Payload::decode(payload).map_err(VerifyError::Malformed)?;
    let verified = verify_feed(&decoded, &config, &expected, &InProgramBackend::new(), &now)?;

    let written = write(published, &config, &verified)?;
    Ok(post_states(feed, price_account, clock, &written, create))
}

/// The price account contents a verified feed produces, created or updated.
fn write(
    published: Option<OraclePriceAccount>,
    config: &FeedConfig<'_>,
    verified: &VerifiedFeed,
) -> Result<OraclePriceAccount, SubmitError> {
    match published {
        None => Ok(publish::price_account(config, verified)),
        Some(mut account) => {
            publish::publish(&mut account, config, verified)?;
            Ok(account)
        }
    }
}

/// The three post-states, in the instruction's account order.
///
/// The claim comes from the same `create` discriminator the write did, rather
/// than from the account's owner: an account with data and a default owner has a
/// claimable owner and an unwritable body, and deciding the claim separately is
/// what would let that state through.
fn post_states(
    feed: AccountWithMetadata,
    price_account: AccountWithMetadata,
    clock: AccountWithMetadata,
    written: &OraclePriceAccount,
    create: bool,
) -> Vec<AccountPostState> {
    let mut account = price_account.account;
    // Six fixed-width fields, so the width `Data` refuses is out of reach.
    account.data = Data::from(written);

    let price_post = if create {
        let feed_seed = *feed.account_id.value();
        let name_seed = seed_from_str(PRICE_ACCOUNT_SEED);
        AutoClaim::pda_from_seeds(&[&feed_seed, &name_seed]).to_post_state(account)
    } else {
        AccountPostState::new(account)
    };

    vec![
        AccountPostState::new(feed.account),
        price_post,
        AccountPostState::new(clock.account),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanon_clock::CLOCK_ACCOUNT_ID;
    use lee_core::account::{AccountId, Nonce};
    use spel_framework::error::SpelError;

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];
    const DEFAULT: ProgramId = [0u32; 8];
    /// The clock account's real owner. Nothing here reads it -- the account id is
    /// what is checked -- but a clock owned by nobody is not a state LEZ has.
    const CLOCK_PROGRAM: ProgramId = [88u32; 8];

    const NOW_MS: u64 = 1_770_000_000_000;

    /// Bytes that are not a payload, so a test reaching the decoder fails rather
    /// than quietly proceeding.
    const NOT_A_PAYLOAD: &[u8] = &[0xFF; 16];

    fn account(owner: ProgramId, data: Vec<u8>, id: [u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: owner,
                balance: 0,
                data: Data::try_from(data).expect("fits"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: AccountId::new(id),
        }
    }

    fn registered() -> FeedAccount {
        let mut feed_id = [0u8; 32];
        feed_id[..3].copy_from_slice(b"BTC");
        FeedAccount {
            feed_id,
            base_asset: [1u8; 32],
            quote_asset: [2u8; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: vec![[3u8; 20], [4u8; 20], [5u8; 20]],
            threshold: 2,
            paused: false,
        }
    }

    fn feed_account(stored: &FeedAccount) -> AccountWithMetadata {
        account(
            OURS,
            borsh::to_vec(stored).expect("serialises"),
            [0xFEu8; 32],
        )
    }

    /// A price account that does not exist yet, which is what a first write is
    /// offered.
    fn no_price_account() -> AccountWithMetadata {
        account(DEFAULT, Vec::new(), [0xACu8; 32])
    }

    fn clock_account(id: [u8; 32], timestamp: u64) -> AccountWithMetadata {
        let mut data = [0u8; 16];
        data[8..].copy_from_slice(&timestamp.to_le_bytes());
        account(CLOCK_PROGRAM, data.to_vec(), id)
    }

    fn clock() -> AccountWithMetadata {
        clock_account(CLOCK_ACCOUNT_ID, NOW_MS)
    }

    fn submit(
        feed: AccountWithMetadata,
        price_account: AccountWithMetadata,
        clock: AccountWithMetadata,
        payload: &[u8],
    ) -> Result<Vec<AccountPostState>, SubmitError> {
        submit_price(feed, price_account, clock, payload, OURS)
    }

    #[test]
    fn a_clock_that_is_not_the_pinned_account_is_refused_before_the_feed_is_read() {
        // ADR 13's guarantee, and the ordering it implies. The feed here is one
        // this program owns whose data is *not* a feed, so a run that reads the
        // feed before the clock answers `FeedUndecodable` and a run that checks
        // the clock first answers `WrongAccount`. Both are refusals, which is
        // why asserting the refusal alone would not pin the order.
        let undecodable = account(OURS, vec![0xFF; 4], [0xFE; 32]);

        for id in [
            *b"/LEZ/ClockProgramAccount/0000010",
            *b"/LEZ/ClockProgramAccount/0000050",
            [0u8; 32],
        ] {
            assert_eq!(
                submit(
                    undecodable.clone(),
                    no_price_account(),
                    clock_account(id, NOW_MS),
                    NOT_A_PAYLOAD,
                ),
                Err(SubmitError::Clock(TimeError::WrongAccount)),
                "the clock has to be settled before anything reads the feed"
            );
        }
    }

    #[test]
    fn a_feed_account_this_program_does_not_own_is_not_a_feed() {
        // The substitution this check exists to stop: an account the caller owns
        // holding a signer set of the caller's choosing. It decodes perfectly as
        // a feed, which is the point -- the data cannot be what distinguishes it.
        let mut forged = registered();
        forged.signers = vec![[0xAA; 20]];
        forged.threshold = 1;
        let bytes = borsh::to_vec(&forged).expect("serialises");

        assert_eq!(
            submit(
                account(SOMEONE_ELSE, bytes, [0xFE; 32]),
                no_price_account(),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::FeedNotOurs)
        );

        // An unregistered account is the same refusal rather than one of its
        // own: it carries the default owner, which is why there is no separate
        // "not initialised" cause.
        assert_eq!(
            submit(
                account(DEFAULT, Vec::new(), [0xFE; 32]),
                no_price_account(),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::FeedNotOurs)
        );
    }

    #[test]
    fn a_paused_feed_refuses_before_a_payload_is_decoded() {
        // The payload is not a payload, so a run that decodes first answers
        // `Malformed` -- which is what pins the order rather than the refusal.
        let mut stored = registered();
        stored.paused = true;

        assert_eq!(
            submit(
                feed_account(&stored),
                no_price_account(),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::FeedPaused),
            "a paused feed should not pay to find out it is paused"
        );
    }

    #[test]
    fn a_non_default_price_account_with_a_default_owner_is_refused() {
        // The state that is neither a create nor a valid update. The account is
        // not default, so LEZ refuses the data write; the owner is default, so
        // deciding the claim on ownership alone would call it a create and hand
        // the chain a post-state it rejects. Refused here, where the error says
        // why.
        let squatted = account(DEFAULT, vec![1, 2, 3], [0xAC; 32]);

        assert_eq!(
            submit(
                feed_account(&registered()),
                squatted,
                clock(),
                NOT_A_PAYLOAD
            ),
            Err(SubmitError::PriceAccountNotOurs)
        );
    }

    #[test]
    fn a_price_account_another_program_owns_is_refused() {
        let theirs = account(SOMEONE_ELSE, vec![1, 2, 3], [0xAC; 32]);

        assert_eq!(
            submit(feed_account(&registered()), theirs, clock(), NOT_A_PAYLOAD),
            Err(SubmitError::PriceAccountNotOurs)
        );
    }

    #[test]
    fn a_feed_this_program_owns_whose_data_is_not_a_feed_reports_it_as_such() {
        assert_eq!(
            submit(
                account(OURS, vec![0xFF; 4], [0xFE; 32]),
                no_price_account(),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::FeedUndecodable)
        );
    }

    #[test]
    fn a_price_account_this_program_owns_whose_data_is_not_a_price_reports_it_as_such() {
        assert_eq!(
            submit(
                feed_account(&registered()),
                account(OURS, vec![0xFF; 4], [0xAC; 32]),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::PriceAccountUndecodable)
        );
    }

    #[test]
    fn bytes_that_are_not_a_payload_are_reported_as_malformed() {
        // Also the only check that the decoder's failure is wrapped the right
        // way round rather than reaching a caller as something else.
        assert_eq!(
            submit(
                feed_account(&registered()),
                no_price_account(),
                clock(),
                NOT_A_PAYLOAD,
            ),
            Err(SubmitError::Verify(VerifyError::Malformed(
                DecodeError::MissingMarker
            )))
        );
    }

    /// Every cause a submission can reach, one representative per leaf.
    ///
    /// Enumerated by hand rather than derived, which is the point: the
    /// exhaustive `match` in `code` makes a new variant a compile error, and
    /// this list makes it a test failure until somebody decides its number.
    ///
    /// Three of `code`'s arms are deliberately absent, and each has its reason
    /// asserted below rather than only stated here: `NoClock` and
    /// `InvalidConfig` because the clock and the configuration are settled before
    /// verification runs, and `PublishError::AssetMismatch` because
    /// `verify_feed` has already compared the same two values.
    fn reachable_causes() -> Vec<SubmitError> {
        let clock = [
            TimeError::Missing,
            TimeError::WrongAccount,
            TimeError::Undecodable,
            TimeError::Unavailable,
        ];
        let config = [
            ConfigError::NoSigners,
            ConfigError::ThresholdZero,
            ConfigError::ThresholdExceedsSigners {
                threshold: 2,
                signers: 1,
            },
            ConfigError::TooManySigners {
                signers: 33,
                max: 32,
            },
            ConfigError::DuplicateSigner,
            ConfigError::ZeroSignerAddress,
            ConfigError::FeedIdTooLong { len: 33 },
            ConfigError::ZeroFeedId,
            ConfigError::DecimalsOutOfRange {
                decimals: 40,
                max: 38,
            },
            ConfigError::MaxAgeZero,
            ConfigError::MaxAgeTooLarge {
                max_age_ms: u64::MAX,
                max: 900_000,
            },
        ];
        let decode = [
            DecodeError::MissingMarker,
            DecodeError::Truncated,
            DecodeError::LengthOutOfRange,
            DecodeError::NumberOverflow,
            DecodeError::TooLong { len: 1, max: 0 },
            DecodeError::NoDataPackages,
            DecodeError::NoDataPoints,
            DecodeError::ZeroWidthValue,
            DecodeError::TrailingBytes(3),
        ];
        let backend = [
            BackendError::InvalidRecoveryId,
            BackendError::InvalidSignature,
            BackendError::RecoveryFailed,
        ];
        let verify = [
            VerifyError::UnauthorisedSigner,
            VerifyError::ThresholdNotMet {
                met: 1,
                required: 2,
            },
            VerifyError::TimestampMismatch {
                expected: 1,
                found: 2,
            },
            VerifyError::TooManyPackages { max: 32 },
            VerifyError::StalePackage,
            VerifyError::FuturePackage,
            VerifyError::AssetMismatch,
            VerifyError::ValueOutOfRange,
            VerifyError::ScalingOutOfRange,
        ];
        let publish = [
            PublishError::SourceMismatch,
            PublishError::NotNewer {
                stored: 2,
                offered: 1,
            },
        ];

        let mut causes = vec![
            SubmitError::FeedNotOurs,
            SubmitError::FeedUndecodable,
            SubmitError::FeedPaused,
            SubmitError::PriceAccountNotOurs,
            SubmitError::PriceAccountUndecodable,
        ];
        causes.extend(clock.into_iter().map(SubmitError::Clock));
        causes.extend(config.into_iter().map(SubmitError::Config));
        causes.extend(
            decode
                .into_iter()
                .map(|err| SubmitError::Verify(VerifyError::Malformed(err))),
        );
        causes.extend(
            backend
                .into_iter()
                .map(|err| SubmitError::Verify(VerifyError::InvalidSignature(err))),
        );
        causes.extend(verify.into_iter().map(SubmitError::Verify));
        causes.extend(publish.into_iter().map(SubmitError::Publish));
        causes
    }

    #[test]
    fn no_two_semantic_causes_share_an_error_code() {
        // ADR 23's assertion, one layer out: a caller that cannot tell two
        // failures apart cannot act on either. Semantic causes rather than enum
        // values -- see the two exclusions below.
        let causes = reachable_causes();
        for (i, left) in causes.iter().enumerate() {
            for right in &causes[i + 1..] {
                assert_ne!(
                    left.code(),
                    right.code(),
                    "{left:?} and {right:?} answer with the same code"
                );
            }
        }
    }

    #[test]
    fn the_writes_pair_check_cannot_answer_because_verification_answered_first() {
        // `publish` compares the account's pair against the configuration's, and
        // `submit_price` has already passed the account's pair to `verify_feed`
        // as the caller's claim -- so the two values `publish` compares are equal
        // by the time it sees them. Its variant keeps a code, because the
        // function is public and its own tests reach it; nothing a submission can
        // do reaches it.
        assert_eq!(
            SubmitError::Publish(PublishError::AssetMismatch).code(),
            701,
            "the code stays assigned even though a submission cannot produce it"
        );
        assert!(
            !reachable_causes().contains(&SubmitError::Publish(PublishError::AssetMismatch)),
            "if a submission can now reach it, it belongs in the list"
        );
    }

    #[test]
    fn a_cause_reached_twice_keeps_one_code() {
        // `NoClock` and `InvalidConfig` are unreachable through this path: the
        // clock is settled before the feed is read and the configuration before
        // the payload is decoded. If they became reachable they would answer
        // with the same codes as the direct causes, which is intended rather
        // than a clash -- these numbers name where a failure came from.
        assert_eq!(
            SubmitError::Verify(VerifyError::NoClock(TimeError::WrongAccount)).code(),
            SubmitError::Clock(TimeError::WrongAccount).code()
        );
        assert_eq!(
            SubmitError::Verify(VerifyError::InvalidConfig(ConfigError::ThresholdZero)).code(),
            SubmitError::Config(ConfigError::ThresholdZero).code()
        );
    }

    #[test]
    fn the_code_a_caller_sees_is_the_frameworks_offset_one() {
        // What reaches a relayer is `SpelError::error_code`, not `code`. The
        // offset belongs to the framework and this is where it is pinned, so a
        // caller reading 6605 can look 605 up.
        for cause in reachable_causes() {
            let spel: SpelError = cause.into();
            assert_eq!(spel.error_code(), 6000 + cause.code());
        }
    }
}
