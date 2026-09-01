//! What this consumer trusts for one feed, and how it changes.
//!
//! One account per feed, at the address the feed's id derives, holding the whole
//! configuration RFP-020 names: the data service, the feed id, the pair, the
//! scale, the staleness window, the authorised signers and the M-of-N threshold.
//!
//! # Per feed, and all of it
//!
//! ADR 30's decision, and its argument reaches further than the roster. A
//! threshold is only meaningful against the set it counts and a window against
//! the cadence of the feed it bounds, so splitting a feed's parameters between a
//! global slot and a per-feed one would mean reading two places and knowing
//! which wins. A global roster is worse than redundant: it cannot represent an
//! estate where one feed's set has moved, and the repair once feeds are live is
//! a migration rather than an edit.
//!
//! This is deliberately the shape the push side stores in its `FeedAccount`, so
//! that ADR 2's "one verification, two modes" is visible to somebody reading the
//! two programs side by side. The two are not identical and the differences are
//! each a decision rather than drift: this carries the `dataServiceId` RFP-020
//! names in a pull consumer's configuration, and it has no pause flag, because
//! pausing is an aggregator's lever over a feed it publishes to others while a
//! consumer that wants to stop acting simply stops sending settlements. What is
//! the same is everything a verification reads.
//!
//! # What a caller may reach, and what only the authority may
//!
//! Nothing here is reachable from a settlement. [`crate::order::settle`] reads a
//! trust account and never writes one, and the instruction that writes one
//! requires the authority to sign ([`crate::authority`]). That is SEC2's pull
//! half: the set comes from the consumer, and "the consumer" means this
//! program's own governed state rather than whoever built the transaction.
//!
//! # Rotation moves the set and the threshold together
//!
//! [`rotate_signers`] takes both, because a threshold is a statement about a
//! set: shrinking a set below its threshold in two steps would leave a window in
//! which the feed cannot verify, and raising a threshold above a set the same
//! way would leave one in which it silently cannot either. The push path's
//! `update_signer_set` takes both for the same reason.
//!
//! Nothing else about a feed rotates. The pair, the scale and the window are
//! fixed for as long as a registration lives, because open orders were priced
//! against them — an order is a claim about a price of *this* pair on *this*
//! scale, and moving either under it would change what the order means rather
//! than what it trusts.
//!
//! [`deregister`] is the one way they move, and it is why an order records them:
//! a registration can be retired and made again on other terms, so an order that
//! carried only its feed id would be settled under whatever the current
//! registration says. `crate::order::settle` refuses that.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use pull_lib::{AssetPair, ConfigError, FeedConfig, PullConfig, SignerAddress};
use serde::{Deserialize, Serialize};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

use crate::authority::{authorise, AuthorityError};

/// The name seed a feed's trust account is derived from.
pub const TRUST_ACCOUNT_SEED: &str = "KANON_PULL_TRUST";

/// What this consumer trusts for one feed.
///
/// Signer addresses are raw bytes rather than [`SignerAddress`], because that
/// type belongs to a `no_std` crate that carries no serialisation.
/// [`Self::config`] converts.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct FeedTrust {
    /// The RedStone data service the roster below belongs to.
    ///
    /// Carried because RFP-020 names it in the configuration a pull consumer
    /// holds, and read by nothing in verification: no payload carries a data
    /// service, and the one envelope slot that could sits outside every
    /// signature. The signers *are* the binding; this is the consumer's own
    /// record of where they came from.
    pub data_service_id: String,
    /// The RedStone feed id, right-padded to the wire width.
    pub feed_id: [u8; 32],
    /// The base asset of the pair this feed prices.
    pub base_asset: [u8; 32],
    /// The quote asset of the pair this feed prices.
    pub quote_asset: [u8; 32],
    /// The power of ten this feed's signers scale values by.
    pub decimals: u8,
    /// How old the oldest package behind a price may be.
    pub max_age_ms: u64,
    /// The authorised signers, in registration order.
    ///
    /// The width is a literal because the IDL generator reads this file as text
    /// and cannot evaluate a path constant. The assertion below keeps it tied to
    /// the type it mirrors.
    pub signers: Vec<[u8; 20]>,
    /// How many distinct authorised signers a price needs.
    pub threshold: u8,
}

