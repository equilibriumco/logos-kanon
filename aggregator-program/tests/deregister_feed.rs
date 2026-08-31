//! Retiring a feed, and registering the same id again afterwards.
//!
//! The round trip is the point. The feed's address is its id's (ADR 33), and LEZ
//! will not let a program hand an account back — rule 4 forbids giving up
//! ownership, rule 3 forbids resetting the nonce — so if a deregistered account
//! could not hold a new registration, retiring `BTC` once would spend the id for
//! the life of the deployment.

use aggregator_program::manage::deregister_feed;
use aggregator_program::publish::{NO_CONFIDENCE_INTERVAL, REDSTONE_SOURCE_ID};
use aggregator_program::register::{register_feed, FEED_ACCOUNT_SEED};
use aggregator_program::submit::{submit_price, PRICE_ACCOUNT_SEED};
use aggregator_program::{FeedAccount, SubmitError};
use borsh::BorshDeserialize;
use kanon_clock::CLOCK_ACCOUNT_ID;
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{validate_execution, AccountPostState, ProgramId};
use spel_framework::pda::{compute_pda, seed_from_str};

const OURS: ProgramId = [7u32; 8];
const WALLET_PROGRAM: ProgramId = [42u32; 8];
const CLOCK_PROGRAM: ProgramId = [88u32; 8];

/// The every-block clock account, the only admissible source of "now" (ADR 13).
fn clock_at(ms: u64) -> AccountWithMetadata {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&ms.to_le_bytes());
    AccountWithMetadata {
        account: Account {
            program_owner: CLOCK_PROGRAM,
            balance: 0,
            data: Data::try_from(data.to_vec()).expect("fits"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(CLOCK_ACCOUNT_ID),
    }
}

fn feed_id(name: &[u8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..name.len()].copy_from_slice(name);
    id
}

fn feed_address(id: &[u8; 32]) -> AccountId {
    compute_pda(&OURS, &[id, &seed_from_str(FEED_ACCOUNT_SEED)])
}

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

/// The price account for a feed that has never published: default, at the
/// address the constraint derives from the feed account's own id. A
/// registration reads it for the pair and passes it through untouched.
fn unpublished_price(feed_at: &AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: compute_pda(
            &OURS,
            &[feed_at.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
        ),
    }
}

fn register(
    feed: AccountWithMetadata,
    id: [u8; 32],
    set: u8,
    threshold: u8,
) -> Vec<AccountPostState> {
    let price = unpublished_price(&feed.account_id);
    register_feed(
        feed,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        price,
        id,
        [0xB7; 32],
        [0x05; 32],
        8,
        60_000,
        signers(set),
        threshold,
        OURS,
    )
    .expect("a usable feed")
}

/// The account as the chain leaves it once a post-state is applied.
fn as_chain_leaves_it(post: &AccountPostState, id: &[u8; 32]) -> AccountWithMetadata {
    let mut account = post.account().clone();
    account.program_owner = OURS;
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: feed_address(id),
    }
}

fn fresh(id: &[u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: feed_address(id),
    }
}

#[test]
fn a_feed_id_survives_being_retired() {
    // register -> deregister -> register again, at one address, with the second
    // registration free to differ from the first.
    let id = feed_id(b"BTC");

    let first = as_chain_leaves_it(&register(fresh(&id), id, 5, 3)[0], &id);

    let pre = vec![first.clone(), on_chain(0xAD, true), on_chain(0xC0, false)];
    let retired = deregister_feed(first, on_chain(0xAD, true), on_chain(0xC0, false), id, OURS)
        .expect("the named feed");
    validate_execution(&pre, &retired, OURS).expect("LEZ accepts the retirement");

    let emptied = as_chain_leaves_it(&retired[0], &id);
    assert!(emptied.account.data.as_ref().is_empty());

    let pre = vec![
        emptied.clone(),
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        unpublished_price(&emptied.account_id),
    ];
    let again = register(emptied, id, 3, 2);
    validate_execution(&pre, &again, OURS).expect("LEZ accepts the re-registration");

    let stored =
        FeedAccount::try_from_slice(again[0].account().data.as_ref()).expect("decodes as a feed");
    assert_eq!(
        stored.signers.len(),
        3,
        "the second registration is its own"
    );
    assert_eq!(stored.threshold, 2);
    assert!(
        again[0].required_claim().is_none(),
        "a re-registration must not claim an account this program already owns"
    );
}

#[test]
fn a_retired_feed_refuses_submissions_as_retired() {
    // Its own cause. An operator needs "you deregistered this" rather than "the
    // bytes were unreadable", and the two go to different people.
    let id = feed_id(b"ETH");
    let live = as_chain_leaves_it(&register(fresh(&id), id, 3, 2)[0], &id);
    let retired = deregister_feed(live, on_chain(0xAD, true), on_chain(0xC0, false), id, OURS)
        .expect("the named feed");
    let emptied = as_chain_leaves_it(&retired[0], &id);

    let outcome = submit_price(
        emptied,
        fresh(&feed_id(b"price")),
        clock_at(1_700_000_000_000),
        &[],
        OURS,
    );
    assert_eq!(outcome, Err(SubmitError::FeedDeregistered));
}

