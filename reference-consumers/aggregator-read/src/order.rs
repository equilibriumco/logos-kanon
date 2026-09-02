//! The domain, which is thin on purpose.
//!
//! A limit order: [`open_order`] records a price somebody is prepared to trade
//! at, [`settle`] fills it if the price the aggregator published has reached it.
//! Deliberately the same domain reference consumer B holds, so the two crates
//! differ in one thing and a reader can diff them.
//!
//! # An order id is spent once
//!
//! LEZ rule 4 forbids a program giving up ownership of an account and rule 3
//! forbids resetting its nonce, so an order account stays this program's after a
//! fill and cannot be handed back as `Account::default()`. An order has no
//! reason to be reused, so re-opening one is not admitted and the cost is a
//! fresh id.
//!
//! # What binds an order, and what does not
//!
//! `terms_still_hold` destructures a whole [`PriceSource`] with no `..`, so a
//! field added to it stops this file compiling until somebody decides whether it
//! binds an open order. Two of the five do. The aggregator's id does not, and
//! that is the decision worth stating: following an aggregator rebuild moves
//! where the price is read from and not what it means, so an order opened before
//! one settles after it.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use kanon_clock::LezClock;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

use crate::read::{read_price, ReadError};
use crate::source::{self, PriceSource, SourceError};

/// The name seed an order account's address is derived from.
pub const ORDER_ACCOUNT_SEED: &str = "KANON_READ_ORDER";

/// One open order: whose it is, what it is against, and the price it fills at.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct OrderAccount {
    /// The account that opened this order and signed for it.
    pub owner: [u8; 32],
    /// The RedStone feed this order is priced against.
    pub feed_id: [u8; 32],
    /// The base asset the owner expected when it signed.
    ///
    /// From the owner's own instruction data, not copied from the source
    /// account. That difference is the point: it is the third record, written by
    /// a different party, which is what makes the pair comparisons real rather
    /// than a value held against itself.
    pub base_asset: [u8; 32],
    /// The quote asset the owner expected when it signed.
    pub quote_asset: [u8; 32],
    /// The staleness window the order was priced under.
    ///
    /// Captured from the registration rather than supplied, because it is the
    /// program's parameter rather than a claim the owner would know to make.
    /// Binding all the same: an owner that accepted a one-minute-old price did
    /// not accept an hour-old one.
    pub max_age_ms: u64,
    /// The price at or above which the order fills, on RFP-019's `Q64.64` scale.
    ///
    /// No `decimals` beside it, unlike reference consumer B's order: the scale
    /// of a published price is fixed by the account standard, so there is no
    /// per-feed exponent that could move under an open order.
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
    AlreadyOpen,
    /// The account at this id's address is not one this program can write.
    OrderAccountUnusable,
    /// The pair the owner signed for is not the one registered for the feed.
    PairMismatch,
    /// The order does not fit in an account's data.
    OrderTooLarge,
    /// The feed's price source could not be read.
    Source(SourceError),
}

impl OpenError {
    /// A stable number per leaf cause, in this program's 1300 block.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::OwnerDidNotSign => 1301,
            Self::AlreadyOpen => 1302,
            Self::OrderAccountUnusable => 1303,
            Self::PairMismatch => 1304,
            Self::OrderTooLarge => 1305,
            Self::Source(err) => err.code(),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerDidNotSign => f.write_str("the order's owner did not sign"),
            Self::AlreadyOpen => f.write_str("an order is already open at that id"),
            Self::OrderAccountUnusable => {
                f.write_str("that order id's address holds an account no program can write")
            }
            Self::PairMismatch => {
                f.write_str("that is not the pair this feed's source is registered for")
            }
            Self::OrderTooLarge => f.write_str("the order does not fit in an account"),
            Self::Source(err) => write!(f, "{err}"),
        }
    }
}

impl From<SourceError> for OpenError {
    fn from(err: SourceError) -> Self {
        Self::Source(err)
    }
}

