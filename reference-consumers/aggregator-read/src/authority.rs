//! Who may change which price account this consumer believes.
//!
//! This consumer reads a price somebody else verified, so what it has to get
//! right is *whose* price. That decision lives in a [`crate::source::PriceSource`]
//! account per feed, and this module holds the one account that says who may
//! write those: an authority, at an address derived from nothing but the program
//! id.
//!
//! # Why an authority at all, when the aggregator's id could be a constant
//!
//! Because a program's id is its RISC0 image id, and the price account's address
//! hashes it. Every change to the aggregator's build — a fix, a dependency bump,
//! a new instruction — moves every price account it writes. A consumer holding
//! that id as a compiled constant would meet each of those with a redeployment,
//! and in LEZ a redeployment is not a restart: this program's own id would move
//! too, taking the config account, the source accounts and every open order with
//! it.
//!
//! So the trigger differs from reference consumer B's and the mechanism is the
//! same one. B follows RedStone's signer rotations; this consumer follows the
//! aggregator's builds. Both are certain to happen and neither is this program's
//! to schedule, which is what makes them configuration rather than constants.
//! `[M3-05:01]` records the decision.
//!
//! What stays compiled is the genesis key below, because there is no earlier
//! state to read it from.
//!
//! # The genesis authority, and the race it closes
//!
//! The first write to the config account decides who governs the program, so an
//! unguarded `establish` is a race that whoever watches for the deployment wins.
//! The authority is therefore a build input: [`establish`] refuses any signer but
//! the compiled key, and refuses a build that configured none. The guest binary
//! carries it, because that is where the build happens.
//!
//! It recurs per build rather than per deployment: a rebuild from unchanged
//! inputs lands on the same config account, and a build with a changed input
//! presents a fresh one. The aggregator's `[M2-06:01]` works through the same
//! mechanism on its own config account.
//!
//! # Handover is two steps
//!
//! [`nominate`] records a key and [`accept`] requires that key to sign, so a
//! mistyped nomination costs a second nomination rather than the whole
//! administrative surface.
//!
//! [`accept`] also refuses a nominee whose account is unowned. LEZ rule 4 forbids
//! a program giving up ownership and rule 7 refuses a post-state whose owner is
//! the default one unless the pre-state was default too, so an authority key that
//! is a pristine account can never be made to sign anything this program would
//! honour. `nominate` cannot see it — the nominee does not sign that transaction
//! — so [`accept`] is the first point the account is visible and the last before
//! the handover is irreversible.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId, DEFAULT_PROGRAM_ID};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

/// The name seed the config account's address is derived from.
pub const CONFIG_ACCOUNT_SEED: &str = "KANON_READ_CONFIG";

/// The address a program's config account lives at.
///
/// Public because a client has to derive it to build any administrative
/// transaction, and every place that spelled the derivation out by hand was a
/// place it could drift from the constraint the guest declares.
#[must_use]
pub fn config_address(self_program_id: &ProgramId) -> lee_core::account::AccountId {
    spel_framework::pda::compute_pda(self_program_id, &[&seed_from_str(CONFIG_ACCOUNT_SEED)])
}

/// The authority, as the config account stores it.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct ConfigAccount {
    /// The key that may register and retire a feed's price source.
    pub authority: [u8; 32],
    /// A key nominated to take over, which is not the authority until it accepts.
    pub pending: Option<[u8; 32]>,
}

