//! The submission path, driven by the committed RedStone capture.
//!
//! The unit tests beside `submit_price` cover what it refuses before any
//! cryptography runs. These cover the other half: a payload going all the way
//! through to an account, and the create-then-update pair that only a payload
//! can reach.
//!
//! What is RedStone's here is the packages and the signatures over them. The
//! envelope around them -- the marker, the package count, the unsigned metadata
//! -- was assembled by `scripts/capture-redstone-vectors.py`, because the gateway
//! serves per-signer JSON with decoded values rather than a finished payload.
//! That is the same division the conformance suite works under, and it is where
//! the value is: the signature covers the part nobody here could have produced.
//!
//! The fixture is the committed capture, included by path rather than shared
//! through a crate for the reason its own header gives -- a fixture reader has no
//! business in the published surface of a crate consumer programs link.

use aggregator_program::manage::deregister_feed;
use aggregator_program::publish::{PublishError, REDSTONE_SOURCE_ID};
use aggregator_program::submit::{submit_price, SubmitError, PRICE_ACCOUNT_SEED};
use aggregator_program::FeedAccount;
use kanon_clock::CLOCK_ACCOUNT_ID;
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, Claim, ProgramId};
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::decode::{EMPTY_ENVELOPE_BYTES, REDSTONE_MARKER};
use verifier_core::feed::MAX_AHEAD_MS;
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

/// The address the derivation produces for `FEED_ACCOUNT_ID` under `OURS`,
/// written out.
///
/// Pinned rather than derived, for the reason `REDSTONE_SOURCE_ID` is pinned: a
/// client checking its own implementation needs a number it can compare against,
/// and a test that derives both sides agrees with itself however the formula
/// moves. It also pins the part that is easy to state wrongly -- SPEL widens
/// every seed to 32 bytes before combining them, so hashing the nineteen bytes
/// of the name derives a different account.
const PRICE_ACCOUNT_FOR_THE_FEED: [u8; 32] = [
    0x57, 0xA3, 0x74, 0x1A, 0x9E, 0x3D, 0x92, 0xA8, 0xCC, 0x05, 0x0E, 0xF6, 0x0C, 0xB5, 0x8E, 0xE1,
    0xBF, 0x08, 0x6B, 0xE4, 0xE6, 0xE8, 0xA3, 0x19, 0x2F, 0x61, 0x74, 0x7E, 0x8D, 0xE4, 0xC2, 0xE8,
];

/// The address the constraint derives, computed the way a client would.
fn price_account_id() -> AccountId {
    compute_pda(
        &OURS,
        &[&FEED_ACCOUNT_ID, &seed_from_str(PRICE_ACCOUNT_SEED)],
    )
}

