//! How a consumer should verify a RedStone payload inline.
//!
//! A limit order, in the smallest program that can hold one and still govern
//! what it trusts. [`order::open_order`] records a price somebody is prepared to
//! trade at; [`order::settle`] fills it if a payload the program verifies itself
//! says the market has reached it. The domain is thin on purpose — what a reader
//! should take from this crate is the seam between a LEZ program and
//! [`pull_lib`], not the trade.
//!
//! Four things that seam has to get right, and each is the reason a module here
//! looks the way it does.
//!
//! # 1. The signer set is the consumer's, and the consumer is not the caller
//!
//! The roster, the threshold, the window, the scale and the pair live in a
//! [`trust::FeedTrust`] account this program owns, one per feed, written only by
//! an instruction the [`authority`] gate stands in front of. Nothing a caller
//! sends reaches any of it: [`order::settle`] takes an order account, a trust
//! account, the clock and a payload, and writes none of the three it reads for
//! configuration.
//!
//! That is the whole security model of pull mode (SEC2), and it is the half
//! `pull-lib` cannot check. The roster arrives there as a slice, and a slice
//! built from program-owned state is indistinguishable from one built from the
//! transaction's own instruction data: a consumer that read its signers off the
//! wire would satisfy every signature in that crate and have handed its caller
//! the right to speak for the feed.
//!
//! **A consumer registers nothing with the aggregator, which is not the same as
//! registering nothing.** F9 asks for a pull path with no dependency on the
//! aggregator's price account, and this crate has none — no aggregator crate is
//! in its build closure and CI asserts it. What it does have is state of its
//! own, governed by its own authority, because the alternative is worse than
//! inconvenient: a compiled-in roster makes RedStone's next signer rotation a
//! redeployment, and in LEZ a redeployment moves every derived address and
//! abandons every open order. `[M3-06:02]` records that and what it replaced.
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
//! One `AccountWithMetadata`, two fields of it. The id and the data cannot
//! disagree because neither is constructed — LEZ handed over both, as one
//! account. `[M3-01:01]` records the residual this closes.
//!
//! # 3. Every typed error refuses the action
//!
//! [`order::SettleError`] enumerates every way a settlement can fail, with a
//! distinct code per leaf cause, and there is no arm that returns a price
//! anyway. A stale payload does not fill at the last price it saw; an
//! unauthorised signer does not fill at a smaller quorum; a malformed payload
//! does not fill at zero. **Refusing is the only thing this program does when it
//! cannot verify a price** (U7), and it refuses by returning an error rather
//! than post-states that happen to change nothing — a caller must be able to
//! tell "not filled" from "could not tell".
//!
//! The numbers are this program's own and are not the aggregator's. What U6 asks
//! to be shared is the typed value: a cause is one [`pull_lib::VerifyError`]
//! variant whichever mode meets it, because both modes call one verifier, and
//! M3-04 asserts that identity over the same inputs. An integer is a program's
//! interface to its own callers, so a consumer numbering its causes in its own
//! space is not a second taxonomy. What would be one is a consumer that
//! collapsed a taxonomy into a single code — including the four `VerifyError`
//! variants that carry another error, each of which dispatches into that layer's
//! block here rather than reporting its whole layer as one number:
//!
//! | block | causes |
//! |---|---|
//! | 1200 | the authority gate |
//! | 1300 | opening an order |
//! | 1400 | settling one |
//! | 1500 | a configuration fault, wherever it surfaces |
//! | 1600 | a verification failure `VerifyError` names itself |
//! | 1700 | the decoder |
//! | 1800 | signature recovery |
//! | 1900 | the clock |
//! | 2000 | registering or rotating a feed's trust |
//!
//! # 4. Where the payload came from is not this program's business, and is
//! somebody's
//!
//! [`order::settle`] verifies bytes it was handed. It cannot know, and does not
//! ask, how they reached the transaction — which is correct, because a payload is
//! either signed by enough authorised keys or it is not, whoever carried it.
//!
//! That leaves a choice for whoever integrates this, and it is not a neutral one.
//! Fetching from RedStone's gateway at read time is the obvious route and it
//! exposes the fetcher's network address to RedStone on every read, which on a
//! privacy-first chain is worth a decision rather than a default. Taking the
//! bytes from a relayer that fetched them, or from instruction data a caller
//! already assembled, moves that exposure somewhere it may be more acceptable —
//! and costs nothing in authenticity, since the signatures are checked here
//! either way.
//!
//! Named here because this is where a consumer author meets it. The
//! recommendation belongs to the SDK doc packet (S5) and the end-to-end README
//! (S4), and neither is written yet.
//!
//! # 5. Verification is not authorisation
//!
//! [`order::settle`] is permissionless: anybody may present a payload for
//! anybody's order, exactly as anybody may submit a price to the push
//! aggregator. Nothing about the sender is checked, because the sender attests
//! to nothing — the signatures in the payload are the only claim of authenticity
//! there is, and they are RedStone's rather than the sender's.
//!
//! [`order::open_order`] requires the owner's signature, because opening an
//! order is a statement about the owner. The instructions in [`trust`] require
//! the authority's, because what this program trusts is a statement about the
//! program.
#![forbid(unsafe_code)]

