//! How a consumer should verify a RedStone payload inline.
//!
//! A limit order, in the smallest program that can hold one. [`open_order`]
//! records a price somebody is prepared to trade at; [`settle`] fills the order
//! if a payload the consumer verifies itself says the market has reached it. The
//! domain is thin on purpose — what a reader should take from this file is the
//! seam between a LEZ program and [`pull_lib`], not the trade.
//!
//! Four things that seam has to get right, and each is the reason a line here
//! looks the way it does.
//!
//! # 1. The configuration is compiled in, and instruction data cannot reach it
//!
//! The signer set, the threshold, the staleness window, the scale and the pair
//! are [`SIGNERS`], [`THRESHOLD`], [`MAX_AGE_MS`], [`DECIMALS`] and [`FEEDS`] —
//! constants in this file. Nothing a caller sends can add an address to the
//! roster, lower the threshold, widen the window or relabel a pair.
//!
//! That is the whole security model of pull mode (SEC2), and it is the half
//! `pull-lib` cannot check. The roster arrives there as a slice, and a slice
//! built from a constant is indistinguishable from one built from the
//! transaction's own instruction data: a consumer that read its signers off the
//! wire would satisfy every signature in that crate and have handed its caller
//! the right to speak for the feed. So the demonstration is structural rather
//! than asserted, and it is this: [`settle`] takes no signer, threshold, window
//! or pair argument. There is no parameter for a caller to reach.
//!
//! What a caller *may* choose is which of the compiled feeds an order is
//! against, by index into [`FEEDS`]. An index is not a roster — an out-of-range
//! one is [`OpenError::UnknownFeed`] — and a consumer serving five pairs needs
//! some way to say which, so this is the narrowest opening that keeps the
//! program useful.
//!
//! # 2. The clock is read from the account the transaction supplied
//!
//! [`pull_lib::verify_price`] takes the clock's id and its bytes as two separate
//! slices, so nothing inside it can check that the bytes came from the account
//! they name. A consumer passing [`pull_lib::CLOCK_ACCOUNT_ID`] alongside
//! sixteen bytes of its own choosing is believed, and every staleness check
//! still passes.
//!
//! Binding the two is only possible where a program meets its dispatcher, and
//! that is here:
//!
//! ```ignore
//! verify_price(payload, &config, &pair, clock.account_id.value(), clock.account.data.as_ref(), ..)
//! ```
//!
//! One [`AccountWithMetadata`], two fields of it. The id and the data cannot
//! disagree because neither is constructed — LEZ handed over both, as one
//! account. `[M3-01:01]` records the residual this closes.
//!
//! # 3. Every typed error refuses the action
//!
//! [`SettleError`] enumerates every way a settlement can fail, with a distinct
//! [`SettleError::code`] per cause, and there is no arm that returns a price
//! anyway. A stale payload does not fill at the last price it saw; an
//! unauthorised signer does not fill at a smaller quorum; a malformed payload
//! does not fill at zero. **Refusing is the only thing this program does when it
//! cannot verify a price** (U7), and it refuses by returning an error rather
//! than by returning post-states that happen to change nothing — a caller must
//! be able to tell "not filled" from "could not tell".
//!
//! The numbers are this program's own and are not the aggregator's. What U6 asks
//! to be shared is the typed value: a cause is one [`VerifyError`] variant
//! whichever mode meets it, because both modes call one verifier, and M3-04
//! asserts that identity over the same inputs. An integer is a program's
//! interface to its own callers, so a consumer numbering its causes in its own
//! space is not a second taxonomy. What would be a second taxonomy is a
//! consumer that collapsed a taxonomy into one code, which is what the blocks in
//! [`SettleError::code`] exist not to do — including the four `VerifyError`
//! variants that carry another error, each of which dispatches into that layer's
//! block rather than reporting its whole layer as one number.
//!
//! # 4. Verification is not authorisation
//!
//! [`settle`] is permissionless: anybody may present a payload for anybody's
//! order, exactly as anybody may submit a price to the push aggregator. Nothing
//! about the sender is checked, because the sender attests to nothing — the
//! signatures in the payload are the only claim of authenticity there is, and
//! they are RedStone's rather than the sender's.
//!
//! [`open_order`] is the opposite: it requires the owner's signature, because
//! opening an order is a statement about the owner rather than about the market.
#![forbid(unsafe_code)]

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use pull_lib::{
    verify_price, AssetPair, BackendError, ConfigError, DecodeError, FeedConfig, InProgramBackend,
    PullConfig, SignerAddress, TimeError, VerifiedFeed, VerifyError,
};
use serde::{Deserialize, Serialize};
use spel_framework_macros::account_type;