/// An account a retirement only passes through: `authorise` is the guest's call,
/// so `deregister_feed` never inspects these.
fn passthrough(tag: u8) -> AccountWithMetadata {
    account(OURS, Vec::new(), [tag; 32])
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
fn the_derived_price_account_address_has_not_moved() {
    // Every other test here derives both sides from the same code, so a changed
    // formula would go unnoticed as long as it changed consistently. This one
    // compares against a number, which is the only way a change becomes visible.
    //
    // What it cannot do is check the prose. `Instruction::SubmitPrice` documents
    // this formula for clients to reproduce, and a doc comment that drifts from
    // the code still passes here -- nothing mechanical reads it. The constant is
    // what a reviewer checks the prose against.
    assert_eq!(
        price_account_id().into_value(),
        PRICE_ACCOUNT_FOR_THE_FEED,
        "the derivation moved: update this constant, and the formula \
         `Instruction::SubmitPrice` publishes, together"
    );
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
    // One address replaced, so the packages are RedStone's and the roster is not.
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
fn the_window_is_two_sided_and_both_edges_are_inclusive() {
    // The test above takes one step past the upper edge; this takes the window
    // apart. ADR 18 makes validity two-sided -- `[now - max_age, now + MAX_AHEAD]`
    // -- and a captured payload is the only way to check that a real signed
    // package lands where the arithmetic says it should.
    //
    // The lower edge is the one nothing in the push suite covered -- `verifier-core`
    // has it. A chain clock that has fallen behind would otherwise accept a package
    // minted for a moment that has not happened yet, which is the shape a
    // replayed-forward payload would take.
    let v = vector("BTC");

    // Accepted: the last instant of each side.
    for (label, now) in [
        (
            "the oldest instant still fresh",
            v.timestamp_ms + MAX_AGE_MS,
        ),
        (
            "the furthest ahead still allowed",
            v.timestamp_ms - MAX_AHEAD_MS,
        ),
    ] {
        submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(now),
            &v.payload,
        )
        .unwrap_or_else(|e| panic!("{label} has to verify, got {e:?}"));
    }

    // Refused: one millisecond further out on each side, and the two sides do
    // not answer with the same cause.
    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(v.timestamp_ms + MAX_AGE_MS + 1),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::StalePackage)),
        "one millisecond past the age limit is stale"
    );
    assert_eq!(
        submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(v.timestamp_ms - MAX_AHEAD_MS - 1),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::FuturePackage)),
        "one millisecond further ahead than the tolerance is future-dated"
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
fn a_submission_to_an_account_another_source_populated_is_refused() {
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

#[test]
fn the_published_pair_is_what_a_re_registration_may_not_change() {
    // Why `RegisterError::PairChanged` exists, shown rather than asserted. This
    // builds by hand the state the guard now refuses to create, and demonstrates
    // that it is terminal: a retirement empties the *feed* account and leaves the
    // *price* account where it was, and `submit_price` reads the expected pair
    // off the published account on every write but the first. So a feed
    // re-registered under a different pair would answer `AssetMismatch` for
    // every payload, for ever.
    //
    // Unreachable through the instruction set, which is the point --
    // `a_feed_id_may_not_change_its_pair_once_it_has_published` in
    // `deregister_feed.rs` is the guard, and this is the cost of not having one.
    let v = vector("BTC");

    let published = written(
        &submit(
            feed_account(&feed_state(&v)),
            no_price_account(),
            clock_at(v.timestamp_ms),
            &v.payload,
        )
        .expect("the first submission publishes"),
    );
    assert_eq!(published.base_asset.into_value(), BASE);

    // A retirement produces three post-states -- feed, admin, config -- and the
    // price account is not one of them.
    let retired = deregister_feed(
        feed_account(&feed_state(&v)),
        passthrough(0xAD),
        passthrough(0xC0),
        feed_state(&v).feed_id,
        OURS,
    )
    .expect("a live feed retires");
    assert_eq!(
        retired.len(),
        3,
        "deregistration does not touch the price account"
    );

    let other_pair = FeedAccount {
        base_asset: [0xEE; 32],
        ..feed_state(&v)
    };
    assert_eq!(
        submit(
            feed_account(&other_pair),
            existing_price_account(&published),
            clock_at(v.timestamp_ms),
            &v.payload,
        ),
        Err(SubmitError::Verify(VerifyError::AssetMismatch)),
        "the published pair outlives the registration it belonged to"
    );
}

/// The payload cut to its first `n` packages, with the envelope rewritten to
/// match. The same reassembly `methods/tests/cost.rs` uses to price a payload by
/// signer count -- packages are a fixed stride apart, so a prefix of them is a
/// well-formed payload with real signatures over real packages.
fn payload_with(v: &Vector, n: usize) -> Vec<u8> {
    let body_len = v.payload.len() - EMPTY_ENVELOPE_BYTES;
    let count = v.signers.len();
    assert_eq!(body_len % count, 0, "packages are not a fixed stride apart");
    let stride = body_len / count;

    let mut out = v.payload[..n * stride].to_vec();
    out.extend_from_slice(&u16::try_from(n).expect("fits").to_be_bytes());
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&REDSTONE_MARKER);
    out
}

