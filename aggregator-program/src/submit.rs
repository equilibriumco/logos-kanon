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

const fn config_code(err: ConfigError) -> u32 {
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
/// The order of the checks is deliberate: nothing pays for a signature recovery
/// that a cheap comparison could have refused. It does not make an oversized
/// payload free — LEZ has read and deserialised the instruction data before this
/// function is entered, which is the residual [ADR 26] records.
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

    // ADR 16 makes the pair the caller's claim checked against the registration.
    // In push mode the standing claim is what the account already publishes, so
    // the comparison runs before the recoveries rather than after them. On a
    // first write there is nothing published yet and the registration is the
    // only claim there is.
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
