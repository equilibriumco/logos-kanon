//! An order, and the settlement that fills it.
//!
//! The action this consumer gates on a verified price. [`open_order`] is the
//! owner's statement; [`settle`] is anybody's presentation of a payload against
//! it.
//!
//! # An order fixes what it is priced against
//!
//! It stores the feed id, and [`settle`] requires the trust account it is handed
//! to be that feed's, so an order opened against BTC cannot be filled from an ETH
//! price by passing a different trust account.
//!
//! It also stores the terms it was priced under — the pair its owner signed for,
//! the scale its limit is on, and the staleness window it accepted — and
//! [`settle`] refuses if the registration no longer matches. That is not
//! belt-and-braces. [`crate::trust::rotate_signers`] moves only the roster, but a
//! registration can be retired and made again on different terms, which is the
//! recovery path for a mistake; without this an order would be filled under a
//! window or an exponent its owner never accepted.
//!
//! What an authority *can* change under an open order is who may speak for its
//! feed. That is the point of having an authority at all, it is deliberately not
//! part of an order's terms, and it is the same exposure a push consumer has to
//! `update_signer_set`.
//!
//! # An order id is spent once
//!
//! LEZ rule 4 forbids a program giving up ownership of an account and rule 3
//! forbids resetting its nonce, so an order account stays this program's after a
//! fill and cannot be handed back as `Account::default()`. Re-opening it would
//! mean admitting a second pre-state the way the push path's `register_feed`
//! does for a retired feed; an order has no reason to be reused, so it is not
//! admitted and the cost is a fresh id.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use pull_lib::{verify_price, AssetPair, InProgramBackend, VerifiedFeed, VerifyError};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

use crate::trust::{self, FeedTrust, TrustError};

/// The name seed an order account's address is derived from.
pub const ORDER_ACCOUNT_SEED: &str = "KANON_PULL_ORDER";

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
    /// Recorded from the owner's own instruction data, not copied from the trust
    /// account, and that difference is the whole point: it is the second,
    /// independently written record that makes `verify_feed`'s pair comparison a
    /// real check rather than a value compared against itself. See
    /// [`open_order`].
    pub base_asset: [u8; 32],
    /// The quote asset the owner expected when it signed.
    pub quote_asset: [u8; 32],
    /// The scale [`Self::limit_price_q64`] was priced on.
    ///
    /// Captured from the registration rather than supplied, because it is the
    /// program's parameter and not a claim the owner would know to make. Bound to
    /// the order all the same: the exponent decides what a payload's integer means,
    /// so the same wire value under a different `decimals` is a different price,
    /// and a limit compared against it would mean something its owner never chose.
    pub decimals: u8,
    /// The staleness window the order was priced under.
    ///
    /// Also captured, and also binding: an owner that accepted a one-minute-old
    /// price did not accept a fifteen-minute-old one.
    pub max_age_ms: u64,
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
    AlreadyOpen,
    /// The account at this id's address is not one this program can write.
    ///
    /// Non-default and unowned: an order's address derives from an id anybody
    /// can guess, so anybody can put an account there by sending it one unit of
    /// balance, after which no instruction of any program can move it. Its own
    /// cause because the advice differs — `AlreadyOpen` means pick another id,
    /// this means this id is gone.
    OrderAccountUnusable,
    /// This program trusts no feed by that id, so an order against it could
    /// never be settled.
    ///
    /// Refused at the open rather than discovered at every settlement, where the
    /// owner can still do something about it.
    Trust(TrustError),
    /// The pair the owner expected is not the pair this program has registered
    /// for that feed.
    ///
    /// The owner's protection, taken at the moment it commits. A registration
    /// that labelled RedStone's `BTC` feed as ETH/USD would otherwise verify real
    /// BTC packages and fill an order its owner believed was for ether — and no
    /// signer attests to which assets a feed prices, so nothing downstream could
    /// catch it. Refused here, where the owner can still walk away.
    PairMismatch,
    /// The serialised order does not fit an account's data.
    ///
    /// Unreachable: an order is 154 bytes against a 100 KiB limit, and every field
    /// is fixed-width so that is the size rather than a maximum.
    /// `an_order_is_far_smaller_than_an_accounts_data_limit` measures it, so the
    /// figure this variant's reachability rests on is not one anybody counted. Kept
    /// because the alternative is `expect`, and a panic in a guest aborts the
    /// transaction instead of refusing the instruction.
    OrderTooLarge,
}

