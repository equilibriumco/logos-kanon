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
    /// 2. `price_account` — the canonical RFP-019 price account, one per feed, at
    ///    `for_public_pda(program, sha256(feed_account_id || zero_pad_32("KANON_PRICE_ACCOUNT")))`.
    ///    The padding is not incidental: SPEL widens every seed to 32 bytes
    ///    before combining them, so hashing the nineteen bytes of the string
    ///    derives a different account and the generated validator refuses it. A
    ///    caller derives this rather than being told it, and the first
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
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked against.
    RegisterFeed {
        /// The RedStone feed id, right-padded to the wire's field width.
        ///
        /// Padded and fixed-width because it is the feed account's PDA seed, and
        /// SPEL's `ToSeed` is implemented for `[u8; 32]` but not for `Vec<u8>`.
        /// `String` is implemented and routes through `seed_from_str`, which
        /// panics above 32 bytes in generated code that runs before any body
        /// here -- a caller-triggerable guest panic, which ADR 4 makes a defect.
        /// So the caller pads, exactly as the wire does.
        feed_id: [u8; 32],
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
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked against.
    UpdateSignerSet {
        /// Which feed to rotate, right-padded to the wire's field width.
        ///
        /// Named as well as addressed: the account's address is derived from
        /// this id (ADR 33) and the id it stores is compared against it, so an
        /// operator rotating several feeds cannot hand the wrong account and
        /// move the wrong signer set. ADR 16 set the same rule for the asset
        /// pair.
        feed_id: [u8; 32],
        /// The signer set replacing the stored one.
        signers: Vec<[u8; SignerAddress::LEN]>,
        /// The threshold replacing the stored one.
        ///
        /// Moves with the set because it is one decision: a threshold is only
        /// meaningful against the set it counts.
        threshold: u8,
    },

    /// Remove a feed's registration.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked against.
    DeregisterFeed {
        /// Which feed to retire, right-padded to the wire's field width.
        ///
        /// Named as well as addressed, for the reason `UpdateSignerSet` is.
        feed_id: [u8; 32],
    },

    /// Stop a feed accepting submissions, leaving its registration intact.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked against.
    PauseFeed,

    /// Let a paused feed accept submissions again.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed's account.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked against.
    UnpauseFeed,

    /// Establish this build's admin authority, once.
    ///
    /// Not gated on being first. The genesis key the build carries is what
    /// authorises it, because a rebuilt program derives a fresh config account
    /// and an unguarded first write would be a race once per deployment rather
    /// than once ever (`[M2-06:01]`). Appended to this enum rather than
    /// inserted, since a variant's position is the discriminant.
    ///
    /// Expected accounts:
    /// 1. `config` — uninitialised, claimed by this program.
    /// 2. `admin` — the genesis authority, authorising itself.
    InitialiseAdmin,

    /// Nominate a key to take the admin authority. It does not take it yet.
    ///
    /// Two steps rather than one, so a key nobody controls never becomes the
    /// authority: an authority that cannot sign can neither administer a feed
    /// nor hand the job on, and rebuilding to recover strands every account the
    /// build created. It also lets a handover run across two parties and two
    /// transactions, which the servicing handover needs.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the current authority.
    NominateAdmin {
        /// The nominated key. The zero key is refused: nothing can sign as it.
        new_admin: [u8; 32],
    },

    /// Take the admin authority, as the nominated key.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the nominated key, signing for itself.
    AcceptAdmin,

    /// Give up the admin authority permanently, cancelling any nomination.
    ///
    /// RFP-001's contract includes this and `[M2-06:01]` records what it costs:
    /// a revoked feed's signer set can never be rotated again, so under the
    /// servicing SLA pausing is the only lever left.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the current authority.
    RevokeAdmin,
}
