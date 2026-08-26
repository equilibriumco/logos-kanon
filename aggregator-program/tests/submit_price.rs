//! The submission path, driven by payloads RedStone published.
//!
//! The unit tests beside `submit_price` cover what it refuses before any
//! cryptography runs. These cover the other half: a real signed payload going
//! all the way through to an account, and the create-then-update pair that only
//! a payload can reach.
//!
//! The fixture is the committed capture, included by path rather than shared
//! through a crate for the reason its own header gives -- a fixture reader has no
//! business in the published surface of a crate consumer programs link.

use aggregator_program::publish::{PublishError, REDSTONE_SOURCE_ID};
use aggregator_program::submit::{submit_price, SubmitError, PRICE_ACCOUNT_SEED};
use aggregator_program::FeedAccount;
use kanon_clock::CLOCK_ACCOUNT_ID;
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, Claim, ProgramId};
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::value::median;
use verifier_core::VerifyError;

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

/// This program, in every test. A `ProgramId` is an ELF image id in production;
/// here it only has to be something other than the default.
const OURS: ProgramId = [7u32; 8];
/// The clock program owns the clock account. Irrelevant to anything
/// `submit_price` decides, which reads the account *id* -- but a clock owned by
/// nobody is a state LEZ does not have, and `validate_execution` refuses a
/// post-state carrying one.
const CLOCK_PROGRAM: ProgramId = [88u32; 8];

/// RedStone's scale for `redstone-primary-prod`, the same constant the
/// conformance suite pins.
const DECIMALS: u8 = 8;

/// Wide enough that these vectors are never stale against a clock set to their
/// own round. Staleness has its own test, against a clock it moves.
const MAX_AGE_MS: u64 = 60_000;

const THRESHOLD: u8 = 3;

const BASE: [u8; 32] = [0xB7; 32];
const QUOTE: [u8; 32] = [0x05; 32];

const FEED_ACCOUNT_ID: [u8; 32] = [0xFE; 32];

fn vector(feed: &str) -> Vector {
    vectors::named(feed)
}

fn feed_state(v: &Vector) -> FeedAccount {
    let mut feed_id = [0u8; 32];
    feed_id[..v.feed_id.len()].copy_from_slice(v.feed_id.as_bytes());
    FeedAccount {
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers: v.signers.iter().map(|s| s.0).collect(),
        threshold: THRESHOLD,
        paused: false,
    }
}

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

fn feed_account(state: &FeedAccount) -> AccountWithMetadata {
    account(
        OURS,
        borsh::to_vec(state).expect("serialises"),
        FEED_ACCOUNT_ID,
    )
}

/// The address the constraint derives, computed the way a client would.
fn price_account_id() -> AccountId {
    compute_pda(
        &OURS,
        &[&FEED_ACCOUNT_ID, &seed_from_str(PRICE_ACCOUNT_SEED)],
    )
}

fn no_price_account() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: price_account_id(),
    }
}

fn existing_price_account(published: &OraclePriceAccount) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: OURS,
            balance: 0,
            data: Data::from(published),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: price_account_id(),
    }
}

fn clock_at(ms: u64) -> AccountWithMetadata {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&ms.to_le_bytes());
    account(CLOCK_PROGRAM, data.to_vec(), CLOCK_ACCOUNT_ID)
}

/// The median the gateway's own decoded values imply, on the account's scale.
///
/// Derived from the capture's `values` rather than from anything this crate
/// decodes, so it is an expectation and not a restatement of the code under
/// test.
fn expected_price(v: &Vector) -> u128 {
    let mut values = v.values.clone();
    median(&mut values)
        .expect("five values have a median")
        .to_q64_64(DECIMALS)
        .expect("a RedStone price fits Q64.64")
}

fn submit(
    feed: AccountWithMetadata,
    price_account: AccountWithMetadata,
    clock: AccountWithMetadata,
    payload: &[u8],
) -> Result<Vec<AccountPostState>, SubmitError> {
    submit_price(feed, price_account, clock, payload, OURS)
}

fn written(post_states: &[AccountPostState]) -> OraclePriceAccount {
    OraclePriceAccount::try_from(&post_states[1].account().data).expect("a price account")
}