/// Why an instruction that reads or changes the authority was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    /// The account offered as the config is not one this program owns.
    ///
    /// Checked before the data is read: a key stored in an account this program
    /// does not own is the caller's claim rather than this program's record. An
    /// unestablished account is the same refusal, since it carries the default
    /// owner.
    ConfigNotOurs,
    /// The config account's bytes are not a config.
    ConfigUndecodable,
    /// The account claiming to be the authority did not sign.
    NotSigned,
    /// The signer is not this program's authority.
    Unauthorised,
    /// This build configured no genesis authority, so nothing can establish it.
    ///
    /// Its own cause rather than folded into [`Self::NotGenesisKey`]: a build
    /// nobody configured has no caller who could pass that check, and an
    /// operator needs to be told to set the key rather than to find the right
    /// one.
    NoGenesisAuthority,
    /// The config account already holds an authority.
    AlreadyEstablished,
    /// The account at the config address is not one this program can write.
    ///
    /// Non-default and unowned: LEZ refuses the data write, because the
    /// pre-state is not default and this program is not the owner, and refuses
    /// the post-state too, because the claim loop assigns ownership only after
    /// validation has run. The config address derives from the program id alone,
    /// and a reproducible build makes the next one computable, so anybody can
    /// put an account there before a deployment does.
    ConfigAccountUnusable,
    /// The signer is not the genesis key this build was configured with.
    NotGenesisKey,
    /// A nomination named the zero key, which nothing can sign for.
    NomineeIsZero,
    /// Nobody has been nominated.
    NoNomination,
    /// The signer is not the nominee.
    NotTheNominee,
    /// The key involved is a pristine account, which can never sign for this
    /// program's own writes. See the module header.
    KeyUnowned,
}

impl AuthorityError {
    /// A stable number per cause, in this program's 1200 block.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::ConfigNotOurs => 1201,
            Self::ConfigUndecodable => 1202,
            Self::NotSigned => 1203,
            Self::Unauthorised => 1204,
            Self::NoGenesisAuthority => 1205,
            Self::AlreadyEstablished => 1206,
            Self::ConfigAccountUnusable => 1207,
            Self::NotGenesisKey => 1208,
            Self::NomineeIsZero => 1209,
            Self::NoNomination => 1210,
            Self::NotTheNominee => 1211,
            Self::KeyUnowned => 1212,
        }
    }
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ConfigNotOurs => "the account offered as the config is not ours",
            Self::ConfigUndecodable => "the config account's bytes are not a config",
            Self::NotSigned => "the authority did not sign",
            Self::Unauthorised => "the signer is not this program's authority",
            Self::NoGenesisAuthority => "this build configured no genesis authority",
            Self::AlreadyEstablished => "this program's authority is already established",
            Self::ConfigAccountUnusable => {
                "the account at the config address is not one this program can write"
            }
            Self::NotGenesisKey => "the signer is not this build's genesis authority",
            Self::NomineeIsZero => "the zero key cannot be nominated",
            Self::NoNomination => "nobody has been nominated",
            Self::NotTheNominee => "the signer is not the nominee",
            Self::KeyUnowned => {
                "that key is a pristine account and can never sign for this program"
            }
        })
    }
}