impl From<OpenError> for spel_framework::error::SpelError {
    fn from(err: OpenError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Why an order was not filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleError {
    /// The account offered as an order is not one this program owns.
    OrderNotOurs,
    /// The account's bytes are not an order.
    OrderUndecodable,
    /// The order has already been filled.
    AlreadyFilled,
    /// The order is against a feed other than the one named.
    OrderIsForAnotherFeed,
    /// The order no longer fits in an account's data.
    OrderTooLarge,
    /// The published price is below the order's limit.
    ///
    /// The only refusal here that is not a fault: the market has not reached
    /// the limit, and the caller should try again later. Every other variant
    /// means nobody could tell what the price was, which is why they are
    /// different numbers.
    LimitNotReached {
        /// What the aggregator published.
        price: u128,
        /// What the order asked for.
        limit: u128,
    },
    /// The source's window is no longer the one the order was priced under.
    WindowChanged {
        /// What the order recorded.
        was: u64,
        /// What the source says now.
        now: u64,
    },
    /// The source's pair is no longer the one the order was opened against.
    ///
    /// Reachable because a source is retirable: an authority may retire a feed
    /// and register the id again under another pair, and an order opened before
    /// that must not fill under the new meaning.
    PairChanged,
    /// The feed's price source could not be read.
    Source(SourceError),
    /// The price account could not be read.
    Read(ReadError),
}

impl SettleError {
    /// A stable number per leaf cause, in this program's 1400 block.
    ///
    /// The two wrappers dispatch into their own layers rather than reporting a
    /// layer as one number.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::OrderNotOurs => 1401,
            Self::OrderUndecodable => 1402,
            Self::AlreadyFilled => 1403,
            Self::OrderIsForAnotherFeed => 1404,
            Self::OrderTooLarge => 1405,
            Self::LimitNotReached { .. } => 1406,
            Self::WindowChanged { .. } => 1407,
            Self::PairChanged => 1408,
            Self::Source(err) => err.code(),
            Self::Read(err) => err.code(),
        }
    }
}

impl fmt::Display for SettleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderNotOurs => f.write_str("the account offered as an order is not ours"),
            Self::OrderUndecodable => f.write_str("the account's bytes are not an order"),
            Self::AlreadyFilled => f.write_str("that order has already been filled"),
            Self::OrderIsForAnotherFeed => f.write_str("that order is against another feed"),
            Self::OrderTooLarge => f.write_str("the order no longer fits in an account"),
            Self::LimitNotReached { price, limit } => {
                write!(f, "the published price {price} is below the limit {limit}")
            }
            Self::WindowChanged { was, now } => write!(
                f,
                "the order was priced under a {was}ms window and the source now says {now}ms"
            ),
            Self::PairChanged => {
                f.write_str("the source no longer prices the pair this order was opened against")
            }
            Self::Source(err) => write!(f, "{err}"),
            Self::Read(err) => write!(f, "{err}"),
        }
    }
}

impl From<SourceError> for SettleError {
    fn from(err: SourceError) -> Self {
        Self::Source(err)
    }
}

impl From<ReadError> for SettleError {
    fn from(err: ReadError) -> Self {
        Self::Read(err)
    }
}

