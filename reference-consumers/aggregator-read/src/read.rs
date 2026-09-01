//! Reading the canonical price account, and every way that read refuses.
//!
//! This is the whole of what push mode asks of a consumer. There is no payload,
//! no signature and no transaction sent to the aggregator: a price account is an
//! account, and reading one is a read.
//!
//! What a consumer needs to do it is small enough to list, and listing it is the
//! point of this module. Two seed strings, one source constant, the account type
//! from `kanon-idl`, and the aggregator's program id out of the feed's
//! [`crate::source::PriceSource`]. No aggregator crate is linked, because a
//! Logos module in another repository could not link one either.
//!
//! Each of the three constants below is a copy of a decision that lives in the
//! aggregator, and `tests/derivation.rs` fails if a copy and its original ever
//! disagree. Copied rather than shared for the same reason
//! `kanon_idl::PRICE_ACCOUNT_FIELDS` is: the assertion is what has value, and it
//! only has value if the two are written down separately.
//!
//! # The order the checks run in
//!
//! Address, then availability, then owner, then contents, then time. Two of
//! those are load-bearing rather than tidy.
//!
//! **The address comes first** because everything after it is a statement about
//! the wrong account otherwise. A caller hands this program the account it
//! wants read; nothing in the six fields says which feed they are for or who
//! wrote them, so a caller free to choose the account is free to choose the
//! price. [`price_account_address`] is what makes the choice the authority's.
//!
//! **The owner is checked even though the address was**, because an address is
//! not a guarantee about who filled it. Anybody may put one unit of balance on a
//! publicly derivable address, and a price account this aggregator has never
//! written is one whose owner is not the aggregator.
//!
//! # Refuse-on-unavailable
//!
//! Every branch below returns [`ReadError`]. None returns a price with a flag,
//! a zero, or the last value it saw. U7 asks a consumer never to fall back to an
//! unsafe default, and the way to not have a fallback is to not have a return
//! path that could carry one.

use borsh::BorshDeserialize;
use core::fmt;
use kanon_clock::{LezClock, TimeError};
use kanon_idl::OraclePriceAccount;
use lee_core::account::{Account, AccountId, AccountWithMetadata};
use lee_core::program::ProgramId;
use spel_framework::pda::{compute_pda, seed_from_str};

use crate::source::{PriceSource, SourceError};

/// The aggregator's name seed for a feed account (ADR 33).
pub const FEED_ACCOUNT_SEED: &str = "KANON_FEED_ACCOUNT";

/// The aggregator's name seed for a feed's price account (ADR 32).
pub const PRICE_ACCOUNT_SEED: &str = "KANON_PRICE_ACCOUNT";

/// What the aggregator writes into the price account's `source_id`: RedStone,
/// ASCII, right-padded with zeroes.
pub const REDSTONE_SOURCE_ID: AccountId = {
    let mut bytes = [0u8; 32];
    let name = b"RedStone";
    let mut i = 0;
    while i < name.len() {
        bytes[i] = name[i];
        i += 1;
    }
    AccountId::new(bytes)
};

/// How far ahead of the chain clock a published timestamp may sit.
///
/// The same three minutes the verifier allows a package, and for the same
/// reason: a published timestamp is a package's timestamp, so a consumer
/// narrower than the verifier would refuse observations the aggregator was
/// entitled to publish. `tests/derivation.rs` pins the two together.
pub const MAX_AHEAD_MS: u64 = 3 * 60 * 1000;

/// The address a feed's price account lives at, under a given aggregator.
///
/// Two derivations, because the aggregator's own are two: a feed account is
/// derived from the feed id, and a price account from the feed *account's*
/// address. A consumer holding only the feed id therefore computes both.
#[must_use]
pub fn price_account_address(aggregator: &ProgramId, feed_id: &[u8; 32]) -> AccountId {
    let feed = compute_pda(aggregator, &[feed_id, &seed_from_str(FEED_ACCOUNT_SEED)]);
    compute_pda(
        aggregator,
        &[feed.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
    )
}

/// A price this consumer is prepared to act on.
///
/// Only the two fields that move. The pair, the source and the confidence
/// interval have all been checked by the time one of these exists, so carrying
/// them would invite a second check somewhere that did not need one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price {
    /// The price on RFP-019's `Q64.64` scale, as published.
    pub price_q64: u128,
    /// When the observation behind it was taken, not when it was written.
    pub timestamp_ms: u64,
}