#[test]
fn a_signature_that_cannot_be_recovered_is_refused_by_the_submission_path() {
    // S3's invalid-signature rejection, on the push path. `verifier-core` covers
    // it over synthesised keys; this covers it over a payload RedStone signed,
    // which is the only way to know the refusal survives real framing.
    //
    // The recovery id is the byte to corrupt, and it is the last of each package.
    // Corrupting anything the signature covers instead recovers a *different*
    // address and answers `UnauthorisedSigner` -- a refusal, but a different one,
    // and not the cause this dimension is about.
    let v = vector("BTC");
    let stride = (v.payload.len() - EMPTY_ENVELOPE_BYTES) / v.signers.len();
    let mut corrupted = v.payload.clone();
    corrupted[stride - 1] ^= 0xFF;

    let refused = submit(
        feed_account(&feed_state(&v)),
        no_price_account(),
        clock_at(v.timestamp_ms),
        &corrupted,
    )
    .expect_err("a signature that cannot be recovered has to be refused");

    assert!(
        matches!(
            refused,
            SubmitError::Verify(VerifyError::InvalidSignature(_))
        ),
        "expected the recovery to fail, got {refused:?}"
    );
}

#[test]
fn the_threshold_is_a_boundary_and_the_submission_path_holds_it() {
    // S3's threshold boundaries, on the push path. Both sides of one step, over a
    // real payload cut to three of its five packages -- so every signer present
    // is authorised and the only thing in question is how many of them reported.
    //
    // This is the case `UnauthorisedSigner` hides in a partially-overlapping
    // roster: there the first unknown signer aborts before any count is reached
    // (ADR 15). Here the roster is the whole published set and the payload is
    // short, which is the only way to reach `ThresholdNotMet` through this path.
    let v = vector("BTC");
    let three_packages = payload_with(&v, 3);

    let at_the_threshold = FeedAccount {
        threshold: 3,
        ..feed_state(&v)
    };
    submit(
        feed_account(&at_the_threshold),
        no_price_account(),
        clock_at(v.timestamp_ms),
        &three_packages,
    )
    .expect("three reports meet a threshold of three");

    let one_above = FeedAccount {
        threshold: 4,
        ..feed_state(&v)
    };
    assert_eq!(
        submit(
            feed_account(&one_above),
            no_price_account(),
            clock_at(v.timestamp_ms),
            &three_packages,
        ),
        Err(SubmitError::Verify(VerifyError::ThresholdNotMet {
            met: 3,
            required: 4
        })),
        "three reports do not meet a threshold of four, and the counts say so"
    );
}

#[test]
fn no_registrable_scale_lets_a_captured_price_overflow_its_conversion() {
    // `ScalingOutOfRange`, and why the push path cannot reach it. Not S3's
    // "invalid value": that is `ValueOutOfRange`, which a package carrying a
    // zero, negative or over-wide value raises before any scaling happens
    // (`feed.rs:411`), and reaching it needs a crafted package rather than a
    // captured one. M2-16 owns it.
    //
    // What this covers is the other end -- `to_q64_64` failing at `feed.rs:468`,
    // where the agreed price will not fit the account's scale. The value sits
    // inside the signed package, so altering it answers `UnauthorisedSigner` or
    // `InvalidSignature` long before the conversion runs; the two tests above are
    // that. The only free parameter left is the feed's own `decimals`, and it
    // appears in a divisor, so it can only make the result smaller. `decimals = 0`
    // is therefore the most hostile scale a registration can choose, and it is
    // what this runs.
    //
    // The headroom is seven orders of magnitude: the largest value RedStone
    // published across these five captures is 6,281,896,137,270, and overflow
    // needs `value / 10^decimals` above `2^64`, about 1.8e19. `verifier-core`
    // covers the cause itself over synthesised values, where it is reachable.
    // What this pins is the claim that this path cannot reach it -- so if a
    // capture, a scale or the conversion moves far enough to make it reachable,
    // this stops passing and the claim gets revisited instead of quietly becoming
    // false.
    for v in vectors::all() {
        let widest = FeedAccount {
            decimals: 0,
            ..feed_state(&v)
        };
        submit(
            feed_account(&widest),
            no_price_account(),
            clock_at(v.timestamp_ms),
            &v.payload,
        )
        .unwrap_or_else(|e| panic!("{} at decimals = 0 has to convert, got {e:?}", v.feed_id));
    }
}
