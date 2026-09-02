//! Runs a prefix of the push instruction, so that differencing two runs gives
//! the cost of the write rather than of the verification (M2-18).
//!
//! Measurement code, not product code, and here for the reason
//! `verify_cost.rs` is: this is the only workspace where the product's
//! `[patch.crates-io]` applies, and a cost figure measured under different
//! patches is a figure for a different program.
//!
//! # The stages
//!
//! | stage | adds |
//! | --- | --- |
//! | 0 | nothing: input, setup, journal |
//! | 1 | `Payload::decode` and [`verify_feed`] — the verification half |
//! | 2 | the real [`submit_price`], whole |
//!
//! So `1 - 0` is the verification, `2 - 0` is the whole instruction body, and
//! **`2 - 1` is what the push write costs on top of verifying** — which is the
//! figure M2-18 exists to publish and the one P1 was missing.
//!
//! Setup is identical in every stage and happens before the branch, so zkVM
//! startup, input deserialization, account construction and the journal commit
//! cancel when two stages are subtracted.
//!
//! # What `2 - 1` contains
//!
//! Named in full, because "the write" would be generous: the ownership check on
//! the feed account, decoding the clock from the account LEZ pins, decoding the
//! stored `FeedAccount` from Borsh, the paused check, rebuilding the signer set
//! and `FeedConfig` from what was stored, triaging the price account between a
//! first write and an update, decoding the published account in the update case,
//! choosing the asset pair to check against, computing the new account, and
//! producing the post-states LEZ applies.
//!
//! It is a residual and it is honest about being one. The finer split — clock
//! decode against Borsh decode against `post_states` — would need `kanon-clock`
//! and `borsh` as *direct* guest dependencies, and it is not worth adding them to
//! break down 0.2% of an instruction. The host passes the clock's account id and
//! the encoded account data in instead.
//!
//! Adding this binary to the package is safe, which was checked rather than
//! assumed, because the aggregator's image id is its program id (`[M2-06:01]`)
//! and a moved id would strand every address derived from it: `AGGREGATOR_ID` is
//! byte-identical with and without this file, and `VERIFY_COST_ID` and
//! `VERIFIER_LINK_ID` are unchanged too. A new binary does not perturb its
//! siblings.
//!
//! # Both price-account states
//!
//! The host measures this twice, because the two are different work: a **first
//! write** builds the account from nothing, while an **update** decodes the
//! published `OraclePriceAccount` first and compares against the pair it already
//! carries. A single figure would be an average of two paths a reader cannot
//! separate.
//!
//! # Panics
//!
//! None, for the reason `verify_cost.rs` gives: a guest panic aborts rather than
//! reporting, and a measurement that aborts is not a measurement. Every fallible
//! step reports through the journal and the host asserts on what it finds there,
//! so a run that took an early exit is a failure rather than a suspiciously
//! cheap component.

use aggregator_program::submit_price;
use nssa_core::{
    account::{Account, AccountId, AccountWithMetadata, Data, Nonce},
    program::ProgramId,
};
use risc0_zkvm::guest::env;
use verifier_core::{
    backend::{InProgramBackend, SignerAddress},
    decode::Payload,
    feed::{verify_feed, AssetPair, FeedConfig},
    time::{TimeError, TimeSource},
};

/// The clock as a `TimeSource`, for the verification stage only.
///
/// The real instruction reads `LezClock` off the pinned account, and that decode
/// is inside what stage 2 adds. This is the same instant, supplied the cheap way,
/// so the verification stage measures verification and not clock plumbing.
struct FixedClock(u64);

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.0)
    }
}

/// What the host reads back: signers counted, the low word of the price, and how
/// many post-states came out.
///
/// The post-state count is the assertion that matters for stage 2. A submission
/// that refused — a feed it does not own, a clock it could not read — returns an
/// error and no post-states, and would report as a very cheap write rather than
/// as a failure.
type Report = (u32, u64, u32);

