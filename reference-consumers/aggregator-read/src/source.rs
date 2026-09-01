//! Which price account this consumer believes, per feed.
//!
//! One account per feed, at an address this program derives, written only behind
//! the [`crate::authority`] gate. It holds the four things a consumer cannot
//! learn from the price account itself.
//!
//! # Why the aggregator's id is stored rather than compiled
//!
//! Nothing in the canonical price account says who wrote it. `source_id` is the
//! constant `"RedStone"` on every Kanon feed, so it names where the numbers came
//! from and not which program published them. The only thing that binds a price
//! account to a particular aggregator is its *address*, which hashes the
//! aggregator's program id — its RISC0 image id.
//!
//! That id moves whenever the aggregator's build inputs move. Storing it is what
//! lets [`update_aggregator`] follow the next aggregator build in one
//! transaction, instead of a rebuild of this consumer that would move every
//! account this program owns. `[M3-05:01]` records the decision.
//!
//! # Why the pair is stored, when the price account carries one
//!
//! Because the account's pair is the aggregator's claim and the point of the
//! check is to compare it against somebody else's. A registration records what
//! *this consumer's authority* believes the feed prices; [`crate::read`] compares
//! that against what the account says, and an order compares its owner's
//! expectation against the registration. Reading the account's pair and checking
//! it against itself would satisfy the words of U7 and verify nothing — the
//! mistake reference consumer B made and fixed, recorded in `[M3-06:02]`.
//!
//! # Why the window is stored, when the aggregator has one too
//!
//! They bound different things. The aggregator's `max_age_ms` bounds how old a
//! *package* may be when it is verified; this one bounds how old the *published
//! price* may be when it is read, which keeps running after the write. RFP-020
//! asks a consumer to reject anything older than its own `maxAge`, and a
//! consumer's is legitimately wider than the verifier's for that reason.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId, DEFAULT_PROGRAM_ID};
use serde::{Deserialize, Serialize};
use spel_framework::pda::{compute_pda, seed_from_str};
use spel_framework::spel_output::AutoClaim;
use spel_framework_macros::account_type;

use crate::authority::{authorise, AuthorityError};

/// The name seed a feed's source account is derived from.
pub const SOURCE_ACCOUNT_SEED: &str = "KANON_READ_SOURCE";

/// The widest staleness window an authority may register, in milliseconds.
///
/// A day. The bound exists because the window is subtracted from the clock, and
/// a `max_age_ms` near `u64::MAX` puts the lower edge at zero and accepts any
/// timestamp at all — a configuration field that silently disables the check it
/// configures. Reference consumer B found the same hole in its own window.
///
/// The ceiling is wider than `verifier_core::MAX_MAX_AGE_MS` on purpose, because
/// a published price starts ageing once it is written and a bound below the
/// verifier's would stop an authority registering a window the aggregator's own
/// freshness rules already allow. It bounds what may be registered and nothing
/// more: anything from a millisecond up is accepted, and how wide a given feed's
/// window is remains the authority's policy.
pub const MAX_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

/// What this consumer's authority says about one feed.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct PriceSource {
    /// The RedStone feed id this source is for, zero-padded to 32 bytes.
    ///
    /// Stored as well as being the address's seed, so a registration can be
    /// checked against the address it was written to.
    pub feed_id: [u8; 32],
    /// The aggregator program whose price account is believed for this feed.
    ///
    /// A `ProgramId` rather than 32 bytes, because that is what the address
    /// derivation hashes and converting between the two would put an endianness
    /// question between a client and the account it is trying to find.
    pub aggregator: ProgramId,
    /// What the authority says this feed prices.
    pub base_asset: [u8; 32],
    /// The other half of the pair.
    pub quote_asset: [u8; 32],
    /// How stale a published price may be before this consumer refuses it.
    pub max_age_ms: u64,
}

/// The address a feed's source account lives at.
#[must_use]
pub fn source_address(self_program_id: &ProgramId, feed_id: &[u8; 32]) -> AccountId {
    compute_pda(
        self_program_id,
        &[feed_id, &seed_from_str(SOURCE_ACCOUNT_SEED)],
    )
}

