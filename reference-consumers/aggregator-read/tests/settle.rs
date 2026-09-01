//! The consumer end to end, over a price account the aggregator really wrote.
//!
//! What this file owns is the seam. Whether a median is right or a threshold is
//! counted correctly belongs to `verifier-core`; whether a published price is
//! correct belongs to `aggregator-program`. Here the question is the one only a
//! reading consumer can answer: given an account somebody else wrote, does this
//! program act on it when it should and refuse when it should not?
//!
//! The price account under test is produced by running the aggregator's own
//! `submit_price` over a payload from the committed RedStone capture. Building
//! one by hand would let this file agree with itself about a layout — and it is
//! the account's *address*, not its bytes, that carries the trust here, so the
//! writer has to be the real one.

use aggregator_program::{submit, FeedAccount};
use borsh::BorshDeserialize;
use kanon_clock::CLOCK_ACCOUNT_ID;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, Claim, ProgramId};
use reference_consumer_aggregator_read::order::{
    self, OrderAccount, SettleError, ORDER_ACCOUNT_SEED,
};
use reference_consumer_aggregator_read::read::{price_account_address, ReadError};
use reference_consumer_aggregator_read::source::{self, PriceSource, SourceError};
use spel_framework::pda::{compute_pda, seed_from_str};

#[path = "../../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

/// This consumer.
const OURS: ProgramId = [7u32; 8];
/// The aggregator it reads.
const AGG: ProgramId = [11u32; 8];
/// The aggregator after a rebuild, which is a different program.
const REBUILT_AGG: ProgramId = [12u32; 8];
const WALLET: ProgramId = [42u32; 8];
const CLOCK_PROGRAM: ProgramId = [88u32; 8];
const DEFAULT: ProgramId = [0u32; 8];

const OWNER: [u8; 32] = [0xA0; 32];
const ORDER_ID: [u8; 32] = [0x0D; 32];
const BASE: [u8; 32] = [1u8; 32];
const QUOTE: [u8; 32] = [2u8; 32];

const DECIMALS: u8 = 8;
/// The aggregator's window, which bounds a package's age at publication.
const AGG_MAX_AGE_MS: u64 = 60_000;
/// This consumer's, which bounds a published price's age at reading. Wider on
/// purpose: a price starts ageing once it is written.
const MAX_AGE_MS: u64 = 120_000;
const THRESHOLD: u8 = 3;

fn padded(feed: &str) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..feed.len()].copy_from_slice(feed.as_bytes());
    id
}

fn account(owner: ProgramId, data: Vec<u8>, id: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance: 0,
            data: Data::try_from(data).expect("fits"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: id,
    }
}

fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
    let mut key = account(WALLET, Vec::new(), AccountId::new(id));
    key.account.balance = 500;
    key.account.nonce = Nonce(3);
    key.is_authorized = signs;
    key
}

fn clock_at(now_ms: u64) -> AccountWithMetadata {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&now_ms.to_le_bytes());
    account(
        CLOCK_PROGRAM,
        data.to_vec(),
        AccountId::new(CLOCK_ACCOUNT_ID),
    )
}

/// The aggregator's feed account, at the address its id derives under `AGG`.
fn aggregator_feed(v: &Vector, aggregator: ProgramId) -> AccountWithMetadata {
    let feed_id = padded(&v.feed_id);
    let stored = FeedAccount {
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: AGG_MAX_AGE_MS,
        signers: v.signers.iter().map(|s| *s.as_bytes()).collect(),
        threshold: THRESHOLD,
        paused: false,
    };
    account(
        aggregator,
        borsh::to_vec(&stored).expect("serialises"),
        compute_pda(
            &aggregator,
            &[&feed_id, &seed_from_str("KANON_FEED_ACCOUNT")],
        ),
    )
}