#[test]
fn a_captured_payload_creates_the_price_account_on_the_first_submission() {
    // Every vector, because the five feeds differ in value width and this is the
    // one test cheap enough to run over all of them.
    for v in vectors::all() {
        let post_states = submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(v.timestamp_ms),
            &v.payload,
        )
        .unwrap_or_else(|err| panic!("{} should publish: {err}", v.feed_id));

        let account = written(&post_states);
        assert_eq!(account.base_asset.into_value(), BASE, "{}", v.feed_id);
        assert_eq!(account.quote_asset.into_value(), QUOTE, "{}", v.feed_id);
        assert_eq!(account.price, expected_price(&v), "{}", v.feed_id);
        assert_eq!(account.timestamp, v.timestamp_ms, "{}", v.feed_id);
        assert_eq!(account.source_id, REDSTONE_SOURCE_ID, "{}", v.feed_id);
        assert_eq!(account.confidence_interval, 0, "{}", v.feed_id);
    }
}

#[test]
fn a_first_submission_claims_the_account_it_creates() {
    let v = vector("BTC");
    let post_states = submit(
        feed_account(&feed_state(&v)),
        no_price_account(),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("publishes");

    assert!(
        matches!(post_states[1].required_claim(), Some(Claim::Pda(_))),
        "a first write has to claim the account or nothing owns it"
    );
    assert!(
        post_states[0].required_claim().is_none() && post_states[2].required_claim().is_none(),
        "only the price account is claimed"
    );
}

#[test]
fn the_claimed_seed_is_the_address_a_client_derives() {
    // The seed literal lives twice -- here through `PRICE_ACCOUNT_SEED`, and as
    // a `r#const` seed in the guest's attribute. This asserts the address the
    // claim commits to, which is the thing that has to match; `tests/idl.rs`
    // covers the literal itself.
    let v = vector("BTC");
    let post_states = submit(
        feed_account(&feed_state(&v)),
        no_price_account(),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("publishes");

    let Some(Claim::Pda(seed)) = post_states[1].required_claim() else {
        panic!("a first write claims a PDA");
    };
    assert_eq!(
        AccountId::for_public_pda(&OURS, &seed),
        price_account_id(),
        "the claim has to name the address the constraint checked"
    );
}

#[test]
fn a_newer_round_updates_the_account_this_program_already_owns() {
    // The capture holds one round per feed, so the update is reached by putting
    // an older observation in the account rather than by finding a newer
    // payload. What is under test is the update path, and an account dated
    // before the payload is exactly the state that path exists for.
    let v = vector("BTC");
    let stale = OraclePriceAccount {
        base_asset: AccountId::new(BASE),
        quote_asset: AccountId::new(QUOTE),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    };

    let post_states = submit(
        feed_account(&feed_state(&v)),
        existing_price_account(&stale),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("a newer round publishes");

    let account = written(&post_states);
    assert_eq!(account.price, expected_price(&v));
    assert_eq!(account.timestamp, v.timestamp_ms);
}

#[test]
fn an_update_emits_no_claim() {
    // The account is already owned, and `validate_execution` refuses a claim on
    // anything but a default-owned account -- so claiming here would turn every
    // update after the first into a rejected transaction.
    let v = vector("BTC");
    let stale = OraclePriceAccount {
        base_asset: AccountId::new(BASE),
        quote_asset: AccountId::new(QUOTE),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    };

    let post_states = submit(
        feed_account(&feed_state(&v)),
        existing_price_account(&stale),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("publishes");

    assert!(post_states.iter().all(|p| p.required_claim().is_none()));
}

#[test]
fn an_update_moves_price_and_timestamp_and_nothing_else() {
    let v = vector("BTC");
    let stale = OraclePriceAccount {
        base_asset: AccountId::new(BASE),
        quote_asset: AccountId::new(QUOTE),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 7,
    };
    let before = existing_price_account(&stale);

    let post_states = submit(
        feed_account(&feed_state(&v)),
        before.clone(),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("publishes");

    let after = post_states[1].account();
    assert_eq!(after.program_owner, before.account.program_owner);
    assert_eq!(after.balance, before.account.balance);
    assert_eq!(after.nonce, before.account.nonce);

    let account = written(&post_states);
    assert_eq!(account.base_asset.into_value(), BASE);
    assert_eq!(account.quote_asset.into_value(), QUOTE);
    assert_eq!(account.source_id, REDSTONE_SOURCE_ID);
    assert_eq!(
        account.confidence_interval, 7,
        "`publish` moves two fields, and the interval is not one of them"
    );
}

#[test]
fn the_post_states_are_the_pre_states_in_order_and_the_same_length() {
    let v = vector("BTC");
    let feed = feed_account(&feed_state(&v));
    let price = no_price_account();
    let clock = clock_at(v.timestamp_ms);
    let pre = [feed.clone(), price.clone(), clock.clone()];

    let post_states = submit(feed, price, clock, &v.payload).expect("publishes");

    assert_eq!(post_states.len(), pre.len());
    assert_eq!(
        *post_states[0].account(),
        pre[0].account,
        "the feed is read and not written"
    );
    assert_eq!(
        *post_states[2].account(),
        pre[2].account,
        "the clock is read and not written"
    );
}

#[test]
fn the_post_states_pass_validate_execution() {
    // LEZ's own check rather than a restatement of its rules. It is what decides
    // whether a transaction lands, and it is the reason the invalid third state
    // of the price account is refused before anything reaches here.
    let v = vector("BTC");
    let feed = feed_account(&feed_state(&v));
    let price = no_price_account();
    let clock = clock_at(v.timestamp_ms);
    let pre = vec![feed.clone(), price.clone(), clock.clone()];

    let post_states = submit(feed, price, clock, &v.payload).expect("publishes");

    validate_execution(&pre, &post_states, OURS).expect("LEZ has to accept a first write");

    // And the update after it, which lands by a different rule: the write is
    // permitted by ownership rather than by the pre-state being default.
    let mut stale = written(&post_states);
    stale.timestamp = v.timestamp_ms - 1;
    let existing = existing_price_account(&stale);

    let feed = feed_account(&feed_state(&v));
    let clock = clock_at(v.timestamp_ms);
    let pre = vec![feed.clone(), existing.clone(), clock.clone()];
    let post_states = submit(feed, existing, clock, &v.payload).expect("publishes");

    validate_execution(&pre, &post_states, OURS).expect("LEZ has to accept an update");
}

#[test]
fn replaying_the_same_payload_cannot_move_the_price() {
    let v = vector("BTC");
    let first = submit(
        feed_account(&feed_state(&v)),
        no_price_account(),
        clock_at(v.timestamp_ms),
        &v.payload,
    )
    .expect("publishes");

    let mut published = first[1].account().clone();
    published.program_owner = OURS;
    let existing = AccountWithMetadata {
        account: published,
        is_authorized: false,
        account_id: price_account_id(),
    };

    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            existing,
            clock_at(v.timestamp_ms),
            &v.payload,
        ),
        Err(SubmitError::Publish(PublishError::NotNewer {
            stored: v.timestamp_ms,
            offered: v.timestamp_ms,
        })),
        "the staleness window admits a payload the account has already recorded"
    );
}

#[test]
fn a_signer_outside_the_registered_set_cannot_publish() {
    // One address replaced, so the payload is RedStone's and the roster is not.
    let v = vector("BTC");
    let mut state = feed_state(&v);
    state.signers[0] = [0xAA; 20];

    assert_eq!(
        submit(
            feed_account(&state),
            no_price_account(),
            clock_at(v.timestamp_ms),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::UnauthorisedSigner))
    );
}

#[test]
fn a_clock_past_the_windows_upper_edge_refuses_a_captured_payload() {
    let v = vector("BTC");
    let past_the_edge = v.timestamp_ms + MAX_AGE_MS + 1;

    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(past_the_edge),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::StalePackage))
    );
}

#[test]
fn a_price_account_registered_against_another_pair_is_refused() {
    // The pair the account publishes is what `verify_feed` is given as the
    // caller's claim, so a misrouted write is refused before the recoveries
    // rather than after them.
    let v = vector("BTC");
    let elsewhere = OraclePriceAccount {
        base_asset: AccountId::new([0xEE; 32]),
        quote_asset: AccountId::new(QUOTE),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    };

    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            existing_price_account(&elsewhere),
            clock_at(v.timestamp_ms),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::AssetMismatch))
    );
}

#[test]
fn an_account_another_source_populated_is_not_this_adaptors_to_write() {
    let v = vector("BTC");
    let theirs = OraclePriceAccount {
        base_asset: AccountId::new(BASE),
        quote_asset: AccountId::new(QUOTE),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: AccountId::new([0x5A; 32]),
        confidence_interval: 0,
    };

    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            existing_price_account(&theirs),
            clock_at(v.timestamp_ms),
            &v.payload,
        ),
        Err(SubmitError::Publish(PublishError::SourceMismatch))
    );
}