/// The literal width above is the one `verifier-core` uses.
const _: () = assert!(SignerAddress::LEN == 20);

impl FeedTrust {
    /// The stored signer set, in the form `verifier-core` takes.
    #[must_use]
    pub fn signer_addresses(&self) -> Vec<SignerAddress> {
        self.signers.iter().copied().map(SignerAddress).collect()
    }

    /// The pair this feed prices.
    #[must_use]
    pub fn pair(&self) -> AssetPair {
        AssetPair::new(self.base_asset, self.quote_asset)
    }

    /// The configuration `pull-lib` verifies against.
    ///
    /// Built through [`FeedConfig::try_new`], which is `verifier-core`'s own
    /// validation and the same function the push path reaches when it decodes a
    /// stored feed. A second set of rules here would be a second answer to the
    /// same question.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the stored parameters do not describe a usable feed.
    pub fn config<'a>(
        &'a self,
        signers: &'a [SignerAddress],
    ) -> Result<PullConfig<'a>, ConfigError> {
        Ok(PullConfig {
            data_service_id: &self.data_service_id,
            feed: FeedConfig::try_new(
                &self.feed_id,
                self.pair(),
                self.decimals,
                self.max_age_ms,
                signers,
                self.threshold,
            )?,
        })
    }
}

/// Why registering or rotating a feed's trust was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustError {
    /// The authority gate refused before anything else ran.
    Authority(AuthorityError),
    /// This feed's trust is already registered at its derived address.
    AlreadyRegistered,
    /// The account at this feed's address is not one this program can write.
    ///
    /// Non-default and unowned. A trust account's address derives from a feed id
    /// anybody can guess, so anybody can put an account there by sending it one
    /// unit of balance, and no instruction of any program can then move it.
    /// Refused here rather than at `validate_execution`, which reports against
    /// the post-state instead of the input.
    TrustAccountUnusable,
    /// No trust is registered for this feed.
    NotRegistered,
    /// This feed's trust was registered and then retired.
    ///
    /// Its own cause because an operator needs "the authority retired this" and
    /// not "the bytes were unreadable", and the two go to different people. It is
    /// also the state a re-registration writes over, which is why an emptied
    /// account is not a fault in [`register`].
    Deregistered,
    /// The trust account's bytes are not a trust.
    TrustUndecodable,
    /// The account addressed is not the feed the instruction named.
    ///
    /// A rotation names its feed as well as addressing it (ADR 33), so an
    /// operator rotating five feeds cannot move the wrong one by transposing two
    /// accounts.
    FeedMismatch,
    /// The parameters do not describe a usable feed.
    ///
    /// Reported through [`ConfigError`] rather than restated, so one cause has
    /// one meaning wherever it surfaces. Registering an unusable configuration
    /// is refused here rather than discovered by every later settlement.
    Config(ConfigError),
    /// The serialised trust does not fit an account's data.
    TrustTooLarge,
}

impl From<AuthorityError> for TrustError {
    fn from(err: AuthorityError) -> Self {
        Self::Authority(err)
    }
}

impl From<ConfigError> for TrustError {
    fn from(err: ConfigError) -> Self {
        Self::Config(err)
    }
}

impl TrustError {
    /// A stable number per leaf cause: this module's own in the 2000 block, and
    /// the layers beneath it in theirs.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::AlreadyRegistered => 2001,
            Self::TrustAccountUnusable => 2002,
            Self::NotRegistered => 2003,
            Self::Deregistered => 2007,
            Self::TrustUndecodable => 2004,
            Self::FeedMismatch => 2005,
            Self::TrustTooLarge => 2006,
            Self::Authority(err) => err.code(),
            Self::Config(err) => crate::config_code(*err),
        }
    }
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRegistered => f.write_str("this feed's trust is already registered"),
            Self::TrustAccountUnusable => {
                f.write_str("the account at this feed's address is not one this program can write")
            }
            Self::NotRegistered => f.write_str("no trust is registered for this feed"),
            Self::Deregistered => f.write_str("this feed's trust was retired"),
            Self::TrustUndecodable => f.write_str("the trust account's bytes are not a trust"),
            Self::FeedMismatch => {
                f.write_str("that account is not the feed this instruction named")
            }
            Self::TrustTooLarge => f.write_str("the serialised trust does not fit the account"),
            Self::Authority(err) => write!(f, "{err}"),
            Self::Config(err) => write!(f, "{err:?}"),
        }
    }
}

