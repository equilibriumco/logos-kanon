//! The admin-gated operations on a feed that already exists.
//!
//! [`register_feed`](crate::register::register_feed) creates a feed; these change
//! one. Signer-set updates are M2-08, deregistration M2-09, pausing M2-10 — all
//! the same shape, an authorised caller naming a registered feed and changing one
//! part of it, so they share a module and the answer to "which feed is this".
//!
//! # Naming the feed
//!
//! Every operation here takes `feed_id` as well as the feed account, and refuses
//! unless the account is the one that id derives *and* the one it stores. That is
//! two checks for one property on purpose:
//!
//! - The **address** is checked by the declared PDA constraint, which is also
//!   what publishes the derivation to a client (ADR 33).
//! - The **stored id** is checked here, because an account this program owns is
//!   not necessarily the feed the caller meant. Without it an operator rotating
//!   five feeds could hand the wrong account and silently move the wrong signer
//!   set — and a signer set is what a price means, so that is not a typo anyone
//!   would notice.
//!
//! It is the same rule ADR 16 set for the asset pair: a caller cannot reach a
//! feed without stating which feed it thought it was asking for.

use borsh::BorshDeserialize;
use lee_core::account::{AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use verifier_core::{AssetPair, ConfigError, FeedConfig, SignerAddress};

use crate::FeedAccount;

/// Why an operation on a registered feed was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageError {
    /// The feed account is not owned by this program, so it is not a feed.
    ///
    /// The same cause `SubmitError` reports, and it keeps that number: an
    /// operator reading a code off a failed transaction should not have to know
    /// which instruction produced it.
    FeedNotOurs,
    /// The account is owned by this program but its data is not a feed.
    FeedUndecodable,
    /// The account is a feed, but not the feed the caller named.
    ///
    /// Reachable only if the declared PDA constraint is absent or wrong, since
    /// the address is derived from the same id. Kept because it is the check
    /// that does not depend on the constraint being right, and because the two
    /// failures want different answers: a caller that derived the address
    /// correctly and named the wrong id has a client bug, and one whose account
    /// does not match its own id is being handed someone else's feed.
    FeedMismatch,
    /// The new parameters do not describe a usable feed.
    Config(ConfigError),
    /// The feed is already paused, so pausing it changes nothing.
    ///
    /// A typed refusal rather than silence: an operator pausing during an
    /// incident learns the feed is already safe, which is what they wanted to
    /// know. Silence would leave them wondering whether the instruction landed.
    AlreadyPaused,
    /// The feed is not paused, so there is nothing to resume.
    NotPaused,
    /// The serialised feed does not fit an account's data.
    ///
    /// Unreachable while [`ConfigError::TooManySigners`] bounds the signer list.
    /// Kept so that raising the bound fails closed rather than panicking in a
    /// guest.
    FeedTooLarge,
}

impl From<ConfigError> for ManageError {
    fn from(err: ConfigError) -> Self {
        Self::Config(err)
    }
}

impl core::fmt::Display for ManageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FeedNotOurs => {
                f.write_str("the account offered as a feed is not one this program owns")
            }
            Self::FeedUndecodable => f.write_str("the feed account's data is not a feed"),
            Self::FeedMismatch => {
                f.write_str("the feed account does not hold the feed id the caller named")
            }
            Self::FeedTooLarge => f.write_str("the serialised feed does not fit an account's data"),
            Self::AlreadyPaused => f.write_str("this feed is already paused"),
            Self::NotPaused => f.write_str("this feed is not paused"),
            Self::Config(err) => write!(f, "{err:?}"),
        }
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<ManageError> for spel_framework::error::SpelError {
    fn from(err: ManageError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

impl ManageError {
    /// A stable number per leaf cause. The two feed-account causes keep
    /// `SubmitError`'s numbers because they are the same causes; what is new to
    /// this module takes the 1000 block, after `RegisterError`'s 900s.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::FeedNotOurs => 101,
            Self::FeedUndecodable => 102,
            Self::FeedMismatch => 1001,
            Self::FeedTooLarge => 1002,
            Self::AlreadyPaused => 1003,
            Self::NotPaused => 1004,
            Self::Config(err) => crate::submit::config_code(*err),
        }
    }
}