impl From<TrustError> for OpenError {
    fn from(err: TrustError) -> Self {
        Self::Trust(err)
    }
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
            Self::Trust(err) => err.code(),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerDidNotSign => f.write_str("the owner of this order did not sign"),
            Self::AlreadyOpen => f.write_str("an order is already open at this id's address"),
            Self::OrderAccountUnusable => {
                f.write_str("the account at this id's address is not one this program can write")
            }
            Self::PairMismatch => {
                f.write_str("the pair you expected is not the one registered for that feed")
            }
            Self::OrderTooLarge => f.write_str("the serialised order does not fit the account"),
            Self::Trust(err) => write!(f, "{err}"),
        }
    }
}

impl From<OpenError> for spel_framework::error::SpelError {
    fn from(err: OpenError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Why a settlement was refused, and every one of them refuses it.
///
/// A distinct [`Self::code`] per leaf cause: a caller that cannot tell two
/// failures apart cannot act on either, and `SpelError` carries one number, so
/// without the dispatch below every verification failure would arrive as one.
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
    /// The order is against a different feed from the one this instruction
    /// named.
    ///
    /// The order is what decides, not the argument: an order priced against BTC
    /// is not settleable from an ETH payload however the accounts are arranged.
    OrderIsForAnotherFeed,
    /// The trust account is not this program's registration for that feed.
    Trust(TrustError),
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
    /// answer was no. It carries both numbers, because "not yet" and "never"
    /// look the same to a caller told only that it was refused.
    LimitNotReached {
        /// What the payload verified to, on the `Q64.64` scale.
        price: u128,
        /// What the order requires.
        limit: u128,
    },
    /// The feed's scale has changed since the order was priced.
    ///
    /// Reachable through the recovery path and only through it: only the roster
    /// and its threshold rotate, but the authority may retire a feed and register
    /// the id again with a different exponent. An order's limit is on the scale it
    /// was priced on, so filling it against another one would answer a question
    /// its owner never asked.
    ScaleChanged {
        /// The scale the order was priced on.
        priced_at: u8,
        /// The scale registered now.
        registered: u8,
    },
    /// The feed's staleness window has changed since the order was priced.
    ///
    /// Same route and the same reasoning. An owner that accepted a one-minute-old
    /// price did not thereby accept a fifteen-minute-old one, and widening the
    /// window under an open order would do exactly that.
    WindowChanged {
        /// The window the order was priced under.
        priced_at: u64,
        /// The window registered now.
        registered: u64,
    },
    /// The serialised order does not fit an account's data. See
    /// [`OpenError::OrderTooLarge`].
    OrderTooLarge,
}

impl From<TrustError> for SettleError {
    fn from(err: TrustError) -> Self {
        Self::Trust(err)
    }
}

impl From<VerifyError> for SettleError {
    fn from(err: VerifyError) -> Self {
        Self::Verify(err)
    }
}

impl SettleError {
    /// A stable number per leaf cause: this instruction's own in the 1400 block,
    /// then one block per layer beneath it. `crate`'s module header carries the
    /// table.
    ///
    /// *Leaf* is the load-bearing word, and [`crate::verify_code`] is where it
    /// is earned: a wrapper answered with one number would report nine decoder
    /// faults as "unreadable". Every match involved is exhaustive with no
    /// wildcard arm, so a variant added anywhere in the taxonomy stops this
    /// compiling until somebody decides what a consumer should tell its callers.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::OrderNotOurs => 1401,
            Self::OrderUndecodable => 1402,
            Self::AlreadyFilled => 1403,
            Self::OrderIsForAnotherFeed => 1404,
            Self::OrderTooLarge => 1405,
            Self::LimitNotReached { .. } => 1406,
            Self::ScaleChanged { .. } => 1407,
            Self::WindowChanged { .. } => 1408,
            Self::Trust(err) => err.code(),
            Self::Verify(err) => crate::verify_code(*err),
        }
    }
}