/// The name seed an order account's address is derived from.
///
/// `for_public_pda(program, sha256(order_id || zero_pad_32("KANON_PULL_ORDER")))`,
/// declared on the guest handlers so the derivation reaches the IDL and a client
/// can compute an order's address without being told it.
pub const ORDER_ACCOUNT_SEED: &str = "KANON_PULL_ORDER";

/// The signers this consumer authorises, and the whole of who may speak for a
/// price it acts on.
///
/// RedStone's `redstone-primary-prod` set for the feeds in [`FEEDS`], as
/// `FEEDS.md` records them. A deployment substitutes its own; what it must not
/// do is take them from anywhere a caller can reach.
///
/// These addresses *are* the data service. [`DATA_SERVICE_ID`] names it for a
/// human and authenticates nothing: no payload carries a data service, and the
/// one envelope slot that could sits outside every signature.
pub const SIGNERS: [[u8; 20]; 5] = [
    [
        0x9c, 0x5a, 0xe8, 0x9c, 0x4a, 0xf6, 0xaa, 0x32, 0xce, 0x58, 0x58, 0x8d, 0xba, 0xf9, 0x0d,
        0x18, 0xa8, 0x55, 0xb6, 0xde,
    ],
    [
        0xde, 0xb2, 0x2f, 0x54, 0x73, 0x8d, 0x54, 0x97, 0x6c, 0x4c, 0x0f, 0xe5, 0xce, 0x6d, 0x40,
        0x8e, 0x40, 0xd8, 0x84, 0x99,
    ],
    [
        0xdd, 0x68, 0x2d, 0xae, 0xc5, 0xa9, 0x0d, 0xd2, 0x95, 0xd1, 0x4d, 0xa4, 0xb0, 0xbe, 0xc9,
        0x28, 0x10, 0x17, 0xb5, 0xbe,
    ],
    [
        0x51, 0xce, 0x04, 0xbe, 0x4b, 0x3e, 0x32, 0x57, 0x2c, 0x4e, 0xc9, 0x13, 0x52, 0x21, 0xd0,
        0x69, 0x1b, 0xa7, 0xd2, 0x02,
    ],
    [
        0x8b, 0xb8, 0xf3, 0x2d, 0xf0, 0x4c, 0x8b, 0x65, 0x49, 0x87, 0xda, 0xae, 0xd5, 0x3d, 0x6b,
        0x60, 0x91, 0xe3, 0xb7, 0x74,
    ],
];

/// How many of [`SIGNERS`] must agree before this consumer acts on a price.
pub const THRESHOLD: u8 = 3;

/// How old the oldest package behind a price may be, in milliseconds.
///
/// The consumer's own tolerance, and not something a payload or its sender can
/// widen.
pub const MAX_AGE_MS: u64 = 60_000;

/// The power of ten [`SIGNERS`] scale their values by.
pub const DECIMALS: u8 = 8;

/// The RedStone data service [`SIGNERS`] belongs to.
///
/// Carried for this consumer's own records — its logs, its own error paths — and
/// read by nothing in verification. See the module header.
pub const DATA_SERVICE_ID: &str = "redstone-primary-prod";

/// One feed this consumer is prepared to price an order against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedSpec {
    /// The RedStone feed id, right-padded to the wire width.
    pub feed_id: [u8; 32],
    /// The base asset of the pair.
    pub base_asset: [u8; 32],
    /// The quote asset of the pair.
    pub quote_asset: [u8; 32],
}

/// The feeds this consumer serves, and the only ones an order can name.
///
/// An order names one by its index here, so the set is fixed at build time. The
/// asset ids are the tickers, right-padded: a deployment replaces them with the
/// LEZ account ids of the real assets, which is the one edit this table needs.
/// The feed ids are RedStone's and are what a payload is matched on.
pub const FEEDS: [FeedSpec; 5] = [
    feed(b"BTC", b"BTC", b"USD"),
    feed(b"ETH", b"ETH", b"USD"),
    feed(b"SOL", b"SOL", b"USD"),
    feed(b"XMR", b"XMR", b"USD"),
    feed(b"ZEC", b"ZEC", b"USD"),
];

const fn feed(feed_id: &[u8], base: &[u8], quote: &[u8]) -> FeedSpec {
    FeedSpec {
        feed_id: padded(feed_id),
        base_asset: padded(base),
        quote_asset: padded(quote),
    }
}

/// Right-pads to the 32 bytes both a feed id and an asset id are carried in.
const fn padded(name: &[u8]) -> [u8; 32] {
    assert!(name.len() <= 32, "an identifier is at most 32 bytes");
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < name.len() {
        out[i] = name[i];
        i += 1;
    }
    out
}

