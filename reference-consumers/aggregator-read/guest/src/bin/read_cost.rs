//! Runs a prefix of a push-mode settlement, so that differencing two runs gives
//! what a read costs when somebody else already verified (M3-08).
//!
//! Measurement code, not product code, and in this guest workspace for the
//! reason `Cargo.toml` gives: it is the only place this consumer's
//! `[patch.crates-io]` applies. That matters less here than it does on the pull
//! side — nothing below recovers a signature — but a figure measured under one
//! configuration and published beside a figure measured under another is a
//! comparison of two programs rather than of two modes.
//!
//! # What a read is, in this mode
//!
//! The aggregator verified once, at submission, and published six fields. A
//! consumer's read is a **decode and four checks**: that the account is the one
//! this feed's price is published to, that the aggregator owns it, that the
//! source is RedStone, that the pair is the one the consumer registered, and
//! that the timestamp is inside the window — in both directions.
//!
//! No signature is recovered, which is the whole asymmetry M3-08 exists to
//! price: the pull side's read is `verify_price` and costs about three million
//! cycles. It is not free of hashing, though, and the guest's `Cargo.toml`
//! pins `sha2` to the risc0 accelerator for that reason — the address check is
//! [`read_price`]'s first move, and deriving the address a feed publishes to
//! runs `compute_pda` twice, once for the feed account and once for the price
//! account under it. Two SHA-256 combinations per read, and they are the
//! largest single item in this figure after the account's decode.
//!
//! `[M3-08:01]` records what "a read" is taken to mean on each side, and
//! `COSTS.md` puts the two figures beside each other.
//!
//! # The stages
//!
//! | stage | adds |
//! | --- | --- |
//! | 0 | nothing: input, setup, journal |
//! | 1 | `LezClock::from_account`, then [`read_price`] — the mode's read |
//! | 2 | the real [`settle`], whole |
//!
//! The registration's decode is *not* in stage 1. It sits above the branch with
//! the rest of the setup, so it cancels: an earlier draft decoded `PriceSource`
//! inside the arm and charged about 1,800 cycles of a consumer's own
//! bookkeeping — a fifth of the published figure — to push mode's read. The pull
//! guest carries the matching correction, and `[M3-08:01]` records why the two
//! stage 1s have to contain the same kind of work for the two residuals beside
//! them to mean the same thing.
//!
//! So `1 - 0` is a push-mode read, `2 - 0` is the whole `settle` body, and
//! **`2 - 1` is what settling costs on top of reading**. The stage numbers line
//! up with `pull_cost.rs` wherever they mean the same thing, so the two tables
//! can be read side by side.
//!
//! Five stages sit outside that chain:
//!
//! | stage | measures |
//! | --- | --- |
//! | 3 | `source::read`, the consumer's own registration |
//! | 4 | `OrderAccount::try_from_slice`, the order the settlement moves |
//! | 5 | `OraclePriceAccount::try_from_slice`, the published account's decode |
//! | 6 | filling the order and encoding it, which is what the write is |
//! | 7 | `LezClock::from_account`, the clock decode inside stage 1 |
//! | 8 | `price_account_address`, the two PDA derivations inside [`read_price`] |
//!
//! Stage 5 is this side's counterpart to the pull guest's roster rebuild: the
//! thing the read cannot happen without. Stage 8 exists because the remainder
//! after stages 5 and 7 is otherwise a place for a wrong explanation to live:
//! it is not five cheap field comparisons, it is two SHA-256 combinations. Stage 7 exists here and not there
//! because of where the decode falls — `read_price` takes a `LezClock` its caller
//! decoded, so it separates cleanly, whereas `verify_price` decodes the clock
//! itself and the cost is inside the pull read.
//!
//! **They are under no obligation to sum to anything**, for the reason M2-18
//! recorded after trying: inlined code does not cost what a separate call costs.
//!
//! Setup is identical in every stage and happens before the branch, so zkVM
//! startup, input deserialization, account construction and the journal commit
//! cancel when two stages are subtracted.
//!
//! # What this does *not* measure
//!
//! `settle` is the delegated body, not the instruction as LEZ runs it: the
//! generated validator, the dispatcher and the `SpelOutput` wrapping are all
//! outside it and none is reachable by calling a function. Nor is what LEZ
//! spends reading the instruction data first, which on this side is small — a
//! feed id rather than a payload — and is exactly why the modes differ.
//!
//! # Panics
//!
//! None. A guest panic aborts the transaction rather than reporting a failure,
//! so every fallible step reports through the journal and the host asserts on
//! what it finds there: a run that took an error path is a failure rather than a
//! suspiciously cheap component.

