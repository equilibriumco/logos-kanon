//! The same failure, asked of both modes, answered the same way.
//!
//! U6 asks for a clear, actionable error per failure mode, and the claim this
//! repository makes about it is stronger than two taxonomies kept in step: one
//! enum in `verifier-core` decides, and `pull-lib` returns it unchanged while
//! `submit_price` wraps it. So the parity is an *identity* on the shared causes,
//! and that is what these tests assert -- `push == SubmitError::Verify(pull)`,
//! value for value, rather than code against code.
//!
//! # Why this file is here and not in `pull-lib`
//!
//! It has to see both callers, and `pull-lib` must never see the aggregator --
//! that is M3-02. So `pull-lib` is a *dev*-dependency of this crate, which is the
//! direction that costs nothing. Two separate things make that true, and they are
//! worth keeping apart.
//!
//! Cargo is why the edge cannot reach: a dev-dependency sits outside
//! `--edges normal,build`, so it does not appear in `pull-lib`'s build closure at
//! all. The dependency also points away from the pull path rather than into it,
//! and it exists only in a test binary.
//!
//! CI is what refuses the reverse. `pull-independence` pins that closure to an
//! allow-list, and it runs against this edge: with this dependency in place both
//! closures still come back as exactly `kanon-clock, pull-lib, verifier-core` and
//! that plus `reference-consumer-pull`. Adding the aggregator to `pull-lib`'s
//! `[dependencies]` is what the job exists to fail.
//!
//! # What parity cannot mean
//!
//! Four asymmetries, each real and none of them a defect. They are named here
//! because the honest version of "the errors agree" has to say where they do not.
//!
//! - **The clock is wrapped differently.** `submit_price` reads it before
//!   verification and reports `SubmitError::Clock(TimeError)`; `verify_price`
//!   folds it into `VerifyError::NoClock(TimeError)`. Same leaf cause, same
//!   number -- both route to `clock_code` -- different variant.
//! - **A configuration failure arrives at a different moment.** Push decodes a
//!   registered feed and can fail at `SubmitError::Config`; a pull consumer built
//!   its `FeedConfig` before calling, so `try_new` refused it there and
//!   `VerifyError::InvalidConfig` is unreachable through `verify_price`.
//! - **The pair the caller claims arrives differently.** Push reads it off the
//!   price account when one is populated (ADR 16, ADR 32); pull takes it as the
//!   `expected` argument. Same claim, same refusal, different carrier -- which is
//!   why `an_asset_mismatch_is_the_same_refusal_from_two_carriers` builds the two
//!   sides differently on purpose rather than sharing one input.
//! - **The account layer has no pull analogue.** A feed account, a price account
//!   and a publish are push's alone. `every_submit_cause_is_classified` is what
//!   keeps that list honest as the push taxonomy grows.

use aggregator_program::publish::REDSTONE_SOURCE_ID;
use aggregator_program::submit::{submit_price, SubmitError, PRICE_ACCOUNT_SEED};
use aggregator_program::FeedAccount;
use kanon_clock::CLOCK_ACCOUNT_ID;
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::ProgramId;
use pull_lib::{
    verify_price, AssetPair, FeedConfig, InProgramBackend, PullConfig, SignerAddress, VerifyError,
};
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::time::TimeError;

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

const OURS: ProgramId = [7u32; 8];
const CLOCK_PROGRAM: ProgramId = [88u32; 8];
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 60_000;
const THRESHOLD: u8 = 3;
const BASE: [u8; 32] = [0xB7; 32];
const QUOTE: [u8; 32] = [0x05; 32];
const FEED_ACCOUNT_ID: [u8; 32] = [0xFE; 32];
const DATA_SERVICE: &str = "redstone-primary-prod";

fn padded(name: &str) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..name.len()].copy_from_slice(name.as_bytes());
    id
}

fn pair() -> AssetPair {
    AssetPair::new(BASE, QUOTE)
}

/// The registered feed, as push reads it.
fn feed_account(v: &Vector, signers: &[SignerAddress], threshold: u8) -> AccountWithMetadata {
    let state = FeedAccount {
        feed_id: padded(&v.feed_id),
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers: signers.iter().map(|s| s.0).collect(),
        threshold,
        paused: false,
    };
    AccountWithMetadata {
        account: Account {
            program_owner: OURS,
            balance: 0,
            data: Data::try_from(borsh::to_vec(&state).expect("serialises")).expect("fits"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(FEED_ACCOUNT_ID),
    }
}

/// The same configuration, as a pull consumer holds it. Built from the same
/// values as `feed_account`, which is what makes the comparison a statement
/// about the two paths rather than about two configurations.
fn pull_config<'a>(v: &Vector, signers: &'a [SignerAddress], threshold: u8) -> PullConfig<'a> {
    PullConfig {
        data_service_id: DATA_SERVICE,
        feed: FeedConfig::try_new(
            v.feed_id.as_bytes(),
            pair(),
            DECIMALS,
            MAX_AGE_MS,
            signers,
            threshold,
        )
        .expect("the same feed the account stores"),
    }
}