pub mod authority;
pub mod order;
pub mod trust;

pub use authority::{AuthorityError, ConfigAccount, CONFIG_ACCOUNT_SEED};
pub use order::{open_order, settle, OpenError, OrderAccount, SettleError, ORDER_ACCOUNT_SEED};
pub use trust::{FeedTrust, TrustError, TRUST_ACCOUNT_SEED};

use pull_lib::{BackendError, ConfigError, DecodeError, TimeError, VerifyError};

/// Right-pads a name to the 32 bytes both a feed id and an asset id travel in.
///
/// `padded(b"BTC")` is the feed id RedStone signs for BTC. Public because a
/// client building a registration has to produce the same bytes, and every place
/// that spelled the padding out by hand was a place it could drift.
#[must_use]
pub const fn padded(name: &[u8]) -> [u8; 32] {
    assert!(name.len() <= 32, "an identifier is at most 32 bytes");
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < name.len() {
        out[i] = name[i];
        i += 1;
    }
    out
}

/// One number per configuration fault, in the 1500 block.
///
/// Reached three ways — a registration refused, a rotation refused, and
/// `VerifyError::InvalidConfig` — and all of them arrive here, because one cause
/// reached by several routes is still one cause and deserves one number.
#[must_use]
pub const fn config_code(err: ConfigError) -> u32 {
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
#[must_use]
pub const fn verify_code(err: VerifyError) -> u32 {
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
#[must_use]
pub const fn decode_code(err: DecodeError) -> u32 {
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
#[must_use]
pub const fn backend_code(err: BackendError) -> u32 {
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
#[must_use]
pub const fn clock_code(err: TimeError) -> u32 {
    match err {
        TimeError::Missing => 1901,
        TimeError::WrongAccount => 1902,
        TimeError::Undecodable => 1903,
        TimeError::Unavailable => 1904,
    }
}

/// Account builders the module tests share.
///
/// Here rather than duplicated three times, and `cfg(test)` so none of it
/// reaches a consumer that links this crate — the reason ADR 21 gives for
/// keeping `verifier-core`'s payload builder behind the same gate.
#[cfg(test)]
pub(crate) mod testing {
    use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
    use lee_core::program::{AccountPostState, ProgramId};

    pub const OURS: ProgramId = [7u32; 8];
    pub const SOMEONE_ELSE: ProgramId = [9u32; 8];
    pub const GENESIS: [u8; 32] = [0x61; 32];

    /// A key with a real account behind it: owned by some wallet program, with a
    /// balance and a nonce.
    pub fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: SOMEONE_ELSE,
                balance: 500,
                data: Data::default(),
                nonce: Nonce(3),
            },
            is_authorized: signs,
            account_id: AccountId::new(id),
        }
    }

    /// An account nobody has touched, at `at`.
    pub fn untouched(at: AccountId) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: at,
        }
    }

    /// The account as the chain leaves it once a post-state is applied: the claim
    /// has been honoured, so it is this program's.
    pub fn as_chain_leaves_it(post: &AccountPostState, at: AccountId) -> AccountWithMetadata {
        let mut account = post.account().clone();
        account.program_owner = OURS;
        AccountWithMetadata {
            account,
            is_authorized: false,
            account_id: at,
        }
    }

    /// The config account with `GENESIS` established as the authority.
    pub fn established_config() -> AccountWithMetadata {
        let at = crate::authority::config_address(&OURS);
        let posts = crate::authority::establish(untouched(at), key(GENESIS, true), &GENESIS, OURS)
            .expect("the genesis key establishes it");
        as_chain_leaves_it(&posts[0], at)
    }

    /// The LEZ clock account: `block_id` then `timestamp`, both little-endian, at
    /// the pinned id.
    pub fn clock_at(ms: u64) -> AccountWithMetadata {
        clock_account(pull_lib::CLOCK_ACCOUNT_ID, ms)
    }

    pub fn clock_account(id: [u8; 32], ms: u64) -> AccountWithMetadata {
        let mut data = [0u8; 16];
        data[8..].copy_from_slice(&ms.to_le_bytes());
        AccountWithMetadata {
            account: Account {
                program_owner: [88u32; 8],
                balance: 0,
                data: Data::try_from(data.to_vec()).expect("sixteen bytes fit"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: AccountId::new(id),
        }
    }
}