use nssa_core::{
    account::{Account, AccountId, AccountWithMetadata, Data, Nonce},
    program::ProgramId,
};
use reference_consumer_aggregator_read::{
    kanon_clock::LezClock,
    kanon_idl::OraclePriceAccount,
    order::{settle, OrderAccount},
    read::{price_account_address, read_price},
    source::{self, PriceSource},
};
use risc0_zkvm::guest::env;
use spel_framework::prelude::{BorshDeserialize, BorshSerialize};

/// What the host reads back: whether the stage's work happened, one value from
/// it, and how many post-states came out.
///
/// The first field is the assertion that matters. A settlement that refused — a
/// price account it will not accept, a price outside the window — returns an
/// error and no post-states, and would otherwise report as a very cheap read
/// rather than as a failure.
type Report = (u32, u64, u32);

fn main() {
    #[allow(clippy::type_complexity)]
    let (
        stage,
        order_data,
        source_data,
        price_data,
        clock_data,
        clock_id,
        price_id,
        feed_id,
        aggregator,
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
        [u32; 8],
        [u32; 8],
    ) = env::read();

    // Everything below this line and above the match is setup, and runs
    // identically in every stage.
    let ours = ProgramId::from(program_id);
    let order = account_owned_by(ours, order_data, [0x51; 32]);
    let source = account_owned_by(ours, source_data, [0x52; 32]);
    // Owned by the aggregator, because that is what makes it the canonical
    // account rather than one a caller could have written, and the check that
    // says so is inside the read.
    let price = account_owned_by(
        ProgramId::from(aggregator),
        price_data,
        <[u8; 32]>::try_from(price_id.as_slice()).unwrap_or([0; 32]),
    );
    let clock = account_owned_by(
        ProgramId::default(),
        clock_data,
        <[u8; 32]>::try_from(clock_id.as_slice()).unwrap_or([0; 32]),
    );
    let feed = <[u8; 32]>::try_from(feed_id.as_slice()).unwrap_or([0; 32]);

    let Ok(registered) = PriceSource::try_from_slice(source.account.data.as_ref()) else {
        env::commit(&(0u32, 0u64, 0u32));
        return;
    };

    // Re-opaqued after the decode above has been taken, so stage 3's `source::read`
    // cannot have its inner decode folded into the setup's.
    let source = core::hint::black_box(source);

    let prepared = Prepared {
        order,
        source,
        price,
        clock,
        feed_id: feed,
        ours,
        registered,
    };
    let report = run(stage, prepared);
    env::commit(&report);
}