/// A price account as the aggregator leaves it, by running its own submission.
///
/// The post-state the aggregator returned, turned back into an account the way
/// LEZ would apply it: the ownership claim it carries is what makes the account
/// the aggregator's.
fn published(v: &Vector, aggregator: ProgramId) -> AccountWithMetadata {
    let feed = aggregator_feed(v, aggregator);
    let address = price_account_address(&aggregator, &padded(&v.feed_id));
    let empty = account(DEFAULT, Vec::new(), address);

    let posts = submit::submit_price(
        feed,
        empty,
        clock_at(v.timestamp_ms),
        &v.payload,
        aggregator,
    )
    .expect("the aggregator publishes the captured payload");

    let mut written = posts[1].account().clone();
    // The claim in the post-state is what LEZ turns into ownership, and every
    // read this consumer does happens after that has been applied.
    written.program_owner = aggregator;
    account(
        aggregator,
        written.data.as_ref().to_vec(),
        AccountId::new(*address.value()),
    )
}

fn registered(v: &Vector, aggregator: ProgramId) -> AccountWithMetadata {
    let stored = PriceSource {
        feed_id: padded(&v.feed_id),
        aggregator,
        base_asset: BASE,
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS,
    };
    account(
        OURS,
        borsh::to_vec(&stored).expect("serialises"),
        source::source_address(&OURS, &padded(&v.feed_id)),
    )
}

fn order_address() -> AccountId {
    compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
}

/// An order opened through `open_order`, not constructed, so what is settled is
/// what the program really writes.
fn opened(v: &Vector, limit_q64: u128) -> AccountWithMetadata {
    let posts = order::open_order(
        account(DEFAULT, Vec::new(), order_address()),
        key(OWNER, true),
        registered(v, AGG),
        padded(&v.feed_id),
        BASE,
        QUOTE,
        limit_q64,
        ORDER_ID,
        OURS,
    )
    .expect("the order opens");

    account(
        OURS,
        posts[0].account().data.as_ref().to_vec(),
        order_address(),
    )
}

fn stored(account: &Account) -> OrderAccount {
    OrderAccount::try_from_slice(account.data.as_ref()).expect("an order")
}

fn settle_at(
    v: &Vector,
    order: AccountWithMetadata,
    source: AccountWithMetadata,
    price: AccountWithMetadata,
    now_ms: u64,
) -> Result<Vec<AccountPostState>, SettleError> {
    order::settle(
        order,
        source,
        price,
        clock_at(now_ms),
        padded(&v.feed_id),
        OURS,
    )
}

#[test]
fn a_published_price_fills_an_order_the_market_has_reached() {
    let v = vectors::named("BTC");
    let posts = settle_at(
        &v,
        opened(&v, 1),
        registered(&v, AGG),
        published(&v, AGG),
        v.timestamp_ms,
    )
    .expect("fills");

    assert!(stored(posts[0].account()).filled);
}

#[test]
fn a_price_below_the_limit_leaves_the_order_open() {
    let v = vectors::named("BTC");
    let err = settle_at(
        &v,
        opened(&v, u128::MAX),
        registered(&v, AGG),
        published(&v, AGG),
        v.timestamp_ms,
    )
    .unwrap_err();

    assert!(matches!(err, SettleError::LimitNotReached { .. }));
}

#[test]
fn a_settlement_returns_one_post_state_per_account_it_was_given() {
    // Three of the four accounts are read and never written, and all three still
    // have to come back: `validate_execution` zips pre-states and post-states
    // positionally and requires equal length (rule 2), so returning the order
    // alone would fail every successful settlement on chain.
    let v = vectors::named("BTC");
    let source = registered(&v, AGG);
    let price = published(&v, AGG);
    let clock = clock_at(v.timestamp_ms);

    let posts = order::settle(
        opened(&v, 1),
        source.clone(),
        price.clone(),
        clock.clone(),
        padded(&v.feed_id),
        OURS,
    )
    .expect("fills");

    assert_eq!(posts.len(), 4);
    assert_eq!(posts[1].account(), &source.account);
    assert_eq!(
        posts[2].account(),
        &price.account,
        "the price account comes back exactly as it was given"
    );
    assert_eq!(posts[3].account(), &clock.account);
    assert!(posts.iter().all(|p| p.required_claim().is_none()));
}

