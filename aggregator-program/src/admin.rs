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
use lee_core::program::{AccountPostState, ProgramId, DEFAULT_PROGRAM_ID};
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
/// `Option` rather than a bare key, so `admin` carries RFP-001's `AdminState`
/// field shape. [`revoke`] is what produces `None`, and it is refused wherever
/// it is read afterwards: an authority nobody can exercise, which is what
/// revocation means.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AdminAccount {
    /// The current authority, or `None` for an authority nothing can exercise.
    pub admin: Option<[u8; 32]>,
    /// A key nominated to take over, which is not the authority until it accepts.
    ///
    /// Why a handover is two steps: a nominee that never signs never becomes the
    /// authority, so a mistyped key costs a second nomination rather than the
    /// program's whole administrative surface. It also lets the servicing
    /// handover run across two transactions and two parties, which a co-signed
    /// transfer would not.
    pub pending: Option<[u8; 32]>,
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
    /// This build carries no genesis authority, so nobody can establish one.
    ///
    /// A different fault from [`Self::AuthorityIsZero`] and a different number,
    /// because they reach different people: this one says the deployment was
    /// never configured, and that one says a caller passed a key nothing can
    /// sign as.
    NoGenesisAuthority,
    /// Nobody has been nominated, so there is nothing to accept.
    NoNomination,
    /// The signer is not the nominated key.
    NotTheNominee,
    /// The account offered as the authority is owned by no program, which LEZ
    /// strands after one transaction.
    ///
    /// Rule 7 refuses a post-state whose owner is the default one unless the
    /// pre-state was pristine, and LEZ bumps every signer's nonce after applying
    /// a state diff, outside program execution. A default-owned key therefore
    /// passes exactly once, while it is still pristine, and afterwards cannot
    /// appear in a valid post-state of any program -- which freezes whatever it
    /// already holds and means it can never be funded either, since receiving a
    /// balance is itself appearing in a post-state. The key still signs; it is
    /// the account that is finished.
    ///
    /// **Both entry points are terminal**, not only [`accept`].
    /// [`initialise_admin`] writes the config and claims it before the key is
    /// stranded, so [`initialise`] afterwards answers
    /// [`Self::AlreadyInitialised`] exactly as it does after an acceptance. What
    /// is specific to [`accept`] is the width of the window rather than the
    /// outcome: it is reachable during normal operation instead of once at
    /// deployment, and it strands an authority the previous holder has already
    /// given up.
    ///
    /// Refused here so that a runbook slip is a typed refusal rather than a
    /// silent and irreversible one. The key must be an account some program
    /// already owns; receiving a balance transfer is one way it acquires that.
    AdminUnowned,
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
    authorised(config, admin, self_program_id).map(|_| ())
}

/// [`authorise`], returning what the account stores, for the instructions that
/// have to write it back.
fn authorised(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    let stored = stored_authority(config, admin, self_program_id)?;

    match stored.admin {
        None => Err(AdminError::NoAuthority),
        Some(key) if key == *admin.account_id.value() => Ok(stored),
        Some(_) => Err(AdminError::Unauthorised),
    }
}