#[test]
fn an_account_holding_a_feed_still_refuses_a_registration() {
    // The third state is not a licence to overwrite: only an emptied account is
    // re-registerable, and a live feed is still `AlreadyRegistered`.
    let id = feed_id(b"SOL");
    let live = as_chain_leaves_it(&register(fresh(&id), id, 5, 3)[0], &id);

    let outcome = register_feed(
        live.clone(),
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        unpublished_price(&live.account_id),
        id,
        [0xB7; 32],
        [0x05; 32],
        8,
        60_000,
        signers(3),
        2,
        OURS,
    );
    assert!(
        matches!(
            outcome,
            Err(aggregator_program::RegisterError::AlreadyRegistered)
        ),
        "a live feed is not re-registerable, got {outcome:?}"
    );
}

#[test]
fn an_emptied_account_owned_by_somebody_else_is_not_re_registerable() {
    // The re-registration branch requires the account be *ours* and empty. An
    // empty account belonging to someone else is neither a first registration nor
    // one this program may write.
    let id = feed_id(b"XMR");
    let mut foreign = fresh(&id);
    foreign.account.program_owner = WALLET_PROGRAM;
    foreign.account.nonce = Nonce(2);

    let outcome = register_feed(
        foreign.clone(),
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        unpublished_price(&foreign.account_id),
        id,
        [0xB7; 32],
        [0x05; 32],
        8,
        60_000,
        signers(3),
        2,
        OURS,
    );
    assert!(matches!(
        outcome,
        Err(aggregator_program::RegisterError::AlreadyRegistered)
    ));
}

/// A price account holding a published pair, at the address the feed derives.
///
/// Built from the fields `submit_price` writes rather than by running it: this
/// file has no payload harness, and what the guard reads is the stored pair.
fn published_pair(feed_at: &AccountId, base: [u8; 32], quote: [u8; 32]) -> AccountWithMetadata {
    let published = OraclePriceAccount {
        base_asset: AccountId::new(base),
        quote_asset: AccountId::new(quote),
        price: 1,
        timestamp: 1_700_000_000_000,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: NO_CONFIDENCE_INTERVAL,
    };
    AccountWithMetadata {
        account: Account {
            program_owner: OURS,
            balance: 0,
            data: Data::from(&published),
            nonce: Nonce(1),
        },
        is_authorized: false,
        account_id: compute_pda(
            &OURS,
            &[feed_at.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
        ),
    }
}

#[test]
fn a_feed_id_may_not_change_its_pair_once_it_has_published() {
    // The guard `the_published_pair_is_what_a_re_registration_may_not_change`
    // in `submit_price.rs` shows the need for: a retirement leaves the price
    // account alone, so a re-registration under a new pair would produce a feed
    // whose every submission answers `AssetMismatch`. Refused at the
    // registration, where the operator can still do something about it.
    let id = feed_id(b"BTC");
    let emptied = retired(&id);
    let price = published_pair(&emptied.account_id, [0xB7; 32], [0x05; 32]);

    let outcome = register_feed(
        emptied,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        price,
        id,
        [0xEE; 32],
        [0x05; 32],
        8,
        60_000,
        signers(3),
        2,
        OURS,
    );

    assert!(
        matches!(outcome, Err(aggregator_program::RegisterError::PairChanged)),
        "a re-registration may not repoint a feed id at another pair, got {outcome:?}"
    );
}

#[test]
fn the_same_pair_re_registers_after_a_retirement() {
    // The other half, and the one M2-09 exists for: the id is not spent. A feed
    // that publishes, retires and comes back under the same pair is the ordinary
    // case, and the guard must not stand in its way.
    let id = feed_id(b"ETH");
    let emptied = retired(&id);
    let price = published_pair(&emptied.account_id, [0xB7; 32], [0x05; 32]);

    register_feed(
        emptied,
        on_chain(0xAD, true),
        on_chain(0xC0, false),
        price,
        id,
        [0xB7; 32],
        [0x05; 32],
        8,
        60_000,
        signers(3),
        2,
        OURS,
    )
    .expect("the same pair is not a change");
}

/// A feed registered, then retired: ours, empty, at its derived address.
fn retired(id: &[u8; 32]) -> AccountWithMetadata {
    let live = as_chain_leaves_it(&register(fresh(id), *id, 5, 3)[0], id);
    let posts = deregister_feed(live, on_chain(0xAD, true), on_chain(0xC0, false), *id, OURS)
        .expect("a live feed retires");
    as_chain_leaves_it(&posts[0], id)
}