#[test]
fn the_post_states_pass_lez() {
    // Rather than restating rules 1 through 8: hand what `settle` returned to the
    // function that enforces them, unmodified.
    let v = vectors::named("BTC");
    let order = opened(&v, 1);
    let source = registered(&v, AGG);
    let price = published(&v, AGG);
    let clock = clock_at(v.timestamp_ms);
    let pre = vec![order.clone(), source.clone(), price.clone(), clock.clone()];

    let posts =
        order::settle(order, source, price, clock, padded(&v.feed_id), OURS).expect("fills");
    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the fill");
}

/// The other write LEZ could reject: opening an order claims its address.
///
/// Rules 1 to 8 only, since `validate_execution` never reads `required_claim`.
/// `the_claimed_seed_derives_the_order_address` is the half that does.
#[test]
fn opening_an_order_passes_lez() {
    let v = vectors::named("BTC");
    let order = account(DEFAULT, Vec::new(), order_address());
    let owner = key(OWNER, true);
    let source = registered(&v, AGG);
    let pre = vec![order.clone(), owner.clone(), source.clone()];

    let posts = order::open_order(
        order,
        owner,
        source,
        padded(&v.feed_id),
        BASE,
        QUOTE,
        1,
        ORDER_ID,
        OURS,
    )
    .expect("the order opens");

    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the new order");
}

/// The rule LEZ's claim loop applies after validation has run: a claim whose seed
/// does not derive the account it is attached to is refused as
/// `MismatchedPdaClaim`. An order that claimed the wrong address would pass the
/// validator and fail the transaction on chain.
#[test]
fn the_claimed_seed_derives_the_order_address() {
    let v = vectors::named("BTC");
    let posts = order::open_order(
        account(DEFAULT, Vec::new(), order_address()),
        key(OWNER, true),
        registered(&v, AGG),
        padded(&v.feed_id),
        BASE,
        QUOTE,
        1,
        ORDER_ID,
        OURS,
    )
    .expect("the order opens");

    let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
        panic!("a new order claims its address as a PDA");
    };

    assert_eq!(
        AccountId::for_public_pda(&OURS, &seed),
        order_address(),
        "the claimed seed has to derive the address the order account is at"
    );
}

/// The whole reason the aggregator's id is state rather than a constant: an
/// order opened before a rebuild settles after one, against the account the new
/// build writes, with no redeployment of this consumer.
#[test]
fn an_order_survives_the_aggregator_being_rebuilt() {
    let v = vectors::named("BTC");
    let order = opened(&v, 1);

    // Before: the new build's account is at an address the old registration does
    // not derive, so it is refused rather than silently read.
    let err = settle_at(
        &v,
        order.clone(),
        registered(&v, AGG),
        published(&v, REBUILT_AGG),
        v.timestamp_ms,
    )
    .unwrap_err();
    assert_eq!(err, SettleError::Read(ReadError::WrongAccount));

    // After one `update_aggregator`, the same order fills against the same feed.
    let moved = source::update_aggregator(
        registered(&v, AGG),
        key(OWNER, true),
        authorised_config(),
        padded(&v.feed_id),
        REBUILT_AGG,
        OURS,
    )
    .expect("the authority follows the rebuild");
    let source = account(
        OURS,
        moved[0].account().data.as_ref().to_vec(),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );

    let posts = settle_at(
        &v,
        order,
        source,
        published(&v, REBUILT_AGG),
        v.timestamp_ms,
    )
    .expect("fills under the new build");
    assert!(stored(posts[0].account()).filled);
}

/// The config account with `OWNER` as the authority, so the administrative
/// half of the test above does not need a whole establishment sequence.
fn authorised_config() -> AccountWithMetadata {
    use reference_consumer_aggregator_read::authority::{config_address, ConfigAccount};
    let stored = ConfigAccount {
        authority: OWNER,
        pending: None,
    };
    account(
        OURS,
        borsh::to_vec(&stored).expect("serialises"),
        config_address(&OURS),
    )
}

