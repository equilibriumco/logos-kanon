//! Who may change what this consumer trusts.
//!
//! The roster a pull consumer verifies against is the consumer's own decision
//! (SEC2), and this module is where that decision is governed. It holds one
//! account, at an address derived from nothing but the program id, and every
//! instruction that changes a feed's trust reads it first.
//!
//! # Why an authority at all, when the roster could be a constant
//!
//! Because RedStone rotates. ADR 30 records that the addresses in the capture
//! are unchanged since it was taken and that a source two and a half years older
//! shares none of them, which is why the push path has [`crate::trust::rotate_signers`]'s
//! counterpart in `update_signer_set` rather than a compiled table. A pull
//! consumer meets the same event on the same schedule.
//!
//! A compiled roster would make that event a redeployment, and in LEZ a
//! redeployment is not a restart. A program's id is its RISC0 image id and every
//! derived address hashes the program id, so changing a compiled input moves
//! every account this program owns — the config account, the trust accounts and
//! every open order — and abandons what the previous build created. Rotating a
//! signer set would cost every order on the books. `[M3-06:02]` records the
//! decision and what it replaced.
//!
//! # The genesis authority, and the race it closes
//!
//! The first write to the config account decides who governs the program, so an
//! unguarded `establish` is a race that whoever watches for the deployment wins.
//! The authority is therefore a build input: `establish` refuses any signer but
//! the compiled key, and refuses a build that configured none. The guest binary
//! carries it, because that is where the build happens.
//!
//! It recurs per build rather than per deployment, and that is the reproducible
//! image id again: a rebuild from unchanged inputs lands on the same config
//! account, and a build with a changed input presents a fresh one. The
//! aggregator's `[M2-06:01]` works through the same mechanism on its own config
//! account.
//!
//! # Handover is two steps
//!
//! [`nominate`] records a key and [`accept`] requires that key to sign. A
//! mistyped nomination therefore costs a second nomination rather than the whole
//! administrative surface, and the two halves can run in two transactions from
//! two parties.
//!
//! [`accept`] also refuses a nominee whose account is unowned, which is not
//! bureaucracy. LEZ rule 4 forbids a program giving up ownership and rule 7
//! refuses a post-state whose owner is the default one unless the pre-state was
//! default too, so an authority key that is a pristine account can never be
//! made to sign anything this program would honour. `nominate` cannot see it —
//! the nominee does not sign that transaction — so [`accept`] is the first point
//! the account is visible and the last before the handover is irreversible. The
//! aggregator found this the hard way on `accept_admin`.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId, DEFAULT_PROGRAM_ID};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

