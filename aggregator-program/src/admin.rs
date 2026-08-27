//! Who may change a feed, and how the program knows.
//!
//! Every instruction other than `submit_price` is administrative, and each one
//! is gated here. The authority lives in one account this program owns: the
//! claimed admin signs the transaction, and [`authorise`] is the comparison
//! between what it signed as and what the account stores.
//!
//! `[M2-06:01]` records why the authority is an account rather than a constant
//! or a field of the feed, and why the check is Rust here rather than
//! `#[require_admin]` on the guest handler: a gate injected into
//! macro-expanded dispatcher code has no reachable test on the host, and SEC2
//! asks for the update path to be tested including the unauthorised-caller
//! reject.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

/// The name seed the config account's address is derived from.
///
/// Declared as a constraint on the instructions rather than checked in code, for
/// the reason [ADR 32] gives about the price account: the derivation is what a
/// client has to reproduce, and the constraint is what publishes it in the IDL.
///
/// [ADR 32]: ../../adr/0032-one-price-account-per-feed-and-anyone-may-fill-it.md
pub const ADMIN_CONFIG_SEED: &str = "KANON_ADMIN_CONFIG";

/// The authority, as the config account stores it.
///
/// `Option` rather than a bare key, so the layout is byte-identical to
/// RFP-001's `AdminState` under borsh and adopting that type later is a
/// deletion rather than a migration. No instruction in this program produces
/// `None`; it is refused wherever it is read, which is the same answer
/// upstream's revocation would give.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AdminAccount {
    /// The current authority, or `None` for an authority nothing can exercise.
    pub admin: Option<[u8; 32]>,
}

/// Why an administrative instruction was refused before it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminError {
    /// The config account is not owned by this program.
    ConfigNotOurs,
    /// The config account's data is not an [`AdminAccount`].
    ConfigUndecodable,
    /// The claimed admin did not sign the transaction.
    NotSigned,
    /// The signer is not the stored authority.
    Unauthorised,
    /// The stored authority is one nothing can exercise.
    NoAuthority,
    /// The signer is not this build's genesis authority.
    NotGenesisAdmin,
    /// The config account already holds an authority.
    AlreadyInitialised,
    /// The key offered as an authority is the zero key, which nothing can sign
    /// as. Refused rather than stored, because storing it is unrecoverable.
    AuthorityIsZero,
}

/// Refuse unless `admin` signed and is the authority `config` stores.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn authorise(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<(), AdminError> {
    // Before the data is read, because a stored key in an account this program
    // does not own is the caller's claim and not this program's.
    if config.account.program_owner != self_program_id {
        return Err(AdminError::ConfigNotOurs);
    }
    if !admin.is_authorized {
        return Err(AdminError::NotSigned);
    }

    let stored = AdminAccount::try_from_slice(config.account.data.as_ref())
        .map_err(|_| AdminError::ConfigUndecodable)?;

    match stored.admin {
        None => Err(AdminError::NoAuthority),
        Some(key) if key == *admin.account_id.value() => Ok(()),
        Some(_) => Err(AdminError::Unauthorised),
    }
}

impl AdminError {
    /// A stable number per leaf cause, in the 800 block: this layer's own, the
    /// way `SubmitError` numbers the layers below it.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::ConfigNotOurs => 801,
            Self::ConfigUndecodable => 802,
            Self::NotSigned => 803,
            Self::Unauthorised => 804,
            Self::NoAuthority => 805,
            Self::NotGenesisAdmin => 806,
            Self::AlreadyInitialised => 807,
            Self::AuthorityIsZero => 808,
        }
    }
}

impl fmt::Display for AdminError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigNotOurs => {
                f.write_str("the account offered as the admin config is not one this program owns")
            }
            Self::ConfigUndecodable => {
                f.write_str("the admin config account's data is not an authority")
            }
            Self::NotSigned => f.write_str("the claimed admin did not sign this transaction"),
            Self::Unauthorised => f.write_str("the signer is not this program's admin authority"),
            Self::NoAuthority => f.write_str("the stored authority is one no signer can exercise"),
            Self::NotGenesisAdmin => {
                f.write_str("the signer is not this build's genesis authority")
            }
            Self::AlreadyInitialised => f.write_str("the admin authority is already established"),
            Self::AuthorityIsZero => {
                f.write_str("the zero key is not an authority any signer could exercise")
            }
        }
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<AdminError> for spel_framework::error::SpelError {
    fn from(err: AdminError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Establish the authority, once, for whoever holds this build's genesis key.
///
/// The genesis key is the bootstrap and not the standing authority: what every
/// administrative instruction checks afterwards is the account this returns. An
/// all-zero `genesis` is a build with no genesis authority configured, and
/// refuses.
///
/// `[M2-06:01]` records why this is gated at all. Because a PDA hashes the
/// program id and the program id is the image id, every rebuild presents a fresh
/// default config account -- so an unguarded first write is not one race but one
/// per deployment, and its winner sets the signer sets that decide what a price
/// means.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn initialise(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    genesis: &[u8; 32],
) -> Result<AdminAccount, AdminError> {
    // First, because a build with no genesis authority configured has no
    // caller who could pass any of the checks below, and saying so is a
    // different fault from a caller getting it wrong.
    if *genesis == [0u8; 32] {
        return Err(AdminError::AuthorityIsZero);
    }
    if !admin.is_authorized {
        return Err(AdminError::NotSigned);
    }
    // The whole account, not only its data: `#[account(init)]` compares against
    // a default account, and an account carrying a balance or a nonce is one
    // this program can never claim.
    if config.account != Account::default() {
        return Err(AdminError::AlreadyInitialised);
    }
    if admin.account_id.value() != genesis {
        return Err(AdminError::NotGenesisAdmin);
    }

    Ok(AdminAccount {
        admin: Some(*genesis),
    })
}

/// Hand the authority to `new_admin`, on the current authority's signature.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn transfer(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    new_admin: [u8; 32],
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    authorise(config, admin, self_program_id)?;

    if new_admin == [0u8; 32] {
        return Err(AdminError::AuthorityIsZero);
    }

    Ok(AdminAccount {
        admin: Some(new_admin),
    })
}