#[test]
fn an_order_fills_once() {
    let v = vectors::named("BTC");
    let posts = settle_at(
        &v,
        opened(&v, 1),
        registered(&v, AGG),
        published(&v, AGG),
        v.timestamp_ms,
    )
    .expect("fills");
    let filled = account(
        OURS,
        posts[0].account().data.as_ref().to_vec(),
        order_address(),
    );

    assert_eq!(
        settle_at(
            &v,
            filled,
            registered(&v, AGG),
            published(&v, AGG),
            v.timestamp_ms
        )
        .unwrap_err(),
        SettleError::AlreadyFilled
    );
}

/// Reachable because a source is retirable: an authority may retire a feed and
/// register the id again under another pair, and an order opened before that
/// must not fill under the new meaning.
#[test]
fn an_order_cannot_be_filled_under_a_pair_its_owner_never_signed_for() {
    let v = vectors::named("BTC");
    let order = opened(&v, 1);

    let mut relabelled = PriceSource {
        feed_id: padded(&v.feed_id),
        aggregator: AGG,
        base_asset: BASE,
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS,
    };
    relabelled.quote_asset = [9u8; 32];
    let source = account(
        OURS,
        borsh::to_vec(&relabelled).expect("serialises"),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );

    assert_eq!(
        settle_at(&v, order, source, published(&v, AGG), v.timestamp_ms).unwrap_err(),
        SettleError::PairChanged
    );
}

#[test]
fn an_order_cannot_be_filled_under_a_window_its_owner_never_accepted() {
    let v = vectors::named("BTC");
    let order = opened(&v, 1);

    let widened = PriceSource {
        feed_id: padded(&v.feed_id),
        aggregator: AGG,
        base_asset: BASE,
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS * 2,
    };
    let source = account(
        OURS,
        borsh::to_vec(&widened).expect("serialises"),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );

    assert_eq!(
        settle_at(&v, order, source, published(&v, AGG), v.timestamp_ms).unwrap_err(),
        SettleError::WindowChanged {
            was: MAX_AGE_MS,
            now: MAX_AGE_MS * 2,
        }
    );
}

#[test]
fn a_price_past_the_registered_window_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    let too_late = v.timestamp_ms + MAX_AGE_MS + 1;

    let err = settle_at(
        &v,
        opened(&v, 1),
        registered(&v, AGG),
        published(&v, AGG),
        too_late,
    )
    .unwrap_err();

    assert!(matches!(err, SettleError::Read(ReadError::Stale { .. })));
}

/// A caller chooses which account it hands over, so this is the refusal that
/// stands between an order and any account that happens to decode as a price.
#[test]
fn a_price_account_the_caller_chose_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    let elsewhere = account(
        AGG,
        published(&v, AGG).account.data.as_ref().to_vec(),
        AccountId::new([0xAB; 32]),
    );

    assert_eq!(
        settle_at(
            &v,
            opened(&v, 1),
            registered(&v, AGG),
            elsewhere,
            v.timestamp_ms
        )
        .unwrap_err(),
        SettleError::Read(ReadError::WrongAccount)
    );
}

#[test]
fn a_feed_nothing_has_been_published_for_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    let pristine = account(
        DEFAULT,
        Vec::new(),
        price_account_address(&AGG, &padded(&v.feed_id)),
    );

    assert_eq!(
        settle_at(
            &v,
            opened(&v, 1),
            registered(&v, AGG),
            pristine,
            v.timestamp_ms
        )
        .unwrap_err(),
        SettleError::Read(ReadError::Unavailable)
    );
}

#[test]
fn a_clock_account_the_caller_chose_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    let mut ours = clock_at(v.timestamp_ms);
    ours.account_id = AccountId::new([0xC1; 32]);

    let err = order::settle(
        opened(&v, 1),
        registered(&v, AGG),
        published(&v, AGG),
        ours,
        padded(&v.feed_id),
        OURS,
    )
    .unwrap_err();

    assert_eq!(
        err.code(),
        reference_consumer_aggregator_read::clock_code(kanon_clock::TimeError::WrongAccount)
    );
}