/// Why an instruction over a feed's price source was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    /// The gate in front of every write refused.
    Authority(AuthorityError),
    /// The account offered as a source is not one this program owns.
    NotOurs,
    /// The account's bytes are not a source.
    Undecodable,
    /// The account named is not the one the feed derives.
    ///
    /// The check a retired account makes necessary: retirement empties the data,
    /// so an empty account remembers no feed id and cannot be asked which feed
    /// it is for. Without comparing the address, a registration could write one
    /// feed's source into another feed's account.
    FeedMismatch,
    /// A source is already registered for this feed.
    AlreadyRegistered,
    /// The address holds an account no program can write. See
    /// [`AuthorityError::ConfigAccountUnusable`], which is the same LEZ rule pair.
    AccountUnusable,
    /// No source is registered for this feed.
    NotRegistered,
    /// The feed id is all zeroes, which names no feed.
    FeedIsZero,
    /// The aggregator id is the default one, which is no program.
    AggregatorIsZero,
    /// The staleness window is zero, so nothing could ever be fresh enough.
    WindowIsZero,
    /// The staleness window is wider than [`MAX_MAX_AGE_MS`].
    WindowTooWide,
}

impl SourceError {
    /// A stable number per leaf cause, in this program's 2000 block.
    ///
    /// [`Self::Authority`] dispatches into the 1200 block rather than reporting
    /// the whole gate as one number: a wrapper is not a cause.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::Authority(err) => err.code(),
            Self::NotOurs => 2001,
            Self::Undecodable => 2002,
            Self::FeedMismatch => 2003,
            Self::AlreadyRegistered => 2004,
            Self::AccountUnusable => 2005,
            Self::NotRegistered => 2006,
            Self::FeedIsZero => 2007,
            Self::AggregatorIsZero => 2008,
            Self::WindowIsZero => 2009,
            Self::WindowTooWide => 2010,
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority(err) => write!(f, "{err}"),
            Self::NotOurs => f.write_str("the account offered as a price source is not ours"),
            Self::Undecodable => f.write_str("the account's bytes are not a price source"),
            Self::FeedMismatch => f.write_str("that account is not the one the named feed derives"),
            Self::AlreadyRegistered => f.write_str("this feed already has a price source"),
            Self::AccountUnusable => {
                f.write_str("the address holds an account no program can write")
            }
            Self::NotRegistered => f.write_str("this feed has no price source"),
            Self::FeedIsZero => f.write_str("the zero feed id names no feed"),
            Self::AggregatorIsZero => f.write_str("the default program id is no aggregator"),
            Self::WindowIsZero => f.write_str("a zero staleness window admits nothing"),
            Self::WindowTooWide => f.write_str("that staleness window is wider than a day"),
        }
    }
}

impl From<AuthorityError> for SourceError {
    fn from(err: AuthorityError) -> Self {
        Self::Authority(err)
    }
}