/// The config account's contents, once the account and the signature are known
/// good -- without deciding whether the signer is the authority, which
/// [`accept`] answers against a different field.
fn stored_authority(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    // Before the data is read, because a stored key in an account this program
    // does not own is the caller's claim and not this program's.
    if config.account.program_owner != self_program_id {
        return Err(AdminError::ConfigNotOurs);
    }
    if !admin.is_authorized {
        return Err(AdminError::NotSigned);
    }

    AdminAccount::try_from_slice(config.account.data.as_ref())
        .map_err(|_| AdminError::ConfigUndecodable)
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
            Self::NoNomination => 809,
            Self::NotTheNominee => 810,
            Self::NoGenesisAuthority => 811,
            Self::AdminUnowned => 812,
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
            Self::NoNomination => f.write_str("no key has been nominated to take the authority"),
            Self::NotTheNominee => f.write_str("the signer is not the nominated key"),
            Self::NoGenesisAuthority => {
                f.write_str("this build carries no genesis authority, so none can be established")
            }
            Self::AdminUnowned => f.write_str(
                "the key offered as the authority is owned by no program, and LEZ strands \
                 such an account after one transaction",
            ),
        }
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<AdminError> for spel_framework::error::SpelError {
    fn from(err: AdminError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Refuse an admin account that no program owns.
///
/// Checked where the authority is established rather than where it is used: LEZ
/// pins an account's owner once it has one (rule 4), so an account that is owned
/// at that moment stays owned. See [`AdminError::AdminUnowned`] for what an
/// unowned key costs.
fn owned(admin: &AccountWithMetadata) -> Result<(), AdminError> {
    if admin.account.program_owner == DEFAULT_PROGRAM_ID {
        return Err(AdminError::AdminUnowned);
    }
    Ok(())
}

/// Establish the authority, once, for whoever holds this build's genesis key.
///
/// The genesis key is the bootstrap and not the standing authority: what every
/// administrative instruction checks afterwards is the account this returns. An
/// all-zero `genesis` is a build with no genesis authority configured, and
/// refuses.
///
/// `[M2-06:01]` records why this is gated at all. Because a PDA hashes the
/// program id and the program id is the image id, a build that moves the image id
/// presents a fresh default config account -- so an unguarded first write is not
/// one race but one per *build*, and its winner sets the signer sets that decide
/// what a price means. Redeploying an unchanged build is not a fresh race: it
/// reproduces the id and meets the config account it already established.
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
        return Err(AdminError::NoGenesisAuthority);
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
    // Last, so only the genesis key holder -- the one who can act on it -- ever
    // sees this refusal.
    owned(admin)?;

    Ok(AdminAccount {
        admin: Some(*genesis),
        pending: None,
    })
}

/// Nominate `new_admin` to take the authority, on the current authority's
/// signature. A nomination is not the authority.
///
/// Replaces any earlier nomination, so only the latest can be accepted.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn nominate(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    new_admin: [u8; 32],
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    let stored = authorised(config, admin, self_program_id)?;

    if new_admin == [0u8; 32] {
        return Err(AdminError::AuthorityIsZero);
    }

    Ok(AdminAccount {
        admin: stored.admin,
        pending: Some(new_admin),
    })
}

/// Take the authority, on the nominee's own signature.
///
/// The second half of a handover, and what makes a mistyped nomination
/// recoverable: a key nobody controls can never sign, so it never becomes the
/// authority.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn accept(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    // Not `authorised`: the signer here is the nominee and not the authority, so
    // the comparison is against the other field.
    let stored = stored_authority(config, admin, self_program_id)?;

    match stored.pending {
        None => Err(AdminError::NoNomination),
        Some(key) if key == *admin.account_id.value() => {
            // After the nominee is known, and before the authority moves. A
            // pristine key can be nominated -- `nominate` cannot see the
            // nominee's account, because the nominee does not sign it -- so this
            // is the first point the account is visible, and the last point
            // before accepting it becomes irreversible.
            owned(admin)?;
            Ok(AdminAccount {
                admin: Some(key),
                pending: None,
            })
        }
        Some(_) => Err(AdminError::NotTheNominee),
    }
}