impl From<AuthorityError> for spel_framework::error::SpelError {
    fn from(err: AuthorityError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Reads the config account and refuses unless `authority` is the authority and
/// signed.
///
/// The gate every mutating instruction runs first. It returns the stored config
/// so a caller that is about to rewrite it does not read the account twice.
///
/// # Errors
///
/// [`AuthorityError`] when the config is not this program's, is unreadable, or
/// the signer is not the authority.
pub fn authorise(
    config: &AccountWithMetadata,
    authority: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<ConfigAccount, AuthorityError> {
    let stored = stored_config(config, authority, self_program_id)?;
    if stored.authority == *authority.account_id.value() {
        Ok(stored)
    } else {
        Err(AuthorityError::Unauthorised)
    }
}

fn stored_config(
    config: &AccountWithMetadata,
    signer: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<ConfigAccount, AuthorityError> {
    if config.account.program_owner != self_program_id {
        return Err(AuthorityError::ConfigNotOurs);
    }
    if !signer.is_authorized {
        return Err(AuthorityError::NotSigned);
    }
    ConfigAccount::try_from_slice(config.account.data.as_ref())
        .map_err(|_| AuthorityError::ConfigUndecodable)
}

fn owned(key: &AccountWithMetadata) -> Result<(), AuthorityError> {
    if key.account.program_owner == DEFAULT_PROGRAM_ID {
        return Err(AuthorityError::KeyUnowned);
    }
    Ok(())
}

/// Establishes the authority from the key this build was compiled with.
///
/// # Errors
///
/// [`AuthorityError`] when the build configured no genesis key, the signer is
/// not it, or the config address already holds something.
pub fn establish(
    config: AccountWithMetadata,
    authority: AccountWithMetadata,
    genesis: &[u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AuthorityError> {
    // First, because a build with no genesis key has no caller who could pass
    // anything below, and that is a different fault from getting the key wrong.
    if *genesis == [0u8; 32] {
        return Err(AuthorityError::NoGenesisAuthority);
    }
    if !authority.is_authorized {
        return Err(AuthorityError::NotSigned);
    }
    if config.account != Account::default() {
        // Ours means established; anything else means the address is occupied by
        // an account no program can write. Two causes, because an operator's
        // next move differs.
        return Err(if config.account.program_owner == self_program_id {
            AuthorityError::AlreadyEstablished
        } else {
            AuthorityError::ConfigAccountUnusable
        });
    }
    if authority.account_id.value() != genesis {
        return Err(AuthorityError::NotGenesisKey);
    }
    // Last, so only the genesis key holder ever sees this refusal.
    owned(&authority)?;

    let established = ConfigAccount {
        authority: *genesis,
        pending: None,
    };
    Ok(vec![
        write_config(config.account, &established)?,
        AccountPostState::new(authority.account),
    ])
}

/// Records a key that may take the authority over once it accepts.
///
/// # Errors
///
/// [`AuthorityError`] when the signer is not the authority or the nominee is the
/// zero key.
pub fn nominate(
    config: AccountWithMetadata,
    authority: AccountWithMetadata,
    nominee: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AuthorityError> {
    let stored = authorise(&config, &authority, self_program_id)?;
    if nominee == [0u8; 32] {
        return Err(AuthorityError::NomineeIsZero);
    }

    let nominated = ConfigAccount {
        authority: stored.authority,
        pending: Some(nominee),
    };
    Ok(vec![
        write_config(config.account, &nominated)?,
        AccountPostState::new(authority.account),
    ])
}

/// Takes the authority over, as the nominee.
///
/// # Errors
///
/// [`AuthorityError`] when nobody is nominated, the signer is not the nominee,
/// or the nominee's account is pristine.
pub fn accept(
    config: AccountWithMetadata,
    nominee: AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, AuthorityError> {
    // Not `authorise`: the signer here is the nominee rather than the authority,
    // so the comparison is against the other field.
    let stored = stored_config(&config, &nominee, self_program_id)?;

    let key = match stored.pending {
        None => return Err(AuthorityError::NoNomination),
        Some(key) if key == *nominee.account_id.value() => key,
        Some(_) => return Err(AuthorityError::NotTheNominee),
    };
    // After the nominee is known and before the authority moves. See the module
    // header on why a pristine key is refused here and cannot be refused
    // earlier.
    owned(&nominee)?;

    let accepted = ConfigAccount {
        authority: key,
        pending: None,
    };
    Ok(vec![
        write_config(config.account, &accepted)?,
        AccountPostState::new(nominee.account),
    ])
}

/// The config account's post-state, claiming the address on a first write.
fn write_config(
    account: Account,
    stored: &ConfigAccount,
) -> Result<AccountPostState, AuthorityError> {
    let first = account == Account::default();
    let mut account = account;
    account.data =
        Data::try_from(borsh::to_vec(stored).map_err(|_| AuthorityError::ConfigUndecodable)?)
            .map_err(|_| AuthorityError::ConfigUndecodable)?;

    // From the same discriminator the caller checked, not from the owner: LEZ
    // refuses a claim on an account whose owner is not the default one, so
    // claiming twice fails the transaction rather than being ignored.
    Ok(if first {
        AutoClaim::pda_from_seeds(&[&seed_from_str(CONFIG_ACCOUNT_SEED)]).to_post_state(account)
    } else {
        AccountPostState::new(account)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{AccountId, Nonce};

    const OURS: ProgramId = [7u32; 8];
    const WALLET: ProgramId = [42u32; 8];
    const DEFAULT: ProgramId = [0u32; 8];

    const GENESIS: [u8; 32] = [0x61; 32];
    const STRANGER: [u8; 32] = [0x5A; 32];
    const NOMINEE: [u8; 32] = [0x0B; 32];

    fn at(id: AccountId, owner: ProgramId, data: Vec<u8>) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: owner,
                balance: 0,
                data: Data::try_from(data).expect("fits"),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: id,
        }
    }

    /// An account some program owns, having signed before, which is what a real
    /// key looks like after its first transaction. A fixture built from
    /// `Account::default()` would pass every assertion while describing a state
    /// no real key is in.
    fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        let mut key = at(AccountId::new(id), WALLET, Vec::new());
        key.account.balance = 500;
        key.account.nonce = Nonce(3);
        key.is_authorized = signs;
        key
    }

    fn unestablished() -> AccountWithMetadata {
        at(config_address(&OURS), DEFAULT, Vec::new())
    }

    fn established(authority: [u8; 32], pending: Option<[u8; 32]>) -> AccountWithMetadata {
        let stored = ConfigAccount { authority, pending };
        at(
            config_address(&OURS),
            OURS,
            borsh::to_vec(&stored).expect("serialises"),
        )
    }

    fn stored_in(posts: &[AccountPostState]) -> ConfigAccount {
        ConfigAccount::try_from_slice(posts[0].account().data.as_ref()).expect("a config")
    }

    #[test]
    fn the_genesis_key_establishes_the_authority() {
        let posts = establish(unestablished(), key(GENESIS, true), &GENESIS, OURS).expect("ok");

        assert_eq!(stored_in(&posts).authority, GENESIS);
        assert!(posts[0].required_claim().is_some());
    }

    #[test]
    fn anybody_but_the_genesis_key_is_refused() {
        assert_eq!(
            establish(unestablished(), key(STRANGER, true), &GENESIS, OURS).unwrap_err(),
            AuthorityError::NotGenesisKey
        );
    }

    /// A build nobody configured has no caller who could pass the key check, and
    /// an operator needs to be told to set the key rather than to find the right
    /// one.
    #[test]
    fn an_unconfigured_build_refuses_to_establish_an_authority() {
        assert_eq!(
            establish(unestablished(), key(GENESIS, true), &[0u8; 32], OURS).unwrap_err(),
            AuthorityError::NoGenesisAuthority
        );
    }

    #[test]
    fn the_authority_is_established_once() {
        assert_eq!(
            establish(
                established(GENESIS, None),
                key(GENESIS, true),
                &GENESIS,
                OURS
            )
            .unwrap_err(),
            AuthorityError::AlreadyEstablished
        );
    }

    /// The config address takes no input an attacker cannot predict, so anybody
    /// may put a unit of balance there before a deployment does. That state is
    /// not "already established" and the advice differs.
    #[test]
    fn a_squatted_config_address_is_its_own_refusal() {
        let mut squatted = unestablished();
        squatted.account.balance = 1;

        assert_eq!(
            establish(squatted, key(GENESIS, true), &GENESIS, OURS).unwrap_err(),
            AuthorityError::ConfigAccountUnusable
        );
    }

    #[test]
    fn the_gate_refuses_a_signer_that_is_not_the_authority() {
        assert_eq!(
            authorise(&established(GENESIS, None), &key(STRANGER, true), OURS).unwrap_err(),
            AuthorityError::Unauthorised
        );
    }

    #[test]
    fn the_gate_refuses_the_authority_when_it_did_not_sign() {
        assert_eq!(
            authorise(&established(GENESIS, None), &key(GENESIS, false), OURS).unwrap_err(),
            AuthorityError::NotSigned
        );
    }

    /// A key stored in an account this program does not own is the caller's
    /// claim rather than this program's record.
    #[test]
    fn a_config_account_this_program_does_not_own_is_not_a_config() {
        let mut theirs = established(GENESIS, None);
        theirs.account.program_owner = WALLET;

        assert_eq!(
            authorise(&theirs, &key(GENESIS, true), OURS).unwrap_err(),
            AuthorityError::ConfigNotOurs
        );
    }

    #[test]
    fn a_handover_takes_two_transactions_and_two_signers() {
        let nominated = nominate(
            established(GENESIS, None),
            key(GENESIS, true),
            NOMINEE,
            OURS,
        )
        .expect("nominates");
        let stored = stored_in(&nominated);
        assert_eq!(stored.authority, GENESIS, "nomination is not a handover");
        assert_eq!(stored.pending, Some(NOMINEE));

        let accepted = accept(
            established(GENESIS, Some(NOMINEE)),
            key(NOMINEE, true),
            OURS,
        )
        .expect("accepts");
        assert_eq!(stored_in(&accepted).authority, NOMINEE);
        assert_eq!(stored_in(&accepted).pending, None);
    }

    #[test]
    fn only_the_nominee_can_accept() {
        assert_eq!(
            accept(
                established(GENESIS, Some(NOMINEE)),
                key(STRANGER, true),
                OURS
            )
            .unwrap_err(),
            AuthorityError::NotTheNominee
        );
    }

    #[test]
    fn nothing_can_be_accepted_when_nobody_was_nominated() {
        assert_eq!(
            accept(established(GENESIS, None), key(NOMINEE, true), OURS).unwrap_err(),
            AuthorityError::NoNomination
        );
    }

    #[test]
    fn the_zero_key_cannot_be_nominated() {
        assert_eq!(
            nominate(
                established(GENESIS, None),
                key(GENESIS, true),
                [0u8; 32],
                OURS
            )
            .unwrap_err(),
            AuthorityError::NomineeIsZero
        );
    }

    /// LEZ rule 4 forbids a program giving up ownership and rule 7 refuses a
    /// post-state whose owner is the default one unless the pre-state was
    /// default too, so a pristine account can never sign for this program's own
    /// writes. `accept` is the first point the nominee's account is visible and
    /// the last before the handover is irreversible.
    #[test]
    fn an_unowned_key_can_neither_establish_nor_accept() {
        let mut pristine = key(GENESIS, true);
        pristine.account.program_owner = DEFAULT;
        assert_eq!(
            establish(unestablished(), pristine, &GENESIS, OURS).unwrap_err(),
            AuthorityError::KeyUnowned
        );

        let mut pristine_nominee = key(NOMINEE, true);
        pristine_nominee.account.program_owner = DEFAULT;
        assert_eq!(
            accept(established(GENESIS, Some(NOMINEE)), pristine_nominee, OURS).unwrap_err(),
            AuthorityError::KeyUnowned
        );
    }

    #[test]
    fn no_two_causes_answer_with_the_same_number() {
        let causes = [
            AuthorityError::ConfigNotOurs,
            AuthorityError::ConfigUndecodable,
            AuthorityError::NotSigned,
            AuthorityError::Unauthorised,
            AuthorityError::NoGenesisAuthority,
            AuthorityError::AlreadyEstablished,
            AuthorityError::ConfigAccountUnusable,
            AuthorityError::NotGenesisKey,
            AuthorityError::NomineeIsZero,
            AuthorityError::NoNomination,
            AuthorityError::NotTheNominee,
            AuthorityError::KeyUnowned,
        ];

        let mut numbers: Vec<u32> = causes.iter().map(AuthorityError::code).collect();
        numbers.sort_unstable();
        let unique = numbers.len();
        numbers.dedup();
        assert_eq!(numbers.len(), unique);
    }
}
