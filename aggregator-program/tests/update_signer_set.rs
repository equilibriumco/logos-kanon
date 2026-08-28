//! A rotation as a transaction, from a feed that was really registered.
//!
//! The unit tests beside `manage.rs` cover what a rotation decides. These cover
//! the seam: that the account `register_feed` produced is one `update_signer_set`
//! accepts, and that the post-states LEZ is handed are ones it takes.

use aggregator_program::manage::{update_signer_set, ManageError};
use aggregator_program::register::{register_feed, FEED_ACCOUNT_SEED};
use aggregator_program::FeedAccount;
use borsh::BorshDeserialize;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, ProgramId};
use spel_framework::pda::{compute_pda, seed_from_str};

const OURS: ProgramId = [7u32; 8];
const WALLET_PROGRAM: ProgramId = [42u32; 8];

fn feed_id(name: &[u8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..name.len()].copy_from_slice(name);
    id
}

fn feed_address(id: &[u8; 32]) -> AccountId {
    compute_pda(&OURS, &[id, &seed_from_str(FEED_ACCOUNT_SEED)])
}

/// An account as one exists on chain: owned, funded, having signed before.
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

fn rotated(n: u8) -> Vec<[u8; 20]> {
    (1..=n).map(|i| [0x80 + i; 20]).collect()
}

/// A feed as the chain leaves it once `register_feed`'s claim is honoured.
fn registered(id: &[u8; 32]) -> AccountWithMetadata {
    let fresh = AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: feed_address(id),
    };
    let posts = register_feed(
        fresh,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        *id,
        [0xB7; 32],
        [0x05; 32],
        8,
        60_000,
        signers(5),
        3,
    )
    .expect("a usable feed");

    let mut account = posts[0].account().clone();
    account.program_owner = OURS;
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: feed_address(id),
    }
}

fn rotate(
    feed: AccountWithMetadata,
    named: [u8; 32],
    set: Vec<[u8; 20]>,
    threshold: u8,
) -> Result<Vec<AccountPostState>, ManageError> {
    update_signer_set(
        feed,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        named,
        set,
        threshold,
        OURS,
    )
}

#[test]
fn a_rotation_lands_on_a_feed_that_was_registered() {
    let id = feed_id(b"BTC");
    let feed = registered(&id);
    let pre = vec![feed.clone(), on_chain(0xAD, true), on_chain(0xC0, false)];

    let posts = rotate(feed, id, rotated(5), 4).expect("a usable set");

    validate_execution(&pre, &posts, OURS).expect("LEZ accepts the post-states");
}

#[test]
fn the_rotated_feed_verifies_against_the_new_set() {
    // The seam that matters: what a rotation writes has to be what the
    // submission path reads, with the new set in force.
    let id = feed_id(b"ETH");

    let posts = rotate(registered(&id), id, rotated(4), 3).expect("a usable set");

    let stored =
        FeedAccount::try_from_slice(posts[0].account().data.as_ref()).expect("decodes as a feed");
    let addresses = stored.signer_addresses();
    let config = stored.config(&addresses).expect("configures");
    assert_eq!(config.threshold(), 3);
    assert_eq!(config.signers().len(), 4);
    assert_eq!(
        config.feed_id(),
        &id,
        "the feed it rotated is the feed it was"
    );
}

#[test]
fn rotating_one_feed_leaves_another_alone() {
    // R3's shape at the admin path rather than the submission path: two feeds are
    // two accounts, so an operation naming one cannot reach the other.
    let btc = feed_id(b"BTC");
    let eth = feed_id(b"ETH");

    let eth_before = registered(&eth);
    let posts = rotate(registered(&btc), btc, rotated(5), 4).expect("a usable set");

    assert_ne!(
        posts[0].account().data.as_ref(),
        eth_before.account.data.as_ref()
    );
    let untouched = FeedAccount::try_from_slice(eth_before.account.data.as_ref()).expect("decodes");
    assert_eq!(untouched.signers, signers(5), "ETH still has its own set");
    assert_eq!(untouched.threshold, 3);
}

#[test]
fn a_registered_feed_cannot_be_rotated_under_another_id() {
    // The address and the stored id are checked separately, and this is the case
    // only the second one catches.
    let btc = feed_id(b"BTC");

    assert_eq!(
        rotate(registered(&btc), feed_id(b"ETH"), rotated(5), 3),
        Err(ManageError::FeedMismatch)
    );
}
