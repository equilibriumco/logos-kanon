//! The state one registered feed keeps, and the configuration it verifies with.
//!
//! One account per feed, holding everything `verifier_core` needs to check a
//! payload against it: the asset pair, the scale, the staleness window, the
//! signer set and the threshold. Nothing here is global, which is
//! [ADR 30](../../adr/0030-the-signer-set-stays-per-feed.md)'s decision rather
//! than a convenience -- a feed's parameters are governed in one place, on one
//! update path.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use spel_framework_macros::account_type;
use verifier_core::{AssetPair, ConfigError, FeedConfig, SignerAddress};

/// One feed's registration, as the aggregator stores it.
///
/// The fields are `FeedConfig`'s, plus the pause flag, which is administrative
/// rather than cryptographic: a paused feed still has a valid configuration and
/// refuses submissions anyway.
///
/// Signer addresses are stored as raw bytes rather than as
/// [`SignerAddress`], because that type belongs to a `no_std` crate that carries
/// no serialisation. [`Self::signer_addresses`] converts, and
/// [`Self::config`] borrows the result.
#[account_type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct FeedAccount {
    /// The RedStone feed id, right-padded to the wire width.
    pub feed_id: [u8; 32],
    /// The base asset of the pair this feed prices.
    pub base_asset: [u8; 32],
    /// The quote asset of the pair this feed prices.
    pub quote_asset: [u8; 32],
    /// The power of ten the signers scale values by.
    pub decimals: u8,
    /// How old the oldest package behind a price may be.
    pub max_age_ms: u64,
    /// The authorised signers, in registration order.
    ///
    /// The width is a literal because the IDL generator reads this file as text
    /// and cannot evaluate a path constant: written as
    /// `[u8; SignerAddress::LEN]` it publishes `[u8; 0]`, and a client
    /// generating code from that gets a zero-length address. The assertion below
    /// is what keeps the literal tied to the type it mirrors.
    pub signers: Vec<[u8; 20]>,
    /// How many distinct authorised signers a price needs.
    pub threshold: u8,
    /// Whether the feed currently refuses submissions.
    pub paused: bool,
}

/// The literal width in [`FeedAccount::signers`] is the one `verifier_core` uses.
const _: () = assert!(SignerAddress::LEN == 20);

impl FeedAccount {
    /// The stored signer set, in the form `verifier_core` takes.
    #[must_use]
    pub fn signer_addresses(&self) -> Vec<SignerAddress> {
        self.signers.iter().copied().map(SignerAddress).collect()
    }

    /// The configuration this feed verifies against.
    ///
    /// Takes the signer slice rather than producing it, because [`FeedConfig`]
    /// borrows the set for as long as it lives, and a temporary built inside
    /// this call would not outlive the return.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the stored fields do not describe a usable feed.
    /// Registration checks them, so reaching this is a corrupt or
    /// out-of-band-written account rather than a bad submission.
    pub fn config<'a>(&self, signers: &'a [SignerAddress]) -> Result<FeedConfig<'a>, ConfigError> {
        FeedConfig::try_new(
            &self.feed_id,
            AssetPair::new(self.base_asset, self.quote_asset),
            self.decimals,
            self.max_age_ms,
            signers,
            self.threshold,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTC: &[u8] = b"BTC";

    fn registered() -> FeedAccount {
        let mut feed_id = [0u8; 32];
        feed_id[..BTC.len()].copy_from_slice(BTC);
        FeedAccount {
            feed_id,
            base_asset: [1u8; 32],
            quote_asset: [2u8; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: vec![[3u8; 20], [4u8; 20], [5u8; 20]],
            threshold: 2,
            paused: false,
        }
    }

    #[test]
    fn a_registered_feed_verifies_against_what_it_stores() {
        let account = registered();
        let signers = account.signer_addresses();
        let config = account.config(&signers).expect("registration is valid");

        assert_eq!(config.assets().base, account.base_asset);
        assert_eq!(config.assets().quote, account.quote_asset);
        assert_eq!(config.decimals(), account.decimals);
        assert_eq!(config.max_age_ms(), account.max_age_ms);
        assert_eq!(config.threshold(), account.threshold);
        assert_eq!(config.signers().len(), account.signers.len());
    }

    #[test]
    fn the_stored_feed_id_survives_the_round_trip_padded() {
        let account = registered();
        let signers = account.signer_addresses();
        let config = account.config(&signers).expect("registration is valid");

        // Stored already padded to the wire width, so configuring it must not
        // pad it a second time or shift the id off the front.
        assert_eq!(config.feed_id(), &account.feed_id);
        assert_eq!(&config.feed_id()[..BTC.len()], BTC);
    }

    #[test]
    fn a_max_age_above_the_ceiling_is_refused_at_the_account_too() {
        let mut account = registered();
        account.max_age_ms = verifier_core::feed::MAX_MAX_AGE_MS + 1;
        let signers = account.signer_addresses();

        assert!(account.config(&signers).is_err());
    }

    #[test]
    fn a_threshold_no_signer_set_can_reach_is_refused() {
        let mut account = registered();
        account.threshold = u8::try_from(account.signers.len()).expect("three fits") + 1;
        let signers = account.signer_addresses();

        assert!(account.config(&signers).is_err());
    }

    #[test]
    fn the_state_round_trips_through_borsh() {
        let account = registered();
        let bytes = borsh::to_vec(&account).expect("serialises");
        let read = FeedAccount::try_from_slice(&bytes).expect("deserialises");

        assert_eq!(read, account);
    }
}