impl fmt::Display for SettleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderNotOurs => f.write_str("the account offered as the order is not ours"),
            Self::OrderUndecodable => f.write_str("the order account's bytes are not an order"),
            Self::AlreadyFilled => f.write_str("this order has already been filled"),
            Self::OrderIsForAnotherFeed => {
                f.write_str("this order is priced against a different feed")
            }
            Self::OrderTooLarge => f.write_str("the serialised order does not fit the account"),
            Self::LimitNotReached { price, limit } => {
                write!(f, "the verified price {price} is below the limit {limit}")
            }
            Self::ScaleChanged {
                priced_at,
                registered,
            } => write!(
                f,
                "this order was priced at {priced_at} decimals and the feed now reports {registered}"
            ),
            Self::WindowChanged {
                priced_at,
                registered,
            } => write!(
                f,
                "this order was priced under a {priced_at} ms window and the feed now allows {registered}"
            ),
            Self::Trust(err) => write!(f, "{err}"),
            Self::Verify(err) => write!(f, "{err:?}"),
        }
    }
}

impl From<SettleError> for spel_framework::error::SpelError {
    fn from(err: SettleError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Opens an order against a feed this program already trusts.
///
/// # Errors
///
/// [`OpenError`] when the owner did not sign, the address already holds
/// something, or no trust is registered for `feed_id`.
#[expect(
    clippy::too_many_arguments,
    reason = "the owner's expected pair is what makes the settlement's pair check real, and it has to be named separately from the feed it is checked against"
)]
pub fn open_order(
    order: AccountWithMetadata,
    owner: AccountWithMetadata,
    trust: AccountWithMetadata,
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
    // trust account's own address is derived from `feed_id`, so this is also
    // what ties the id the order stores to a registration that exists.
    let registered = trust::read(&trust, self_program_id)?;
    if registered.feed_id != feed_id {
        return Err(OpenError::Trust(TrustError::FeedMismatch));
    }
    // The owner's expectation against the program's registration, once, here.
    // Nothing on the wire says which assets a feed prices -- no signer attests to
    // it -- so this comparison is the only place a mislabelled registration can
    // be caught, and the owner is the party that has to catch it.
    if registered.pair() != AssetPair::new(base_asset, quote_asset) {
        return Err(OpenError::PairMismatch);
    }

    let stored = OrderAccount {
        owner: *owner.account_id.value(),
        feed_id,
        base_asset,
        quote_asset,
        decimals: registered.decimals,
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
        AccountPostState::new(trust.account),
    ])
}

/// Fills an order if a payload this program verifies itself says the market has
/// reached its limit.
///
/// Permissionless: the sender is not checked, because the sender attests to
/// nothing. The signatures in `payload` are the only claim of authenticity, and
/// they are checked against the roster `trust` holds.
///
/// `clock` is the LEZ clock account the transaction supplied. Its id and its
/// bytes are read from the one struct, which is what binds them — see the crate
/// header.
///
/// # Errors
///
/// [`SettleError`], one variant per cause, and every one of them leaves the
/// order unfilled. Nothing here falls back to a price it could not verify.
pub fn settle(
    order: AccountWithMetadata,
    trust: AccountWithMetadata,
    clock: AccountWithMetadata,
    feed_id: [u8; 32],
    payload: &[u8],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SettleError> {
    // The accounts first, and every check here is a decode or a comparison:
    // nothing reads the clock or recovers a signature, so cheap refusals cost a
    // caller nothing beyond the payload it was already charged for reading
    // (ADR 26).
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

    let registered = trust::read(&trust, self_program_id)?;
    if registered.feed_id != feed_id {
        return Err(SettleError::Trust(TrustError::FeedMismatch));
    }

    terms_still_hold(&registered, &stored)?;

    let verified = verify(
        &registered,
        &AssetPair::new(stored.base_asset, stored.quote_asset),
        payload,
        &clock,
    )?;
    if verified.price < stored.limit_price_q64 {
        return Err(SettleError::LimitNotReached {
            price: verified.price,
            limit: stored.limit_price_q64,
        });
    }

    stored.filled = true;
    let mut account = order.account;
    account.data = write(&stored).ok_or(SettleError::OrderTooLarge)?;

    // All three accounts, in the instruction's own order, with the trust and the
    // clock unchanged. `validate_execution` zips pre-states and post-states
    // positionally and requires equal length (rule 2), so an account this
    // instruction only reads still has to come back. `SpelOutput::execute`
    // passes these through and adds nothing.
    //
    // No claim on any of them: this program already owns the order and the
    // trust, and LEZ refuses a claim on an account whose owner is not the
    // default one.
    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(trust.account),
        AccountPostState::new(clock.account),
    ])
}