fn clock_bytes(now_ms: u64) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&now_ms.to_le_bytes());
    data
}

fn clock_account(now_ms: u64) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: CLOCK_PROGRAM,
            balance: 0,
            data: Data::try_from(clock_bytes(now_ms).to_vec()).expect("fits"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(CLOCK_ACCOUNT_ID),
    }
}

fn unwritten_price_account() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: compute_pda(
            &OURS,
            &[&FEED_ACCOUNT_ID, &seed_from_str(PRICE_ACCOUNT_SEED)],
        ),
    }
}

/// One case, put to both paths over identical inputs.
fn both(
    v: &Vector,
    payload: &[u8],
    signers: &[SignerAddress],
    threshold: u8,
    now_ms: u64,
) -> (VerifyError, SubmitError) {
    let pull = verify_price(
        payload,
        &pull_config(v, signers, threshold),
        &pair(),
        &CLOCK_ACCOUNT_ID,
        &clock_bytes(now_ms),
        &InProgramBackend::new(),
    )
    .expect_err("this case is a failure in both modes");

    let push = submit_price(
        feed_account(v, signers, threshold),
        unwritten_price_account(),
        clock_account(now_ms),
        payload,
        OURS,
    )
    .expect_err("this case is a failure in both modes");

    (pull, push)
}

#[test]
fn every_shared_cause_is_the_same_value_in_both_modes() {
    // The identity, over the causes the same input reaches in both paths. Not a
    // code comparison: `pull-lib` assigns no numbers on purpose -- a library
    // consumer decides what a failure means to it -- so the thing both modes
    // share is the value, and the number push puts on it is derived from that.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();

    // A stranger in the roster's place: the payload's own signer is no longer
    // authorised, so the walk refuses at it.
    let mut stranger = signers.clone();
    stranger[0] = SignerAddress([0xAA; 20]);

    // Two addresses that signed nothing, and a threshold the payload cannot
    // reach without them.
    let mut padded_roster = signers.clone();
    padded_roster.push(SignerAddress([0xA1; 20]));
    padded_roster.push(SignerAddress([0xA2; 20]));

    let truncated = &v.payload[..v.payload.len() / 2];

    let cases: Vec<Case<'_>> = vec![
        (
            "a payload that is not a payload",
            truncated,
            &signers,
            THRESHOLD,
            v.timestamp_ms,
        ),
        (
            "a signer the roster does not name",
            &v.payload,
            &stranger,
            THRESHOLD,
            v.timestamp_ms,
        ),
        (
            "a threshold the payload cannot reach",
            &v.payload,
            &padded_roster,
            6,
            v.timestamp_ms,
        ),
        (
            "a clock past the window",
            &v.payload,
            &signers,
            THRESHOLD,
            v.timestamp_ms + MAX_AGE_MS + 1,
        ),
        (
            // Past `MAX_AHEAD_MS`, the skew RedStone's own validator allows: a
            // smaller gap is absorbed rather than refused, which is the point of
            // the constant and not a looseness either mode invents.
            "a clock behind the payload by more than the allowed skew",
            &v.payload,
            &signers,
            THRESHOLD,
            v.timestamp_ms - verifier_core::feed::MAX_AHEAD_MS - 1,
        ),
    ];

    for (name, payload, roster, threshold, now) in cases {
        let (pull, push) = both(&v, payload, roster, threshold, now);
        assert_eq!(
            push,
            SubmitError::Verify(pull),
            "{name}: push wrapped a different cause than pull returned"
        );
    }
}

#[test]
fn a_clock_failure_is_one_cause_under_two_wrappers() {
    // The first asymmetry, asserted rather than only described. `submit_price`
    // reads the clock before verification and so owns the failure; `verify_price`
    // has no layer of its own to put it in and folds it into `VerifyError`. The
    // leaf is the same `TimeError`, which is what an operator acts on and what
    // both paths number identically through `clock_code`.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let wrong_account = [0x11; 32];

    let pull = verify_price(
        &v.payload,
        &pull_config(&v, &signers, THRESHOLD),
        &pair(),
        &wrong_account,
        &clock_bytes(v.timestamp_ms),
        &InProgramBackend::new(),
    )
    .expect_err("a clock the consumer chose is refused");

    let mut elsewhere = clock_account(v.timestamp_ms);
    elsewhere.account_id = AccountId::new(wrong_account);
    let push = submit_price(
        feed_account(&v, &signers, THRESHOLD),
        unwritten_price_account(),
        elsewhere,
        &v.payload,
        OURS,
    )
    .expect_err("and so is one the relayer chose");

    assert_eq!(pull, VerifyError::NoClock(TimeError::WrongAccount));
    assert_eq!(push, SubmitError::Clock(TimeError::WrongAccount));
    assert_ne!(
        push,
        SubmitError::Verify(pull),
        "the wrappers differ, and this is the test that says so"
    );

    // And the half that makes the difference cosmetic: both route to
    // `clock_code`, so the number a caller acts on is the same. Asserted rather
    // than described, because an edit to `verify_code`'s `NoClock` arm would
    // split them silently and this is the test that claims they agree.
    assert_eq!(push.code(), SubmitError::Verify(pull).code());
}