/// [`initialise`], as the post-states LEZ is handed.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn initialise_admin(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    genesis: &[u8; 32],
) -> Result<Vec<AccountPostState>, AdminError> {
    let established = initialise(&config, &admin, genesis)?;

    let mut account = config.account;
    account.data = data_of(&established);

    // The claim is what makes the address the constraint checked this program's.
    // Unconditional rather than `new_claimed_if_default`, because `initialise`
    // has already refused every pre-state but the default one -- deciding it
    // twice, on different rules, is how the third price-account state got
    // through in ADR 32.
    let claimed =
        AutoClaim::pda_from_seeds(&[&seed_from_str(ADMIN_CONFIG_SEED)]).to_post_state(account);

    Ok(vec![claimed, AccountPostState::new(admin.account)])
}

/// [`transfer`], as the post-states LEZ is handed.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn transfer_admin(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    new_admin: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AdminError> {
    let moved = transfer(&config, &admin, new_admin, self_program_id)?;

    let mut account = config.account;
    account.data = data_of(&moved);

    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(admin.account),
    ])
}

/// The authority as account data.
///
/// One `Option` and a key, so the width `Data` refuses is out of reach and the
/// serialisation cannot fail on anything this program constructs.
fn data_of(state: &AdminAccount) -> Data {
    Data::try_from(borsh::to_vec(state).expect("an Option and a key serialise"))
        .expect("thirty-three bytes fit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{Account, AccountId, Data, Nonce};

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];

    const ADMIN: [u8; 32] = [0xAD; 32];
    const STRANGER: [u8; 32] = [0x5A; 32];

    fn config_account(stored: &AdminAccount) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: OURS,
                balance: 0,
                data: Data::try_from(borsh::to_vec(stored).expect("serialises")).expect("fits"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: AccountId::new([0xC0; 32]),
        }
    }

    fn signer(id: [u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: [0u32; 8],
                balance: 0,
                data: Data::try_from(Vec::new()).expect("fits"),
                nonce: Nonce(0),
            },
            is_authorized: true,
            account_id: AccountId::new(id),
        }
    }

    #[test]
    fn a_signer_who_is_not_the_stored_admin_is_refused() {
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        assert_eq!(
            authorise(&config, &signer(STRANGER), OURS),
            Err(AdminError::Unauthorised)
        );
    }

    #[test]
    fn the_stored_admin_is_authorised() {
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        assert_eq!(authorise(&config, &signer(ADMIN), OURS), Ok(()));
    }

    fn fresh_config() -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: AccountId::new([0xC0; 32]),
        }
    }

    /// The state a post-state carries, read back the way a later instruction
    /// would read it.
    fn stored_in(post: &AccountPostState) -> AdminAccount {
        AdminAccount::try_from_slice(post.account().data.as_ref()).expect("decodes")
    }

    #[test]
    fn no_two_causes_share_an_error_code() {
        // The same obligation `SubmitError` carries: a caller that cannot tell
        // two refusals apart cannot act on either.
        let causes = [
            AdminError::ConfigNotOurs,
            AdminError::ConfigUndecodable,
            AdminError::NotSigned,
            AdminError::Unauthorised,
            AdminError::NoAuthority,
            AdminError::NotGenesisAdmin,
            AdminError::AlreadyInitialised,
            AdminError::AuthorityIsZero,
        ];

        let mut codes: Vec<u32> = causes.iter().map(AdminError::code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();

        assert_eq!(codes.len(), before, "two causes answer with one code");
    }

    #[test]
    fn establishing_the_authority_claims_the_config_account() {
        let posts = initialise_admin(fresh_config(), signer(ADMIN), &ADMIN).expect("genesis signs");

        assert_eq!(
            posts.len(),
            2,
            "config and admin, in the instruction's order"
        );
        assert_eq!(stored_in(&posts[0]), AdminAccount { admin: Some(ADMIN) });
        assert!(
            posts[0].required_claim().is_some(),
            "a first write has to claim the address the constraint checked"
        );
    }

    #[test]
    fn moving_the_authority_writes_the_account_without_claiming_it_again() {
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        let posts = transfer_admin(config, signer(ADMIN), STRANGER, OURS).expect("admin signs");

        assert_eq!(
            stored_in(&posts[0]),
            AdminAccount {
                admin: Some(STRANGER)
            }
        );
        assert!(
            posts[0].required_claim().is_none(),
            "the account is already this program's; claiming it again is not an update"
        );
    }

    #[test]
    fn the_signer_is_handed_back_unchanged() {
        let admin = signer(ADMIN);
        let before = admin.account.clone();

        let posts = initialise_admin(fresh_config(), admin, &ADMIN).expect("genesis signs");

        assert_eq!(*posts[1].account(), before);
    }

    #[test]
    fn the_genesis_key_holder_establishes_the_authority() {
        let established =
            initialise(&fresh_config(), &signer(ADMIN), &ADMIN).expect("genesis signs");

        assert_eq!(established, AdminAccount { admin: Some(ADMIN) });
    }

    #[test]
    fn a_signer_who_is_not_the_genesis_key_cannot_establish_the_authority() {
        assert_eq!(
            initialise(&fresh_config(), &signer(STRANGER), &ADMIN),
            Err(AdminError::NotGenesisAdmin)
        );
    }

    #[test]
    fn a_build_with_no_genesis_authority_cannot_be_initialised_by_anyone() {
        // An all-zero constant is a build nobody configured. Without this the
        // zero key would be the authority, and it is a key anyone can name.
        assert_eq!(
            initialise(&fresh_config(), &signer([0u8; 32]), &[0u8; 32]),
            Err(AdminError::AuthorityIsZero)
        );
    }

    #[test]
    fn an_authority_already_established_is_not_established_again() {
        let occupied = config_account(&AdminAccount { admin: Some(ADMIN) });

        assert_eq!(
            initialise(&occupied, &signer(ADMIN), &ADMIN),
            Err(AdminError::AlreadyInitialised)
        );
    }

    #[test]
    fn initialising_needs_a_signature_from_the_genesis_holder() {
        let mut unsigned = signer(ADMIN);
        unsigned.is_authorized = false;

        assert_eq!(
            initialise(&fresh_config(), &unsigned, &ADMIN),
            Err(AdminError::NotSigned)
        );
    }

    #[test]
    fn the_authority_moves_to_the_key_the_admin_names() {
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        let moved = transfer(&config, &signer(ADMIN), STRANGER, OURS).expect("admin signs");

        assert_eq!(
            moved,
            AdminAccount {
                admin: Some(STRANGER)
            }
        );
    }

    #[test]
    fn nobody_but_the_authority_can_move_it() {
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        assert_eq!(
            transfer(&config, &signer(STRANGER), STRANGER, OURS),
            Err(AdminError::Unauthorised)
        );
    }

    #[test]
    fn the_authority_cannot_be_moved_to_the_zero_key() {
        // Unrecoverable if stored: no signer can produce it, and the transfer
        // path is the only way out of it.
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });

        assert_eq!(
            transfer(&config, &signer(ADMIN), [0u8; 32], OURS),
            Err(AdminError::AuthorityIsZero)
        );
    }

    #[test]
    fn a_config_account_this_program_does_not_own_is_refused() {
        // The attack this closes: supply any account naming yourself as admin.
        // Ownership is what makes the stored key this program's claim rather
        // than the caller's.
        let mut forged = config_account(&AdminAccount {
            admin: Some(STRANGER),
        });
        forged.account.program_owner = SOMEONE_ELSE;

        assert_eq!(
            authorise(&forged, &signer(STRANGER), OURS),
            Err(AdminError::ConfigNotOurs)
        );
    }

    #[test]
    fn an_admin_that_did_not_sign_is_refused() {
        // The dispatcher's `#[account(signer)]` refuses this before a handler
        // runs, so reaching it means the annotation was dropped. Checked here
        // too, because that is where a test can see it.
        let config = config_account(&AdminAccount { admin: Some(ADMIN) });
        let mut unsigned = signer(ADMIN);
        unsigned.is_authorized = false;

        assert_eq!(
            authorise(&config, &unsigned, OURS),
            Err(AdminError::NotSigned)
        );
    }

    #[test]
    fn a_config_account_that_does_not_decode_is_refused() {
        let mut rubbish = config_account(&AdminAccount { admin: Some(ADMIN) });
        rubbish.account.data = Data::try_from(vec![0xFF; 3]).expect("fits");

        assert_eq!(
            authorise(&rubbish, &signer(ADMIN), OURS),
            Err(AdminError::ConfigUndecodable)
        );
    }

    #[test]
    fn an_authority_nothing_can_exercise_refuses_everyone() {
        // No instruction writes `None`. An account holding it is a corruption or
        // an out-of-band write, and the answer is the same one revocation would
        // give rather than an accidental accept.
        let config = config_account(&AdminAccount { admin: None });

        assert_eq!(
            authorise(&config, &signer(ADMIN), OURS),
            Err(AdminError::NoAuthority)
        );
    }
}