/// Why a price account could not be turned into a price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// The feed's price source could not be read.
    Source(SourceError),
    /// The clock account was not the chain's.
    Clock(TimeError),
    /// The account offered is not the one this feed's source derives.
    WrongAccount,
    /// Nothing has ever been published for this feed.
    ///
    /// The pristine account, which is what a registered feed looks like before
    /// its first submission. Distinct from [`Self::NotTheAggregators`] because
    /// an operator's next move differs: wait, or fix the registration.
    Unavailable,
    /// The account exists but the aggregator this feed names does not own it.
    NotTheAggregators,
    /// The account's bytes are not a price account.
    Undecodable,
    /// The account names a source other than RedStone, so it is not one this
    /// adaptor wrote.
    NotRedStone,
    /// The account prices a pair other than the one registered for this feed.
    AssetMismatch,
    /// The published price is older than this consumer's window allows.
    Stale {
        /// When the observation was taken.
        published_ms: u64,
        /// What the chain clock says now.
        now_ms: u64,
        /// The window the authority registered.
        max_age_ms: u64,
    },
    /// The published price is dated further ahead of the clock than skew explains.
    ///
    /// Its own cause rather than [`Self::Stale`]'s: an operator chasing a clock
    /// that disagrees with RedStone needs to be told which direction it is out.
    AheadOfClock {
        /// When the observation claims to have been taken.
        published_ms: u64,
        /// What the chain clock says now.
        now_ms: u64,
    },
}

impl ReadError {
    /// A stable number per leaf cause, in this program's 2100 block.
    ///
    /// [`Self::Source`] and [`Self::Clock`] dispatch into their own layers'
    /// blocks rather than reporting a layer as one number.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::Source(err) => err.code(),
            Self::Clock(err) => crate::clock_code(*err),
            Self::WrongAccount => 2101,
            Self::Unavailable => 2102,
            Self::NotTheAggregators => 2103,
            Self::Undecodable => 2104,
            Self::NotRedStone => 2105,
            Self::AssetMismatch => 2106,
            Self::Stale { .. } => 2107,
            Self::AheadOfClock { .. } => 2108,
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(err) => write!(f, "{err}"),
            Self::Clock(err) => write!(f, "{err:?}"),
            Self::WrongAccount => {
                f.write_str("that is not the price account this feed's source derives")
            }
            Self::Unavailable => f.write_str("no price has ever been published for this feed"),
            Self::NotTheAggregators => {
                f.write_str("the aggregator this feed names does not own that account")
            }
            Self::Undecodable => f.write_str("the account's bytes are not a price account"),
            Self::NotRedStone => f.write_str("that price account names another source"),
            Self::AssetMismatch => f.write_str("that price account prices another pair"),
            Self::Stale {
                published_ms,
                now_ms,
                max_age_ms,
            } => write!(
                f,
                "the published price is from {published_ms} and the window at {now_ms} is {max_age_ms}ms"
            ),
            Self::AheadOfClock {
                published_ms,
                now_ms,
            } => write!(f, "the published price is dated {published_ms}, ahead of {now_ms}"),
        }
    }
}

impl From<SourceError> for ReadError {
    fn from(err: SourceError) -> Self {
        Self::Source(err)
    }
}

impl From<TimeError> for ReadError {
    fn from(err: TimeError) -> Self {
        Self::Clock(err)
    }
}

