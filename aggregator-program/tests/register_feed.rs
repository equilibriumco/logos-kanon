//! Registration as a transaction, not as a decision.
//!
//! The unit tests beside `register.rs` cover what a registration decides. These
//! cover the part only a transaction can show: that the post-states LEZ is
//! handed are ones it accepts, and that the claim names the address the declared
//! constraint derives.

use aggregator_program::register::{register_feed, RegisterError, FEED_ACCOUNT_SEED};
use aggregator_program::FeedAccount;
use borsh::BorshDeserialize;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, ProgramId};
use spel_framework::pda::{compute_pda, seed_from_str};

const OURS: ProgramId = [7u32; 8];
/// Whatever owns an operator's key on chain -- not this program, and not nobody.
const WALLET_PROGRAM: ProgramId = [42u32; 8];

const BASE: [u8; 32] = [0xB7; 32];
const QUOTE: [u8; 32] = [0x05; 32];
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 60_000;

fn feed_id(name: &[u8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..name.len()].copy_from_slice(name);
    id
}

/// The address the constraint derives, computed the way a client would.
fn feed_address(id: &[u8; 32]) -> AccountId {
    compute_pda(&OURS, &[id, &seed_from_str(FEED_ACCOUNT_SEED)])
}

fn unregistered(id: &[u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: feed_address(id),
    }
}

/// An account as one exists on chain: owned, funded, having signed before. Not
/// `Account::default()`, for the reason `tests/admin.rs` records -- LEZ strands a
/// default-owned account after its first transaction.
fn on_chain(tag: u8, signs: bool) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: WALLET_PROGRAM,
            balance: 500,
            data: Data::default(),
            nonce: Nonce(7),
        },
        is_authorized: signs,
        account_id: AccountId::new([tag; 32]),
    }
}

fn signers(n: u8) -> Vec<[u8; 20]> {
    (1..=n).map(|i| [i; 20]).collect()
}

fn register(
    feed: AccountWithMetadata,
    id: [u8; 32],
) -> Result<Vec<AccountPostState>, RegisterError> {
    register_feed(
        feed,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        id,
        BASE,
        QUOTE,
        DECIMALS,
        MAX_AGE_MS,
        signers(5),
        3,
    )
}

#[test]
fn a_registration_lands_and_claims_the_address_the_constraint_checks() {
    let id = feed_id(b"BTC");
    let feed = unregistered(&id);
    let pre = vec![feed.clone(), on_chain(0xAD, true), on_chain(0xC0, false)];

    let posts = register(feed, id).expect("a usable feed");

    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the post-states");
    assert_eq!(
        posts[0].account().program_owner,
        Account::default().program_owner,
        "the owner is set by the honoured claim, not by the program"
    );
}

#[test]
fn the_registered_feed_is_the_one_a_submission_reads() {
    // The seam between this instruction and `submit_price`: what registration
    // writes has to decode as the feed the submission path reads, and configure.
    let id = feed_id(b"ETH");

    let posts = register(unregistered(&id), id).expect("a usable feed");

    let stored =
        FeedAccount::try_from_slice(posts[0].account().data.as_ref()).expect("decodes as a feed");
    let addresses = stored.signer_addresses();
    let config = stored.config(&addresses).expect("configures");
    assert_eq!(config.feed_id(), &id);
    assert_eq!(config.threshold(), 3);
    assert!(!stored.paused);
}

#[test]
fn registering_the_same_feed_id_twice_is_refused() {
    // R2, and it costs no mechanism: one feed id has one address, so the second
    // registration arrives at an account this program already owns. Nothing
    // partial can be written because the write never begins.
    let id = feed_id(b"SOL");
    let posts = register(unregistered(&id), id).expect("the first registration");

    let mut existing = unregistered(&id);
    existing.account = posts[0].account().clone();
    existing.account.program_owner = OURS;

    assert_eq!(
        register(existing, id),
        Err(RegisterError::AlreadyRegistered),
        "the address is the feed id's, so a duplicate cannot be a fresh account"
    );
}

#[test]
fn two_feeds_are_two_accounts() {
    // The other half of the address decision: distinct ids cannot collide, so
    // registering one feed can never disturb another. R2 again, from the side
    // the requirement is written from.
    let btc = feed_id(b"BTC");
    let eth = feed_id(b"ETH");
    assert_ne!(feed_address(&btc), feed_address(&eth));

    for id in [btc, eth] {
        let feed = unregistered(&id);
        let pre = vec![feed.clone(), on_chain(0xAD, true), on_chain(0xC0, false)];
        let posts = register(feed, id).expect("a usable feed");
        validate_execution(&pre, &posts, OURS).expect("each lands on its own");
    }
}
