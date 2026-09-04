//! Runs a prefix of a pull settlement, so that differencing two runs gives what
//! a read costs in public mode (M3-08).
//!
//! Measurement code, not product code, and in this guest workspace for the
//! reason `Cargo.toml` gives: it is the only place this consumer's
//! `[patch.crates-io]` applies, and a cycle figure measured under different
//! patches is a figure for a different program. The push side's equivalents live
//! in `methods/guest` for the same reason.
//!
//! # What a read is, in this mode
//!
//! There is no published price to fetch. A pull consumer is handed bytes and
//! establishes the price itself, so **its read is a verification** — which is the
//! asymmetry M3-08 exists to price. `[M3-08:01]` records why two figures are
//! published rather than one: `verify_price` is the mode's cost and travels to
//! any consumer, while `settle` is what a read costs inside a program that also
//! holds a limit order, and that order is this reference consumer's domain rather
//! than the mode's.
//!
//! # The stages
//!
//! | stage | adds |
//! | --- | --- |
//! | 0 | nothing: input, setup, journal |
//! | 1 | rebuilding the roster, then `verify_price` — the mode's read |
//! | 2 | the real [`settle`], whole |
//!
//! So `1 - 0` is a pull read, `2 - 0` is the whole `settle` body, and **`2 - 1`
//! is what settling costs on top of reading**.
//!
//! Four stages sit outside that chain, because a residual nobody has taken apart
//! is a place for a wrong explanation to live:
//!
//! | stage | measures |
//! | --- | --- |
//! | 3 | `trust::read`, the consumer's own registration |
//! | 4 | `OrderAccount::try_from_slice`, the order the settlement moves |
//! | 5 | `signer_addresses` and `FeedTrust::config`, the roster rebuild |
//! | 6 | filling the order and encoding it, which is what the write is |
//!
//! Stage 5 is the one with a job beyond curiosity: `1 - 5` isolates
//! `verify_price` from the roster rebuild that has to precede it.
//!
//! # What the read is not allowed to contain
//!
//! Stage 1 is a measurement of one operation, not a replay of `settle`'s opening
//! moves — `settle` decodes the order first, reaches the registration through
//! `trust::read`, and checks the order's terms before it verifies anything.
//! Reproducing that order here would buy nothing and did cost something: an
//! earlier draft decoded the registration *and* the `OrderAccount` inside stage
//! 1, which charged the reference consumer's limit order to the pull mode's read
//! while the push guest charged its own order decode to the residual. Two
//! residuals published side by side then meant different things.
//!
//! So both decodes moved above the branch, into the setup every stage pays and
//! every difference cancels. What stage 1 measures is the roster rebuild and
//! `verify_price`, which is what `[M3-08:01]` says a pull read is, and what the
//! push guest's stage 1 measures is the same shape: the mode's own work over
//! values a consumer already holds.
//!
//! **They are under no obligation to sum to anything.** M2-18 tried to make
//! isolated figures account for a difference between two paths and could not —
//! inlined code does not cost what a separate call costs — and `COSTS.md`
//! records that so a third attempt is not made. Each figure here says what one
//! operation costs. None of them is a component of the residual.
//!
//! The clock decode is not among them, and not because it is free. It sits
//! *inside* `verify_price`, which takes the account's id and bytes and decodes
//! them itself, so on this side it is part of the read rather than beside it. The
//! push side is where it separates: `read_price` takes a `LezClock` its caller
//! decoded, so `read_cost.rs` prices the same decode there.
//!
//! Setup is identical in every stage and happens before the branch, so zkVM
//! startup, input deserialization, account construction and the journal commit
//! cancel when two stages are subtracted.
//!
//! # What this does *not* measure
//!
//! `settle` is the delegated body, not the instruction as LEZ runs it. The
//! generated handler validates its accounts first and the dispatcher decodes the
//! instruction and wraps the result afterwards, none of which is reachable by
//! calling a function. Neither is what LEZ spends reading the instruction data
//! before the program is entered — about 113 cycles a byte, which for a payload
//! is the dominant term and is `MAX_PAYLOAD_BYTES`'s subject (ADR 26).
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
use reference_consumer_pull::{
    order::settle,
    pull_lib::{verify_price, AssetPair, InProgramBackend},
    trust::{self, FeedTrust},
    OrderAccount,
};
use risc0_zkvm::guest::env;
use spel_framework::prelude::{BorshDeserialize, BorshSerialize};

/// What the host reads back: whether the stage's work happened, one value from
/// it, and how many post-states came out.
///
/// The first field is the assertion that matters. A settlement that refused — an
/// order it does not own, a roster that could not meet the threshold — returns an
/// error and no post-states, and would otherwise report as a very cheap read
/// rather than as a failure.
type Report = (u32, u64, u32);

