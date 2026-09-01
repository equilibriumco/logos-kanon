//! The consumer end to end, over payloads from the committed RedStone capture.
//!
//! What this file owns is the seam, not the verification. Whether a median is
//! right, a threshold is counted correctly or a window is applied at the right
//! edge belongs to `verifier-core` and is tested there once for both modes. Here
//! the questions are the ones only a program can answer: does a real payload move
//! real account state, does LEZ accept what comes back, does a rotation change
//! what verifies, and does every typed refusal leave the order where it was.
//!
//! The signatures are RedStone's. The envelope around them was assembled by
//! `scripts/capture-redstone-vectors.py`, because the gateway serves per-signer
//! JSON rather than a finished payload.

use borsh::BorshDeserialize;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, ProgramId};
use pull_lib::verifier_core::value::median;
use pull_lib::{TimeError, VerifyError, CLOCK_ACCOUNT_ID};
use reference_consumer_pull::authority::{self, config_address};
use reference_consumer_pull::trust::{self, FeedTrust, TRUST_ACCOUNT_SEED};
use reference_consumer_pull::{
    open_order, padded, settle, OrderAccount, SettleError, TrustError, ORDER_ACCOUNT_SEED,
};
use spel_framework::pda::{compute_pda, seed_from_str};

#[path = "../../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

const OURS: ProgramId = [7u32; 8];
const WALLET: ProgramId = [42u32; 8];
const CLOCK_PROGRAM: ProgramId = [88u32; 8];
const GENESIS: [u8; 32] = [0x61; 32];
const OWNER: [u8; 32] = [0xA0; 32];
const ORDER_ID: [u8; 32] = [0x0D; 32];

/// RedStone's scale and this consumer's tolerance for the captured feeds.
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 60_000;
const THRESHOLD: u8 = 3;
const SERVICE: &str = "redstone-primary-prod";

/// How far past the clock a package may be dated before it reads as future.
/// `verifier-core`'s constant, restated because it is not part of the surface a
/// consumer links.
const MAX_AHEAD_MS: u64 = 3 * 60 * 1000;

fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: WALLET,
            balance: 500,
            data: Data::default(),
            nonce: Nonce(3),
        },
        is_authorized: signs,
        account_id: AccountId::new(id),
    }
}

fn untouched(at: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: at,
    }
}