impl From<SourceError> for spel_framework::error::SpelError {
    fn from(err: SourceError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

fn check(feed_id: &[u8; 32], aggregator: &ProgramId, max_age_ms: u64) -> Result<(), SourceError> {
    if *feed_id == [0u8; 32] {
        return Err(SourceError::FeedIsZero);
    }
    if *aggregator == DEFAULT_PROGRAM_ID {
        return Err(SourceError::AggregatorIsZero);
    }
    if max_age_ms == 0 {
        return Err(SourceError::WindowIsZero);
    }
    if max_age_ms > MAX_MAX_AGE_MS {
        return Err(SourceError::WindowTooWide);
    }
    Ok(())
}

/// Registers the price account this consumer will believe for a feed.
///
/// # Errors
///
/// [`SourceError`] when the gate refuses, the account is not the one the feed
/// derives, a source is already registered, or a parameter is unusable.
#[allow(clippy::too_many_arguments)]
pub fn register(
    source: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    aggregator: ProgramId,
    base_asset: [u8; 32],
    quote_asset: [u8; 32],
    max_age_ms: u64,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SourceError> {
    authorise(&config, &authority, self_program_id)?;
    check(&feed_id, &aggregator, max_age_ms)?;

    // Three pre-states, not two. A retired source is an account this program
    // owns whose data is empty, and registering into it again is what stops one
    // wrong registration spending a feed id for ever -- LEZ forbids handing an
    // account back, so retirement cannot release the address.
    let first = source.account == Account::default();
    let retired = !first
        && source.account.program_owner == self_program_id
        && source.account.data.as_ref().is_empty();
    if !first && !retired {
        return Err(if source.account.program_owner == self_program_id {
            SourceError::AlreadyRegistered
        } else {
            SourceError::AccountUnusable
        });
    }

    // The address is what ties this registration to the feed it names. A retired
    // account carries no feed id to compare against, so the comparison has to be
    // against the address the named feed derives.
    if source.account_id != source_address(&self_program_id, &feed_id) {
        return Err(SourceError::FeedMismatch);
    }

    let registered = PriceSource {
        feed_id,
        aggregator,
        base_asset,
        quote_asset,
        max_age_ms,
    };
    Ok(vec![
        write_source(source.account, first, &feed_id, Some(&registered))?,
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Points a feed at another aggregator build, leaving everything else alone.
///
/// The operation the whole design exists for: an aggregator rebuild moves every
/// price account it writes, and this is how a consumer follows without moving
/// any account of its own. The pair and the window do not change, because
/// following a rebuild is not a change of what the feed means.
///
/// # Errors
///
/// [`SourceError`] when the gate refuses, no source is registered, or the new
/// aggregator id is the zero one.
pub fn update_aggregator(
    source: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    aggregator: ProgramId,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SourceError> {
    authorise(&config, &authority, self_program_id)?;
    let stored = read(&source, self_program_id)?;
    if aggregator == DEFAULT_PROGRAM_ID {
        return Err(SourceError::AggregatorIsZero);
    }
    // Named as well as addressed (ADR 33), so an operator following one rebuild
    // across five feeds cannot move the wrong one.
    if stored.feed_id != feed_id {
        return Err(SourceError::FeedMismatch);
    }

    let updated = PriceSource {
        aggregator,
        ..stored
    };
    Ok(vec![
        write_source(source.account, false, &feed_id, Some(&updated))?,
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Retires a feed's price source, leaving the account empty and still ours.
///
/// # Errors
///
/// [`SourceError`] when the gate refuses or no source is registered.
pub fn deregister(
    source: AccountWithMetadata,
    authority: AccountWithMetadata,
    config: AccountWithMetadata,
    feed_id: [u8; 32],
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, SourceError> {
    authorise(&config, &authority, self_program_id)?;
    let stored = read(&source, self_program_id)?;
    if stored.feed_id != feed_id {
        return Err(SourceError::FeedMismatch);
    }

    Ok(vec![
        write_source(source.account, false, &feed_id, None)?,
        AccountPostState::new(authority.account),
        AccountPostState::new(config.account),
    ])
}

/// Reads a registered source out of the account offered as one.
///
/// # Errors
///
/// [`SourceError::NotOurs`] for an account this program does not own,
/// [`SourceError::NotRegistered`] for a retired one, and
/// [`SourceError::Undecodable`] for bytes that are not a source.
pub fn read(
    source: &AccountWithMetadata,
    self_program_id: ProgramId,
) -> Result<PriceSource, SourceError> {
    if source.account.program_owner != self_program_id {
        return Err(SourceError::NotOurs);
    }
    // Before the decode, because an empty account is a retirement rather than a
    // damaged one, and an operator's next move differs.
    if source.account.data.as_ref().is_empty() {
        return Err(SourceError::NotRegistered);
    }
    PriceSource::try_from_slice(source.account.data.as_ref()).map_err(|_| SourceError::Undecodable)
}

fn write_source(
    account: Account,
    first: bool,
    feed_id: &[u8; 32],
    stored: Option<&PriceSource>,
) -> Result<AccountPostState, SourceError> {
    let bytes = match stored {
        Some(source) => borsh::to_vec(source).map_err(|_| SourceError::Undecodable)?,
        None => Vec::new(),
    };
    let mut account = account;
    account.data = Data::try_from(bytes).map_err(|_| SourceError::Undecodable)?;

    Ok(if first {
        AutoClaim::pda_from_seeds(&[feed_id, &seed_from_str(SOURCE_ACCOUNT_SEED)])
            .to_post_state(account)
    } else {
        AccountPostState::new(account)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::{config_address, ConfigAccount};
    use lee_core::account::Nonce;
    use lee_core::program::{validate_execution, Claim};

    const OURS: ProgramId = [7u32; 8];
    const WALLET: ProgramId = [42u32; 8];
    const AGGREGATOR: ProgramId = [11u32; 8];
    const REBUILT: ProgramId = [12u32; 8];
    const DEFAULT: ProgramId = [0u32; 8];

    const AUTHORITY: [u8; 32] = [0xA1; 32];
    const STRANGER: [u8; 32] = [0x5A; 32];
    const BASE: [u8; 32] = [1u8; 32];
    const QUOTE: [u8; 32] = [2u8; 32];
    const MAX_AGE_MS: u64 = 60_000;

    fn feed_id() -> [u8; 32] {
        let mut id = [0u8; 32];
        id[..3].copy_from_slice(b"BTC");
        id
    }

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

    fn key(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        let mut key = at(AccountId::new(id), WALLET, Vec::new());
        key.account.balance = 500;
        key.account.nonce = Nonce(3);
        key.is_authorized = signs;
        key
    }

    fn config() -> AccountWithMetadata {
        let stored = ConfigAccount {
            authority: AUTHORITY,
            pending: None,
        };
        at(
            config_address(&OURS),
            OURS,
            borsh::to_vec(&stored).expect("serialises"),
        )
    }

    fn empty_source() -> AccountWithMetadata {
        at(source_address(&OURS, &feed_id()), DEFAULT, Vec::new())
    }

    fn registered_source(stored: &PriceSource) -> AccountWithMetadata {
        at(
            source_address(&OURS, &stored.feed_id),
            OURS,
            borsh::to_vec(stored).expect("serialises"),
        )
    }

    fn register_at(
        source: AccountWithMetadata,
        signer: [u8; 32],
        feed: [u8; 32],
        aggregator: ProgramId,
        max_age_ms: u64,
    ) -> Result<Vec<AccountPostState>, SourceError> {
        register(
            source,
            key(signer, true),
            config(),
            feed,
            aggregator,
            BASE,
            QUOTE,
            max_age_ms,
            OURS,
        )
    }

    fn stored_in(posts: &[AccountPostState]) -> PriceSource {
        PriceSource::try_from_slice(posts[0].account().data.as_ref()).expect("a source")
    }

    #[test]
    fn a_registration_stores_what_it_was_given() {
        let posts =
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).expect("ok");

        assert_eq!(
            stored_in(&posts),
            PriceSource {
                feed_id: feed_id(),
                aggregator: AGGREGATOR,
                base_asset: BASE,
                quote_asset: QUOTE,
                max_age_ms: MAX_AGE_MS,
            }
        );
    }

    #[test]
    fn a_registration_returns_one_post_state_per_account_it_was_given() {
        let posts =
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).expect("ok");

        assert_eq!(posts.len(), 3);
    }

    #[test]
    fn a_first_registration_claims_its_address() {
        let posts =
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).expect("ok");

        assert!(posts[0].required_claim().is_some());
        assert!(posts[1].required_claim().is_none());
    }

    #[test]
    fn only_the_authority_can_register_a_price_source() {
        assert_eq!(
            register_at(empty_source(), STRANGER, feed_id(), AGGREGATOR, MAX_AGE_MS).unwrap_err(),
            SourceError::Authority(AuthorityError::Unauthorised)
        );
    }

    /// Without the address comparison a registration could write one feed's
    /// source into another feed's account, leaving the second answering
    /// `AlreadyRegistered` to a registration and `FeedMismatch` to a retirement,
    /// which are the only two ways out.
    #[test]
    fn a_registration_cannot_be_written_into_another_feeds_account() {
        let mut other = feed_id();
        other[0] = b'E';
        let theirs = at(source_address(&OURS, &other), DEFAULT, Vec::new());

        assert_eq!(
            register_at(theirs, AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).unwrap_err(),
            SourceError::FeedMismatch
        );
    }

    #[test]
    fn a_feed_that_already_has_a_source_is_not_registered_twice() {
        let existing = registered_source(&PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        });

        assert_eq!(
            register_at(existing, AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).unwrap_err(),
            SourceError::AlreadyRegistered
        );
    }

    /// LEZ forbids handing an account back, so a retirement cannot release the
    /// address. Admitting the retired pre-state is what stops one wrong
    /// registration spending a feed id for ever.
    #[test]
    fn a_retired_feed_can_be_registered_again() {
        let retired = at(source_address(&OURS, &feed_id()), OURS, Vec::new());

        let posts =
            register_at(retired, AUTHORITY, feed_id(), REBUILT, MAX_AGE_MS).expect("registers");
        assert_eq!(stored_in(&posts).aggregator, REBUILT);
        assert!(
            posts[0].required_claim().is_none(),
            "the address is already this program's, and LEZ refuses a second claim"
        );
    }

    #[test]
    fn an_unusable_address_is_its_own_refusal() {
        let mut squatted = at(source_address(&OURS, &feed_id()), DEFAULT, Vec::new());
        squatted.account.balance = 1;

        assert_eq!(
            register_at(squatted, AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).unwrap_err(),
            SourceError::AccountUnusable
        );
    }

    #[test]
    fn every_unusable_parameter_has_its_own_cause() {
        assert_eq!(
            register_at(empty_source(), AUTHORITY, [0u8; 32], AGGREGATOR, MAX_AGE_MS).unwrap_err(),
            SourceError::FeedIsZero
        );
        assert_eq!(
            register_at(empty_source(), AUTHORITY, feed_id(), DEFAULT, MAX_AGE_MS).unwrap_err(),
            SourceError::AggregatorIsZero
        );
        assert_eq!(
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, 0).unwrap_err(),
            SourceError::WindowIsZero
        );
        assert_eq!(
            register_at(
                empty_source(),
                AUTHORITY,
                feed_id(),
                AGGREGATOR,
                MAX_MAX_AGE_MS + 1
            )
            .unwrap_err(),
            SourceError::WindowTooWide
        );
    }

    #[test]
    fn the_widest_registrable_window_is_registrable() {
        assert!(register_at(
            empty_source(),
            AUTHORITY,
            feed_id(),
            AGGREGATOR,
            MAX_MAX_AGE_MS
        )
        .is_ok());
    }

    /// The operation the design exists for, and the only field it moves.
    #[test]
    fn following_a_rebuild_moves_the_aggregator_and_nothing_else() {
        let before = PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        };

        let posts = update_aggregator(
            registered_source(&before),
            key(AUTHORITY, true),
            config(),
            feed_id(),
            REBUILT,
            OURS,
        )
        .expect("follows");

        assert_eq!(
            stored_in(&posts),
            PriceSource {
                aggregator: REBUILT,
                ..before
            }
        );
    }

    #[test]
    fn only_the_authority_can_follow_a_rebuild() {
        let stored = PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        };

        assert_eq!(
            update_aggregator(
                registered_source(&stored),
                key(STRANGER, true),
                config(),
                feed_id(),
                REBUILT,
                OURS,
            )
            .unwrap_err(),
            SourceError::Authority(AuthorityError::Unauthorised)
        );
    }

    /// Named as well as addressed (ADR 33), so an operator following one rebuild
    /// across five feeds cannot move the wrong one by transposing two accounts.
    #[test]
    fn an_update_names_the_feed_it_moves() {
        let stored = PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        };
        let mut other = feed_id();
        other[0] = b'E';

        assert_eq!(
            update_aggregator(
                registered_source(&stored),
                key(AUTHORITY, true),
                config(),
                other,
                REBUILT,
                OURS,
            )
            .unwrap_err(),
            SourceError::FeedMismatch
        );
    }

    #[test]
    fn a_retirement_empties_the_account_and_keeps_it() {
        let stored = PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        };

        let posts = deregister(
            registered_source(&stored),
            key(AUTHORITY, true),
            config(),
            feed_id(),
            OURS,
        )
        .expect("retires");

        assert!(posts[0].account().data.as_ref().is_empty());
        assert_eq!(posts[0].account().program_owner, OURS);
        assert_eq!(posts.len(), 3);
    }

    #[test]
    fn a_retired_source_reads_as_retired_rather_than_damaged() {
        let retired = at(source_address(&OURS, &feed_id()), OURS, Vec::new());

        assert_eq!(
            read(&retired, OURS).unwrap_err(),
            SourceError::NotRegistered
        );
    }

    #[test]
    fn an_account_this_program_does_not_own_is_not_a_source() {
        let theirs = at(source_address(&OURS, &feed_id()), WALLET, vec![1, 2, 3]);

        assert_eq!(read(&theirs, OURS).unwrap_err(), SourceError::NotOurs);
    }

    #[test]
    fn bytes_that_are_not_a_source_are_reported_as_such() {
        let rubbish = at(source_address(&OURS, &feed_id()), OURS, vec![0xFF; 3]);

        assert_eq!(read(&rubbish, OURS).unwrap_err(), SourceError::Undecodable);
    }

    /// Rather than restating LEZ's rules: hand what each instruction returned to
    /// the function that enforces them, over the pre-states it was really given.
    ///
    /// What it does not cover is the claim. `validate_execution` never reads
    /// `required_claim` -- the claim loop in LEZ's state machine applies it
    /// afterwards -- so the branch in `write_source` needs the two assertions
    /// beside this one, and `the_claimed_seed_derives_the_address_it_was_written_to`
    /// for the half neither of those reaches.
    #[test]
    fn every_write_to_a_source_passes_lez() {
        let stored = PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        };

        let pre = vec![empty_source(), key(AUTHORITY, true), config()];
        let posts =
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).expect("ok");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts a first registration");

        let retired = at(source_address(&OURS, &feed_id()), OURS, Vec::new());
        let pre = vec![retired.clone(), key(AUTHORITY, true), config()];
        let posts = register_at(retired, AUTHORITY, feed_id(), REBUILT, MAX_AGE_MS).expect("ok");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts a re-registration");

        let pre = vec![registered_source(&stored), key(AUTHORITY, true), config()];
        let posts = update_aggregator(
            registered_source(&stored),
            key(AUTHORITY, true),
            config(),
            feed_id(),
            REBUILT,
            OURS,
        )
        .expect("follows");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts a rebuild being followed");

        let pre = vec![registered_source(&stored), key(AUTHORITY, true), config()];
        let posts = deregister(
            registered_source(&stored),
            key(AUTHORITY, true),
            config(),
            feed_id(),
            OURS,
        )
        .expect("retires");
        validate_execution(&pre, &posts, OURS).expect("LEZ accepts a retirement");
    }

    /// The rule `validate_execution` cannot see. LEZ's claim loop refuses a claim
    /// whose seed does not derive the account it is attached to
    /// (`MismatchedPdaClaim`), and a registration that claimed the wrong address
    /// would be refused on chain while passing every assertion above.
    #[test]
    fn the_claimed_seed_derives_the_address_it_was_written_to() {
        let posts =
            register_at(empty_source(), AUTHORITY, feed_id(), AGGREGATOR, MAX_AGE_MS).expect("ok");

        let Some(Claim::Pda(seed)) = posts[0].required_claim() else {
            panic!("a first registration claims its address as a PDA");
        };

        assert_eq!(
            AccountId::for_public_pda(&OURS, &seed),
            source_address(&OURS, &feed_id()),
            "the claimed seed has to derive the address the account is at"
        );
    }

    #[test]
    fn no_two_causes_answer_with_the_same_number() {
        let causes = [
            SourceError::NotOurs,
            SourceError::Undecodable,
            SourceError::FeedMismatch,
            SourceError::AlreadyRegistered,
            SourceError::AccountUnusable,
            SourceError::NotRegistered,
            SourceError::FeedIsZero,
            SourceError::AggregatorIsZero,
            SourceError::WindowIsZero,
            SourceError::WindowTooWide,
        ];

        let mut numbers: Vec<u32> = causes.iter().map(SourceError::code).collect();
        numbers.sort_unstable();
        let unique = numbers.len();
        numbers.dedup();
        assert_eq!(numbers.len(), unique);
    }
}