/// The registered feed an operation names, or the reason it is not one.
///
/// Shared by every operation in this module, so they agree about what "the feed
/// the caller named" means rather than each deciding it.
fn named_feed(
    feed: &AccountWithMetadata,
    feed_id: &[u8; 32],
    self_program_id: ProgramId,
) -> Result<FeedAccount, ManageError> {
    if feed.account.program_owner != self_program_id {
        return Err(ManageError::FeedNotOurs);
    }
    let stored = FeedAccount::try_from_slice(feed.account.data.as_ref())
        .map_err(|_| ManageError::FeedUndecodable)?;
    if &stored.feed_id != feed_id {
        return Err(ManageError::FeedMismatch);
    }
    Ok(stored)
}

/// The post-states for an operation that rewrites the feed and touches nothing
/// else, in the instruction's account order.
///
/// No claim: the feed is already this program's, and re-claiming an account it
/// owns is refused by LEZ rather than ignored.
fn rewritten(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    stored: &FeedAccount,
) -> Result<Vec<AccountPostState>, ManageError> {
    let mut account = feed.account;
    account.data = Data::try_from(borsh::to_vec(stored).map_err(|_| ManageError::FeedTooLarge)?)
        .map_err(|_| ManageError::FeedTooLarge)?;

    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(admin.account),
        AccountPostState::new(config.account),
    ])
}

/// Replaces a feed's signer set and threshold, and nothing else.
///
/// The set and the threshold move together because they are one decision: a
/// threshold is only meaningful against the set it counts, and applying them in
/// two transactions would leave a window in which the feed verified against a
/// threshold its set could not reach — or, worse, one it could reach too easily.
///
/// Per feed, because the set is (ADR 30). An upstream rotation touching several
/// feeds is several of these, each naming its own feed.
///
/// # Errors
///
/// [`ManageError`] when the account is not the named feed, or when the new set
/// and threshold do not describe a usable feed.
pub fn update_signer_set(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    signers: Vec<[u8; SignerAddress::LEN]>,
    threshold: u8,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, ManageError> {
    let stored = named_feed(&feed, &feed_id, self_program_id)?;

    // Validated against the feed's own stored parameters, so the rule that
    // admits a registration admits an update. `FeedConfig::try_new` is the only
    // place that decides what a usable feed is; nothing here restates it.
    let addresses: Vec<SignerAddress> = signers.iter().copied().map(SignerAddress).collect();
    FeedConfig::try_new(
        &stored.feed_id,
        AssetPair::new(stored.base_asset, stored.quote_asset),
        stored.decimals,
        stored.max_age_ms,
        &addresses,
        threshold,
    )?;

    let updated = FeedAccount {
        signers,
        threshold,
        // Everything else is the registration's, including `paused`: a rotation
        // is not a reason to start or stop accepting prices, and an update that
        // silently unpaused a feed would be a way around M2-10's gate.
        ..stored
    };

    rewritten(feed, admin, config, &updated)
}

/// Retires a feed, leaving its account able to hold a new registration.
///
/// # What "retired" can mean
///
/// Not `Account::default()`. LEZ's rule 4 forbids a program giving up ownership
/// and rule 3 forbids resetting the nonce, so a feed account is this program's
/// from its first registration onwards and cannot be handed back. Since the
/// address is the feed id's (ADR 33), a deregistration that could not be undone
/// would spend the feed id permanently — and RedStone's ids are fixed strings, so
/// losing `BTC` once would mean losing it for the life of the deployment.
///
/// So the account stays ours and empties: `Data::default()`, which is a state
/// nothing else this program writes produces. `register_feed` accepts it as a
/// re-registration, and `submit_price` refuses it as `FeedDeregistered` rather
/// than as undecodable bytes.
///
/// A paused feed can be deregistered. Requiring an unpause first would mean
/// briefly accepting prices for a feed being retired, which is the opposite of
/// what pausing is for.
///
/// # Errors
///
/// [`ManageError`] when the account is not the named feed.
pub fn deregister_feed(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, ManageError> {
    // Read for its side effect: this refuses unless the account is the feed the
    // caller named, which is the whole of what deregistration decides.
    let _ = named_feed(&feed, &feed_id, self_program_id)?;

    let mut account = feed.account;
    account.data = Data::default();

    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(admin.account),
        AccountPostState::new(config.account),
    ])
}