fn as_chain_leaves_it(post: &AccountPostState, at: AccountId) -> AccountWithMetadata {
    let mut account = post.account().clone();
    account.program_owner = OURS;
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: at,
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

fn established() -> AccountWithMetadata {
    let at = config_address(&OURS);
    let posts = authority::establish(untouched(at), key(GENESIS, true), &GENESIS, OURS)
        .expect("the genesis key establishes the authority");
    as_chain_leaves_it(&posts[0], at)
}

fn feed_id(v: &Vector) -> [u8; 32] {
    padded(v.feed_id.as_bytes())
}

fn trust_address(feed: &[u8; 32]) -> AccountId {
    compute_pda(&OURS, &[feed, &seed_from_str(TRUST_ACCOUNT_SEED)])
}

fn order_address() -> AccountId {
    compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
}

/// A trust account for this capture's feed, holding the roster that actually
/// signed it.
fn registered(v: &Vector) -> AccountWithMetadata {
    registered_with(v, v.signers.iter().map(|s| s.0).collect(), THRESHOLD)
}

/// A registration that labels this capture's feed with a pair of the caller's
/// choosing, which is how a mislabelling is reached.
fn registered_as(v: &Vector, base: [u8; 32], quote: [u8; 32]) -> AccountWithMetadata {
    let feed = feed_id(v);
    let posts = trust::register(
        untouched(trust_address(&feed)),
        key(GENESIS, true),
        established(),
        SERVICE.to_owned(),
        feed,
        base,
        quote,
        DECIMALS,
        MAX_AGE_MS,
        v.signers.iter().map(|s| s.0).collect(),
        THRESHOLD,
        OURS,
    )
    .expect("a usable registration");
    as_chain_leaves_it(&posts[0], trust_address(&feed))
}

fn registered_with(v: &Vector, signers: Vec<[u8; 20]>, threshold: u8) -> AccountWithMetadata {
    let feed = feed_id(v);
    let posts = trust::register(
        untouched(trust_address(&feed)),
        key(GENESIS, true),
        established(),
        SERVICE.to_owned(),
        feed,
        feed,
        padded(b"USD"),
        DECIMALS,
        MAX_AGE_MS,
        signers,
        threshold,
        OURS,
    )
    .expect("a usable registration");
    as_chain_leaves_it(&posts[0], trust_address(&feed))
}

/// The trust account after the authority rotates its roster.
fn rotated(v: &Vector, signers: Vec<[u8; 20]>, threshold: u8) -> AccountWithMetadata {
    let feed = feed_id(v);
    let posts = trust::rotate_signers(
        registered(v),
        key(GENESIS, true),
        established(),
        feed,
        signers,
        threshold,
        OURS,
    )
    .expect("a usable rotation");
    as_chain_leaves_it(&posts[0], trust_address(&feed))
}

fn opened(v: &Vector, limit: u128) -> AccountWithMetadata {
    let posts = open_order(
        untouched(order_address()),
        key(OWNER, true),
        registered(v),
        feed_id(v),
        feed_id(v),
        padded(b"USD"),
        limit,
        ORDER_ID,
        OURS,
    )
    .expect("a usable order");
    as_chain_leaves_it(&posts[0], order_address())
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
    // Every captured feed, and the whole path: establish the authority, register
    // what the program trusts, open an order, settle it. The limit is one, which
    // every real price clears.
    for v in vectors::all() {
        let posts = settle(
            opened(&v, 1),
            registered(&v),
            clock_at(v.timestamp_ms),
            feed_id(&v),
            &v.payload,
            OURS,
        )
        .unwrap_or_else(|err| panic!("{} should fill: {err:?}", v.feed_id));

        assert_eq!(
            posts.len(),
            3,
            "{}: the order, the trust and the clock",
            v.feed_id
        );
        assert!(stored(posts[0].account()).filled, "{}", v.feed_id);
    }
}

#[test]
fn a_settlement_returns_one_post_state_per_account_it_was_given() {
    // Two accounts here are read and never written, and both still have to come
    // back: `validate_execution` zips pre-states and post-states positionally and
    // requires equal length (rule 2), so returning the order alone would fail
    // every successful settlement on chain.
    let v = vectors::named("BTC");
    let trust = registered(&v);
    let clock = clock_at(v.timestamp_ms);

    let posts = settle(
        opened(&v, 1),
        trust.clone(),
        clock.clone(),
        feed_id(&v),
        &v.payload,
        OURS,
    )
    .expect("fills");

    assert_eq!(posts.len(), 3);
    assert!(stored(posts[0].account()).filled);
    assert_eq!(
        posts[1].account(),
        &trust.account,
        "the trust account comes back exactly as it was given"
    );
    assert_eq!(posts[2].account(), &clock.account, "and so does the clock");
    assert!(posts.iter().all(|p| p.required_claim().is_none()));
}

#[test]
fn the_post_states_pass_lez() {
    // Rather than restating rules 1 through 8: hand what `settle` returned to the
    // function that enforces them, unmodified. Unmodified is the whole value of
    // this test -- a version that appended the accounts the program forgot would
    // assert that LEZ accepts a list this program does not produce.
    let v = vectors::named("BTC");
    let pre = vec![opened(&v, 1), registered(&v), clock_at(v.timestamp_ms)];

    let posts = settle(
        opened(&v, 1),
        registered(&v),
        clock_at(v.timestamp_ms),
        feed_id(&v),
        &v.payload,
        OURS,
    )
    .expect("fills");
    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the fill");
}

#[test]
fn a_fill_moves_the_filled_flag_and_nothing_else() {
    let v = vectors::named("BTC");
    let before = opened(&v, 1);
    let was = stored(&before.account);

    let posts = settle(
        before.clone(),
        registered(&v),
        clock_at(v.timestamp_ms),
        feed_id(&v),
        &v.payload,
        OURS,
    )
    .expect("the order fills");
    let now = stored(posts[0].account());

    assert_eq!(now.owner, was.owner);
    assert_eq!(now.feed_id, was.feed_id);
    assert_eq!(now.limit_price_q64, was.limit_price_q64);
    assert!(!was.filled && now.filled, "only the flag moves");

    assert_eq!(
        posts[0].account().program_owner,
        before.account.program_owner
    );
    assert_eq!(posts[0].account().balance, before.account.balance);
    assert_eq!(posts[0].account().nonce, before.account.nonce);
}

#[test]
fn a_rotation_changes_which_payloads_verify_without_a_redeployment() {
    // The reason the roster is state rather than a constant, asserted end to end
    // over real signatures. RedStone rotates -- ADR 30 records that the addresses
    // in the capture are unchanged since it was taken and that a source two and a
    // half years older shares none of them -- and a consumer that met that event
    // with a rebuild would move every derived address and abandon every open
    // order.
    let v = vectors::named("BTC");
    let feed = feed_id(&v);
    let captured: Vec<[u8; 20]> = v.signers.iter().map(|s| s.0).collect();

    // As registered, the payload fills.
    settle(
        opened(&v, 1),
        registered(&v),
        clock_at(v.timestamp_ms),
        feed,
        &v.payload,
        OURS,
    )
    .expect("the captured roster verifies the captured payload");

    // Rotated to a roster that drops one of the signers who signed it, the same
    // payload no longer does. Every package carries the requested feed, so the
    // dropped signer's package is recovered and refused outright (ADR 15) rather
    // than going uncounted -- narrowing a roster refuses payloads rather than
    // verifying against the remainder.
    let narrowed = rotated(&v, captured[..4].to_vec(), THRESHOLD);
    assert_eq!(
        settle(
            opened(&v, 1),
            narrowed,
            clock_at(v.timestamp_ms),
            feed,
            &v.payload,
            OURS
        ),
        Err(SettleError::Verify(VerifyError::UnauthorisedSigner))
    );

    // And a rotation that adds is backward compatible: an added signer that did
    // not report contributes nothing towards the threshold, and every payload
    // that verified before still verifies.
    let mut widened = captured.clone();
    widened.push([0xEE; 20]);
    settle(
        opened(&v, 1),
        rotated(&v, widened, THRESHOLD),
        clock_at(v.timestamp_ms),
        feed,
        &v.payload,
        OURS,
    )
    .expect("adding a signer invalidates nothing");
}

#[test]
fn an_order_opened_before_a_rotation_is_settled_under_the_roster_in_force() {
    // The exposure that comes with having an authority, stated as a test rather
    // than left implicit. An order is a claim about a price, and who may speak for
    // that price is the program's to change while the order is open -- which is
    // the same exposure a push consumer has to `update_signer_set`.
    //
    // What an authority cannot change under an order is what the order is priced
    // against: the pair, the scale and the window are fixed at registration and
    // `rotate_signers` moves only the roster and its threshold.
    let v = vectors::named("BTC");
    let order = opened(&v, 1);
    let captured: Vec<[u8; 20]> = v.signers.iter().map(|s| s.0).collect();

    let outcome = settle(
        order,
        rotated(&v, captured[..4].to_vec(), THRESHOLD),
        clock_at(v.timestamp_ms),
        feed_id(&v),
        &v.payload,
        OURS,
    );
    assert_eq!(
        outcome,
        Err(SettleError::Verify(VerifyError::UnauthorisedSigner)),
        "the roster in force at settlement is the one that decides"
    );
}

/// The trust account after the authority retires the feed.
fn retired(v: &Vector) -> AccountWithMetadata {
    let feed = feed_id(v);
    let posts = trust::deregister(registered(v), key(GENESIS, true), established(), feed, OURS)
        .expect("a registered feed retires");
    as_chain_leaves_it(&posts[0], trust_address(&feed))
}

/// The same feed id registered again, under a pair the orders were not opened
/// against.
fn re_registered_under_another_pair(v: &Vector) -> AccountWithMetadata {
    let feed = feed_id(v);
    let posts = trust::register(
        retired(v),
        key(GENESIS, true),
        established(),
        SERVICE.to_owned(),
        feed,
        padded(b"XBT"),
        padded(b"EUR"),
        DECIMALS,
        MAX_AGE_MS,
        v.signers.iter().map(|s| s.0).collect(),
        THRESHOLD,
        OURS,
    )
    .expect("the id is not spent");
    as_chain_leaves_it(&posts[0], trust_address(&feed))
}

#[test]
fn a_mislabelled_registration_cannot_fill_an_order_its_owner_did_not_intend() {
    // U7's asset-pair verification, and the attack it answers. RedStone's `BTC`
    // feed registered as ETH/USD verifies real BTC packages perfectly well: the
    // signatures are genuine, the threshold is met, the timestamp is fresh. No
    // signer attests to which assets a feed prices, so nothing in the payload can
    // catch it.
    //
    // The owner's expectation is what catches it, at the open, where the owner can
    // still walk away.
    let v = vectors::named("BTC");
    let feed = feed_id(&v);
    let mislabelled = registered_as(&v, padded(b"ETH"), padded(b"USD"));

    assert_eq!(
        open_order(
            untouched(order_address()),
            key(OWNER, true),
            mislabelled,
            feed,
            padded(b"BTC"),
            padded(b"USD"),
            1,
            ORDER_ID,
            OURS
        ),
        Err(reference_consumer_pull::OpenError::PairMismatch)
    );
}

#[test]
fn an_order_cannot_be_filled_under_a_pair_its_owner_never_signed_for() {
    // The settlement half of the same check, and the reason the order stores the
    // pair rather than re-reading it. An authority may retire a feed and register
    // the id again under another pair -- that is the recovery path for a
    // mis-registration -- and an order opened before that must not fill under the
    // new meaning.
    //
    // This is also what makes `AssetMismatch` reachable in this consumer at all.
    // A version that passed the trust account's own pair as the expected one had a
    // comparison that could not fail, which is not asset-pair verification.
    let v = vectors::named("BTC");
    let order = opened(&v, 1);

    assert_eq!(
        settle(
            order,
            re_registered_under_another_pair(&v),
            clock_at(v.timestamp_ms),
            feed_id(&v),
            &v.payload,
            OURS
        ),
        Err(SettleError::Verify(VerifyError::AssetMismatch))
    );
}

#[test]
fn a_retired_feed_refuses_a_settlement_as_retired() {
    // Its own cause. An operator needs "the authority retired this" rather than
    // "the bytes were unreadable", and the two go to different people.
    let v = vectors::named("BTC");
    assert_eq!(
        settle(
            opened(&v, 1),
            retired(&v),
            clock_at(v.timestamp_ms),
            feed_id(&v),
            &v.payload,
            OURS
        ),
        Err(SettleError::Trust(TrustError::Deregistered))
    );
}

#[test]
fn the_same_pair_re_registers_and_the_order_still_fills() {
    // The other half, and the one the recovery path exists for: retiring a feed
    // does not spend its id, and an order priced against a pair that comes back
    // unchanged is unaffected.
    let v = vectors::named("BTC");
    let order = opened(&v, 1);
    let feed = feed_id(&v);
    let posts = trust::register(
        retired(&v),
        key(GENESIS, true),
        established(),
        SERVICE.to_owned(),
        feed,
        feed,
        padded(b"USD"),
        DECIMALS,
        MAX_AGE_MS,
        v.signers.iter().map(|s| s.0).collect(),
        THRESHOLD,
        OURS,
    )
    .expect("the id is not spent");

    settle(
        order,
        as_chain_leaves_it(&posts[0], trust_address(&feed)),
        clock_at(v.timestamp_ms),
        feed,
        &v.payload,
        OURS,
    )
    .expect("the same pair is not a change");
}

#[test]
fn a_price_below_the_limit_leaves_the_order_open() {
    let v = vectors::named("BTC");
    let reached = price(&v);
    assert_eq!(
        settle(
            opened(&v, reached + 1),
            registered(&v),
            clock_at(v.timestamp_ms),
            feed_id(&v),
            &v.payload,
            OURS
        ),
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
    settle(
        opened(&v, price(&v)),
        registered(&v),
        clock_at(v.timestamp_ms),
        feed_id(&v),
        &v.payload,
        OURS,
    )
    .expect("an order at the market price fills");
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
                opened(&v, 1),
                registered(&v),
                clock_account(id, v.timestamp_ms),
                feed_id(&v),
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
fn a_payload_past_the_registered_window_cannot_fill_an_order() {
    // `max_age_ms` is what the authority registered for this feed, and is not
    // something a payload or its sender can widen. One millisecond past it is the
    // whole test: the window's edges are inclusive and `verifier-core` owns which
    // side.
    let v = vectors::named("BTC");
    settle(
        opened(&v, 1),
        registered(&v),
        clock_at(v.timestamp_ms + MAX_AGE_MS),
        feed_id(&v),
        &v.payload,
        OURS,
    )
    .expect("the far edge of the window is inside it");

    assert_eq!(
        settle(
            opened(&v, 1),
            registered(&v),
            clock_at(v.timestamp_ms + MAX_AGE_MS + 1),
            feed_id(&v),
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
            opened(&v, 1),
            registered(&v),
            clock_at(v.timestamp_ms - MAX_AHEAD_MS - 1),
            feed_id(&v),
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
            opened(&v, 1),
            registered(&v),
            clock_at(v.timestamp_ms),
            feed_id(&v),
            &[0xFF; 64],
            OURS
        ),
        Err(SettleError::Verify(VerifyError::Malformed(_)))
    ));
}

#[test]
fn a_roster_a_registration_would_refuse_never_reaches_a_settlement() {
    // The guards are `verifier-core`'s and are tested there. What this asserts is
    // that they run at the registration, so a feed whose roster cannot be used is
    // never a feed a settlement has to diagnose.
    let v = vectors::named("BTC");
    let feed = feed_id(&v);
    let outcome = trust::register(
        untouched(trust_address(&feed)),
        key(GENESIS, true),
        established(),
        SERVICE.to_owned(),
        feed,
        feed,
        padded(b"USD"),
        DECIMALS,
        MAX_AGE_MS,
        vec![[1u8; 20], [1u8; 20], [3u8; 20]],
        2,
        OURS,
    );
    assert!(matches!(outcome, Err(TrustError::Config(_))));
}

#[test]
fn no_refusal_returns_a_post_state() {
    // U7's requirement, stated as the thing that would break it. A consumer that
    // answered a failed verification with `Ok` and unchanged accounts would leave
    // a caller unable to tell "the market has not reached your limit" from
    // "nobody could verify a price" -- and a caller that cannot tell those apart
    // retries the wrong one. So every cause below is an `Err`, and there is no arm
    // of `settle` that returns a price it could not verify.
    let v = vectors::named("BTC");
    let feed = feed_id(&v);
    let now = v.timestamp_ms;
    let reached = price(&v);
    let captured: Vec<[u8; 20]> = v.signers.iter().map(|s| s.0).collect();

    let cases: Vec<(&str, Result<Vec<AccountPostState>, SettleError>)> = vec![
        (
            "an unreadable payload",
            settle(
                opened(&v, 1),
                registered(&v),
                clock_at(now),
                feed,
                &[0xFF; 64],
                OURS,
            ),
        ),
        (
            "a clock the caller chose",
            settle(
                opened(&v, 1),
                registered(&v),
                clock_account([0u8; 32], now),
                feed,
                &v.payload,
                OURS,
            ),
        ),
        (
            "a payload past the window",
            settle(
                opened(&v, 1),
                registered(&v),
                clock_at(now + MAX_AGE_MS + 1),
                feed,
                &v.payload,
                OURS,
            ),
        ),
        (
            "a roster the authority narrowed",
            settle(
                opened(&v, 1),
                rotated(&v, captured[..4].to_vec(), THRESHOLD),
                clock_at(now),
                feed,
                &v.payload,
                OURS,
            ),
        ),
        (
            "a trust account for another feed",
            settle(
                opened(&v, 1),
                registered(&vectors::named("ETH")),
                clock_at(now),
                feed,
                &v.payload,
                OURS,
            ),
        ),
        (
            "a limit the market has not reached",
            settle(
                opened(&v, reached + 1),
                registered(&v),
                clock_at(now),
                feed,
                &v.payload,
                OURS,
            ),
        ),
    ];

    for (what, outcome) in cases {
        assert!(
            outcome.is_err(),
            "{what} produced post-states instead of a refusal"
        );
    }
}

#[test]
fn the_registered_configuration_is_what_verification_reads() {
    // The trust account is the single source, so this is also the check that the
    // registration round-trips: the roster the authority wrote is the roster a
    // recovered address is looked up in.
    let v = vectors::named("BTC");
    let registered = FeedTrust::try_from_slice(registered(&v).account.data.as_ref())
        .expect("decodes as a trust");

    assert_eq!(
        registered.signers,
        v.signers.iter().map(|s| s.0).collect::<Vec<_>>()
    );
    assert_eq!(registered.threshold, THRESHOLD);
    assert_eq!(registered.max_age_ms, MAX_AGE_MS);
    assert_eq!(registered.decimals, DECIMALS);
    assert_eq!(registered.data_service_id, SERVICE);
}
