//! Settling an order against payloads from the committed RedStone capture.
//!
//! What this file owns is the seam, not the verification. Whether a median is
//! right, a threshold is counted correctly or a window is applied at the right
//! edge belongs to `verifier-core` and is tested there once for both modes. Here
//! the questions are the ones only a program can answer: does a real payload
//! move real account state, does LEZ accept what comes back, and does every
//! typed refusal leave the order exactly where it was.
//!
//! The signatures are RedStone's. The envelope around them was assembled by
//! `scripts/capture-redstone-vectors.py`, because the gateway serves per-signer
//! JSON rather than a finished payload.

use borsh::BorshDeserialize;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, ProgramId};
use pull_lib::verifier_core::value::median;
use pull_lib::{TimeError, VerifyError, CLOCK_ACCOUNT_ID};
use reference_consumer_pull::{
    open_order, settle, OrderAccount, SettleError, DECIMALS, FEEDS, MAX_AGE_MS, ORDER_ACCOUNT_SEED,
};
use spel_framework::pda::{compute_pda, seed_from_str};

#[path = "../../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

const OURS: ProgramId = [7u32; 8];
const OWNER_PROGRAM: ProgramId = [42u32; 8];
const CLOCK_PROGRAM: ProgramId = [88u32; 8];
const ORDER_ID: [u8; 32] = [0x0D; 32];

/// How far past the clock a package may be dated before it reads as future.
/// `verifier-core`'s constant, restated because it is not part of the surface a
/// consumer links.
const MAX_AHEAD_MS: u64 = 3 * 60 * 1000;

fn order_address() -> AccountId {
    compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
}

fn owner() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: OWNER_PROGRAM,
            balance: 500,
            data: Data::default(),
            nonce: Nonce(3),
        },
        is_authorized: true,
        account_id: AccountId::new([0xA0; 32]),
    }
}

/// The LEZ clock account: `block_id` then `timestamp`, both little-endian.
///
/// Built as a whole account rather than as an id and some bytes, which is the
/// point of the reference consumer. `settle` reads both halves out of this one
/// struct, so a test cannot pair the pinned id with a timestamp of its own any
/// more than a caller can.
fn clock_at(ms: u64) -> AccountWithMetadata {
    clock_account(CLOCK_ACCOUNT_ID, ms)
}

fn clock_account(id: [u8; 32], ms: u64) -> AccountWithMetadata {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&ms.to_le_bytes());
    AccountWithMetadata {
        account: Account {
            program_owner: CLOCK_PROGRAM,
            balance: 0,
            data: Data::try_from(data.to_vec()).expect("sixteen bytes fit"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(id),
    }
}

fn unopened() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: order_address(),
    }
}

/// The account as the chain leaves it once a post-state is applied: the claim has
/// been honoured, so the order is this program's.
fn as_chain_leaves_it(account: Account) -> AccountWithMetadata {
    let mut account = account;
    account.program_owner = OURS;
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: order_address(),
    }
}

/// An open order against `feed`, at `limit`.
fn opened(feed: u8, limit: u128) -> AccountWithMetadata {
    let posts =
        open_order(unopened(), owner(), feed, limit, ORDER_ID, OURS).expect("a usable order");
    as_chain_leaves_it(posts[0].account().clone())
}

/// Which of `FEEDS` carries this capture's feed id.
fn index_of(v: &Vector) -> u8 {
    let mut wanted = [0u8; 32];
    wanted[..v.feed_id.len()].copy_from_slice(v.feed_id.as_bytes());
    let position = FEEDS
        .iter()
        .position(|spec| spec.feed_id == wanted)
        .unwrap_or_else(|| panic!("{} is not one of the compiled feeds", v.feed_id));
    u8::try_from(position).expect("five feeds fit in a u8")
}

/// What the capture's packages agree on, on the account's `Q64.64` scale.
fn price(v: &Vector) -> u128 {
    let mut values = v.values.clone();
    median(&mut values)
        .expect("five values have a median")
        .to_q64_64(DECIMALS)
        .expect("a RedStone price fits Q64.64")
}

fn stored(account: &Account) -> OrderAccount {
    OrderAccount::try_from_slice(account.data.as_ref()).expect("decodes as an order")
}