impl From<ReadError> for spel_framework::error::SpelError {
    fn from(err: ReadError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

/// Turns the account a caller offered into a price, or refuses.
///
/// `source` is what this consumer's authority registered for the feed;
/// `account` is what the transaction supplied. The two are joined by the
/// address, which is the first thing checked.
///
/// # Errors
///
/// [`ReadError`], one variant per cause. There is no branch that returns a
/// price without having checked all of them.
pub fn read_price(
    source: &PriceSource,
    account: &AccountWithMetadata,
    clock: &LezClock,
) -> Result<Price, ReadError> {
    if account.account_id != price_account_address(&source.aggregator, &source.feed_id) {
        return Err(ReadError::WrongAccount);
    }
    if account.account == Account::default() {
        return Err(ReadError::Unavailable);
    }
    // Covers the third state as well as a foreign owner: an account somebody
    // squatted on this address carries the default owner, which is not the
    // aggregator either.
    if account.account.program_owner != source.aggregator {
        return Err(ReadError::NotTheAggregators);
    }

    let published = OraclePriceAccount::try_from_slice(account.account.data.as_ref())
        .map_err(|_| ReadError::Undecodable)?;
    if published.source_id != REDSTONE_SOURCE_ID {
        return Err(ReadError::NotRedStone);
    }
    if *published.base_asset.value() != source.base_asset
        || *published.quote_asset.value() != source.quote_asset
    {
        return Err(ReadError::AssetMismatch);
    }

    let now_ms = clock.timestamp_ms();
    // Saturating on both sides: a chain clock below either bound is possible on
    // a fresh devnet, and a panic in a guest aborts the transaction.
    if published.timestamp < now_ms.saturating_sub(source.max_age_ms) {
        return Err(ReadError::Stale {
            published_ms: published.timestamp,
            now_ms,
            max_age_ms: source.max_age_ms,
        });
    }
    if published.timestamp > now_ms.saturating_add(MAX_AHEAD_MS) {
        return Err(ReadError::AheadOfClock {
            published_ms: published.timestamp,
            now_ms,
        });
    }

    Ok(Price {
        price_q64: published.price,
        timestamp_ms: published.timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanon_clock::CLOCK_ACCOUNT_ID;
    use lee_core::account::{Data, Nonce};

    const AGGREGATOR: ProgramId = [11u32; 8];
    const ANOTHER_AGGREGATOR: ProgramId = [12u32; 8];
    const DEFAULT: ProgramId = [0u32; 8];

    const BASE: [u8; 32] = [1u8; 32];
    const QUOTE: [u8; 32] = [2u8; 32];
    const NOW_MS: u64 = 1_770_000_000_000;
    const MAX_AGE_MS: u64 = 60_000;
    const PRICE_Q64: u128 = 42 << 64;

    fn feed_id() -> [u8; 32] {
        let mut id = [0u8; 32];
        id[..3].copy_from_slice(b"BTC");
        id
    }

    fn registered() -> PriceSource {
        PriceSource {
            feed_id: feed_id(),
            aggregator: AGGREGATOR,
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: MAX_AGE_MS,
        }
    }

    fn published(timestamp: u64) -> OraclePriceAccount {
        OraclePriceAccount {
            base_asset: AccountId::new(BASE),
            quote_asset: AccountId::new(QUOTE),
            price: PRICE_Q64,
            timestamp,
            source_id: REDSTONE_SOURCE_ID,
            confidence_interval: 0,
        }
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

    /// The account the aggregator would really have written, at the address it
    /// would really have written it to.
    fn price_account(stored: &OraclePriceAccount) -> AccountWithMetadata {
        at(
            price_account_address(&AGGREGATOR, &feed_id()),
            AGGREGATOR,
            borsh::to_vec(stored).expect("serialises"),
        )
    }

    fn clock(now_ms: u64) -> LezClock {
        let mut data = [0u8; 16];
        data[8..].copy_from_slice(&now_ms.to_le_bytes());
        LezClock::from_account(&CLOCK_ACCOUNT_ID, &data).expect("the pinned account")
    }

    #[test]
    fn a_published_price_at_its_own_address_is_read() {
        let read = read_price(
            &registered(),
            &price_account(&published(NOW_MS)),
            &clock(NOW_MS),
        )
        .expect("readable");

        assert_eq!(
            read,
            Price {
                price_q64: PRICE_Q64,
                timestamp_ms: NOW_MS
            }
        );
    }

    /// The check everything else rests on. A caller chooses which account it
    /// hands over, so an account that decodes as a price is not the same thing as
    /// this feed's price.
    #[test]
    fn an_account_that_is_not_this_feeds_price_account_is_refused_first() {
        let elsewhere = at(
            AccountId::new([0xAB; 32]),
            AGGREGATOR,
            borsh::to_vec(&published(NOW_MS)).expect("serialises"),
        );

        assert_eq!(
            read_price(&registered(), &elsewhere, &clock(NOW_MS)),
            Err(ReadError::WrongAccount)
        );
    }

    /// A price account written under one aggregator build cannot be read under
    /// another, which is what makes `update_aggregator` necessary rather than
    /// convenient.
    #[test]
    fn a_price_account_from_another_aggregator_build_is_at_another_address() {
        let source = PriceSource {
            aggregator: ANOTHER_AGGREGATOR,
            ..registered()
        };

        assert_eq!(
            read_price(&source, &price_account(&published(NOW_MS)), &clock(NOW_MS)),
            Err(ReadError::WrongAccount)
        );
    }

    #[test]
    fn a_feed_nothing_has_been_published_for_is_unavailable() {
        let pristine = at(
            price_account_address(&AGGREGATOR, &feed_id()),
            DEFAULT,
            Vec::new(),
        );

        assert_eq!(
            read_price(&registered(), &pristine, &clock(NOW_MS)),
            Err(ReadError::Unavailable)
        );
    }

    /// The third state: somebody put a unit of balance on the address before the
    /// aggregator wrote there. It is not pristine and no program owns it, so it
    /// is neither unavailable nor the aggregator's.
    #[test]
    fn an_account_squatting_the_address_is_not_the_aggregators() {
        let mut squatted = at(
            price_account_address(&AGGREGATOR, &feed_id()),
            DEFAULT,
            Vec::new(),
        );
        squatted.account.balance = 1;

        assert_eq!(
            read_price(&registered(), &squatted, &clock(NOW_MS)),
            Err(ReadError::NotTheAggregators)
        );
    }

    #[test]
    fn an_account_another_program_owns_is_not_the_aggregators() {
        let mut theirs = price_account(&published(NOW_MS));
        theirs.account.program_owner = ANOTHER_AGGREGATOR;

        assert_eq!(
            read_price(&registered(), &theirs, &clock(NOW_MS)),
            Err(ReadError::NotTheAggregators)
        );
    }

    #[test]
    fn bytes_that_are_not_a_price_account_are_reported_as_such() {
        let rubbish = at(
            price_account_address(&AGGREGATOR, &feed_id()),
            AGGREGATOR,
            vec![0xFF; 8],
        );

        assert_eq!(
            read_price(&registered(), &rubbish, &clock(NOW_MS)),
            Err(ReadError::Undecodable)
        );
    }

    /// The canonical account is a standard other oracles write too. One at this
    /// address would mean the aggregator's own claim was wrong, and reading it
    /// would be reading somebody else's price.
    #[test]
    fn a_price_account_naming_another_source_is_refused() {
        let mut other = published(NOW_MS);
        other.source_id = AccountId::new([0x5A; 32]);

        assert_eq!(
            read_price(&registered(), &price_account(&other), &clock(NOW_MS)),
            Err(ReadError::NotRedStone)
        );
    }

    /// The check U7 asks for, and it is a real one because the two pairs are
    /// written by different parties: the aggregator wrote the account's, this
    /// consumer's authority wrote the source's.
    #[test]
    fn a_price_account_pricing_another_pair_is_refused() {
        let mut mislabelled = published(NOW_MS);
        mislabelled.quote_asset = AccountId::new([9u8; 32]);

        assert_eq!(
            read_price(&registered(), &price_account(&mislabelled), &clock(NOW_MS)),
            Err(ReadError::AssetMismatch)
        );
    }

    #[test]
    fn the_far_edge_of_the_window_is_inside_it() {
        let edge = NOW_MS - MAX_AGE_MS;

        assert!(read_price(
            &registered(),
            &price_account(&published(edge)),
            &clock(NOW_MS)
        )
        .is_ok());
    }

    #[test]
    fn one_millisecond_past_the_window_is_stale() {
        let past = NOW_MS - MAX_AGE_MS - 1;

        assert_eq!(
            read_price(
                &registered(),
                &price_account(&published(past)),
                &clock(NOW_MS)
            ),
            Err(ReadError::Stale {
                published_ms: past,
                now_ms: NOW_MS,
                max_age_ms: MAX_AGE_MS,
            })
        );
    }

    /// The other side of the window, and a different cause: an operator chasing a
    /// clock that disagrees with RedStone needs to be told which direction it is
    /// out.
    #[test]
    fn a_price_dated_further_ahead_than_skew_explains_is_refused() {
        let ahead = NOW_MS + MAX_AHEAD_MS + 1;

        assert_eq!(
            read_price(
                &registered(),
                &price_account(&published(ahead)),
                &clock(NOW_MS)
            ),
            Err(ReadError::AheadOfClock {
                published_ms: ahead,
                now_ms: NOW_MS,
            })
        );
    }

    #[test]
    fn the_near_edge_of_the_skew_allowance_is_inside_it() {
        let ahead = NOW_MS + MAX_AHEAD_MS;

        assert!(read_price(
            &registered(),
            &price_account(&published(ahead)),
            &clock(NOW_MS)
        )
        .is_ok());
    }

    /// A fresh devnet clock is below the window's width, and a guest that
    /// panicked there would abort the transaction rather than refuse it.
    #[test]
    fn a_clock_below_the_windows_width_does_not_panic() {
        let early = 5;

        assert!(read_price(
            &registered(),
            &price_account(&published(early)),
            &clock(early)
        )
        .is_ok());
    }

    #[test]
    fn no_two_causes_answer_with_the_same_number() {
        let causes = [
            ReadError::WrongAccount,
            ReadError::Unavailable,
            ReadError::NotTheAggregators,
            ReadError::Undecodable,
            ReadError::NotRedStone,
            ReadError::AssetMismatch,
            ReadError::Stale {
                published_ms: 0,
                now_ms: 0,
                max_age_ms: 0,
            },
            ReadError::AheadOfClock {
                published_ms: 0,
                now_ms: 0,
            },
        ];

        let mut numbers: Vec<u32> = causes.iter().map(ReadError::code).collect();
        numbers.sort_unstable();
        let unique = numbers.len();
        numbers.dedup();
        assert_eq!(numbers.len(), unique, "two read causes share a number");
    }
}