impl From<TrustError> for spel_framework::error::SpelError {
    fn from(err: TrustError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Registers what this consumer trusts for one feed.
///
/// # Errors
///
/// [`TrustError`] when the signer is not the authority, the address already
/// holds something, or the parameters do not describe a usable feed.
#[expect(
    clippy::too_many_arguments,
    reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
)]
pub fn register(
    trust: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    data_service_id: String,
    feed_id: [u8; 32],
    base_asset: [u8; 32],
    quote_asset: [u8; 32],
    decimals: u8,
    max_age_ms: u64,
    signers: Vec<[u8; 20]>,
    threshold: u8,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, TrustError> {
    authorise(&config, &authority, self_program_id)?;

    // Three states, not two, and the third is what makes a mis-registration
    // recoverable. LEZ rule 4 forbids this program giving up ownership and rule 3
    // forbids resetting the nonce, so a retired account stays ours for ever and
    // cannot be handed back as `Account::default()`. A registration therefore has
    // to accept an emptied account of our own, or retiring `BTC` once would spend
    // the id for the life of the build.
    let first = trust.account == Account::default();
    let retired = !first
        && trust.account.program_owner == self_program_id
        && trust.account.data.as_ref().is_empty();
    if !first && !retired {
        return Err(if trust.account.program_owner == self_program_id {
            TrustError::AlreadyRegistered
        } else {
            TrustError::TrustAccountUnusable
        });
    }

    let stored = FeedTrust {
        data_service_id,
        feed_id,
        base_asset,
        quote_asset,
        decimals,
        max_age_ms,
        signers,
        threshold,
    };
    // Constructed and dropped: what is wanted is the refusal. An unusable
    // configuration stored here would answer every later settlement with the
    // same `ConfigError`, at a point where nobody can act on it.
    stored.config(&stored.signer_addresses())?;

    let mut account = trust.account;
    account.data = serialise(&stored)?;

    // From the same discriminator the check used, and not from the account's
    // owner. A re-registration must not claim: LEZ refuses a claim on an account
    // whose owner is not the default one, so claiming again would fail the
    // transaction rather than be ignored.
    let trust_post = if first {
        AutoClaim::pda_from_seeds(&[&feed_id, &seed_from_str(TRUST_ACCOUNT_SEED)])
            .to_post_state(account)
    } else {
        AccountPostState::new(account)
    };

    Ok(vec![
        trust_post,
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Retires what this consumer trusts for one feed.
///
/// The recovery path for a registration that was wrong. Only the roster and its
/// threshold rotate, because the pair, the scale and the window are what open
/// orders were priced against — so a mistyped pair or exponent has no in-place
/// correction, and without this it would spend the feed id for the life of the
/// build.
///
/// What it costs is stated rather than hedged: **orders against a retired feed
/// cannot settle.** [`crate::order::settle`] answers [`TrustError::Deregistered`]
/// until the authority registers the feed again, and if it comes back under a
/// different pair those orders answer `AssetMismatch` for ever, because an order
/// carries the pair its owner signed for. That is the intended outcome — an order
/// priced against a pair this feed no longer claims is an order whose meaning
/// changed — and it is why retiring is the authority's decision and not a
/// caller's.
///
/// The data is emptied and the ownership is not released, because LEZ does not
/// allow the second. [`register`] recognises the result as its third pre-state.
///
/// # Errors
///
/// [`TrustError`] when the signer is not the authority, or the account is not the
/// named feed's registration.
pub fn deregister(
    trust: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, TrustError> {
    authorise(&config, &authority, self_program_id)?;

    let stored = read(&trust, self_program_id)?;
    if stored.feed_id != feed_id {
        return Err(TrustError::FeedMismatch);
    }

    let mut account = trust.account;
    account.data = Data::default();

    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Replaces one feed's signer set and threshold, together.
///
/// # Errors
///
/// [`TrustError`] when the signer is not the authority, the account is not the
/// named feed's trust, or the new set and threshold are not usable together.
pub fn rotate_signers(
    trust: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    signers: Vec<[u8; 20]>,
    threshold: u8,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, TrustError> {
    authorise(&config, &authority, self_program_id)?;

    let mut stored = read(&trust, self_program_id)?;
    if stored.feed_id != feed_id {
        return Err(TrustError::FeedMismatch);
    }

    stored.signers = signers;
    stored.threshold = threshold;
    // Before the write, so a rotation that would leave the feed unverifiable --
    // a threshold above the new set, a duplicate address, an empty roster -- is
    // refused while the old set is still in place.
    stored.config(&stored.signer_addresses())?;

    let mut account = trust.account;
    account.data = serialise(&stored)?;

    // No claim: this program already owns the account.
    Ok(vec![
        AccountPostState::new(account),
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Reads a registered trust account this program owns.
///
/// # Errors
///
/// [`TrustError`] when the account is not this program's or does not decode.
pub fn read(
    trust: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<FeedTrust, TrustError> {
    // Before the data is read: a configuration in an account this program does
    // not own is a signer set of the caller's choosing.
    if trust.account.program_owner != self_program_id {
        return Err(TrustError::NotRegistered);
    }
    // Before the decode, so a retirement reads as one rather than as corruption.
    if trust.account.data.as_ref().is_empty() {
        return Err(TrustError::Deregistered);
    }
    FeedTrust::try_from_slice(trust.account.data.as_ref()).map_err(|_| TrustError::TrustUndecodable)
}

fn serialise(stored: &FeedTrust) -> Result<Data, TrustError> {
    Data::try_from(borsh::to_vec(stored).map_err(|_| TrustError::TrustTooLarge)?)
        .map_err(|_| TrustError::TrustTooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{as_chain_leaves_it, established_config, key, untouched, GENESIS, OURS};
    use lee_core::account::AccountId;
    use lee_core::program::validate_execution;
    use spel_framework::pda::compute_pda;

    const BTC: [u8; 32] = crate::padded(b"BTC");
    const SERVICE: &str = "redstone-primary-prod";

    fn five() -> Vec<[u8; 20]> {
        (1..=5u8).map(|i| [i; 20]).collect()
    }

    fn trust_address(feed_id: &[u8; 32]) -> AccountId {
        compute_pda(&OURS, &[feed_id, &seed_from_str(TRUST_ACCOUNT_SEED)])
    }

    fn register_btc(signers: Vec<[u8; 20]>, threshold: u8) -> Vec<AccountPostState> {
        register(
            untouched(trust_address(&BTC)),
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            signers,
            threshold,
            OURS,
        )
        .expect("a usable registration")
    }

    fn registered() -> AccountWithMetadata {
        as_chain_leaves_it(&register_btc(five(), 3)[0], trust_address(&BTC))
    }

    #[test]
    fn a_registration_stores_the_whole_configuration_and_claims_the_address() {
        let stored = read(&registered(), OURS).expect("decodes");
        assert_eq!(stored.data_service_id, SERVICE);
        assert_eq!(stored.feed_id, BTC);
        assert_eq!(
            stored.pair(),
            AssetPair::new(crate::padded(b"BTC"), crate::padded(b"USD"))
        );
        assert_eq!(stored.decimals, 8);
        assert_eq!(stored.max_age_ms, 60_000);
        assert_eq!(stored.signers, five());
        assert_eq!(stored.threshold, 3);
    }

    #[test]
    fn a_registration_passes_lez() {
        let pre = vec![
            untouched(trust_address(&BTC)),
            key(GENESIS, true),
            established_config(),
        ];
        validate_execution(&pre, &register_btc(five(), 3), OURS).expect("LEZ accepts it");
    }

    #[test]
    fn only_the_authority_can_register_a_feeds_trust() {
        // The whole point of the module. Without the gate anybody could register
        // a feed with a roster of their own, and every order against it would
        // verify payloads they signed themselves.
        let outcome = register(
            untouched(trust_address(&BTC)),
            key([0xAA; 32], true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            five(),
            3,
            OURS,
        );
        assert_eq!(
            outcome,
            Err(TrustError::Authority(AuthorityError::Unauthorised))
        );
    }

    #[test]
    fn a_second_registration_of_the_same_feed_is_refused() {
        let outcome = register(
            registered(),
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            five(),
            3,
            OURS,
        );
        assert_eq!(outcome, Err(TrustError::AlreadyRegistered));
    }

    #[test]
    fn a_squatted_trust_address_is_refused_as_unusable_and_not_as_registered() {
        let mut squatted = untouched(trust_address(&BTC));
        squatted.account.balance = 1;
        let outcome = register(
            squatted,
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            five(),
            3,
            OURS,
        );
        assert_eq!(outcome, Err(TrustError::TrustAccountUnusable));
    }

    #[test]
    fn an_unusable_configuration_is_refused_at_the_registration() {
        // Rather than stored and rediscovered by every settlement, where nobody
        // can act on it. The rules are `verifier-core`'s, reached through the
        // same `FeedConfig::try_new` the push path uses.
        let outcome = register(
            untouched(trust_address(&BTC)),
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            five(),
            6,
            OURS,
        );
        assert_eq!(
            outcome,
            Err(TrustError::Config(ConfigError::ThresholdExceedsSigners {
                threshold: 6,
                signers: 5
            }))
        );
    }

    #[test]
    fn a_rotation_replaces_the_set_and_the_threshold_together() {
        // The instruction RedStone's rotation schedule is the reason for, and the
        // reason a compiled roster was the wrong answer: this is a transaction
        // rather than a redeployment.
        let posts = rotate_signers(
            registered(),
            key(GENESIS, true),
            established_config(),
            BTC,
            vec![[9u8; 20], [8u8; 20], [7u8; 20]],
            2,
            OURS,
        )
        .expect("a usable rotation");

        let rotated =
            read(&as_chain_leaves_it(&posts[0], trust_address(&BTC)), OURS).expect("decodes");
        assert_eq!(rotated.signers, vec![[9u8; 20], [8u8; 20], [7u8; 20]]);
        assert_eq!(rotated.threshold, 2);
    }

    #[test]
    fn a_rotation_changes_nothing_else_about_a_feed() {
        // The pair, the scale and the window are what open orders were priced
        // against. Moving either under an order would change what the order
        // means rather than what it trusts.
        let before = read(&registered(), OURS).expect("decodes");
        let posts = rotate_signers(
            registered(),
            key(GENESIS, true),
            established_config(),
            BTC,
            vec![[9u8; 20], [8u8; 20], [7u8; 20]],
            2,
            OURS,
        )
        .expect("ok");
        let after = read(&as_chain_leaves_it(&posts[0], trust_address(&BTC)), OURS).expect("ok");

        assert_eq!(after.data_service_id, before.data_service_id);
        assert_eq!(after.feed_id, before.feed_id);
        assert_eq!(after.pair(), before.pair());
        assert_eq!(after.decimals, before.decimals);
        assert_eq!(after.max_age_ms, before.max_age_ms);
    }

    #[test]
    fn only_the_authority_can_rotate() {
        assert_eq!(
            rotate_signers(
                registered(),
                key([0xAA; 32], true),
                established_config(),
                BTC,
                five(),
                3,
                OURS
            ),
            Err(TrustError::Authority(AuthorityError::Unauthorised))
        );
    }

    #[test]
    fn a_rotation_names_the_feed_it_moves() {
        // ADR 33's rule reaching the consumer: an operator rotating five feeds
        // cannot move the wrong one by transposing two accounts, because the
        // account and the argument have to agree.
        assert_eq!(
            rotate_signers(
                registered(),
                key(GENESIS, true),
                established_config(),
                crate::padded(b"ETH"),
                five(),
                3,
                OURS
            ),
            Err(TrustError::FeedMismatch)
        );
    }

    #[test]
    fn a_rotation_that_would_leave_a_feed_unverifiable_is_refused() {
        // Checked before the write, so the old set is still in place when it
        // fails. A threshold above the new set would otherwise be stored and then
        // answer every settlement with the same `ConfigError`.
        assert_eq!(
            rotate_signers(
                registered(),
                key(GENESIS, true),
                established_config(),
                BTC,
                vec![[9u8; 20], [8u8; 20]],
                3,
                OURS
            ),
            Err(TrustError::Config(ConfigError::ThresholdExceedsSigners {
                threshold: 3,
                signers: 2
            }))
        );
        assert_eq!(
            rotate_signers(
                registered(),
                key(GENESIS, true),
                established_config(),
                BTC,
                vec![[9u8; 20], [9u8; 20], [7u8; 20]],
                2,
                OURS
            ),
            Err(TrustError::Config(ConfigError::DuplicateSigner))
        );
    }

    /// A feed registered and then retired: ours, empty, at its derived address.
    fn retired() -> AccountWithMetadata {
        let posts = deregister(
            registered(),
            key(GENESIS, true),
            established_config(),
            BTC,
            OURS,
        )
        .expect("a registered feed retires");
        as_chain_leaves_it(&posts[0], trust_address(&BTC))
    }

    #[test]
    fn a_feed_id_survives_being_retired() {
        // The round trip is the point. A trust account's address is its feed id's,
        // and LEZ will not let a program hand an account back -- rule 4 forbids
        // giving up ownership, rule 3 forbids resetting the nonce -- so if a
        // retired account could not hold a new registration, one wrong pair would
        // spend the id for the life of the build.
        let emptied = retired();
        assert!(emptied.account.data.as_ref().is_empty());
        assert_eq!(read(&emptied, OURS), Err(TrustError::Deregistered));

        let pre = vec![emptied.clone(), key(GENESIS, true), established_config()];
        let again = register(
            emptied,
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            // A different pair, which is the whole reason to retire: only the
            // roster and the threshold rotate, so a mistyped pair has no in-place
            // correction.
            crate::padded(b"XBT"),
            crate::padded(b"EUR"),
            6,
            30_000,
            vec![[9u8; 20], [8u8; 20], [7u8; 20]],
            2,
            OURS,
        )
        .expect("the id is not spent");
        validate_execution(&pre, &again, OURS).expect("LEZ accepts the re-registration");
        assert!(
            again[0].required_claim().is_none(),
            "a re-registration must not claim an account this program already owns"
        );

        let stored = read(&as_chain_leaves_it(&again[0], trust_address(&BTC)), OURS).expect("ok");
        assert_eq!(
            stored.pair(),
            AssetPair::new(crate::padded(b"XBT"), crate::padded(b"EUR"))
        );
        assert_eq!(stored.decimals, 6);
        assert_eq!(stored.max_age_ms, 30_000);
    }

    #[test]
    fn a_deregistration_passes_lez() {
        let pre = vec![registered(), key(GENESIS, true), established_config()];
        let posts = deregister(
            registered(),
            key(GENESIS, true),
            established_config(),
            BTC,
            OURS,
        )
        .expect("ok");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts the retirement");
    }

    #[test]
    fn only_the_authority_can_deregister() {
        assert_eq!(
            deregister(
                registered(),
                key([0xAA; 32], true),
                established_config(),
                BTC,
                OURS
            ),
            Err(TrustError::Authority(AuthorityError::Unauthorised))
        );
    }

    #[test]
    fn a_deregistration_names_the_feed_it_retires() {
        assert_eq!(
            deregister(
                registered(),
                key(GENESIS, true),
                established_config(),
                crate::padded(b"ETH"),
                OURS
            ),
            Err(TrustError::FeedMismatch)
        );
    }

    #[test]
    fn a_retired_feed_cannot_be_retired_twice() {
        assert_eq!(
            deregister(
                retired(),
                key(GENESIS, true),
                established_config(),
                BTC,
                OURS
            ),
            Err(TrustError::Deregistered)
        );
    }

    #[test]
    fn a_registered_feed_is_still_refused_a_registration() {
        // The third state is not a licence to overwrite: only an emptied account
        // is re-registerable, and a live one is `AlreadyRegistered`.
        let outcome = register(
            registered(),
            key(GENESIS, true),
            established_config(),
            SERVICE.to_owned(),
            BTC,
            crate::padded(b"BTC"),
            crate::padded(b"USD"),
            8,
            60_000,
            five(),
            3,
            OURS,
        );
        assert_eq!(outcome, Err(TrustError::AlreadyRegistered));
    }

    #[test]
    fn a_trust_account_this_program_does_not_own_is_not_a_registration() {
        // Before the data is read: an account a caller owns whose bytes decode as
        // a trust is a signer set of the caller's choosing.
        let mut foreign = registered();
        foreign.account.program_owner = crate::testing::SOMEONE_ELSE;
        assert_eq!(read(&foreign, OURS), Err(TrustError::NotRegistered));
        assert_eq!(
            read(&untouched(trust_address(&BTC)), OURS),
            Err(TrustError::NotRegistered)
        );
    }
}