/// What the setup prepared, so a stage arm measures its own work and not the
/// decoding that got there.
struct Prepared {
    order: AccountWithMetadata,
    source: AccountWithMetadata,
    price: AccountWithMetadata,
    clock: AccountWithMetadata,
    feed_id: [u8; 32],
    ours: ProgramId,
    registered: PriceSource,
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

fn run(stage: u8, prepared: Prepared) -> Report {
    match stage {
        0 => (0, 0, 0),

        // The real body, whole. Everything stage 1 does happens inside it, which
        // is what makes the difference a figure about this code rather than about
        // a restatement of it.
        2 => match settle(
            prepared.order,
            prepared.source,
            prepared.price,
            prepared.clock,
            prepared.feed_id,
            prepared.ours,
        ) {
            // The post-states through `black_box` before their length is taken,
            // for the reason stage 5 needs one: a settlement observed only by
            // how many accounts it returned is one whose account contents
            // nothing reads, and the optimiser may decline to build what never
            // escapes.
            Ok(post_states) => {
                core::hint::black_box(&post_states);
                (1, 0, post_states.len() as u32)
            }
            Err(_) => (0, 0, 0),
        },

        // A push-mode read: decode the clock off the account LEZ pins, then read
        // the published price against the registration the setup decoded.
        1 => {
            let Ok(now) = LezClock::from_account(
                prepared.clock.account_id.value(),
                prepared.clock.account.data.as_ref(),
            ) else {
                return (0, 0, 0);
            };
            match read_price(&prepared.registered, &prepared.price, &now) {
                Ok(published) => (1, published.timestamp_ms, 0),
                Err(_) => (0, 0, 0),
            }
        }

        // The consumer's own registration, read the way `settle` reads it: the
        // owner and empty-data guards included, which is why this is dearer than
        // the bare decode the setup performed.
        3 => match source::read(&prepared.source, prepared.ours) {
            Ok(registered) => (1, registered.max_age_ms, 0),
            Err(_) => (0, 0, 0),
        },

        4 => match OrderAccount::try_from_slice(prepared.order.account.data.as_ref()) {
            Ok(stored) => {
                core::hint::black_box(&stored);
                (1, stored.limit_price_q64 as u64, 0)
            }
            Err(_) => (0, 0, 0),
        },

        // The published account's decode: what this mode's read is *for*, and the
        // counterpart to the pull guest's roster rebuild.
        5 => match OraclePriceAccount::try_from_slice(prepared.price.account.data.as_ref()) {
            // The whole value through `black_box`, not one field of it. Reading
            // only `timestamp` reported 45 cycles for a 136-byte decode, which
            // `cost.rs` measures at 1,442: the optimiser had elided every field
            // nothing looked at. M2-18 recorded that a figure too cheap to be
            // plausible is the same signal as a test that cannot fail.
            Ok(published) => {
                core::hint::black_box(&published);
                (1, published.timestamp, 0)
            }
            Err(_) => (0, 0, 0),
        },

        6 => write(&prepared.order),

        // The clock decode inside stage 1, priced on its own. The same decode is
        // inside the pull side's read, where it cannot be separated.
        7 => match LezClock::from_account(
            prepared.clock.account_id.value(),
            prepared.clock.account.data.as_ref(),
        ) {
            Ok(read) => (1, read.timestamp_ms(), 0),
            Err(_) => (0, 0, 0),
        },

        // The address check `read_price` opens with, priced on its own. A feed
        // account is derived from the feed id and the price account from the
        // feed account's address, so a consumer holding only a feed id computes
        // both -- two `compute_pda` calls, each a SHA-256 over its seeds.
        8 => {
            let derived = price_account_address(&prepared.registered.aggregator, &prepared.feed_id);
            core::hint::black_box(&derived);
            (1, u64::from(derived.value()[0]), 0)
        }

        _ => (0, 0, 0),
    }
}

/// Stage 6: what the write is.
///
/// Flip `filled` and encode the account, both, because the real path does both —
/// `settle` assigns the encoded bytes straight after — and observed through
/// `black_box` because an observation that generates work of its own charges that
/// work to the figure it is taking. M2-18 found that a fold over the encoded
/// bytes cost about 1,360 cycles more than the thing being measured.
fn write(order: &AccountWithMetadata) -> Report {
    let Ok(mut stored) = OrderAccount::try_from_slice(order.account.data.as_ref()) else {
        return (0, 0, 0);
    };
    stored.filled = true;
    let mut encoded = Vec::new();
    if stored.serialize(&mut encoded).is_err() {
        return (0, 0, 0);
    }
    core::hint::black_box(&encoded);
    (1, encoded.len() as u64, 0)
}