/// Stops a feed accepting submissions, leaving everything else about it in place.
///
/// Pausing is the one administrative lever that does not change what a price
/// means: the signer set, the threshold, the window and the pair are all
/// untouched, so resuming restores exactly the feed that was paused. That is why
/// it is the right response to an incident and `update_signer_set` is not.
///
/// `submit_price` is what enforces it, and it does so before any cryptography, so
/// a paused feed does not pay to discover it is paused.
///
/// # Errors
///
/// [`ManageError`] when the account is not the named feed, or when the feed is
/// already paused.
pub fn pause_feed(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, ManageError> {
    set_paused(feed, admin, config, feed_id, self_program_id, true)
}

/// Lets a paused feed accept submissions again.
///
/// # Errors
///
/// [`ManageError`] when the account is not the named feed, or when the feed is
/// not paused.
pub fn unpause_feed(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, ManageError> {
    set_paused(feed, admin, config, feed_id, self_program_id, false)
}

/// The body both pause and unpause are, so they cannot disagree about anything
/// but the value they are setting.
fn set_paused(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
    paused: bool,
) -> Result<Vec<AccountPostState>, ManageError> {
    let stored = named_feed(&feed, &feed_id, self_program_id)?;

    // Refused rather than written again. A no-op write would cost a transaction
    // and tell the operator nothing about the state they were trying to reach.
    if stored.paused == paused {
        return Err(if paused {
            ManageError::AlreadyPaused
        } else {
            ManageError::NotPaused
        });
    }

    rewritten(feed, admin, config, &FeedAccount { paused, ..stored })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{Account, AccountId, Nonce};
    use verifier_core::feed::MAX_SIGNERS;

    const OURS: ProgramId = [7u32; 8];
    const SOMEONE_ELSE: ProgramId = [9u32; 8];
    const BTC: &[u8] = b"BTC";
    const ETH: &[u8] = b"ETH";

    fn feed_id(name: &[u8]) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[..name.len()].copy_from_slice(name);
        id
    }

    fn registered(name: &[u8], set: u8, threshold: u8, paused: bool) -> FeedAccount {
        FeedAccount {
            feed_id: feed_id(name),
            base_asset: [0xB7; 32],
            quote_asset: [0x05; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: signers(set),
            threshold,
            paused,
        }
    }

    fn signers(n: u8) -> Vec<[u8; SignerAddress::LEN]> {
        (1..=n).map(|i| [i; SignerAddress::LEN]).collect()
    }

    /// A distinct set of the same size, so a rotation is visible.
    fn rotated(n: u8) -> Vec<[u8; SignerAddress::LEN]> {
        (1..=n).map(|i| [0x80 + i; SignerAddress::LEN]).collect()
    }

    fn feed_account(state: &FeedAccount, owner: ProgramId) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: owner,
                balance: 0,
                data: Data::try_from(borsh::to_vec(state).expect("serialises")).expect("fits"),
                nonce: Nonce(3),
            },
            is_authorized: false,
            account_id: AccountId::new([0xFE; 32]),
        }
    }

    fn passthrough(tag: u8) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: [42u32; 8],
                balance: 5,
                data: Data::default(),
                nonce: Nonce(1),
            },
            is_authorized: true,
            account_id: AccountId::new([tag; 32]),
        }
    }

    fn update(
        feed: AccountWithMetadata,
        named: [u8; 32],
        set: Vec<[u8; SignerAddress::LEN]>,
        threshold: u8,
    ) -> Result<Vec<AccountPostState>, ManageError> {
        update_signer_set(
            feed,
            passthrough(0xAD),
            passthrough(0xC0),
            named,
            set,
            threshold,
            OURS,
        )
    }

    fn stored_in(post: &AccountPostState) -> FeedAccount {
        FeedAccount::try_from_slice(post.account().data.as_ref()).expect("decodes")
    }

    #[test]
    fn a_rotation_replaces_the_set_and_the_threshold() {
        let before = registered(BTC, 5, 3, false);
        let posts =
            update(feed_account(&before, OURS), feed_id(BTC), rotated(5), 4).expect("a usable set");

        let after = stored_in(&posts[0]);
        assert_eq!(after.signers, rotated(5));
        assert_eq!(after.threshold, 4);
    }

    #[test]
    fn a_rotation_changes_nothing_else_about_the_feed() {
        // Including `paused`. An update that silently unpaused a feed would be a
        // way around M2-10's gate.
        let before = registered(BTC, 5, 3, true);
        let posts =
            update(feed_account(&before, OURS), feed_id(BTC), rotated(3), 2).expect("a usable set");

        let after = stored_in(&posts[0]);
        assert_eq!(after.feed_id, before.feed_id);
        assert_eq!(after.base_asset, before.base_asset);
        assert_eq!(after.quote_asset, before.quote_asset);
        assert_eq!(after.decimals, before.decimals);
        assert_eq!(after.max_age_ms, before.max_age_ms);
        assert!(after.paused, "a rotation is not a reason to unpause");
    }

    #[test]
    fn the_feed_is_not_re_claimed() {
        // It is already this program's. LEZ refuses a claim on an account whose
        // owner is not the default one, so claiming again would fail the
        // transaction rather than be ignored.
        let posts = update(
            feed_account(&registered(BTC, 3, 2, false), OURS),
            feed_id(BTC),
            rotated(3),
            2,
        )
        .expect("a usable set");

        assert!(posts[0].required_claim().is_none());
    }

    #[test]
    fn an_account_this_program_does_not_own_is_not_a_feed() {
        let state = registered(BTC, 3, 2, false);
        assert_eq!(
            update(
                feed_account(&state, SOMEONE_ELSE),
                feed_id(BTC),
                rotated(3),
                2
            ),
            Err(ManageError::FeedNotOurs)
        );
    }

    #[test]
    fn an_account_of_ours_that_is_not_a_feed_is_refused() {
        let mut feed = feed_account(&registered(BTC, 3, 2, false), OURS);
        feed.account.data = Data::try_from(vec![0xFFu8; 8]).expect("fits");
        assert_eq!(
            update(feed, feed_id(BTC), rotated(3), 2),
            Err(ManageError::FeedUndecodable)
        );
    }

    #[test]
    fn a_feed_that_is_not_the_one_named_is_refused() {
        // The check the PDA constraint cannot make on its own: an account this
        // program owns, holding a real feed, that is not the feed the caller
        // said it was rotating. Handing the wrong account while rotating five
        // feeds is the mistake this refuses.
        let btc = registered(BTC, 5, 3, false);
        assert_eq!(
            update(feed_account(&btc, OURS), feed_id(ETH), rotated(5), 3),
            Err(ManageError::FeedMismatch)
        );
    }

    #[test]
    fn a_new_set_is_refused_by_the_rule_the_verifier_already_had() {
        // Validated against the feed's own stored parameters, so what admits a
        // registration admits an update. `verifier-core` owns these rules; this
        // is one of each shape reachable from a rotation.
        let state = registered(BTC, 5, 3, false);
        let cases: Vec<(ManageError, Vec<[u8; SignerAddress::LEN]>, u8)> = vec![
            (
                ManageError::Config(ConfigError::ThresholdZero),
                rotated(3),
                0,
            ),
            (ManageError::Config(ConfigError::NoSigners), Vec::new(), 1),
            (
                ManageError::Config(ConfigError::DuplicateSigner),
                vec![[1u8; 20], [1u8; 20]],
                1,
            ),
            (
                ManageError::Config(ConfigError::ZeroSignerAddress),
                vec![[0u8; 20]],
                1,
            ),
            (
                ManageError::Config(ConfigError::ThresholdExceedsSigners {
                    threshold: 4,
                    signers: 3,
                }),
                rotated(3),
                4,
            ),
        ];

        for (expected, set, threshold) in cases {
            assert_eq!(
                update(feed_account(&state, OURS), feed_id(BTC), set, threshold),
                Err(expected)
            );
        }
    }

    #[test]
    fn a_threshold_the_new_set_cannot_reach_is_refused_with_the_set() {
        // Why the two move together. Applying them in two transactions would
        // leave a window in which the feed verified against a threshold its set
        // could not reach; here the pair is refused or accepted as one.
        let state = registered(BTC, 5, 3, false);
        assert_eq!(
            update(feed_account(&state, OURS), feed_id(BTC), rotated(2), 3),
            Err(ManageError::Config(ConfigError::ThresholdExceedsSigners {
                threshold: 3,
                signers: 2
            })),
            "shrinking the set below the threshold has to be one refusal"
        );
    }

    #[test]
    fn shrinking_the_set_and_the_threshold_together_is_allowed() {
        // The other side of the same property: a rotation that removes signers
        // is fine as long as the threshold comes with it, which is what an
        // upstream roster shrinking looks like.
        let state = registered(BTC, 5, 3, false);
        let posts = update(feed_account(&state, OURS), feed_id(BTC), rotated(2), 2)
            .expect("two of two is usable");

        let after = stored_in(&posts[0]);
        assert_eq!(after.signers.len(), 2);
        assert_eq!(after.threshold, 2);
    }

    #[test]
    fn the_maximum_set_still_fits_an_account() {
        let state = registered(BTC, 5, 3, false);
        let max = u8::try_from(MAX_SIGNERS).expect("MAX_SIGNERS fits a u8");
        let posts = update(feed_account(&state, OURS), feed_id(BTC), rotated(max), max)
            .expect("the maximum fits");
        assert_eq!(stored_in(&posts[0]).signers.len(), MAX_SIGNERS);
    }

    #[test]
    fn a_shared_cause_keeps_the_number_it_has_elsewhere() {
        use crate::SubmitError;
        assert_eq!(
            ManageError::FeedNotOurs.code(),
            SubmitError::FeedNotOurs.code(),
            "one cause, one number, whichever instruction reports it"
        );
        assert_eq!(
            ManageError::FeedUndecodable.code(),
            SubmitError::FeedUndecodable.code()
        );
        assert_eq!(ManageError::FeedMismatch.code(), 1001);
        assert_eq!(ManageError::FeedTooLarge.code(), 1002);
        assert_eq!(
            ManageError::Config(ConfigError::ThresholdZero).code(),
            crate::submit::config_code(ConfigError::ThresholdZero)
        );
    }
    #[test]
    fn a_deregistration_empties_the_account_and_keeps_it() {
        // Rule 4 forbids giving up ownership and rule 3 forbids resetting the
        // nonce, so this is the most a program can do: the account stays ours,
        // at its nonce, holding nothing.
        let before = feed_account(&registered(BTC, 5, 3, false), OURS);
        let owner = before.account.program_owner;
        let nonce = before.account.nonce;

        let posts = deregister_feed(
            before,
            passthrough(0xAD),
            passthrough(0xC0),
            feed_id(BTC),
            OURS,
        )
        .expect("the named feed");

        assert!(
            posts[0].account().data.as_ref().is_empty(),
            "no feed is left"
        );
        assert_eq!(
            posts[0].account().program_owner,
            owner,
            "still ours (rule 4)"
        );
        assert_eq!(posts[0].account().nonce, nonce, "unchanged (rule 3)");
        assert!(posts[0].required_claim().is_none(), "already claimed");
    }

    #[test]
    fn a_paused_feed_can_be_deregistered() {
        // Requiring an unpause first would mean briefly accepting prices for a
        // feed being retired, which is the opposite of what pausing is for.
        let posts = deregister_feed(
            feed_account(&registered(BTC, 3, 2, true), OURS),
            passthrough(0xAD),
            passthrough(0xC0),
            feed_id(BTC),
            OURS,
        )
        .expect("a paused feed retires");
        assert!(posts[0].account().data.as_ref().is_empty());
    }

    #[test]
    fn a_deregistration_refuses_a_feed_it_was_not_named() {
        let btc = feed_account(&registered(BTC, 5, 3, false), OURS);
        assert_eq!(
            deregister_feed(
                btc,
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(ETH),
                OURS
            ),
            Err(ManageError::FeedMismatch)
        );
    }

    #[test]
    fn a_deregistration_refuses_an_account_that_is_not_ours() {
        let feed = feed_account(&registered(BTC, 3, 2, false), SOMEONE_ELSE);
        assert_eq!(
            deregister_feed(
                feed,
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS
            ),
            Err(ManageError::FeedNotOurs)
        );
    }

    #[test]
    fn an_already_deregistered_feed_cannot_be_deregistered_again() {
        // The account is ours and empty, so there is no feed to name. Reported
        // as undecodable rather than as a mismatch, because that is what an
        // empty account is: no feed at all.
        let mut feed = feed_account(&registered(BTC, 3, 2, false), OURS);
        feed.account.data = Data::default();
        assert_eq!(
            deregister_feed(
                feed,
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS
            ),
            Err(ManageError::FeedUndecodable)
        );
    }
    #[test]
    fn pausing_stops_submissions_and_changes_nothing_else() {
        // The one lever that does not change what a price means, which is why it
        // is the right response to an incident.
        let before = registered(BTC, 5, 3, false);
        let posts = pause_feed(
            feed_account(&before, OURS),
            passthrough(0xAD),
            passthrough(0xC0),
            feed_id(BTC),
            OURS,
        )
        .expect("a live feed pauses");

        let after = stored_in(&posts[0]);
        assert!(after.paused);
        assert_eq!(after.signers, before.signers, "the set is untouched");
        assert_eq!(after.threshold, before.threshold);
        assert_eq!(after.max_age_ms, before.max_age_ms);
        assert_eq!(after.feed_id, before.feed_id);
    }

    #[test]
    fn unpausing_restores_exactly_the_feed_that_was_paused() {
        let live = registered(BTC, 5, 3, false);
        let paused = stored_in(
            &pause_feed(
                feed_account(&live, OURS),
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS,
            )
            .expect("pauses")[0],
        );

        let resumed = stored_in(
            &unpause_feed(
                feed_account(&paused, OURS),
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS,
            )
            .expect("resumes")[0],
        );

        assert_eq!(resumed, live, "a pause and an unpause is a round trip");
    }

    #[test]
    fn pausing_a_paused_feed_says_so() {
        // Silence would leave an operator wondering whether the instruction
        // landed; this tells them the feed is already safe.
        assert_eq!(
            pause_feed(
                feed_account(&registered(BTC, 3, 2, true), OURS),
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS
            ),
            Err(ManageError::AlreadyPaused)
        );
    }

    #[test]
    fn unpausing_a_live_feed_says_so() {
        assert_eq!(
            unpause_feed(
                feed_account(&registered(BTC, 3, 2, false), OURS),
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(BTC),
                OURS
            ),
            Err(ManageError::NotPaused)
        );
    }

    #[test]
    fn pausing_refuses_a_feed_it_was_not_named() {
        assert_eq!(
            pause_feed(
                feed_account(&registered(BTC, 5, 3, false), OURS),
                passthrough(0xAD),
                passthrough(0xC0),
                feed_id(ETH),
                OURS
            ),
            Err(ManageError::FeedMismatch)
        );
    }

    #[test]
    fn every_cause_in_this_module_has_its_own_number() {
        let causes = [
            ManageError::FeedNotOurs,
            ManageError::FeedUndecodable,
            ManageError::FeedMismatch,
            ManageError::FeedTooLarge,
            ManageError::AlreadyPaused,
            ManageError::NotPaused,
        ];
        // No wildcard arm, so a new variant fails to compile here. It does not
        // prove the list is complete -- an author can add the arm and forget the
        // entry -- so the list is kept by hand; what this buys is that the
        // omission is loud.
        for cause in &causes {
            match cause {
                ManageError::FeedNotOurs
                | ManageError::FeedUndecodable
                | ManageError::FeedMismatch
                | ManageError::FeedTooLarge
                | ManageError::AlreadyPaused
                | ManageError::NotPaused
                | ManageError::Config(_) => {}
            }
        }
        let mut codes: Vec<u32> = causes.iter().map(ManageError::code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before, "two causes answer with one code");
    }
}
