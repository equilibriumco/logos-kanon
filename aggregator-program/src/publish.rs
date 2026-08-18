//! Turning a verified feed into the canonical price account consumers read.
//!
//! The account is RFP-019's, re-exported through [`kanon_idl`] rather than
//! restated, so this module only decides what goes in the six fields: the price
//! on the account's `Q64.64` scale, the timestamp of the observation rather than
//! of the write, the adaptor itself as the source, and a zero confidence
//! interval because RedStone supplies none.

use kanon_idl::{AccountId, OraclePriceAccount, ProgramId};
use verifier_core::{AssetPair, FeedConfig, VerifiedFeed};

/// The adaptor's own identifier, as the price account's `source_id` holds it.
///
/// The field names the source that populated the account, and the account's own
/// documentation gives "a TWAP program or external adaptor" as what goes there,
/// so the aggregator names itself. A LEZ program reads its own `ProgramId` at
/// run time, which is why this is a conversion rather than a constant: nothing
/// a program asserts about its own identity is worth more than the identity the
/// chain already knows it by.
///
/// Public because a reader needs it too: checking that a price account came
/// from a particular adaptor means deriving the same `AccountId` from that
/// program's id. The writing path takes a [`ProgramId`] and calls this itself,
/// so no caller has to remember to.
///
/// `ProgramId` is `[u32; 8]`, reinterpreted little-endian, which is how LEZ
/// itself lays a program id out when it feeds one to a hash.
#[must_use]
pub fn adaptor_id(program_id: ProgramId) -> AccountId {
    let mut bytes = [0u8; 32];
    for (word, slot) in program_id.iter().zip(bytes.chunks_exact_mut(4)) {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    AccountId::new(bytes)
}

/// RedStone supplies no confidence interval, and the account documents zero as
/// what a source without one writes.
pub const NO_CONFIDENCE_INTERVAL: u128 = 0;

/// Why a verified price was not written to the account it was offered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishError {
    /// The account prices a different pair than the feed does.
    AssetMismatch,
    /// The account names a different source, so it is not this adaptor's to
    /// write.
    SourceMismatch,
    /// The account already holds an observation at least as recent.
    NotNewer { stored: u64, offered: u64 },
}

/// The price account a verified feed populates.
///
/// For the first write, where there is no account yet to check against. Every
/// later write goes through [`publish`], which has something to compare.
#[must_use]
pub fn price_account(
    program_id: ProgramId,
    config: &FeedConfig<'_>,
    feed: &VerifiedFeed,
) -> OraclePriceAccount {
    OraclePriceAccount {
        base_asset: AccountId::new(config.assets().base),
        quote_asset: AccountId::new(config.assets().quote),
        price: feed.price,
        timestamp: feed.timestamp_ms,
        source_id: adaptor_id(program_id),
        confidence_interval: NO_CONFIDENCE_INTERVAL,
    }
}