/// The name seed the config account's address is derived from.
pub const CONFIG_ACCOUNT_SEED: &str = "KANON_PULL_CONFIG";

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
    /// The key that may register a feed's trust and rotate its signers.
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
    use lee_core::program::{validate_execution, Claim};

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];
    const GENESIS: [u8; 32] = [0x61; 32];

    fn config_address() -> AccountId {
        super::config_address(&OURS)
    }

    fn unestablished() -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: config_address(),
        }
    }

    /// A key with a real account behind it: owned by some wallet program, with a
    /// balance and a nonce.
    fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: SOMEONE_ELSE,
                balance: 500,
                data: Data::default(),
                nonce: Nonce(3),
            },
            is_authorized: signs,
            account_id: AccountId::new(id),
        }
    }

    /// A key nobody has ever used: default owner, no balance, no nonce.
    fn pristine(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: signs,
            account_id: AccountId::new(id),
        }
    }

    fn as_chain_leaves_it(post: &AccountPostState) -> AccountWithMetadata {
        let mut account = post.account().clone();
        account.program_owner = OURS;
        AccountWithMetadata {
            account,
            is_authorized: false,
            account_id: config_address(),
        }
    }

    fn established() -> AccountWithMetadata {
        let posts = establish(unestablished(), key(GENESIS, true), &GENESIS, OURS)
            .expect("the genesis key establishes it");
        as_chain_leaves_it(&posts[0])
    }

    fn stored(config: &AccountWithMetadata) -> ConfigAccount {
        ConfigAccount::try_from_slice(config.account.data.as_ref()).expect("a config")
    }

    #[test]
    fn the_genesis_key_establishes_the_authority_and_claims_the_address() {
        let posts = establish(unestablished(), key(GENESIS, true), &GENESIS, OURS)
            .expect("the genesis key establishes it");

        let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
            panic!("a first write claims a PDA");
        };
        assert_eq!(
            AccountId::for_public_pda(&OURS, &seed),
            config_address(),
            "the claim has to name the address the constraint checked"
        );
        assert_eq!(stored(&as_chain_leaves_it(&posts[0])).authority, GENESIS);
        assert_eq!(stored(&as_chain_leaves_it(&posts[0])).pending, None);
    }

    #[test]
    fn an_establish_passes_lez() {
        let pre = vec![unestablished(), key(GENESIS, true)];
        let posts = establish(unestablished(), key(GENESIS, true), &GENESIS, OURS).expect("ok");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts the establish");
    }

    #[test]
    fn anybody_but_the_genesis_key_is_refused() {
        // The race this exists to close: without the compiled key the first
        // caller to reach a fresh deployment owns the program.
        assert_eq!(
            establish(unestablished(), key([0xAA; 32], true), &GENESIS, OURS),
            Err(AuthorityError::NotGenesisKey)
        );
    }

    #[test]
    fn a_build_with_no_genesis_key_cannot_be_established_by_anyone() {
        // Its own cause, and reported before any other check: a build nobody
        // configured has no caller who could pass them, so "you are not the
        // genesis key" would send an operator looking for a key that does not
        // exist.
        assert_eq!(
            establish(unestablished(), key([0u8; 32], true), &[0u8; 32], OURS),
            Err(AuthorityError::NoGenesisAuthority)
        );
    }

    #[test]
    fn an_unsigned_genesis_key_is_refused() {
        assert_eq!(
            establish(unestablished(), key(GENESIS, false), &GENESIS, OURS),
            Err(AuthorityError::NotSigned)
        );
    }

    #[test]
    fn establishing_twice_is_refused_as_established() {
        assert_eq!(
            establish(established(), key(GENESIS, true), &GENESIS, OURS),
            Err(AuthorityError::AlreadyEstablished)
        );
    }

    #[test]
    fn a_squatted_config_address_is_refused_as_unusable_and_not_as_established() {
        // The config address derives from the program id alone, and a
        // reproducible build makes the next one computable from published
        // source, so anybody can put an account there before a deployment does.
        // That account is non-default with a default owner, which LEZ will never
        // let any program write.
        //
        // Its own cause because the outcome differs: `AlreadyEstablished` means
        // somebody got there first, this means the build is unusable and needs a
        // changed input to move its addresses.
        let mut squatted = unestablished();
        squatted.account.balance = 1;

        assert_eq!(
            establish(squatted, key(GENESIS, true), &GENESIS, OURS),
            Err(AuthorityError::ConfigAccountUnusable)
        );
    }

    #[test]
    fn a_pristine_genesis_key_cannot_establish_the_authority() {
        // An account with the default owner can never be made to sign for a
        // write this program would honour, so an authority that is one is an
        // authority that cannot act. Refused last, so only the holder of the
        // genesis key sees it.
        assert_eq!(
            establish(unestablished(), pristine(GENESIS, true), &GENESIS, OURS),
            Err(AuthorityError::KeyUnowned)
        );
    }

    #[test]
    fn the_gate_refuses_a_config_this_program_does_not_own() {
        // Before the data is read: a key stored in an account the caller owns is
        // the caller's claim, not this program's record.
        let mut foreign = established();
        foreign.account.program_owner = SOMEONE_ELSE;
        assert_eq!(
            authorise(&foreign, &key(GENESIS, true), OURS),
            Err(AuthorityError::ConfigNotOurs)
        );
        assert_eq!(
            authorise(&unestablished(), &key(GENESIS, true), OURS),
            Err(AuthorityError::ConfigNotOurs)
        );
    }

    #[test]
    fn the_gate_refuses_a_signer_that_is_not_the_authority() {
        assert_eq!(
            authorise(&established(), &key([0xAA; 32], true), OURS),
            Err(AuthorityError::Unauthorised)
        );
        assert_eq!(
            authorise(&established(), &key(GENESIS, false), OURS),
            Err(AuthorityError::NotSigned)
        );
    }

    #[test]
    fn a_handover_takes_two_transactions_and_two_signers() {
        // The nomination alone does not move the authority, which is the whole
        // point: a mistyped key costs a second nomination rather than the
        // program.
        let nominee = [0xBB; 32];
        let posts = nominate(established(), key(GENESIS, true), nominee, OURS).expect("nominated");
        let nominated = as_chain_leaves_it(&posts[0]);
        assert_eq!(stored(&nominated).authority, GENESIS, "still the old key");
        assert_eq!(stored(&nominated).pending, Some(nominee));

        // The old authority still governs until the nominee accepts.
        authorise(&nominated, &key(GENESIS, true), OURS).expect("the old key still governs");

        let posts = accept(nominated, key(nominee, true), OURS).expect("accepted");
        let accepted = as_chain_leaves_it(&posts[0]);
        assert_eq!(stored(&accepted).authority, nominee);
        assert_eq!(stored(&accepted).pending, None);
        assert_eq!(
            authorise(&accepted, &key(GENESIS, true), OURS),
            Err(AuthorityError::Unauthorised),
            "the old key no longer governs"
        );
    }

    #[test]
    fn only_the_nominee_can_accept() {
        let posts = nominate(established(), key(GENESIS, true), [0xBB; 32], OURS).expect("ok");
        let nominated = as_chain_leaves_it(&posts[0]);
        assert_eq!(
            accept(nominated, key([0xCC; 32], true), OURS),
            Err(AuthorityError::NotTheNominee)
        );
    }

    #[test]
    fn accepting_with_nobody_nominated_is_refused() {
        assert_eq!(
            accept(established(), key([0xBB; 32], true), OURS),
            Err(AuthorityError::NoNomination)
        );
    }

    #[test]
    fn a_pristine_nominee_is_refused_at_the_accept_and_cannot_be_refused_earlier() {
        // `nominate` never sees the nominee's account -- the nominee does not
        // sign that transaction -- so this is the first point it is visible and
        // the last before the handover is irreversible. Handing the authority to
        // a key that can never sign for this program's writes would freeze every
        // trust account.
        let nominee = [0xBB; 32];
        let posts = nominate(established(), key(GENESIS, true), nominee, OURS)
            .expect("a pristine key can be nominated, which is the problem");
        assert_eq!(
            accept(as_chain_leaves_it(&posts[0]), pristine(nominee, true), OURS),
            Err(AuthorityError::KeyUnowned)
        );
    }

    #[test]
    fn the_zero_key_cannot_be_nominated() {
        assert_eq!(
            nominate(established(), key(GENESIS, true), [0u8; 32], OURS),
            Err(AuthorityError::NomineeIsZero)
        );
    }

    #[test]
    fn only_the_authority_can_nominate() {
        assert_eq!(
            nominate(established(), key([0xAA; 32], true), [0xBB; 32], OURS),
            Err(AuthorityError::Unauthorised)
        );
    }
}