/// One row of the parity table: what it is called, and the four inputs that
/// steer both paths to the same cause.
type Case<'a> = (&'a str, &'a [u8], &'a [SignerAddress], u8, u64);

/// How far a `SubmitError` variant reaches across the two modes.
#[derive(Debug, PartialEq, Eq)]
enum Reach {
    /// The same value pull returns, wrapped.
    Shared,
    /// The same leaf cause pull returns, under a different wrapper.
    SameCauseOtherWrapper,
    /// Pull reaches this cause, but earlier -- when the consumer built its
    /// configuration rather than when it verified.
    SharedEarlier,
    /// Push's alone: there is no feed account, price account or publish in pull.
    PushOnly,
}

#[test]
fn every_submit_cause_is_classified() {
    // The push taxonomy grew four times while M3 was being written -- 812, the
    // 900 block, the 1000 block, and `FeedDeregistered`. This match has no
    // wildcard arm, so the next variant added to `SubmitError` fails to compile
    // here until someone decides whether pull can reach it.
    //
    // What it does not do is prove the classification is right, or that the
    // asymmetries listed in this file's header are complete; an author can pick
    // an arm to make the build pass. What it buys is that the decision is
    // forced at all, which nothing else in the repository does.
    fn reach(err: &SubmitError) -> Reach {
        match err {
            SubmitError::Verify(_) => Reach::Shared,
            SubmitError::Clock(_) => Reach::SameCauseOtherWrapper,
            SubmitError::Config(_) => Reach::SharedEarlier,
            SubmitError::FeedNotOurs
            | SubmitError::FeedDeregistered
            | SubmitError::FeedUndecodable
            | SubmitError::FeedPaused
            | SubmitError::PriceAccountNotOurs
            | SubmitError::PriceAccountUndecodable
            | SubmitError::Publish(_) => Reach::PushOnly,
        }
    }

    assert_eq!(
        reach(&SubmitError::Verify(VerifyError::UnauthorisedSigner)),
        Reach::Shared
    );
    assert_eq!(
        reach(&SubmitError::Clock(TimeError::WrongAccount)),
        Reach::SameCauseOtherWrapper
    );
    assert_eq!(reach(&SubmitError::FeedPaused), Reach::PushOnly);
}

#[test]
fn an_asset_mismatch_is_the_same_refusal_from_two_carriers() {
    // U6 names asset mismatch, so it is asserted rather than left to the
    // structural argument -- but it cannot go in the table above, because the
    // caller's claim about the pair is a parameter in pull and account state in
    // push. The inputs differ in shape on purpose; the refusal must not.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let elsewhere = AssetPair::new([0xEE; 32], QUOTE);

    let pull = verify_price(
        &v.payload,
        &pull_config(&v, &signers, THRESHOLD),
        &elsewhere,
        &CLOCK_ACCOUNT_ID,
        &clock_bytes(v.timestamp_ms),
        &InProgramBackend::new(),
    )
    .expect_err("the consumer contradicted its own configuration");

    // The push carrier: a price account already published under the other pair,
    // which is what `submit_price` takes its expectation from.
    let published = OraclePriceAccount {
        base_asset: AccountId::new(elsewhere.base),
        quote_asset: AccountId::new(elsewhere.quote),
        price: 1,
        timestamp: v.timestamp_ms - 1,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    };
    let push = submit_price(
        feed_account(&v, &signers, THRESHOLD),
        AccountWithMetadata {
            account: Account {
                program_owner: OURS,
                balance: 0,
                data: Data::try_from(borsh::to_vec(&published).expect("serialises")).expect("fits"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: unwritten_price_account().account_id,
        },
        clock_account(v.timestamp_ms),
        &v.payload,
        OURS,
    )
    .expect_err("the account says one pair and the feed says another");

    assert_eq!(pull, VerifyError::AssetMismatch);
    assert_eq!(push, SubmitError::Verify(pull));
}