/// One open order: whose it is, what it is against, and the price it fills at.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct OrderAccount {
    /// The account that opened this order and signed for it.
    pub owner: [u8; 32],
    /// Which of `FEEDS` this order is priced against, by index.
    pub feed: u8,
    /// The price at or above which the order fills, on the `Q64.64` scale
    /// `verifier-core` converts a RedStone value to.
    pub limit_price_q64: u128,
    /// Whether the order has been filled. An order fills once.
    pub filled: bool,
}

/// Why an order could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// The owner did not sign.
    OwnerDidNotSign,
    /// An order is already open at this id's address.
    ///
    /// An order id is spent once. LEZ rule 4 forbids a program giving up
    /// ownership of an account and rule 3 forbids resetting its nonce, so this
    /// account stays this program's after a fill and cannot be handed back as
    /// `Account::default()`. Re-opening it would mean admitting a second
    /// pre-state the way `register_feed` does for a retired feed; an order has
    /// no reason to be reused, so it is not admitted and the cost is a fresh id.
    AlreadyOpen,
    /// The account at this id's address is not one this program can write.
    ///
    /// Its own cause because the diagnosis is different and the outcome is
    /// terminal. The whole account is compared and not its owner: an account
    /// that holds a balance or a nonce but is still unowned has a claimable
    /// owner and an unwritable body, and no instruction of any program can move
    /// it. Since an order's address is derived from an id anybody can guess,
    /// anybody can put an account there by sending it one unit of balance.
    ///
    /// Letting it through would move the refusal to `validate_execution`, which
    /// reports against the post-state rather than the input.
    OrderAccountUnusable,
    /// No feed sits at that index in [`FEEDS`].
    UnknownFeed,
    /// The serialised order does not fit an account's data.
    ///
    /// Unreachable: an order is fifty-eight bytes against a 100 KiB limit. Kept
    /// because the alternative is `expect`, and a panic in a guest aborts the
    /// transaction instead of refusing the instruction.
    OrderTooLarge,
}

/// Why a settlement was refused, and every one of them refuses it.
///
/// A distinct [`Self::code`] per leaf cause: a caller that cannot tell two
/// failures apart cannot act on either, and `SpelError` carries one number, so
/// without the match below every verification failure would arrive as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleError {
    /// The account offered as the order is not one this program owns.
    ///
    /// What stops a caller supplying an account of its own whose bytes decode as
    /// an order — which would be a limit price of the caller's choosing. An
    /// unopened account is the same refusal: it carries the default owner.
    OrderNotOurs,
    /// The order account's bytes are not an order.
    OrderUndecodable,
    /// The order has already been filled. An order fills once.
    AlreadyFilled,
    /// The stored feed index is not one in [`FEEDS`].
    ///
    /// Reachable only by shrinking or reordering [`FEEDS`] under orders that are
    /// already open, which is a deployment mistake rather than a caller's. It
    /// fails closed here instead of pricing the order against whatever feed
    /// moved into that slot.
    UnknownFeed,
    /// The compiled-in configuration is not a usable one.
    ///
    /// Also a deployment mistake rather than a caller's: every input to
    /// [`FeedConfig::try_new`] is a constant in this file. Reported through
    /// [`ConfigError`] rather than restated, so one cause has one meaning
    /// wherever it surfaces. `the_compiled_in_configuration_is_usable` is what
    /// makes this unreachable in this build rather than merely unlikely.
    Config(ConfigError),
    /// Verification refused the payload.
    ///
    /// The typed taxonomy, unchanged: `pull-lib` returns `verifier-core`'s error
    /// and this carries it rather than flattening it, which is what leaves a
    /// stale package distinguishable from an unauthorised signer at the
    /// program's boundary.
    Verify(VerifyError),
    /// The verified price has not reached the order's limit.
    ///
    /// The one refusal that is not a failure: verification succeeded and the
    /// answer was no. It carries both numbers because "not yet" and "never" look
    /// the same to a caller that is told only that it was refused.
    LimitNotReached {
        /// What the payload verified to, on the `Q64.64` scale.
        price: u128,
        /// What the order requires.
        limit: u128,
    },
    /// The serialised order does not fit an account's data. See
    /// [`OpenError::OrderTooLarge`].
    OrderTooLarge,
}

impl From<ConfigError> for SettleError {
    fn from(err: ConfigError) -> Self {
        Self::Config(err)
    }
}

impl From<VerifyError> for SettleError {
    fn from(err: VerifyError) -> Self {
        Self::Verify(err)
    }
}

impl OpenError {
    /// A stable number per cause, in this program's 1300 block.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::OwnerDidNotSign => 1301,
            Self::AlreadyOpen => 1302,
            Self::OrderAccountUnusable => 1303,
            Self::UnknownFeed => 1304,
            Self::OrderTooLarge => 1305,
        }
    }
}