/// Refuses a settlement whose registration no longer describes the order's terms.
///
/// Every field of a registration is accounted for here, in one destructuring with
/// no `..`, so a field added to [`FeedTrust`] stops this compiling until somebody
/// decides whether it binds an open order. That is the only mechanism keeping the
/// two in step — an order that silently stopped tracking a new parameter would be
/// filled against terms its owner never saw — and it is the same reason the error
/// blocks have no wildcard arm.
///
/// Cheap, and before the verification, so nothing cryptographic is paid for a
/// settlement that cannot happen.
fn terms_still_hold(registered: &FeedTrust, order: &OrderAccount) -> Result<(), SettleError> {
    let FeedTrust {
        // A label for the consumer's own records, read by nothing in
        // verification, so it cannot change what an order means.
        data_service_id: _,
        // Checked in `settle` against the order and against the instruction's own
        // argument, which the guest constrains this account's address to.
        feed_id: _,
        // Checked by `verify_feed`, against the pair the owner signed for. Left to
        // it deliberately: that comparison is what U7 asks a consumer to
        // demonstrate, and duplicating it here would make it dead again.
        base_asset: _,
        quote_asset: _,
        // Bound to the order. These two decide what its limit means and how old a
        // payload may be, and neither is something an owner agreed to have moved.
        decimals,
        max_age_ms,
        // Deliberately mutable under an open order: rotating who may speak for a
        // feed is the whole reason this program has an authority, and an order is
        // a claim about a price rather than about a roster.
        signers: _,
        threshold: _,
    } = registered;

    if *decimals != order.decimals {
        return Err(SettleError::ScaleChanged {
            priced_at: order.decimals,
            registered: *decimals,
        });
    }
    if *max_age_ms != order.max_age_ms {
        return Err(SettleError::WindowChanged {
            priced_at: order.max_age_ms,
            registered: *max_age_ms,
        });
    }
    Ok(())
}

