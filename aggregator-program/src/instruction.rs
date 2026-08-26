//! The instruction set, and what each instruction expects.
//!
//! The enum is the wire form LEZ decodes into. It sits here rather than in the
//! guest so the host side -- the SDK, the relayer, the CLI -- builds
//! transactions against the same definition the program dispatches on.
//!
//! Every instruction other than [`Instruction::SubmitPrice`] is
//! administrative and gated on the RFP-001 authority. Which authority, and how
//! it is checked, is deliberately not decided here: M1-07 established what to
//! build against and M2-06 is the shim if nothing has merged upstream, so the
//! gate lives behind that boundary rather than being spelled into each
//! instruction.

use serde::{Deserialize, Serialize};
use verifier_core::SignerAddress;

/// What the aggregator can be asked to do.
/// Serde and not Borsh, because serde is what the wire uses: `read_lee_inputs`
/// deserialises this with `risc0_zkvm::serde`, and the variant's position in this
/// declaration is the discriminant a caller has to encode. Deriving Borsh as well
/// would offer a second encoding that no one reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instruction {
    /// Verify a signed RedStone payload and publish the price it carries.
    ///
    /// The one instruction any caller may send, and the only one that writes a
    /// price. Verification and the write happen in one transaction, so a
    /// payload that fails any check publishes nothing.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account, read and never written, and
    ///    required to be owned by this program: an account a caller owns would
    ///    decode as a feed with a signer set of the caller's choosing.
    /// 2. `price_account` — the canonical RFP-019 price account, one per feed at
    ///    `for_public_pda(program, sha256(feed_account_id || "KANON_PRICE_ACCOUNT"))`.
    ///    A caller derives it rather than being told it, and the first
    ///    submission creates it (ADR 32).
    /// 3. `clock` — the LEZ clock program's every-block account, which is where
    ///    staleness gets its "now" and the only place it may come from
    ///    (ADR 13).
    SubmitPrice {
        /// The payload as RedStone serialises it, packages and envelope.
        payload: Vec<u8>,
    },

    /// Register a feed, with the parameters every later submission is checked
    /// against.
    ///
    /// Expected accounts:
    /// 1. `feed` — uninitialised account for this feed, claimed by the program.
    /// 2. `admin` — the RFP-001 authority, authorising the registration.
    RegisterFeed {
        /// The RedStone feed id, unpadded as RedStone publishes it.
        feed_id: Vec<u8>,
        /// The base asset of the pair.
        base_asset: [u8; 32],
        /// The quote asset of the pair.
        quote_asset: [u8; 32],
        /// The power of ten the signers scale values by.
        decimals: u8,
        /// How old the oldest package behind a price may be.
        max_age_ms: u64,
        /// The authorised signers for this feed.
        signers: Vec<[u8; SignerAddress::LEN]>,
        /// How many distinct authorised signers a price needs.
        threshold: u8,
    },

    /// Replace a feed's signer set and threshold.
    ///
    /// Per feed, because the set is (ADR 30). An upstream rotation that touches
    /// several feeds is several of these.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the RFP-001 authority.
    UpdateSignerSet {
        /// The signer set replacing the stored one.
        signers: Vec<[u8; SignerAddress::LEN]>,
        /// The threshold replacing the stored one.
        threshold: u8,
    },

    /// Remove a feed's registration.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the RFP-001 authority.
    DeregisterFeed,

    /// Stop a feed accepting submissions, leaving its registration intact.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the RFP-001 authority.
    PauseFeed,

    /// Let a paused feed accept submissions again.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the RFP-001 authority.
    UnpauseFeed,
}