/// Writes a verified price into an account that already exists.
///
/// Only `price` and `timestamp` move. The three identifiers are what the
/// account *is*, so a mismatch in any of them means this is the wrong account
/// rather than an account needing an update — and overwriting them would turn
/// a misrouted write into a silently plausible one.
///
/// # Errors
///
/// [`PublishError`] when the account prices another pair, was written by
/// another source, or already holds an observation at least as recent as this
/// one.
pub fn publish(
    program_id: ProgramId,
    account: &mut OraclePriceAccount,
    config: &FeedConfig<'_>,
    feed: &VerifiedFeed,
) -> Result<(), PublishError> {
    let stored = AssetPair::new(
        account.base_asset.into_value(),
        account.quote_asset.into_value(),
    );
    if &stored != config.assets() {
        return Err(PublishError::AssetMismatch);
    }
    if account.source_id != adaptor_id(program_id) {
        return Err(PublishError::SourceMismatch);
    }
    // Strictly newer, so a package that is inside the staleness window but older
    // than what is already published cannot move the price backwards. The
    // window bounds how old a package may be against the clock; it says nothing
    // about how old it is against the last one written, and replaying a payload
    // from earlier in the same window is free.
    if feed.timestamp_ms <= account.timestamp {
        return Err(PublishError::NotNewer {
            stored: account.timestamp,
            offered: feed.timestamp_ms,
        });
    }

    account.price = feed.price;
    account.timestamp = feed.timestamp_ms;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use verifier_core::{SignerAddress, Value};

    const DECIMALS: u8 = 8;
    const MAX_AGE_MS: u64 = 60_000;

    /// Stands in for what a LEZ program reads as its own id at run time.
    const ADAPTOR: ProgramId = [0x0A0B_0C0D; 8];
    /// A second build of it, which is what any recompilation produces.
    const OTHER: ProgramId = [0x0102_0304; 8];

    fn pair() -> AssetPair {
        AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
    }

    fn signers() -> [SignerAddress; 1] {
        [SignerAddress([1; SignerAddress::LEN])]
    }

    fn config(signers: &[SignerAddress], assets: AssetPair) -> FeedConfig<'_> {
        FeedConfig::try_new(b"BTC", assets, DECIMALS, MAX_AGE_MS, signers, 1).expect("valid config")
    }

    fn verified(price: u128, timestamp_ms: u64) -> VerifiedFeed {
        VerifiedFeed {
            value: Value::from_be_slice(&[42]).expect("fits"),
            price,
            signers: 1,
            timestamp_ms,
        }
    }

    #[test]
    fn a_fresh_account_takes_every_field_from_the_feed_and_its_configuration() {
        let signers = signers();
        let account = price_account(ADAPTOR, &config(&signers, pair()), &verified(7, 1_000));

        assert_eq!(account.base_asset.into_value(), pair().base);
        assert_eq!(account.quote_asset.into_value(), pair().quote);
        assert_eq!(account.price, 7);
        assert_eq!(account.timestamp, 1_000);
        assert_eq!(account.source_id, adaptor_id(ADAPTOR));
        assert_eq!(account.confidence_interval, 0);
    }

    #[test]
    fn a_program_id_lays_out_little_endian_the_way_lez_hashes_one() {
        // `ProgramId` is `[u32; 8]` and the account field is 32 bytes, so the
        // conversion has an endianness and getting it wrong is silent: every id
        // still maps to a distinct 32 bytes, just not the ones the chain knows
        // the program by.
        let mut expected = [0u8; 32];
        for slot in expected.chunks_exact_mut(4) {
            slot.copy_from_slice(&0x0A0B_0C0Du32.to_le_bytes());
        }
        assert_eq!(adaptor_id(ADAPTOR).into_value(), expected);
    }

    #[test]
    fn two_deployments_of_the_adaptor_do_not_share_a_source_id() {
        // The point of naming the writer rather than asserting a label: a second
        // deployment is a different source, and a consumer that trusts one has
        // not thereby trusted the other.
        assert_ne!(adaptor_id(ADAPTOR), adaptor_id(OTHER));
    }

    #[test]
    fn a_newer_observation_moves_the_price_and_the_timestamp_and_nothing_else() {
        let signers = signers();
        let config = config(&signers, pair());
        let mut account = price_account(ADAPTOR, &config, &verified(7, 1_000));

        assert_eq!(
            publish(ADAPTOR, &mut account, &config, &verified(9, 2_000)),
            Ok(())
        );

        assert_eq!(account.price, 9);
        assert_eq!(account.timestamp, 2_000);
        assert_eq!(account.base_asset.into_value(), pair().base);
        assert_eq!(account.source_id, adaptor_id(ADAPTOR));
        assert_eq!(account.confidence_interval, 0);
    }

    #[test]
    fn an_account_for_another_pair_is_refused_rather_than_repointed() {
        let signers = signers();
        let mut account = price_account(ADAPTOR, &config(&signers, pair()), &verified(7, 1_000));
        let elsewhere = AssetPair::new([0xEE; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN]);

        assert_eq!(
            publish(
                ADAPTOR,
                &mut account,
                &config(&signers, elsewhere),
                &verified(9, 2_000)
            ),
            Err(PublishError::AssetMismatch)
        );
        assert_eq!(account.price, 7, "a refused write leaves the account alone");
    }

    #[test]
    fn an_account_another_program_populated_is_not_this_adaptors_to_write() {
        let signers = signers();
        let config = config(&signers, pair());
        let mut account = price_account(OTHER, &config, &verified(7, 1_000));

        assert_eq!(
            publish(ADAPTOR, &mut account, &config, &verified(9, 2_000)),
            Err(PublishError::SourceMismatch)
        );
        assert_eq!(account.price, 7);
    }

    #[test]
    fn an_older_observation_inside_the_window_cannot_move_the_price_backwards() {
        // The staleness window bounds a package's age against the clock, not
        // against what is already published, so replaying an earlier payload
        // from the same window costs nothing and would otherwise be accepted.
        let signers = signers();
        let config = config(&signers, pair());
        let mut account = price_account(ADAPTOR, &config, &verified(7, 2_000));

        assert_eq!(
            publish(ADAPTOR, &mut account, &config, &verified(9, 1_999)),
            Err(PublishError::NotNewer {
                stored: 2_000,
                offered: 1_999,
            })
        );
        assert_eq!(account.price, 7);
    }

    #[test]
    fn the_same_observation_twice_is_not_an_update() {
        let signers = signers();
        let config = config(&signers, pair());
        let mut account = price_account(ADAPTOR, &config, &verified(7, 2_000));

        assert_eq!(
            publish(ADAPTOR, &mut account, &config, &verified(7, 2_000)),
            Err(PublishError::NotNewer {
                stored: 2_000,
                offered: 2_000,
            })
        );
    }
}