fn main() {
    #[allow(clippy::type_complexity)]
    let (stage, payload_bytes, order_data, trust_data, clock_data, clock_id, feed_id, program_id): (
        u8,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        [u32; 8],
    ) = env::read();

    // Everything below this line and above the match is setup, and runs
    // identically in every stage.
    let ours = ProgramId::from(program_id);
    let order = account_owned_by(ours, order_data, [0x31; 32]);
    let trust = account_owned_by(ours, trust_data, [0x32; 32]);
    let clock = match <[u8; 32]>::try_from(clock_id.as_slice()) {
        Ok(id) => account_owned_by(ProgramId::default(), clock_data, id),
        Err(_) => account_owned_by(ProgramId::default(), clock_data, [0; 32]),
    };
    let feed = <[u8; 32]>::try_from(feed_id.as_slice()).unwrap_or([0; 32]);

    let (registered, stored) = match (
        FeedTrust::try_from_slice(trust.account.data.as_ref()),
        OrderAccount::try_from_slice(order.account.data.as_ref()),
    ) {
        (Ok(registered), Ok(stored)) => (registered, stored),
        _ => {
            env::commit(&(0u32, 0u64, 0u32));
            return;
        }
    };

    // Re-opaqued after the decodes above have been taken, so that stage 4's
    // isolated decode of the same bytes cannot be folded into the setup's. A
    // `black_box` before the first decode would not do it: two pure decodes of
    // one opaque value are still common subexpressions.
    let order = core::hint::black_box(order);
    let trust = core::hint::black_box(trust);

    let prepared = Prepared {
        order,
        trust,
        clock,
        feed_id: feed,
        ours,
        registered,
        stored,
    };
    let report = run(stage, &payload_bytes, prepared);
    env::commit(&report);
}

/// What the setup prepared, so a stage arm measures its own work and not the
/// decoding that got there.
struct Prepared {
    order: AccountWithMetadata,
    trust: AccountWithMetadata,
    clock: AccountWithMetadata,
    feed_id: [u8; 32],
    ours: ProgramId,
    registered: FeedTrust,
    stored: OrderAccount,
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

fn run(stage: u8, payload_bytes: &[u8], prepared: Prepared) -> Report {
    match stage {
        0 => (0, 0, 0),

        // The real body, whole. Everything stage 1 does happens inside it, which
        // is what makes the difference a figure about this code rather than about
        // a restatement of it.
        2 => match settle(
            prepared.order,
            prepared.trust,
            prepared.clock,
            prepared.feed_id,
            payload_bytes,
            prepared.ours,
        ) {
            // The post-states through `black_box` before their length is taken.
            // A settlement observed only by how many accounts it returned is one
            // whose account contents nothing reads, and the optimiser is
            // entitled to skip building what never escapes -- the same trap the
            // 45-cycle decode fell into, in the stage that dominates this table.
            Ok(post_states) => {
                core::hint::black_box(&post_states);
                (1, 0, post_states.len() as u32)
            }
            Err(_) => (0, 0, 0),
        },

        1 | 5 => read(stage, payload_bytes, &prepared),

        // The consumer's own registration, read the way `settle` reads it: the
        // owner and empty-data guards included, which is why this is dearer than
        // the bare decode the setup performed.
        3 => match trust::read(&prepared.trust, prepared.ours) {
            Ok(registered) => (1, registered.max_age_ms, 0),
            Err(_) => (0, 0, 0),
        },

        4 => match OrderAccount::try_from_slice(prepared.order.account.data.as_ref()) {
            // The whole value through `black_box`, not one field of it: the
            // optimiser elides every field nothing looks at, and a decode figure
            // that priced one `u128` would be a figure for something else.
            // M2-18 recorded that trap.
            Ok(stored) => {
                core::hint::black_box(&stored);
                (1, stored.limit_price_q64 as u64, 0)
            }
            Err(_) => (0, 0, 0),
        },

        6 => write(&prepared.order),

        _ => (0, 0, 0),
    }
}

/// Stage 1, and stage 5 as its prefix.
///
/// The two public calls the consumer's own private `verify` is built from, over
/// a registration and an order the setup already decoded. The pair verified
/// against is the *order's*, which is the point of that argument — the
/// configuration carries the authority's, so the check holds two independently
/// written records against each other rather than a value against itself
/// (ADR 32, and `order.rs` says it at length).
fn read(stage: u8, payload_bytes: &[u8], prepared: &Prepared) -> Report {
    let signers = prepared.registered.signer_addresses();
    let Ok(config) = prepared.registered.config(&signers) else {
        return (0, 0, 0);
    };

    if stage == 5 {
        // The roster rebuild alone, so `1 - 5` is the verification without it.
        // `black_box` because a configuration nothing reads is one the compiler
        // may decline to build.
        core::hint::black_box(&config);
        return (1, signers.len() as u64, 0);
    }

    match verify_price(
        payload_bytes,
        &config,
        &AssetPair::new(prepared.stored.base_asset, prepared.stored.quote_asset),
        prepared.clock.account_id.value(),
        prepared.clock.account.data.as_ref(),
        &InProgramBackend::new(),
    ) {
        Ok(verified) => (u32::from(verified.signers), verified.price as u64, 0),
        Err(_) => (0, 0, 0),
    }
}

/// Stage 6: what the write is.
///
/// Flip `filled` and encode the account, both, because the real path does both —
/// `settle` assigns the encoded bytes straight after — and because an observation
/// that keeps the value alive without generating work of its own is the only kind
/// that measures this honestly. M2-18 learned that the hard way: a fold over the
/// encoded bytes charged about 1,360 cycles of its own to the figure it was
/// taking, so `black_box` is the tool rather than a checksum.
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