#[test]
fn an_order_against_another_feed_is_refused() {
    let v = vectors::named("BTC");
    let other = vectors::named("ETH");

    assert_eq!(
        settle_at(
            &other,
            opened(&v, 1),
            registered(&other, AGG),
            published(&other, AGG),
            other.timestamp_ms
        )
        .unwrap_err(),
        SettleError::OrderIsForAnotherFeed
    );
}

#[test]
fn an_order_this_program_does_not_own_is_not_an_order() {
    let v = vectors::named("BTC");
    let mut theirs = opened(&v, 1);
    theirs.account.program_owner = WALLET;

    assert_eq!(
        settle_at(
            &v,
            theirs,
            registered(&v, AGG),
            published(&v, AGG),
            v.timestamp_ms
        )
        .unwrap_err(),
        SettleError::OrderNotOurs
    );
}

#[test]
fn a_retired_source_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    let retired = account(
        OURS,
        Vec::new(),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );

    assert_eq!(
        settle_at(
            &v,
            opened(&v, 1),
            retired,
            published(&v, AGG),
            v.timestamp_ms
        )
        .unwrap_err(),
        SettleError::Source(SourceError::NotRegistered)
    );
}

/// The mistake U7 is really asking about: nothing the aggregator publishes says
/// which assets a feed prices, so a source that labelled RedStone's BTC feed as
/// ETH/USD would read real BTC prices under a name its owner never chose. The
/// owner's own expectation is the only thing that catches it.
#[test]
fn a_mislabelled_source_cannot_open_an_order_its_owner_did_not_intend() {
    let v = vectors::named("BTC");
    let mislabelled = PriceSource {
        feed_id: padded(&v.feed_id),
        aggregator: AGG,
        base_asset: [0xEE; 32],
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS,
    };
    let source = account(
        OURS,
        borsh::to_vec(&mislabelled).expect("serialises"),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );

    let err = order::open_order(
        account(DEFAULT, Vec::new(), order_address()),
        key(OWNER, true),
        source,
        padded(&v.feed_id),
        BASE,
        QUOTE,
        1,
        ORDER_ID,
        OURS,
    )
    .unwrap_err();

    assert_eq!(err, order::OpenError::PairMismatch);
}

/// No refusal above returns post-states, and that is the property rather than an
/// accident of how each branch is written: a caller must be able to tell "the
/// market has not reached your limit" from "nobody could tell me the price".
#[test]
fn no_refusal_returns_a_post_state() {
    let v = vectors::named("BTC");
    let pristine = account(
        DEFAULT,
        Vec::new(),
        price_account_address(&AGG, &padded(&v.feed_id)),
    );
    let retired = account(
        OURS,
        Vec::new(),
        source::source_address(&OURS, &padded(&v.feed_id)),
    );
    let elsewhere = account(
        AGG,
        published(&v, AGG).account.data.as_ref().to_vec(),
        AccountId::new([0xAB; 32]),
    );

    let refusals: Vec<Result<Vec<AccountPostState>, SettleError>> = vec![
        settle_at(
            &v,
            opened(&v, u128::MAX),
            registered(&v, AGG),
            published(&v, AGG),
            v.timestamp_ms,
        ),
        settle_at(
            &v,
            opened(&v, 1),
            registered(&v, AGG),
            pristine,
            v.timestamp_ms,
        ),
        settle_at(
            &v,
            opened(&v, 1),
            retired,
            published(&v, AGG),
            v.timestamp_ms,
        ),
        settle_at(
            &v,
            opened(&v, 1),
            registered(&v, AGG),
            elsewhere,
            v.timestamp_ms,
        ),
        settle_at(
            &v,
            opened(&v, 1),
            registered(&v, AGG),
            published(&v, AGG),
            v.timestamp_ms + MAX_AGE_MS + 1,
        ),
    ];

    for refusal in refusals {
        assert!(
            refusal.is_err(),
            "a case meant to refuse returned post-states"
        );
    }
}
