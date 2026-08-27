//! The administrative path as a transaction, not as a set of decisions.
//!
//! The unit tests beside `admin.rs` cover what each operation decides. These
//! cover the part only a transaction can show: that the post-states LEZ is
//! handed are ones it accepts, that the claim `initialise` emits names the
//! address the constraint checks, and that the account it writes is one the next
//! instruction can administer against.
//!
//! What is still out of reach here is a build with a genesis key configured.
//! `GENESIS_ADMIN` is resolved at compile time in the guest, so a test can only
//! pass a genesis in as an argument -- which is what these do. The guest's own
//! suite covers the other half: that the constant is wired through, and that a
//! build without one refuses.

use aggregator_program::admin::{
    accept_admin, authorise, initialise_admin, nominate_admin, revoke_admin, AdminAccount,
    AdminError, ADMIN_CONFIG_SEED,
};
use borsh::BorshDeserialize;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{
    validate_execution, AccountPostState, Claim, ExecutionValidationError, ProgramId,
};
use spel_framework::pda::{compute_pda, seed_from_str};

/// This program, in every test.
const OURS: ProgramId = [7u32; 8];

const GENESIS: [u8; 32] = [0xAD; 32];
const NEXT: [u8; 32] = [0x5A; 32];
const STRANGER: [u8; 32] = [0x11; 32];

/// The address the constraint accepts, derived the way a client would.
fn config_id() -> AccountId {
    compute_pda(&OURS, &[&seed_from_str(ADMIN_CONFIG_SEED)])
}

/// The config account before anything has been written to it.
fn fresh_config() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: config_id(),
    }
}

/// The config account as it stands after `posts` were applied, owned by this
/// program the way the chain would leave it once the claim is honoured.
fn config_after(posts: &[AccountPostState]) -> AccountWithMetadata {
    let mut account = posts[0].account().clone();
    account.program_owner = OURS;
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: config_id(),
    }
}

/// Whatever owns an operator's account on chain. Not this program, and not
/// nobody: see `an_unowned_admin_that_has_transacted_cannot_be_used` for why the
/// distinction is the whole of this fixture.
const WALLET_PROGRAM: ProgramId = [42u32; 8];

/// An admin key as one exists on chain: owned, funded, and having signed before.
///
/// Deliberately not `Account::default()`. LEZ increments every signer's nonce
/// after applying a state diff, outside program execution, so an account that
/// has signed once is never pristine again -- and a default-*owned* account that
/// is not pristine cannot appear in any post-state at all (rule 7). A fixture
/// built from `Account::default()` would pass every assertion in this file while
/// describing a state no real key is in after its first transaction.
fn signer(id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: WALLET_PROGRAM,
            balance: 500,
            data: Data::try_from(Vec::new()).expect("fits"),
            nonce: Nonce(7),
        },
        is_authorized: true,
        account_id: AccountId::new(id),
    }
}

/// The same key, unowned and having transacted: the state that cannot be used.
fn unowned_signer(id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: [0u32; 8],
            balance: 0,
            data: Data::try_from(Vec::new()).expect("fits"),
            nonce: Nonce(1),
        },
        is_authorized: true,
        account_id: AccountId::new(id),
    }
}

fn stored(posts: &[AccountPostState]) -> AdminAccount {
    AdminAccount::try_from_slice(posts[0].account().data.as_ref()).expect("decodes")
}

#[test]
fn an_initialisation_lands_and_claims_the_address_the_constraint_checks() {
    let pre = vec![fresh_config(), signer(GENESIS)];

    let posts = initialise_admin(fresh_config(), signer(GENESIS), &GENESIS).expect("genesis signs");

    validate_execution(&pre, &posts, OURS).expect("LEZ has to accept the first write");

    // The claim is the part a restatement of LEZ's rules would not catch: it has
    // to name the seed the constraint derives from, or the address the client
    // computed is not the address this program claimed.
    let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
        panic!("a first write has to claim the address by its seed");
    };
    assert_eq!(
        AccountId::for_public_pda(&OURS, &seed),
        config_id(),
        "the claimed address is not the one the constraint accepts"
    );
    assert_eq!(
        stored(&posts),
        AdminAccount {
            admin: Some(GENESIS),
            pending: None,
        }
    );
}

#[test]
fn the_authority_an_initialisation_writes_can_administer_a_feed() {
    // The other half of "the path is executable": not only that the write lands,
    // but that the account it leaves behind is one the gate accepts.
    let posts = initialise_admin(fresh_config(), signer(GENESIS), &GENESIS).expect("genesis signs");
    let config = config_after(&posts);

    assert_eq!(authorise(&config, &signer(GENESIS), OURS), Ok(()));
    assert_eq!(
        authorise(&config, &signer(STRANGER), OURS),
        Err(AdminError::Unauthorised)
    );
}