/// The verification itself, over what this program trusts for the feed.
///
/// Separate from [`settle`] so the configuration is assembled in one place and
/// its only inputs are program-owned state and the two accounts LEZ supplied. A
/// reader looking for what a caller can influence should find nothing here.
fn verify(
    registered: &FeedTrust,
    expected: &AssetPair,
    payload: &[u8],
    clock: &AccountWithMetadata,
) -> Result<VerifiedFeed, SettleError> {
    let signers = registered.signer_addresses();
    let config = registered.config(&signers).map_err(TrustError::Config)?;

    // `expected` is the order's, and the configuration is the trust account's, so
    // this is two independently written records held against each other rather
    // than a value compared with itself. Which is what the parameter is for: the
    // push path passes the price account's pair against the feed account's
    // configuration for exactly the same reason (ADR 32), and a version of this
    // consumer that read both from one account had a comparison that could not
    // fail.
    //
    // Reachable, and not only in principle. An authority may retire a feed and
    // register the id again under another pair, at which point every order opened
    // before that answers `AssetMismatch` instead of filling under a meaning its
    // owner never signed for.
    Ok(verify_price(
        payload,
        &config,
        expected,
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
    use crate::testing::{
        as_chain_leaves_it, clock_at, established_config, key, untouched, GENESIS, OURS,
        SOMEONE_ELSE,
    };
    use crate::AuthorityError;
    use lee_core::account::AccountId;
    use lee_core::program::{validate_execution, Claim};
    use pull_lib::{BackendError, ConfigError, DecodeError, TimeError};
    use spel_framework::pda::compute_pda;

    const BTC: [u8; 32] = crate::padded(b"BTC");
    const ETH: [u8; 32] = crate::padded(b"ETH");
    const ORDER_ID: [u8; 32] = [0x0D; 32];

    fn order_address() -> AccountId {
        compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
    }

    fn trust_address(feed_id: &[u8; 32]) -> AccountId {
        compute_pda(
            &OURS,
            &[feed_id, &seed_from_str(crate::trust::TRUST_ACCOUNT_SEED)],
        )
    }

    /// A registered trust account for `feed_id`, with five signers and a
    /// threshold of three.
    fn registered(feed_id: [u8; 32]) -> AccountWithMetadata {
        let posts = crate::trust::register(
            untouched(trust_address(&feed_id)),
            key(GENESIS, true),
            established_config(),
            "redstone-primary-prod".to_owned(),
            feed_id,
            feed_id,
            crate::padded(b"USD"),
            8,
            60_000,
            (1..=5u8).map(|i| [i; 20]).collect(),
            3,
            OURS,
        )
        .expect("a usable registration");
        as_chain_leaves_it(&posts[0], trust_address(&feed_id))
    }

    fn opened(feed_id: [u8; 32], limit: u128) -> AccountWithMetadata {
        let posts = open_order(
            untouched(order_address()),
            key([0xA0; 32], true),
            registered(feed_id),
            feed_id,
            feed_id,
            crate::padded(b"USD"),
            limit,
            ORDER_ID,
            OURS,
        )
        .expect("a usable order");
        as_chain_leaves_it(&posts[0], order_address())
    }

    fn stored(order: &AccountWithMetadata) -> OrderAccount {
        OrderAccount::try_from_slice(order.account.data.as_ref()).expect("an order")
    }

    #[test]
    fn an_order_records_its_owner_and_its_feed_and_claims_its_address() {
        let posts = open_order(
            untouched(order_address()),
            key([0xA0; 32], true),
            registered(BTC),
            BTC,
            BTC,
            crate::padded(b"USD"),
            42,
            ORDER_ID,
            OURS,
        )
        .expect("ok");

        let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
            panic!("a first open claims a PDA");
        };
        assert_eq!(
            AccountId::for_public_pda(&OURS, &seed),
            order_address(),
            "the claim has to name the address the constraint checked"
        );

        let order = stored(&as_chain_leaves_it(&posts[0], order_address()));
        assert_eq!(order.owner, [0xA0; 32]);
        assert_eq!(order.feed_id, BTC);
        assert_eq!(order.limit_price_q64, 42);
        assert!(!order.filled);
    }

    #[test]
    fn an_open_passes_lez_and_leaves_the_owner_and_the_trust_alone() {
        let pre = vec![
            untouched(order_address()),
            key([0xA0; 32], true),
            registered(BTC),
        ];
        let posts = open_order(
            untouched(order_address()),
            key([0xA0; 32], true),
            registered(BTC),
            BTC,
            BTC,
            crate::padded(b"USD"),
            1,
            ORDER_ID,
            OURS,
        )
        .expect("ok");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts the open");
        assert_eq!(posts[1].account(), &key([0xA0; 32], true).account);
        assert_eq!(posts[2].account(), &registered(BTC).account);
    }

    #[test]
    fn an_owner_that_did_not_sign_cannot_open_an_order() {
        assert_eq!(
            open_order(
                untouched(order_address()),
                key([0xA0; 32], false),
                registered(BTC),
                BTC,
                BTC,
                crate::padded(b"USD"),
                1,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::OwnerDidNotSign)
        );
    }

    #[test]
    fn an_order_id_is_spent_once() {
        assert_eq!(
            open_order(
                opened(BTC, 1),
                key([0xA0; 32], true),
                registered(BTC),
                BTC,
                BTC,
                crate::padded(b"USD"),
                2,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::AlreadyOpen)
        );
    }

    #[test]
    fn a_squatted_order_address_is_refused_as_unusable_and_not_as_taken() {
        let mut squatted = untouched(order_address());
        squatted.account.balance = 1;
        assert_eq!(
            open_order(
                squatted,
                key([0xA0; 32], true),
                registered(BTC),
                BTC,
                BTC,
                crate::padded(b"USD"),
                1,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::OrderAccountUnusable)
        );
    }

    #[test]
    fn a_pair_the_owner_did_not_expect_is_refused_at_the_open() {
        // The attack this exists to close: a registration that labels RedStone's
        // `BTC` feed as ETH/USD verifies real BTC packages, so an order opened
        // against it would fill at the bitcoin price while its owner believed it
        // was trading ether. No signer attests to which assets a feed prices, so
        // the owner's own expectation is the only thing that can catch it.
        assert_eq!(
            open_order(
                untouched(order_address()),
                key([0xA0; 32], true),
                registered(BTC),
                BTC,
                crate::padded(b"ETH"),
                crate::padded(b"USD"),
                1,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::PairMismatch)
        );
    }

    #[test]
    fn an_order_is_far_smaller_than_an_accounts_data_limit() {
        // `OrderTooLarge` documents itself as unreachable, and the claim rests on a
        // byte count. Measured here so the claim has something behind it: every
        // field is fixed-width, so there is one size rather than a maximum, and a
        // field added to the order moves it.
        let order = stored(&opened(BTC, u128::MAX));
        assert_eq!(borsh::to_vec(&order).expect("serialises").len(), 154);
    }

    #[test]
    fn an_order_carries_the_pair_its_owner_signed_for() {
        // Stored rather than re-read at settlement, and that is what makes
        // `verify_feed`'s pair comparison two records held against each other
        // rather than one compared with itself.
        let order = stored(&opened(BTC, 1));
        assert_eq!(order.base_asset, BTC);
        assert_eq!(order.quote_asset, crate::padded(b"USD"));
    }

    #[test]
    fn an_order_against_a_feed_this_program_does_not_trust_is_refused_at_the_open() {
        // It could never settle, so refusing it later would only move the
        // discovery to a point where the owner has already committed.
        assert_eq!(
            open_order(
                untouched(order_address()),
                key([0xA0; 32], true),
                untouched(trust_address(&BTC)),
                BTC,
                BTC,
                crate::padded(b"USD"),
                1,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::Trust(TrustError::NotRegistered))
        );
    }

    #[test]
    fn an_open_that_names_one_feed_and_addresses_another_is_refused() {
        assert_eq!(
            open_order(
                untouched(order_address()),
                key([0xA0; 32], true),
                registered(ETH),
                BTC,
                BTC,
                crate::padded(b"USD"),
                1,
                ORDER_ID,
                OURS
            ),
            Err(OpenError::Trust(TrustError::FeedMismatch))
        );
    }

    #[test]
    fn an_account_this_program_does_not_own_is_not_an_order() {
        // Without this a caller supplies an account it controls whose bytes
        // decode as an order, which is a limit price of the caller's choosing.
        let mut foreign = opened(BTC, 1);
        foreign.account.program_owner = SOMEONE_ELSE;
        assert_eq!(
            settle(foreign, registered(BTC), clock_at(1), BTC, &[], OURS),
            Err(SettleError::OrderNotOurs)
        );
        assert_eq!(
            settle(
                untouched(order_address()),
                registered(BTC),
                clock_at(1),
                BTC,
                &[],
                OURS
            ),
            Err(SettleError::OrderNotOurs)
        );
    }

    #[test]
    fn an_order_accounts_bytes_that_are_not_an_order_are_reported_as_such() {
        let mut wrong = opened(BTC, 1);
        wrong.account.data = Data::try_from(vec![0xFF; 3]).expect("three bytes fit");
        assert_eq!(
            settle(wrong, registered(BTC), clock_at(1), BTC, &[], OURS),
            Err(SettleError::OrderUndecodable)
        );
    }

    #[test]
    fn a_filled_order_refuses_a_second_fill_before_any_verification() {
        // Before, not after: the payload is empty here, so a run that verified
        // first would answer `Malformed` and this is what pins the order.
        let mut filled = opened(BTC, 1);
        let mut order = stored(&filled);
        order.filled = true;
        filled.account.data = write(&order).expect("an order fits");

        assert_eq!(
            settle(filled, registered(BTC), clock_at(1), BTC, &[], OURS),
            Err(SettleError::AlreadyFilled)
        );
    }

    #[test]
    fn the_order_decides_which_feed_it_is_settled_against() {
        // Not the argument and not the account. An order priced against BTC
        // cannot be filled from an ETH price however the accounts are arranged.
        assert_eq!(
            settle(opened(BTC, 1), registered(ETH), clock_at(1), ETH, &[], OURS),
            Err(SettleError::OrderIsForAnotherFeed)
        );
    }

    #[test]
    fn a_trust_account_for_another_feed_cannot_be_substituted() {
        // The guest constrains this account's address to the named feed, so this
        // is the body's half of the same rule: a caller that reached here with a
        // mismatched pair is refused before any signature is recovered.
        assert_eq!(
            settle(opened(BTC, 1), registered(ETH), clock_at(1), BTC, &[], OURS),
            Err(SettleError::Trust(TrustError::FeedMismatch))
        );
        assert_eq!(
            settle(
                opened(BTC, 1),
                untouched(trust_address(&BTC)),
                clock_at(1),
                BTC,
                &[],
                OURS
            ),
            Err(SettleError::Trust(TrustError::NotRegistered))
        );
    }

    #[test]
    fn no_two_causes_answer_with_the_same_number() {
        // A caller that cannot tell two failures apart cannot act on either, and
        // `SpelError` carries one number.
        //
        // Every variant of every layer, and that is the point rather than
        // thoroughness for its own sake: a collapse inside a wrapper is invisible
        // to a test that constructs one member of it. Sampling one `DecodeError`
        // would pass against a `verify_code` answering all nine decoder faults
        // with a single number, which is the failure U6 is about.
        let mut codes: Vec<u32> = every_cause().iter().map(SettleError::code).collect();
        codes.extend(every_open_cause().iter().map(OpenError::code));
        codes.extend(every_authority_cause().iter().map(AuthorityError::code));
        codes.extend(every_trust_cause().iter().map(TrustError::code));

        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two causes share a number");
    }

    #[test]
    fn the_number_a_caller_sees_is_the_frameworks_offset_one() {
        // What reaches a caller is `SpelError::error_code`, not `code`. The offset
        // belongs to the framework and this is where it is pinned, so a caller
        // reading 7406 can look 1406 up.
        for cause in every_cause() {
            let spel: spel_framework::error::SpelError = cause.into();
            assert_eq!(spel.error_code(), 6000 + cause.code());
        }
    }

    #[test]
    fn a_configuration_fault_has_one_number_by_every_route() {
        // Three routes reach a `ConfigError`: a registration refused, a rotation
        // refused, and `verify_feed` refusing one. They are the same fault and
        // get the same number, which is why the counting test lists each leaf
        // once rather than once per route -- a shared number there would look
        // like a collision.
        for fault in every_config_fault() {
            let via_trust = TrustError::Config(fault).code();
            assert_eq!(via_trust, crate::config_code(fault));
            assert_eq!(
                SettleError::Verify(VerifyError::InvalidConfig(fault)).code(),
                via_trust,
                "{fault:?} answers with two numbers"
            );
        }
    }

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

    fn every_authority_cause() -> [AuthorityError; 12] {
        use AuthorityError as A;
        [
            A::ConfigNotOurs,
            A::ConfigUndecodable,
            A::NotSigned,
            A::Unauthorised,
            A::NoGenesisAuthority,
            A::AlreadyEstablished,
            A::ConfigAccountUnusable,
            A::NotGenesisKey,
            A::NomineeIsZero,
            A::NoNomination,
            A::NotTheNominee,
            A::KeyUnowned,
        ]
    }

    fn every_trust_cause() -> Vec<TrustError> {
        vec![
            TrustError::AlreadyRegistered,
            TrustError::TrustAccountUnusable,
            TrustError::NotRegistered,
            TrustError::TrustUndecodable,
            TrustError::FeedMismatch,
            TrustError::Deregistered,
            TrustError::TrustTooLarge,
        ]
    }

    fn every_open_cause() -> Vec<OpenError> {
        vec![
            OpenError::OwnerDidNotSign,
            OpenError::AlreadyOpen,
            OpenError::OrderAccountUnusable,
            OpenError::PairMismatch,
            OpenError::OrderTooLarge,
        ]
    }

    /// Every leaf cause a settlement can report, listed rather than derived.
    ///
    /// The taxonomy is `verifier-core`'s, so nothing here can enumerate it; the
    /// wildcard-free matches in `crate`'s `verify_code`, `decode_code`,
    /// `backend_code`, `clock_code` and `config_code` are what force this list to
    /// grow, because each stops compiling when a variant is added upstream. That
    /// is the only mechanism keeping the two in step, which is why none of them
    /// may gain a `_ =>` arm.
    fn every_cause() -> Vec<SettleError> {
        let mut causes = vec![
            SettleError::OrderNotOurs,
            SettleError::OrderUndecodable,
            SettleError::AlreadyFilled,
            SettleError::OrderIsForAnotherFeed,
            SettleError::OrderTooLarge,
            SettleError::LimitNotReached { price: 1, limit: 2 },
            SettleError::ScaleChanged {
                priced_at: 8,
                registered: 6,
            },
            SettleError::WindowChanged {
                priced_at: 60_000,
                registered: 900_000,
            },
        ];
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
        causes.extend(every_config_fault().map(|err| SettleError::Trust(TrustError::Config(err))));
        causes
    }
}