fn main() {
    #[allow(clippy::type_complexity)]
    let (
        stage,
        payload_bytes,
        feed_data,
        price_data,
        clock_data,
        clock_id,
        feed_id,
        base_asset,
        quote_asset,
        signer_bytes,
        threshold,
        decimals,
        now_ms,
        max_age_ms,
        program_id,
    ): (
        u8,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        u8,
        u8,
        u64,
        u64,
        [u32; 8],
    ) = env::read();

    // Everything below this line and above the match is setup, and runs
    // identically in every stage.
    let signers: Vec<SignerAddress> = signer_bytes
        .chunks_exact(SignerAddress::LEN)
        .filter_map(|chunk| chunk.try_into().ok().map(SignerAddress))
        .collect();
    let backend = InProgramBackend::new();
    let clock = FixedClock(now_ms);
    let assets = match (
        <[u8; AssetPair::ID_LEN]>::try_from(base_asset.as_slice()),
        <[u8; AssetPair::ID_LEN]>::try_from(quote_asset.as_slice()),
    ) {
        (Ok(base), Ok(quote)) => Some(AssetPair::new(base, quote)),
        _ => None,
    };
    let config = FeedConfig::try_new(
        &feed_id,
        assets.unwrap_or_else(|| AssetPair::new([0; AssetPair::ID_LEN], [0; AssetPair::ID_LEN])),
        decimals,
        max_age_ms,
        &signers,
        threshold,
    );

    let ours = ProgramId::from(program_id);
    let feed = account(ours, feed_data, [0x11; 32]);
    // A first write is a fully default account, which is how `submit_price`
    // distinguishes the two cases: an empty `price_data` means create.
    let price_account = if price_data.is_empty() {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: AccountId::new([0x22; 32]),
        }
    } else {
        account(ours, price_data, [0x22; 32])
    };
    let clock_account = match <[u8; 32]>::try_from(clock_id.as_slice()) {
        Ok(id) => account_owned_by(ProgramId::default(), clock_data, id),
        Err(_) => account_owned_by(ProgramId::default(), clock_data, [0; 32]),
    };

    let report: Report = match (config, assets) {
        (Ok(config), Some(assets)) => run(
            stage,
            &payload_bytes,
            &config,
            &assets,
            &backend,
            &clock,
            feed,
            price_account,
            clock_account,
            ours,
        ),
        _ => (0, 0, 0),
    };

    env::commit(&report);
}

fn account(owner: ProgramId, data: Vec<u8>, id: [u8; 32]) -> AccountWithMetadata {
    account_owned_by(owner, data, id)
}

fn account_owned_by(owner: ProgramId, data: Vec<u8>, id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance: 0,
            data: Data::try_from(data).unwrap_or_default(),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(id),
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    stage: u8,
    payload_bytes: &[u8],
    config: &FeedConfig<'_>,
    expected: &AssetPair,
    backend: &InProgramBackend,
    clock: &FixedClock,
    feed: AccountWithMetadata,
    price_account: AccountWithMetadata,
    clock_account: AccountWithMetadata,
    ours: ProgramId,
) -> Report {
    if stage == 0 {
        return (0, 0, 0);
    }

    if stage >= 2 {
        // The real instruction, whole. Everything stage 1 does happens inside it,
        // which is what makes the difference a figure about this code rather than
        // about a restatement of it.
        return match submit_price(feed, price_account, clock_account, payload_bytes, ours) {
            Ok(post_states) => (0, 0, post_states.len() as u32),
            Err(_) => (0, 0, 0),
        };
    }

    // Stage 1: the verification half, called exactly as `submit_price` calls it.
    let Ok(payload) = Payload::decode(payload_bytes) else {
        return (0, 0, 0);
    };
    match verify_feed(&payload, config, expected, backend, clock) {
        Ok(verified) => (u32::from(verified.signers), verified.price as u64, 0),
        Err(_) => (0, 0, 0),
    }
}