#[test]
fn a_handover_lands_in_two_transactions_and_moves_the_authority_once() {
    let established = initialise_admin(fresh_config(), signer(GENESIS), &GENESIS).expect("signs");
    let config = config_after(&established);

    // Transaction one: the outgoing authority nominates.
    let pre = vec![config.clone(), signer(GENESIS)];
    let nominated =
        nominate_admin(config.clone(), signer(GENESIS), NEXT, OURS).expect("the authority signs");
    validate_execution(&pre, &nominated, OURS).expect("LEZ has to accept the nomination");

    // Nothing has moved yet, which is the point of the two steps.
    let after_nomination = config_after(&nominated);
    assert_eq!(authorise(&after_nomination, &signer(GENESIS), OURS), Ok(()));
    assert_eq!(
        authorise(&after_nomination, &signer(NEXT), OURS),
        Err(AdminError::Unauthorised)
    );

    // Transaction two: the nominee accepts, signing for itself.
    let pre = vec![after_nomination.clone(), signer(NEXT)];
    let accepted = accept_admin(after_nomination, signer(NEXT), OURS).expect("the nominee signs");
    validate_execution(&pre, &accepted, OURS).expect("LEZ has to accept the handover");

    let after_handover = config_after(&accepted);
    assert_eq!(authorise(&after_handover, &signer(NEXT), OURS), Ok(()));
    assert_eq!(
        authorise(&after_handover, &signer(GENESIS), OURS),
        Err(AdminError::Unauthorised),
        "the outgoing authority keeps nothing"
    );
}

#[test]
fn a_revocation_lands_and_leaves_nobody_able_to_administer() {
    let established = initialise_admin(fresh_config(), signer(GENESIS), &GENESIS).expect("signs");
    let config = config_after(&established);

    let pre = vec![config.clone(), signer(GENESIS)];
    let revoked = revoke_admin(config, signer(GENESIS), OURS).expect("the authority signs");
    validate_execution(&pre, &revoked, OURS).expect("LEZ has to accept the revocation");

    let after = config_after(&revoked);
    assert_eq!(
        authorise(&after, &signer(GENESIS), OURS),
        Err(AdminError::NoAuthority)
    );
    // And it cannot be undone by initialising again: the account is no longer
    // default, so the genesis key is spent.
    assert_eq!(
        initialise_admin(after, signer(GENESIS), &GENESIS),
        Err(AdminError::AlreadyInitialised)
    );
}

#[test]
fn an_unowned_admin_that_has_transacted_cannot_be_used() {
    // The deployment property this whole file rests on, asserted rather than
    // assumed. LEZ's rule 7 refuses a post-state whose owner is the default one
    // unless the pre-state was pristine, and rules 3 and 4 forbid changing the
    // nonce or the owner to escape it -- so an account that is unowned *and* has
    // signed before can never appear in a post-state again, however this program
    // constructs one. `Claim::Authorized` does not help: the claim is honoured
    // after validation, so the post-state still carries the default owner when
    // rule 7 runs.
    //
    // The consequence for an operator: the admin key must be an account that
    // already exists on chain, not a freshly generated address whose first
    // transaction is the initialisation. A fresh address would initialise once
    // and then be unusable, because LEZ would have bumped its nonce.
    let admin = unowned_signer(GENESIS);
    let pre = vec![fresh_config(), admin.clone()];

    // The decision is fine -- this is not the program refusing.
    let posts = initialise_admin(fresh_config(), admin, &GENESIS).expect("the genesis key signs");

    let refused = validate_execution(&pre, &posts, OURS)
        .expect_err("LEZ cannot accept an unowned signer that has transacted");
    assert!(
        matches!(
            refused,
            ExecutionValidationError::NonDefaultAccountWithDefaultOwner { .. }
        ),
        "expected rule 7 to refuse it, got {refused:?}"
    );

    // And the same operation with the same key, owned, is accepted -- so it is
    // the account's ownership that decides and nothing about this program.
    let owned = signer(GENESIS);
    let pre = vec![fresh_config(), owned.clone()];
    let posts = initialise_admin(fresh_config(), owned, &GENESIS).expect("the genesis key signs");
    validate_execution(&pre, &posts, OURS).expect("an owned signer is accepted at any nonce");
}