impl SettleError {
    /// A stable number per leaf cause: this program's own in the 1400 block,
    /// then one block per layer beneath it — 1500 for a configuration fault,
    /// 1600 for a verification failure `VerifyError` names itself, 1700 for the
    /// decoder, 1800 for signature recovery, 1900 for the clock.
    ///
    /// *Leaf* is the load-bearing word. Four `VerifyError` variants carry another
    /// error, and each dispatches into that layer's block rather than taking a
    /// number of its own: `Malformed` covers nine decoder faults and would
    /// otherwise report all nine as "unreadable". A wrapper answered with one
    /// number is the collapse U6 exists to prevent, and it is what the push path
    /// avoids the same way.
    ///
    /// The blocks are this program's and are not the aggregator's — see the
    /// module header on why an integer space is per program while the typed
    /// value is shared. What matters is the property: two causes never answer
    /// with the same number, and every match involved is exhaustive with no
    /// wildcard arm, so a variant added anywhere in the taxonomy stops this
    /// compiling until somebody decides what a consumer should tell its callers
    /// about it.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::OrderNotOurs => 1401,
            Self::OrderUndecodable => 1402,
            Self::AlreadyFilled => 1403,
            Self::UnknownFeed => 1404,
            Self::OrderTooLarge => 1405,
            Self::LimitNotReached { .. } => 1406,
            Self::Config(err) => config_code(*err),
            Self::Verify(err) => verify_code(*err),
        }
    }
}

/// One number per configuration fault, in the 1500 block.
///
/// Reached two ways — a configuration this program failed to build, and
/// `VerifyError::InvalidConfig` — and both arrive here, because one cause reached
/// by two routes is still one cause and deserves one number.
const fn config_code(err: ConfigError) -> u32 {
    match err {
        ConfigError::MaxAgeZero => 1501,
        ConfigError::MaxAgeTooLarge { .. } => 1502,
        ConfigError::DecimalsOutOfRange { .. } => 1503,
        ConfigError::NoSigners => 1504,
        ConfigError::ThresholdZero => 1505,
        ConfigError::TooManySigners { .. } => 1506,
        ConfigError::ThresholdExceedsSigners { .. } => 1507,
        ConfigError::ZeroSignerAddress => 1508,
        ConfigError::DuplicateSigner => 1509,
        ConfigError::FeedIdTooLong { .. } => 1510,
        ConfigError::ZeroFeedId => 1511,
    }
}

/// One number per verification failure that `VerifyError` names itself, in the
/// 1600 block.
///
/// Four of its variants carry another error, and each of those dispatches to the
/// block for that layer instead of consuming a number here. That matters more
/// than it looks: `Malformed` alone covers nine decoder faults, so answering it
/// with one number would tell a caller that a payload was unreadable and not
/// whether the marker was missing, the length was absurd or a value had zero
/// width. Collapsing a wrapper is exactly the failure U6 is about, and it is the
/// shape the push path already avoids the same way.
const fn verify_code(err: VerifyError) -> u32 {
    match err {
        VerifyError::UnauthorisedSigner => 1601,
        VerifyError::ThresholdNotMet { .. } => 1602,
        VerifyError::AssetMismatch => 1603,
        VerifyError::StalePackage => 1604,
        VerifyError::FuturePackage => 1605,
        VerifyError::ValueOutOfRange => 1606,
        VerifyError::ScalingOutOfRange => 1607,
        VerifyError::TimestampMismatch { .. } => 1608,
        VerifyError::TooManyPackages { .. } => 1609,
        VerifyError::Malformed(err) => decode_code(err),
        VerifyError::InvalidSignature(err) => backend_code(err),
        VerifyError::NoClock(err) => clock_code(err),
        VerifyError::InvalidConfig(err) => config_code(err),
    }
}

/// One number per decoder fault, in the 1700 block.
const fn decode_code(err: DecodeError) -> u32 {
    match err {
        DecodeError::MissingMarker => 1701,
        DecodeError::Truncated => 1702,
        DecodeError::LengthOutOfRange => 1703,
        DecodeError::NumberOverflow => 1704,
        DecodeError::TooLong { .. } => 1705,
        DecodeError::NoDataPackages => 1706,
        DecodeError::NoDataPoints => 1707,
        DecodeError::ZeroWidthValue => 1708,
        DecodeError::TrailingBytes(_) => 1709,
    }
}

/// One number per signature-recovery fault, in the 1800 block.
const fn backend_code(err: BackendError) -> u32 {
    match err {
        BackendError::InvalidRecoveryId => 1801,
        BackendError::InvalidSignature => 1802,
        BackendError::RecoveryFailed => 1803,
    }
}

/// One number per clock fault, in the 1900 block.
///
/// `WrongAccount` is the one a caller of this program can actually cause, by
/// naming an account that is not the pinned every-block one.
const fn clock_code(err: TimeError) -> u32 {
    match err {
        TimeError::Missing => 1901,
        TimeError::WrongAccount => 1902,
        TimeError::Undecodable => 1903,
        TimeError::Unavailable => 1904,
    }
}

impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OwnerDidNotSign => f.write_str("the owner of this order did not sign"),
            Self::AlreadyOpen => f.write_str("an order is already open at this id's address"),
            Self::OrderAccountUnusable => {
                f.write_str("the account at this id's address is not one this program can write")
            }
            Self::UnknownFeed => f.write_str("this program serves no feed at that index"),
            Self::OrderTooLarge => f.write_str("the serialised order does not fit the account"),
        }
    }
}

impl core::fmt::Display for SettleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OrderNotOurs => f.write_str("the account offered as the order is not ours"),
            Self::OrderUndecodable => f.write_str("the order account's bytes are not an order"),
            Self::AlreadyFilled => f.write_str("this order has already been filled"),
            Self::UnknownFeed => f.write_str("this program serves no feed at that index"),
            Self::OrderTooLarge => f.write_str("the serialised order does not fit the account"),
            Self::LimitNotReached { price, limit } => {
                write!(f, "the verified price {price} is below the limit {limit}")
            }
            Self::Config(err) => write!(f, "{err:?}"),
            Self::Verify(err) => write!(f, "{err:?}"),
        }
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<OpenError> for spel_framework::error::SpelError {
    fn from(err: OpenError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<SettleError> for spel_framework::error::SpelError {
    fn from(err: SettleError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Opens an order, and returns the post-states in the instruction's account
/// order.
///
/// # Errors
///
/// [`OpenError`] when the owner did not sign, the address already holds
/// something, or no feed sits at `feed`.
pub fn open_order(
    order: AccountWithMetadata,
    owner: AccountWithMetadata,
    feed: u8,
    limit_price_q64: u128,
    order_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, OpenError> {
    if !owner.is_authorized {
        return Err(OpenError::OwnerDidNotSign);
    }
    if order.account != Account::default() {
        // Ours means an order was opened here; anything else means the address
        // is occupied by an account no program can write. Two causes because an
        // operator's next move differs: pick another id, or give up on this one.
        return Err(if order.account.program_owner == self_program_id {
            OpenError::AlreadyOpen
        } else {
            OpenError::OrderAccountUnusable
        });
    }
    if usize::from(feed) >= FEEDS.len() {
        return Err(OpenError::UnknownFeed);
    }

    let stored = OrderAccount {
        owner: *owner.account_id.value(),
        feed,
        limit_price_q64,
        filled: false,
    };

    let mut account = order.account;
    account.data = write(&stored).ok_or(OpenError::OrderTooLarge)?;

    Ok(vec![
        spel_framework::spel_output::AutoClaim::pda_from_seeds(&[
            &order_id,
            &spel_framework::pda::seed_from_str(ORDER_ACCOUNT_SEED),
        ])
        .to_post_state(account),
        AccountPostState::new(owner.account),
    ])
}

/// Fills an order if a payload this program verifies itself says the market has
/// reached its limit.
///
/// Permissionless: the sender is not checked, because the sender attests to
/// nothing. The signatures in `payload` are the only claim of authenticity, and
/// they are checked against [`SIGNERS`].
///
/// `clock` is the LEZ clock account the transaction supplied. Its id and its
/// bytes are read from the one struct, which is what binds them — see the module
/// header.
///
/// # Errors
///
/// [`SettleError`], one variant per cause, and every one of them leaves the
/// order unfilled. Nothing here falls back to a price it could not verify.
pub fn settle(
    order: AccountWithMetadata,
    clock: AccountWithMetadata,
    payload: &[u8],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SettleError> {
    // The order first, and every check here is a decode or a comparison: nothing
    // reads the clock or recovers a signature, so cheap refusals cost a caller
    // nothing beyond the payload it was already charged for reading (ADR 26).
    if order.account.program_owner != self_program_id {
        return Err(SettleError::OrderNotOurs);
    }
    let mut stored = OrderAccount::try_from_slice(order.account.data.as_ref())
        .map_err(|_| SettleError::OrderUndecodable)?;
    if stored.filled {
        return Err(SettleError::AlreadyFilled);
    }
    let spec = FEEDS
        .get(usize::from(stored.feed))
        .ok_or(SettleError::UnknownFeed)?;

    let verified = verify(spec, payload, &clock)?;
    if verified.price < stored.limit_price_q64 {
        return Err(SettleError::LimitNotReached {
            price: verified.price,
            limit: stored.limit_price_q64,
        });
    }

    stored.filled = true;
    let mut account = order.account;
    account.data = write(&stored).ok_or(SettleError::OrderTooLarge)?;

    // Both accounts, in the instruction's own order, and the clock unchanged.
    // `validate_execution` zips pre-states and post-states positionally and
    // requires equal length (rule 2), so an account this instruction only reads
    // still has to come back -- returning the order alone would have failed every
    // successful settlement on chain. `SpelOutput::execute` passes these through
    // and adds nothing. The push path returns its read-only feed and clock for the
    // same reason.
    //
    // No claim on either: this program already owns the order, and LEZ refuses a
    // claim on an account whose owner is not the default one.
    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(clock.account),
    ])
}

/// The verification itself, over the consumer's own configuration.
///
/// Separate from [`settle`] so the configuration is assembled in one place and
/// the only inputs are constants and the two accounts LEZ supplied. A reader
/// looking for what a caller can influence should find nothing here.
fn verify(
    spec: &FeedSpec,
    payload: &[u8],
    clock: &AccountWithMetadata,
) -> Result<VerifiedFeed, SettleError> {
    let signers: Vec<SignerAddress> = SIGNERS.iter().copied().map(SignerAddress).collect();
    let pair = AssetPair::new(spec.base_asset, spec.quote_asset);
    let config = PullConfig {
        data_service_id: DATA_SERVICE_ID,
        feed: FeedConfig::try_new(
            &spec.feed_id,
            pair,
            DECIMALS,
            MAX_AGE_MS,
            &signers,
            THRESHOLD,
        )?,
    };

    Ok(verify_price(
        payload,
        &config,
        &pair,
        clock.account_id.value(),
        clock.account.data.as_ref(),
        &InProgramBackend::new(),
    )?)
}

/// Serialises an order into an account's data, or `None` if it does not fit.
fn write(order: &OrderAccount) -> Option<Data> {
    Data::try_from(borsh::to_vec(order).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{AccountId, Nonce};
    use lee_core::program::{validate_execution, Claim};
    use spel_framework::pda::{compute_pda, seed_from_str};

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];
    const ORDER_ID: [u8; 32] = [0x0D; 32];
    const BTC: u8 = 0;

    fn order_address() -> AccountId {
        compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
    }

    /// An account nobody has touched, at the address `ORDER_ID` derives.
    fn unopened() -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: order_address(),
        }
    }

    fn owner(signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: SOMEONE_ELSE,
                balance: 500,
                data: Data::default(),
                nonce: Nonce(3),
            },
            is_authorized: signs,
            account_id: AccountId::new([0xA0; 32]),
        }
    }

    /// The account as the chain leaves it once a post-state is applied: the claim
    /// has been honoured, so the order is this program's.
    fn as_chain_leaves_it(post: &AccountPostState) -> AccountWithMetadata {
        let mut account = post.account().clone();
        account.program_owner = OURS;
        AccountWithMetadata {
            account,
            is_authorized: false,
            account_id: order_address(),
        }
    }

    fn opened(limit: u128) -> AccountWithMetadata {
        let posts = open_order(unopened(), owner(true), BTC, limit, ORDER_ID, OURS)
            .expect("a usable order");
        as_chain_leaves_it(&posts[0])
    }

    #[test]
    fn the_compiled_in_configuration_is_usable() {
        // `SettleError::Config` exists so a deployment mistake fails closed
        // rather than panicking in a guest. This is what makes it unreachable in
        // this build rather than merely unlikely: every input to
        // `FeedConfig::try_new` is a constant in this file, so if the constants
        // are usable the variant cannot be produced.
        let signers: Vec<SignerAddress> = SIGNERS.iter().copied().map(SignerAddress).collect();
        for spec in &FEEDS {
            FeedConfig::try_new(
                &spec.feed_id,
                AssetPair::new(spec.base_asset, spec.quote_asset),
                DECIMALS,
                MAX_AGE_MS,
                &signers,
                THRESHOLD,
            )
            .unwrap_or_else(|err| panic!("{:?} is not a usable feed: {err:?}", spec.feed_id));
        }
    }

    #[test]
    fn an_order_claims_the_address_a_client_derives() {
        // The claim and the guest's `pda` constraint have to agree, and they are
        // written in two files from two spellings of the same seed. Asserted
        // against an independent derivation rather than by comparing literals.
        let posts =
            open_order(unopened(), owner(true), BTC, 1, ORDER_ID, OURS).expect("a usable order");
        let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
            panic!("a first open claims a PDA");
        };
        assert_eq!(
            AccountId::for_public_pda(&OURS, &seed),
            order_address(),
            "the claim has to name the address the constraint checked"
        );
    }

    #[test]
    fn an_owner_that_did_not_sign_cannot_open_an_order() {
        assert_eq!(
            open_order(unopened(), owner(false), BTC, 1, ORDER_ID, OURS),
            Err(OpenError::OwnerDidNotSign)
        );
    }

    #[test]
    fn an_order_id_is_spent_once() {
        // Not a limitation worth hiding: LEZ rule 4 forbids this program giving
        // up the account, so re-opening would mean admitting a second pre-state
        // the way `register_feed` does for a retired feed. An order has no
        // reason to be reused, so the answer is a fresh id.
        let existing = opened(1);
        assert_eq!(
            open_order(existing, owner(true), BTC, 2, ORDER_ID, OURS),
            Err(OpenError::AlreadyOpen)
        );
    }

    #[test]
    fn a_squatted_order_address_is_refused_as_unusable_and_not_as_taken() {
        // An order's address derives from an id anybody can guess, so anybody can
        // put an account there by sending it one unit of balance. That account is
        // non-default with a default owner: LEZ refuses the data write, because
        // the pre-state is not default and this program is not the owner, and
        // refuses the post-state too, because the claim loop assigns ownership
        // only after validation. No instruction of any program can move it.
        //
        // Its own cause because the advice differs. `AlreadyOpen` means pick
        // another id; this means this id is gone.
        let mut squatted = unopened();
        squatted.account.balance = 1;

        assert_eq!(
            open_order(squatted, owner(true), BTC, 1, ORDER_ID, OURS),
            Err(OpenError::OrderAccountUnusable)
        );
    }

    #[test]
    fn an_index_outside_the_compiled_feeds_is_refused() {
        // The one thing instruction data may influence, and this is its bound. An
        // index is not a roster, and an out-of-range one is not a feed.
        let over = u8::try_from(FEEDS.len()).expect("five feeds fit in a u8");
        assert_eq!(
            open_order(unopened(), owner(true), over, 1, ORDER_ID, OURS),
            Err(OpenError::UnknownFeed)
        );
    }

    #[test]
    fn an_open_passes_lez_and_leaves_the_owner_alone() {
        // Rather than restating LEZ's rules: hand the post-states to the function
        // that enforces them. The owner is returned untouched, which is what
        // keeps rule 2's equal lengths and rule 8's balance total satisfied.
        let pre = vec![unopened(), owner(true)];
        let posts =
            open_order(unopened(), owner(true), BTC, 1, ORDER_ID, OURS).expect("a usable order");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts the open");
        assert_eq!(posts[1].account(), &owner(true).account);
    }

    #[test]
    fn an_account_this_program_does_not_own_is_not_an_order() {
        // Without this a caller supplies an account it controls whose bytes
        // decode as an order, which is a limit price of the caller's choosing.
        // An unopened account is the same refusal: it carries the default owner.
        let mut foreign = opened(1);
        foreign.account.program_owner = SOMEONE_ELSE;
        assert_eq!(
            settle(foreign, clock(1), &[], OURS),
            Err(SettleError::OrderNotOurs)
        );
        assert_eq!(
            settle(unopened(), clock(1), &[], OURS),
            Err(SettleError::OrderNotOurs)
        );
    }

    #[test]
    fn an_order_accounts_bytes_that_are_not_an_order_are_reported_as_such() {
        let mut wrong = opened(1);
        wrong.account.data = Data::try_from(vec![0xFF; 3]).expect("three bytes fit");
        assert_eq!(
            settle(wrong, clock(1), &[], OURS),
            Err(SettleError::OrderUndecodable)
        );
    }

    #[test]
    fn a_filled_order_refuses_a_second_fill_before_any_verification() {
        // Before, not after: the payload is empty here, so a run that verified
        // first would answer `Malformed` and this is what pins the order.
        let mut filled = opened(1);
        let mut stored =
            OrderAccount::try_from_slice(filled.account.data.as_ref()).expect("an order");
        stored.filled = true;
        filled.account.data = write(&stored).expect("an order fits");

        assert_eq!(
            settle(filled, clock(1), &[], OURS),
            Err(SettleError::AlreadyFilled)
        );
    }

    #[test]
    fn no_two_causes_answer_with_the_same_number() {
        // A caller that cannot tell two failures apart cannot act on either, and
        // `SpelError` carries one number. Every leaf cause is listed rather than
        // every enum value, so the configuration and verification blocks are
        // included -- they are where a collapse would actually happen.
        let mut codes: Vec<u32> = every_cause().iter().map(SettleError::code).collect();
        codes.extend(
            [
                OpenError::OwnerDidNotSign,
                OpenError::AlreadyOpen,
                OpenError::OrderAccountUnusable,
                OpenError::UnknownFeed,
                OpenError::OrderTooLarge,
            ]
            .iter()
            .map(OpenError::code),
        );

        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two causes share a number");
    }

    #[test]
    fn the_number_a_caller_sees_is_the_frameworks_offset_one() {
        // What reaches a caller is `SpelError::error_code`, not `code`. The
        // offset belongs to the framework and this is where it is pinned, so a
        // caller reading 7406 can look 1406 up.
        for cause in every_cause() {
            let spel: spel_framework::error::SpelError = cause.into();
            assert_eq!(spel.error_code(), 6000 + cause.code());
        }
    }

    /// Every leaf cause a settlement can report, listed rather than derived.
    ///
    /// `VerifyError` and `ConfigError` are `verifier-core`'s, so nothing here can
    /// enumerate them; the match in `verify_code` is what forces this list to
    /// grow, because it has no wildcard arm and stops compiling when a variant is
    /// added upstream.
    fn every_cause() -> Vec<SettleError> {
        let mut causes = vec![
            SettleError::OrderNotOurs,
            SettleError::OrderUndecodable,
            SettleError::AlreadyFilled,
            SettleError::UnknownFeed,
            SettleError::OrderTooLarge,
            SettleError::LimitNotReached { price: 1, limit: 2 },
        ];
        causes.extend(every_config_fault().map(SettleError::Config));
        causes.extend(
            [
                VerifyError::UnauthorisedSigner,
                VerifyError::ThresholdNotMet {
                    met: 1,
                    required: 3,
                },
                VerifyError::AssetMismatch,
                VerifyError::StalePackage,
                VerifyError::FuturePackage,
                VerifyError::ValueOutOfRange,
                VerifyError::ScalingOutOfRange,
                VerifyError::TimestampMismatch {
                    expected: 1,
                    found: 2,
                },
                VerifyError::TooManyPackages { max: 1 },
            ]
            .map(SettleError::Verify),
        );
        // Every member of each wrapped layer, not one apiece.
        causes.extend(
            [
                DecodeError::MissingMarker,
                DecodeError::Truncated,
                DecodeError::LengthOutOfRange,
                DecodeError::NumberOverflow,
                DecodeError::TooLong { len: 1, max: 0 },
                DecodeError::NoDataPackages,
                DecodeError::NoDataPoints,
                DecodeError::ZeroWidthValue,
                DecodeError::TrailingBytes(1),
            ]
            .map(|err| SettleError::Verify(VerifyError::Malformed(err))),
        );
        causes.extend(
            [
                BackendError::InvalidRecoveryId,
                BackendError::InvalidSignature,
                BackendError::RecoveryFailed,
            ]
            .map(|err| SettleError::Verify(VerifyError::InvalidSignature(err))),
        );
        causes.extend(
            [
                TimeError::Missing,
                TimeError::WrongAccount,
                TimeError::Undecodable,
                TimeError::Unavailable,
            ]
            .map(|err| SettleError::Verify(VerifyError::NoClock(err))),
        );
        causes
    }

    /// Every configuration fault `FeedConfig::try_new` can report.
    fn every_config_fault() -> [ConfigError; 11] {
        [
            ConfigError::MaxAgeZero,
            ConfigError::MaxAgeTooLarge {
                max_age_ms: 1,
                max: 0,
            },
            ConfigError::DecimalsOutOfRange {
                decimals: 1,
                max: 0,
            },
            ConfigError::NoSigners,
            ConfigError::ThresholdZero,
            ConfigError::TooManySigners { signers: 1, max: 0 },
            ConfigError::ThresholdExceedsSigners {
                threshold: 1,
                signers: 0,
            },
            ConfigError::ZeroSignerAddress,
            ConfigError::DuplicateSigner,
            ConfigError::FeedIdTooLong { len: 33 },
            ConfigError::ZeroFeedId,
        ]
    }

    #[test]
    fn a_configuration_fault_has_one_number_by_either_route() {
        // Two routes reach a `ConfigError`: this program failing to build its own
        // configuration, and `verify_feed` refusing one. They are the same fault
        // and get the same number, which is why the counting test above lists each
        // leaf once rather than once per route -- a shared number there would look
        // like a collision.
        //
        // Asserted rather than left to the reader, because the alternative is a
        // second number for one cause, and an operator comparing two failed
        // transactions would have no way to know they had hit the same thing.
        for fault in every_config_fault() {
            assert_eq!(
                SettleError::Config(fault).code(),
                SettleError::Verify(VerifyError::InvalidConfig(fault)).code(),
                "{fault:?} answers with two numbers"
            );
        }
    }

    /// The clock account's sixteen bytes: `block_id` then `timestamp`, both
    /// little-endian, at the pinned id.
    fn clock(now_ms: u64) -> AccountWithMetadata {
        let mut data = [0u8; 16];
        data[8..].copy_from_slice(&now_ms.to_le_bytes());
        AccountWithMetadata {
            account: Account {
                program_owner: [88u32; 8],
                balance: 0,
                data: Data::try_from(data.to_vec()).expect("sixteen bytes fit"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: AccountId::new(pull_lib::CLOCK_ACCOUNT_ID),
        }
    }
}