#[test]
fn a_captured_payload_fills_an_order_the_market_has_reached() {
    // Every captured feed, so the compiled table is exercised rather than one
    // row of it. The limit is one, which every real price clears.
    for v in vectors::all() {
        let feed = index_of(&v);
        let order = opened(feed, 1);

        let posts = settle(order, clock_at(v.timestamp_ms), &v.payload, OURS)
            .unwrap_or_else(|err| panic!("{} should fill: {err:?}", v.feed_id));

        assert_eq!(posts.len(), 1, "{}: only the order changes", v.feed_id);
        assert!(stored(posts[0].account()).filled, "{}", v.feed_id);
    }
}

#[test]
fn a_fill_moves_the_filled_flag_and_nothing_else() {
    // The order the owner opened has to be the order that filled. A program that
    // rewrote the limit, the feed or the owner on the way through would still
    // pass the test above.
    let v = vectors::named("BTC");
    let feed = index_of(&v);
    let before = opened(feed, 1);
    let was = stored(&before.account);

    let posts = settle(before.clone(), clock_at(v.timestamp_ms), &v.payload, OURS)
        .expect("the order fills");
    let now = stored(posts[0].account());

    assert_eq!(now.owner, was.owner);
    assert_eq!(now.feed, was.feed);
    assert_eq!(now.limit_price_q64, was.limit_price_q64);
    assert!(!was.filled && now.filled, "only the flag moves");

    // And the account, as against its contents.
    assert_eq!(
        posts[0].account().program_owner,
        before.account.program_owner
    );
    assert_eq!(posts[0].account().balance, before.account.balance);
    assert_eq!(posts[0].account().nonce, before.account.nonce);
}

#[test]
fn a_fill_claims_nothing() {
    // This program already owns the order account. LEZ refuses a claim on an
    // account whose owner is not the default one, so claiming again would fail
    // the transaction rather than be ignored.
    let v = vectors::named("BTC");
    let order = opened(index_of(&v), 1);
    let posts = settle(order, clock_at(v.timestamp_ms), &v.payload, OURS).expect("fills");
    assert!(posts[0].required_claim().is_none());
}

#[test]
fn the_post_states_pass_lez() {
    // Rather than restating rules 1 through 8: hand the post-states to the
    // function that enforces them.
    let v = vectors::named("BTC");
    let order = opened(index_of(&v), 1);
    let pre = vec![order.clone(), clock_at(v.timestamp_ms)];

    let mut posts = settle(order, clock_at(v.timestamp_ms), &v.payload, OURS).expect("fills");
    // `settle` returns the order alone, because the clock is read and never
    // written. LEZ zips pre-states and post-states positionally and requires
    // equal length, so a caller returning a subset has to be the dispatcher's
    // concern rather than this function's -- the clock is appended here to make
    // the check the one LEZ actually runs.
    posts.push(lee_core::program::AccountPostState::new(
        clock_at(v.timestamp_ms).account,
    ));
    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the fill");
}

#[test]
fn a_price_below_the_limit_leaves_the_order_open() {
    // The one refusal that is not a failure: verification succeeded and the
    // answer was no. It carries both numbers, because "not yet" and "never" look
    // identical to a caller told only that it was refused.
    let v = vectors::named("BTC");
    let reached = price(&v);
    let order = opened(index_of(&v), reached + 1);

    assert_eq!(
        settle(order, clock_at(v.timestamp_ms), &v.payload, OURS),
        Err(SettleError::LimitNotReached {
            price: reached,
            limit: reached + 1,
        })
    );
}

#[test]
fn the_limit_is_reached_at_the_price_and_not_past_it() {
    // At or above, so an order at exactly the market price fills. One assertion
    // either side of the edge, because a `>` and a `>=` differ nowhere else.
    let v = vectors::named("BTC");
    let reached = price(&v);

    settle(
        opened(index_of(&v), reached),
        clock_at(v.timestamp_ms),
        &v.payload,
        OURS,
    )
    .expect("an order at the market price fills");
}

#[test]
fn an_order_against_another_feed_does_not_fill_from_this_payload() {
    // The feed an order is priced against is the compiled one its index names,
    // and it is what the payload is searched for. A BTC payload carries no ETH
    // package, so nothing reports and the threshold is not met -- rather than
    // the order filling at whatever price the payload happened to carry.
    let btc = vectors::named("BTC");
    let eth = index_of(&vectors::named("ETH"));

    assert_eq!(
        settle(
            opened(eth, 1),
            clock_at(btc.timestamp_ms),
            &btc.payload,
            OURS
        ),
        Err(SettleError::Verify(VerifyError::ThresholdNotMet {
            met: 0,
            required: reference_consumer_pull::THRESHOLD,
        }))
    );
}