/// Give up the authority permanently, on the current authority's signature.
///
/// Clears any pending nomination with it: leaving one would let a nominee accept
/// afterwards and take an authority its holder had already given up.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn revoke(
    config: &AccountWithMetadata,
    admin: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<AdminAccount, AdminError> {
    authorised(config, admin, self_program_id)?;

    Ok(AdminAccount {
        admin: None,
        pending: None,
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

/// [`nominate`], as the post-states LEZ is handed.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn nominate_admin(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    new_admin: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AdminError> {
    let nominated = nominate(&config, &admin, new_admin, self_program_id)?;
    Ok(written(config, admin, &nominated))
}

/// [`accept`], as the post-states LEZ is handed.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn accept_admin(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AdminError> {
    let taken = accept(&config, &admin, self_program_id)?;
    Ok(written(config, admin, &taken))
}

/// [`revoke`], as the post-states LEZ is handed.
///
/// # Errors
///
/// [`AdminError`], one leaf cause per refusal.
pub fn revoke_admin(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AdminError> {
    let given_up = revoke(&config, &admin, self_program_id)?;
    Ok(written(config, admin, &given_up))
}

/// The post-states every instruction but `initialise` produces: the config
/// account written, the signer untouched, and no claim -- the account is already
/// this program's, and claiming it again is not an update.
fn written(
    config: AccountWithMetadata,
    admin: AccountWithMetadata,
    state: &AdminAccount,
) -> Vec<AccountPostState> {
    let mut account = config.account;
    account.data = data_of(state);

    vec![
        AccountPostState::new(account),
        AccountPostState::new(admin.account),
    ]
}

/// The authority as account data.
///
/// Two `Option`s and their keys, so the width `Data` refuses is out of reach and
/// the serialisation cannot fail on anything this program constructs.
fn data_of(state: &AdminAccount) -> Data {
    Data::try_from(borsh::to_vec(state).expect("two Options and their keys serialise"))
        .expect("sixty-six bytes fit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{Account, AccountId, Data, Nonce};

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];

    const ADMIN: [u8; 32] = [0xAD; 32];
    const STRANGER: [u8; 32] = [0x5A; 32];
    const THIRD: [u8; 32] = [0x33; 32];

    /// An authority held by `admin`, with nobody nominated.
    fn holding(admin: [u8; 32]) -> AdminAccount {
        AdminAccount {
            admin: Some(admin),
            pending: None,
        }
    }

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

    /// Whatever owns an operator's key on chain -- not this program, and not
    /// nobody.
    const WALLET_PROGRAM: ProgramId = [42u32; 8];

    /// An admin key as one exists on chain: owned, and having signed before.
    ///
    /// Deliberately not default-owned. A default-owned key is refused by
    /// [`owned`], and a fixture built from one would have described a state no
    /// key survives its first transaction in.
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

    /// A freshly generated key: signs, and no program owns it.
    ///
    /// The state a deployment runbook produces by default, and the one LEZ
    /// strands after one transaction.
    fn pristine(id: [u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: true,
            account_id: AccountId::new(id),
        }
    }

    #[test]
    fn a_signer_who_is_not_the_stored_admin_is_refused() {
        let config = config_account(&holding(ADMIN));

        assert_eq!(
            authorise(&config, &signer(STRANGER), OURS),
            Err(AdminError::Unauthorised)
        );
    }

    #[test]
    fn the_stored_admin_is_authorised() {
        let config = config_account(&holding(ADMIN));

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
        // Every variant, so adding one without a number fails here.
        let causes = [
            AdminError::ConfigNotOurs,
            AdminError::ConfigUndecodable,
            AdminError::NotSigned,
            AdminError::Unauthorised,
            AdminError::NoAuthority,
            AdminError::NotGenesisAdmin,
            AdminError::AlreadyInitialised,
            AdminError::AuthorityIsZero,
            AdminError::NoNomination,
            AdminError::NotTheNominee,
            AdminError::NoGenesisAuthority,
            AdminError::AdminUnowned,
        ];

        // No wildcard arm, so a twelfth variant fails to compile here. That
        // does not prove the list above is complete -- an author can add the arm
        // and forget the entry -- so the list is maintained by hand. What the
        // match buys is that the omission is loud: `code()` forces a new variant
        // to be given a number, and nothing else forces anyone to come here at
        // all. Earning "by construction" would mean generating the enum and an
        // iterable list of its variants from one macro.
        for cause in &causes {
            match cause {
                AdminError::ConfigNotOurs
                | AdminError::ConfigUndecodable
                | AdminError::NotSigned
                | AdminError::Unauthorised
                | AdminError::NoAuthority
                | AdminError::NotGenesisAdmin
                | AdminError::AlreadyInitialised
                | AdminError::AuthorityIsZero
                | AdminError::NoGenesisAuthority
                | AdminError::NoNomination
                | AdminError::NotTheNominee
                | AdminError::AdminUnowned => {}
            }
        }
        assert_eq!(causes.len(), 12, "every variant is in the list above");

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
        assert_eq!(stored_in(&posts[0]), holding(ADMIN));
        assert!(
            posts[0].required_claim().is_some(),
            "a first write has to claim the address the constraint checked"
        );
    }

    #[test]
    fn nominating_writes_the_account_without_claiming_it_again() {
        let posts = nominate_admin(
            config_account(&holding(ADMIN)),
            signer(ADMIN),
            STRANGER,
            OURS,
        )
        .expect("admin signs");

        assert_eq!(stored_in(&posts[0]).pending, Some(STRANGER));
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

        assert_eq!(established, holding(ADMIN));
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
            Err(AdminError::NoGenesisAuthority)
        );
    }

    #[test]
    fn an_authority_already_established_is_not_established_again() {
        let occupied = config_account(&holding(ADMIN));

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
    fn a_nomination_is_recorded_without_handing_the_authority_over() {
        let config = config_account(&holding(ADMIN));

        let nominated = nominate(&config, &signer(ADMIN), STRANGER, OURS).expect("admin signs");

        assert_eq!(
            nominated.admin,
            Some(ADMIN),
            "the authority has not moved yet"
        );
        assert_eq!(nominated.pending, Some(STRANGER));
    }

    #[test]
    fn nobody_but_the_authority_can_nominate() {
        let config = config_account(&holding(ADMIN));

        assert_eq!(
            nominate(&config, &signer(STRANGER), STRANGER, OURS),
            Err(AdminError::Unauthorised)
        );
    }

    #[test]
    fn the_zero_key_cannot_be_nominated() {
        let config = config_account(&holding(ADMIN));

        assert_eq!(
            nominate(&config, &signer(ADMIN), [0u8; 32], OURS),
            Err(AdminError::AuthorityIsZero)
        );
    }

    #[test]
    fn a_second_nomination_replaces_the_first() {
        let config = config_account(&AdminAccount {
            admin: Some(ADMIN),
            pending: Some(STRANGER),
        });

        let nominated = nominate(&config, &signer(ADMIN), THIRD, OURS).expect("admin signs");

        assert_eq!(
            nominated.pending,
            Some(THIRD),
            "only the latest can be accepted"
        );
    }

    #[test]
    fn the_nominee_takes_the_authority_by_signing_for_it() {
        // The half that makes a mistyped nomination survivable: until this
        // happens, the authority has not moved anywhere.
        let config = config_account(&AdminAccount {
            admin: Some(ADMIN),
            pending: Some(STRANGER),
        });

        let taken = accept(&config, &signer(STRANGER), OURS).expect("the nominee signs");

        assert_eq!(taken, holding(STRANGER), "and the nomination is spent");
    }

    #[test]
    fn nobody_but_the_nominee_can_accept() {
        let config = config_account(&AdminAccount {
            admin: Some(ADMIN),
            pending: Some(STRANGER),
        });

        assert_eq!(
            accept(&config, &signer(THIRD), OURS),
            Err(AdminError::NotTheNominee)
        );
        // Including the outgoing authority, which cannot hand itself the key it
        // nominated somebody else for.
        assert_eq!(
            accept(&config, &signer(ADMIN), OURS),
            Err(AdminError::NotTheNominee)
        );
    }

    #[test]
    fn there_is_nothing_to_accept_without_a_nomination() {
        let config = config_account(&holding(ADMIN));

        assert_eq!(
            accept(&config, &signer(STRANGER), OURS),
            Err(AdminError::NoNomination)
        );
    }

    #[test]
    fn the_authority_can_be_given_up_permanently() {
        // RFP-001's contract includes this. What it costs is in `[M2-06:01]`: a
        // revoked feed's signer set can never be rotated again.
        let config = config_account(&holding(ADMIN));

        let given_up = revoke(&config, &signer(ADMIN), OURS).expect("admin signs");

        assert_eq!(given_up.admin, None);
    }

    #[test]
    fn nobody_but_the_authority_can_give_it_up() {
        let config = config_account(&holding(ADMIN));

        assert_eq!(
            revoke(&config, &signer(STRANGER), OURS),
            Err(AdminError::Unauthorised)
        );
    }

    #[test]
    fn giving_up_the_authority_also_cancels_a_pending_nomination() {
        // Otherwise the nominee could accept afterwards and take an authority
        // its holder had already given up -- revocation would be undoable by
        // whoever was nominated last.
        let config = config_account(&AdminAccount {
            admin: Some(ADMIN),
            pending: Some(STRANGER),
        });

        let given_up = revoke(&config, &signer(ADMIN), OURS).expect("admin signs");

        assert_eq!(given_up.pending, None);
        assert_eq!(
            accept(&config_account(&given_up), &signer(STRANGER), OURS),
            Err(AdminError::NoNomination)
        );
    }

    #[test]
    fn a_config_account_this_program_does_not_own_is_refused() {
        // The attack this closes: supply any account naming yourself as admin.
        // Ownership is what makes the stored key this program's claim rather
        // than the caller's.
        let mut forged = config_account(&holding(STRANGER));
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
        let config = config_account(&holding(ADMIN));
        let mut unsigned = signer(ADMIN);
        unsigned.is_authorized = false;

        assert_eq!(
            authorise(&config, &unsigned, OURS),
            Err(AdminError::NotSigned)
        );
    }

    #[test]
    fn a_config_account_that_does_not_decode_is_refused() {
        let mut rubbish = config_account(&holding(ADMIN));
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
        let config = config_account(&AdminAccount {
            admin: None,
            pending: None,
        });

        assert_eq!(
            authorise(&config, &signer(ADMIN), OURS),
            Err(AdminError::NoAuthority)
        );
    }

    #[test]
    fn a_pristine_key_cannot_establish_the_authority() {
        // The state a runbook produces by default, and terminal rather than
        // merely wasteful: `initialise_admin` writes the config and claims it
        // before LEZ bumps the key's nonce, so the authority is established on an
        // account that can never appear in a post-state again, and `initialise`
        // answers `AlreadyInitialised` from then on. Refusing it here is what
        // turns that into a typed error.
        assert_eq!(
            initialise(&fresh_config(), &pristine(ADMIN), &ADMIN),
            Err(AdminError::AdminUnowned)
        );
    }

    #[test]
    fn a_pristine_nominee_cannot_accept_the_authority() {
        // Terminal in the same way `initialise` is, and reachable during normal
        // operation rather than once at deployment. Acceptance moves the
        // authority onto a key LEZ strands one transaction later, and
        // `initialise` refuses a config account that is no longer default, so
        // nothing could take it back.
        let config = config_account(&AdminAccount {
            admin: Some(ADMIN),
            pending: Some(STRANGER),
        });

        assert_eq!(
            accept(&config, &pristine(STRANGER), OURS),
            Err(AdminError::AdminUnowned)
        );
    }

    #[test]
    fn a_pristine_key_may_still_be_nominated() {
        // `nominate` never sees the nominee's account, because the nominee does
        // not sign the nomination. That is safe: a nomination is not the
        // authority, and `accept` is where the account becomes visible.
        let config = config_account(&holding(ADMIN));

        let nominated = nominate(&config, &signer(ADMIN), STRANGER, OURS).expect("admin signs");

        assert_eq!(nominated.pending, Some(STRANGER));
    }
}
