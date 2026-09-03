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
//! So `1 - 0` is the verification, `2 - 0` is the whole `submit_price` body, and
//! **`2 - 1` is what the write costs on top of verifying** — the figure M2-18
//! exists to publish and the one P1 was missing.
//!
//! Three stages sit outside that chain, each isolating one thing the residual
//! bundles, because a residual nobody has taken apart is a place for a wrong
//! explanation to live:
//!
//! | stage | measures |
//! | --- | --- |
//! | 3 | the PDA derivation SPEL's generated validator performs before the body |
//! | 4 | `OraclePriceAccount::try_from`, the read only an update does |
//! | 5 | `AutoClaim::pda_from_seeds`, the claim only a first write does |
//!
//! Stages 4 and 5 are what make the gap between the two cases explicable rather
//! than merely reported: the update pays 4 and the first write pays 5, so the
//! difference between them should be `4 - 5`, and
//! `the_gap_between_the_cases_is_the_read_less_the_claim` asserts it is.
//! @frenzox asked for exactly that on #60, having noticed the first version
//! called the gap "the account read" when the first-write path was doing
//! create-only work of its own.
//!
//! Setup is identical in every stage and happens before the branch, so zkVM
//! startup, input deserialization, account construction and the journal commit
//! cancel when two stages are subtracted.
//!
//! # What this does *not* measure
//!
//! `submit_price` is the delegated body, not the instruction as LEZ runs it. The
//! generated handler validates its accounts first -- `price_account` carries
//! `#[account(mut, pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]`, so
//! SPEL derives and checks that address before the body is entered -- and the
//! dispatcher decodes the instruction and wraps the result in `SpelOutput`
//! afterwards. None of that is inside stage 2.
//!
//! Stage 3 puts a number on the validator's substantive work, the PDA
//! derivation. The dispatcher and the `SpelOutput` wrapping remain outside every
//! figure here, because reaching them means going through the macro's generated
//! entry point, which takes a whole LEZ transaction rather than a function call.
//! So the published figures are the body plus a named validator cost, and
//! `COSTS.md` says so rather than calling them the instruction.
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

use aggregator_program::{kanon_idl::OraclePriceAccount, submit::PRICE_ACCOUNT_SEED, submit_price};
use nssa_core::{
    account::{Account, AccountId, AccountWithMetadata, Data, Nonce},
    program::ProgramId,
};
use risc0_zkvm::guest::env;
use spel_framework::{
    pda::{compute_pda, seed_from_str},
    spel_output::AutoClaim,
};
use verifier_core::{
    backend::{InProgramBackend, SignerAddress},
    decode::Payload,
    feed::{verify_feed, AssetPair, FeedConfig},
    time::{TimeError, TimeSource},
};

/// A realistic eight-decimal price, for the stages that isolate the write. Fixed
/// rather than taken from the payload, so those figures do not move when the
/// vectors are re-captured.
const SAMPLE_VALUE: u128 = 300_012_345_678;

/// The timestamp the clock account carries, read the cheap way.
///
/// Not `LezClock`: that decode is one of the things the residual contains, and
/// pulling it into a stage that is meant to isolate the write would put it in
/// two places at once.
fn now_from(clock: &AccountWithMetadata) -> u64 {
    let data = clock.account.data.as_ref();
    match <[u8; 8]>::try_from(&data[8.min(data.len())..16.min(data.len())]) {
        Ok(bytes) => u64::from_le_bytes(bytes),
        Err(_) => 0,
    }
}

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

    if stage == 2 {
        // The real body, whole. Everything stage 1 does happens inside it, which
        // is what makes the difference a figure about this code rather than about
        // a restatement of it.
        return match submit_price(feed, price_account, clock_account, payload_bytes, ours) {
            Ok(post_states) => (0, 0, post_states.len() as u32),
            Err(_) => (0, 0, 0),
        };
    }

    // The three stages outside the prefix chain. Each calls exactly what the
    // shipped path calls, so the figure is about that code and not a model of it.
    if stage == 3 {
        // What the generated validator does before the body: derive the address
        // `#[account(pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]`
        // declares, which is the same derivation `post_states` repeats on a first
        // write and a caller has to reproduce.
        let derived = compute_pda(
            &ours,
            &[feed.account_id.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
        );
        // Committed so the derivation cannot be optimised away, and so the host
        // can tell a stage that ran from one that folded to a constant.
        return (1, u64::from(derived.value()[0]), 0);
    }

    if stage == 4 {
        // The read only an update does. On a first write the account is default
        // and this decodes nothing, which is why the host asks for it per case.
        return match OraclePriceAccount::try_from(&price_account.account.data) {
            Ok(account) => (1, account.timestamp, 0),
            Err(_) => (0, 0, 0),
        };
    }

    if stage == 6 || stage == 7 {
        // The two write halves, isolated. A `VerifiedFeed` is built here rather
        // than verified, because what is under measurement is the write and a
        // verification either side of it would swamp it. Its fields are public,
        // so this is the real type and not a stand-in.
        let verified = verifier_core::feed::VerifiedFeed {
            value: verifier_core::value::Value::from_be_slice(&SAMPLE_VALUE.to_be_bytes())
                .unwrap_or_default(),
            price: SAMPLE_VALUE,
            signers: 5,
            timestamp_ms: now_from(&clock_account),
        };

        if stage == 6 {
            // The create side: build the account from nothing.
            // Build *and encode*, which is what the real path does: `post_states`
            // assigns `Data::from(written)` straight after. Encoding is also what
            // makes the figure measurable at all -- a construction whose fields
            // are never all read is dead code, and observing one field only keeps
            // that field alive. With the checksum over `built.timestamp` alone the
            // stage reported the same 43 cycles whether the call was there or not,
            // which is what @frenzox found on #60; with a checksum over one byte
            // per field it reported 51, still eliding the 32-byte copies. Over the
            // encoded bytes nothing can be skipped.
            let built = aggregator_program::publish::price_account(config, &verified);
            let encoded = Data::from(&built);
            let checksum = encoded
                .as_ref()
                .iter()
                .fold(0u64, |acc, byte| acc.rotate_left(1) ^ u64::from(*byte));
            return (1, checksum, 0);
        }

        // The update side: the three checks `publish` makes before it writes.
        return match OraclePriceAccount::try_from(&price_account.account.data) {
            Ok(mut published) => {
                match aggregator_program::publish::publish(&mut published, config, &verified) {
                    Ok(()) => (1, published.timestamp, 0),
                    Err(_) => (0, 0, 0),
                }
            }
            Err(_) => (0, 0, 0),
        };
    }

    if stage == 5 {
        // The claim only a first write does, as `post_states` spells it.
        let feed_seed = *feed.account_id.value();
        let name_seed = seed_from_str(PRICE_ACCOUNT_SEED);
        let claimed = AutoClaim::pda_from_seeds(&[&feed_seed, &name_seed])
            .to_post_state(price_account.account);
        return (
            1,
            u64::from(claimed.account().data.as_ref().len() as u32),
            0,
        );
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