impl From<SettleError> for spel_framework::error::SpelError {
    fn from(err: SettleError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Opens an order against a feed this program already has a source for.
///
/// # Errors
///
/// [`OpenError`] when the owner did not sign, the address already holds
/// something, or no source is registered for `feed_id`.
#[expect(
    clippy::too_many_arguments,
    reason = "the owner's expected pair is what makes the settlement's pair check real, and it has to be named separately from the feed it is checked against"
)]
pub fn open_order(
    order: AccountWithMetadata,
    owner: AccountWithMetadata,
    source: AccountWithMetadata,
    feed_id: [u8; 32],
    base_asset: [u8; 32],
    quote_asset: [u8; 32],
    limit_price_q64: u128,
    order_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, OpenError> {
    if !owner.is_authorized {
        return Err(OpenError::OwnerDidNotSign);
    }
    if order.account != Account::default() {
        return Err(if order.account.program_owner == self_program_id {
            OpenError::AlreadyOpen
        } else {
            OpenError::OrderAccountUnusable
        });
    }

    // An order against a feed nobody registered could never settle, and the
    // source account's own address is derived from `feed_id`, so this is also
    // what ties the id the order stores to a registration that exists.
    let registered = source::read(&source, self_program_id)?;
    if registered.feed_id != feed_id {
        return Err(OpenError::Source(SourceError::FeedMismatch));
    }
    // The owner's expectation against the authority's registration, once, here.
    // Nothing the aggregator publishes says which assets a feed really prices --
    // no signer attests to it -- so a source labelling RedStone's `BTC` feed as
    // ETH/USD would read real BTC prices and fill an order whose owner believed
    // it was trading ether. The owner is the only party that can catch it.
    if registered.base_asset != base_asset || registered.quote_asset != quote_asset {
        return Err(OpenError::PairMismatch);
    }

    let stored = OrderAccount {
        owner: *owner.account_id.value(),
        feed_id,
        base_asset,
        quote_asset,
        max_age_ms: registered.max_age_ms,
        limit_price_q64,
        filled: false,
    };

    let mut account = order.account;
    account.data = write(&stored).ok_or(OpenError::OrderTooLarge)?;

    Ok(vec![
        AutoClaim::pda_from_seeds(&[&order_id, &seed_from_str(ORDER_ACCOUNT_SEED)])
            .to_post_state(account),
        AccountPostState::new(owner.account),
        AccountPostState::new(source.account),
    ])
}

/// Fills an order if the price the aggregator published has reached its limit.
///
/// Permissionless: the sender is not checked, because the sender attests to
/// nothing. What makes the price trustworthy is that the account it was read
/// from is the one this feed's source derives, under an aggregator this
/// program's authority named.
///
/// `clock` is the LEZ clock account the transaction supplied. Its id and its
/// bytes are read from the one struct, which is what binds them.
///
/// # Errors
///
/// [`SettleError`], one variant per cause, and every one of them leaves the
/// order unfilled. Nothing here falls back to a price it could not read.
pub fn settle(
    order: AccountWithMetadata,
    source: AccountWithMetadata,
    price: AccountWithMetadata,
    clock: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SettleError> {
    // The accounts first: every check down to the clock read is a decode or a
    // comparison, so a settlement that cannot happen is refused before anything
    // is paid for hashing an address.
    if order.account.program_owner != self_program_id {
        return Err(SettleError::OrderNotOurs);
    }
    let mut stored = OrderAccount::try_from_slice(order.account.data.as_ref())
        .map_err(|_| SettleError::OrderUndecodable)?;
    if stored.filled {
        return Err(SettleError::AlreadyFilled);
    }
    if stored.feed_id != feed_id {
        return Err(SettleError::OrderIsForAnotherFeed);
    }

    let registered = source::read(&source, self_program_id)?;
    if registered.feed_id != feed_id {
        return Err(SettleError::Source(SourceError::FeedMismatch));
    }

    terms_still_hold(&registered, &stored)?;

    let now = LezClock::from_account(clock.account_id.value(), clock.account.data.as_ref())
        .map_err(ReadError::Clock)?;
    let published = read_price(&registered, &price, &now)?;
    if published.price_q64 < stored.limit_price_q64 {
        return Err(SettleError::LimitNotReached {
            price: published.price_q64,
            limit: stored.limit_price_q64,
        });
    }

    stored.filled = true;
    let mut account = order.account;
    account.data = write(&stored).ok_or(SettleError::OrderTooLarge)?;

    // All four accounts, in the instruction's own order, with the source, the
    // price account and the clock unchanged. `validate_execution` zips
    // pre-states and post-states positionally and requires equal length (rule
    // 2), so an account this instruction only reads still has to come back.
    //
    // No claim on any of them: this program already owns the order and the
    // source, it does not own the price account and never will, and LEZ refuses
    // a claim on an account whose owner is not the default one.
    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(source.account),
        AccountPostState::new(price.account),
        AccountPostState::new(clock.account),
    ])
}

/// Refuses a settlement whose registration no longer describes the order's terms.
///
/// Every field of a [`PriceSource`] is accounted for here, in one destructuring
/// with no `..`, so a field added to it stops this compiling until somebody
/// decides whether it binds an open order.
///
/// Cheap, and before the clock is read, so nothing is paid for a settlement that
/// cannot happen.
fn terms_still_hold(registered: &PriceSource, order: &OrderAccount) -> Result<(), SettleError> {
    let PriceSource {
        // Checked in `settle` against the order and against the instruction's own
        // argument, which the guest constrains this account's address to.
        feed_id: _,
        // Deliberately mutable under an open order. An aggregator rebuild moves
        // where a price is read from, not what it means, and refusing to settle
        // across one would make every aggregator fix cost the order book.
        aggregator: _,
        // Bound. A retirement and a re-registration can put another pair behind
        // the same feed id, and an order is a claim about a price for a
        // particular pair.
        base_asset,
        quote_asset,
        // Bound. It decides how old a published price may be, which is not
        // something an owner agreed to have moved after signing.
        max_age_ms,
    } = registered;

    if *base_asset != order.base_asset || *quote_asset != order.quote_asset {
        return Err(SettleError::PairChanged);
    }
    if *max_age_ms != order.max_age_ms {
        return Err(SettleError::WindowChanged {
            was: order.max_age_ms,
            now: *max_age_ms,
        });
    }
    Ok(())
}

fn write(order: &OrderAccount) -> Option<Data> {
    Data::try_from(borsh::to_vec(order).ok()?).ok()
}