#[test]
fn a_clock_account_the_caller_chose_cannot_fill_an_order() {
    // The 10- and 50-block accounts hold real timestamps up to that many blocks
    // stale, so they are the plausible mistake rather than an obvious one. The
    // refusal comes before any signature is recovered.
    let v = vectors::named("BTC");
    for id in [
        *b"/LEZ/ClockProgramAccount/0000010",
        *b"/LEZ/ClockProgramAccount/0000050",
        [0u8; 32],
    ] {
        assert_eq!(
            settle(
                opened(index_of(&v), 1),
                clock_account(id, v.timestamp_ms),
                &v.payload,
                OURS
            ),
            Err(SettleError::Verify(VerifyError::NoClock(
                TimeError::WrongAccount
            ))),
            "a consumer must not be able to choose its own clock"
        );
    }
}

#[test]
fn a_payload_past_the_consumers_window_cannot_fill_an_order() {
    // `MAX_AGE_MS` is this consumer's own tolerance and is not something a
    // payload or its sender can widen. One millisecond past it is the whole test:
    // the window's edges are inclusive, and `verifier-core` owns which side.
    let v = vectors::named("BTC");

    settle(
        opened(index_of(&v), 1),
        clock_at(v.timestamp_ms + MAX_AGE_MS),
        &v.payload,
        OURS,
    )
    .expect("the far edge of the window is inside it");

    assert_eq!(
        settle(
            opened(index_of(&v), 1),
            clock_at(v.timestamp_ms + MAX_AGE_MS + 1),
            &v.payload,
            OURS
        ),
        Err(SettleError::Verify(VerifyError::StalePackage))
    );
}

#[test]
fn a_payload_dated_past_the_clock_cannot_fill_an_order() {
    // The other edge, and a different cause: a package from the future is not a
    // stale one, and an operator chasing a clock skew needs to be told which.
    let v = vectors::named("BTC");

    assert_eq!(
        settle(
            opened(index_of(&v), 1),
            clock_at(v.timestamp_ms - MAX_AHEAD_MS - 1),
            &v.payload,
            OURS
        ),
        Err(SettleError::Verify(VerifyError::FuturePackage))
    );
}

#[test]
fn bytes_that_are_not_a_payload_cannot_fill_an_order() {
    let v = vectors::named("BTC");
    assert!(matches!(
        settle(
            opened(index_of(&v), 1),
            clock_at(v.timestamp_ms),
            &[0xFF; 64],
            OURS
        ),
        Err(SettleError::Verify(VerifyError::Malformed(_)))
    ));
}

#[test]
fn no_refusal_returns_a_post_state() {
    // U7's requirement, stated as the thing that would break it. A consumer that
    // answered a failed verification with `Ok` and an unchanged account would
    // leave a caller unable to tell "the market has not reached your limit" from
    // "nobody could verify a price" -- and a caller that cannot tell those apart
    // is one that retries the wrong one. So every cause below is an `Err`, and
    // there is no arm of `settle` that returns a price it could not verify.
    let v = vectors::named("BTC");
    let feed = index_of(&v);
    let now = v.timestamp_ms;
    let reached = price(&v);

    let cases: Vec<(
        &str,
        Result<Vec<lee_core::program::AccountPostState>, SettleError>,
    )> = vec![
        (
            "an unreadable payload",
            settle(opened(feed, 1), clock_at(now), &[0xFF; 64], OURS),
        ),
        (
            "a clock the caller chose",
            settle(
                opened(feed, 1),
                clock_account([0u8; 32], now),
                &v.payload,
                OURS,
            ),
        ),
        (
            "a payload past the window",
            settle(
                opened(feed, 1),
                clock_at(now + MAX_AGE_MS + 1),
                &v.payload,
                OURS,
            ),
        ),
        (
            "a payload for another feed",
            settle(
                opened(index_of(&vectors::named("ETH")), 1),
                clock_at(now),
                &v.payload,
                OURS,
            ),
        ),
        (
            "a limit the market has not reached",
            settle(opened(feed, reached + 1), clock_at(now), &v.payload, OURS),
        ),
    ];

    for (what, outcome) in cases {
        assert!(
            outcome.is_err(),
            "{what} produced post-states instead of a refusal"
        );
    }
}
