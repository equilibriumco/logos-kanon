//! What a price read costs in each mode, measured and pinned (M3-08).
//!
//! `cost.rs` prices verification and the push write. This prices the thing a
//! *consumer* pays to obtain a usable price, which is where the two modes stop
//! being variations on one design:
//!
//! - **Pull.** There is nothing published to fetch. The consumer is handed bytes
//!   and establishes the price itself, so its read is a verification, and it
//!   pays that on every read.
//! - **Push.** The aggregator verified once, at submission. A consumer reads the
//!   account it wrote and checks provenance, freshness and the asset pair. No
//!   signature is recovered; the hashing that remains is the two PDA derivations
//!   that establish *which* account it is reading, and they are the majority of
//!   the figure.
//!
//! So the same requirement — P2's "per mode" — is answered by two figures a
//! factor of 447 apart, and that gap is the point rather than an artefact. `COSTS.md` states it, and `[M3-08:01]` records what "a read" is
//! taken to mean on each side and why two figures per mode are published rather
//! than one.
//!
//! # Method
//!
//! The same prefix-differencing `cost.rs` uses, in each consumer's own guest
//! workspace so the figures are measured under the `[patch.crates-io]` that
//! consumer ships with. Two guests, stages documented in each:
//!
//! ```text
//! reference-consumers/pull/guest/src/bin/pull_cost.rs
//! reference-consumers/aggregator-read/guest/src/bin/read_cost.rs
//! ```
//!
//! Both are `stage 1 - stage 0` for the mode's read and `stage 2 - stage 0` for
//! the whole `settle` body, with the isolated stages above 2 pricing what the
//! residual contains.
//!
//! The two stage 1s contain the same *kind* of work, which is what lets the two
//! residuals beside them be compared. Everything a consumer holds before it
//! reads — the registration's decode on both sides, and the order's on the pull
//! side, where `verify_price` needs the pair off it — happens in the setup above
//! the branch and cancels. An earlier draft did those decodes inside stage 1 and
//! so charged the reference consumer's limit order to pull mode's read and a
//! fifth of the push figure to push mode's; the two guests carry the correction
//! and `[M3-08:01]` records it.
//!
//! # Why the figures are pinned by exact equality
//!
//! For the reason `cost.rs` gives: a cycle count is a deterministic function of
//! the ELF and its input, so a range would only hide a change. A figure that
//! moves means the guest changed, and the test names what to do about it.

#![allow(clippy::items_after_test_module)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use borsh::BorshSerialize;
use kanon_clock::CLOCK_ACCOUNT_ID;
use kanon_methods::{PULL_COST_ELF, READ_COST_ELF};
use lee_core::program::ProgramId;
use reference_consumer_aggregator_read::kanon_idl::{AccountId, OraclePriceAccount};
use reference_consumer_aggregator_read::read::{price_account_address, REDSTONE_SOURCE_ID};
use reference_consumer_aggregator_read::source::PriceSource;
use reference_consumer_pull::{FeedTrust, OrderAccount};
use risc0_zkvm::{default_executor, ExecutorEnv};

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

#[path = "support/lez.rs"]
mod lez;
use vectors::Vector;

/// The feed every figure here is measured over, so the pull read is comparable
/// with `cost.rs`'s verification table rather than a different payload's.
const FEED: &str = "BTC";

/// Five packages, five signatures, threshold three: the shape `FEEDS.md`
/// observed and RFP-020 defaults to.
const THRESHOLD: u8 = 3;
const DECIMALS: u8 = 8;

/// Five minutes, well inside `MAX_MAX_AGE_MS`. The clock is the vector's own
/// timestamp, so every package is current whatever this is; what is under
/// measurement is the work.
const MAX_AGE_MS: u64 = 300_000;

const BASE_ASSET: [u8; 32] = [0xB7; 32];
const QUOTE_ASSET: [u8; 32] = [0x05; 32];
const OWNER: [u8; 32] = [0x41; 32];
const PROGRAM_ID: [u32; 8] = [7; 8];

/// Stands in for the push aggregator's image id, which is what a consumer
/// registers and what the price account's address hashes. A fixed value rather
/// than `kanon_methods::AGGREGATOR_ID`, so a rebuild of the aggregator does not
/// move the read figures -- what is under measurement is the consumer's work,
/// and the address it derives is the same shape whatever the id is.
const AGGREGATOR_ID: [u32; 8] = [9; 8];

/// `65_000.00000000` on the `Q64.64` scale the aggregator publishes, so the
/// order below fills.
const PUBLISHED_Q64: u128 = (65_000u128 << 64) + 1;

/// A limit far below anything the capture carries, so the settlement fills.
///
/// What is being measured is the work a read does, and a refusal does less of
/// it: an order that could not fill returns an error and no post-states, which
/// the guest reports and every assertion here checks for.
const LIMIT_Q64: u128 = 1;

/// The stage numbers both guests answer to. They mean the same thing on both
/// sides wherever they can, so the two tables read side by side.
mod stage {
    pub const FLOOR: u8 = 0;
    pub const READ: u8 = 1;
    pub const SETTLE: u8 = 2;
    pub const REGISTRATION: u8 = 3;
    pub const ORDER_DECODE: u8 = 4;
    /// The thing each read cannot happen without: the roster rebuild on the pull
    /// side, the published account's decode on the push side.
    pub const PREREQUISITE: u8 = 5;
    pub const ORDER_WRITE: u8 = 6;
    /// Push side only. `read_price` takes a clock its caller decoded, so the
    /// decode separates; `verify_price` performs it internally, so on the pull
    /// side it is inside the read.
    pub const CLOCK: u8 = 7;
    /// Push side only. The two PDA derivations `read_price` opens with.
    pub const ADDRESS: u8 = 8;
}

/// Published in `COSTS.md`, and asserted here so the two cannot drift.
mod pull {
    /// The mode's read: rebuild the registered roster, then verify the payload
    /// against it. Five packages at a threshold of three.
    pub const READ: u64 = 3_041_385;
    /// What that read is of LEZ's per-transaction budget, as `COSTS.md` prints it.
    pub const BUDGET_SHARE: &str = "9.06%";
    /// `verify_price` without the roster rebuild that precedes it.
    ///
    /// Within 722 cycles of `cost.rs`'s 3,039,790 for the same five-signer
    /// payload, and that agreement is the point: a pull read *is* an update's
    /// verification, plus one clock decode and the library call around it.
    pub const VERIFY: u64 = 3_040_512;
    /// The whole `settle` body.
    pub const SETTLE: u64 = 3_048_613;
    /// What that body is of the per-transaction budget, as `COSTS.md` prints it.
    pub const SETTLE_BUDGET_SHARE: &str = "9.09%";
    /// What settling costs on top of reading.
    pub const BEYOND_READ: u64 = 7_228;
    pub const FLOOR: u64 = 147_529;
    pub const REGISTRATION: u64 = 3_385;
    pub const ORDER_DECODE: u64 = 2_031;
    /// `signer_addresses` and `config`, over a `FeedTrust` the setup decoded.
    ///
    /// Small because it is only the rebuild: an earlier draft measured 6,263 here
    /// and most of that was the trust account's decode and the order's, which
    /// have since moved into the setup that cancels.
    pub const ROSTER: u64 = 873;
    pub const ORDER_WRITE: u64 = 1_024;
}

/// The other mode, on the same terms. Also published in `COSTS.md`.
mod push {
    /// The mode's read: decode the clock, then read the published account
    /// against the registration the setup decoded.
    pub const READ: u64 = 6_803;
    /// What that read is of LEZ's per-transaction budget, as `COSTS.md` prints it.
    pub const BUDGET_SHARE: &str = "0.0203%";
    /// The whole `settle` body.
    pub const SETTLE: u64 = 13_404;
    /// What that body is of the per-transaction budget, as `COSTS.md` prints it.
    pub const SETTLE_BUDGET_SHARE: &str = "0.0399%";
    /// What settling costs on top of reading.
    pub const BEYOND_READ: u64 = 6_601;
    pub const FLOOR: u64 = 73_332;
    pub const REGISTRATION: u64 = 1_839;
    pub const ORDER_DECODE: u64 = 2_025;
    /// `OraclePriceAccount::try_from_slice`. `cost.rs` measures the same decode
    /// inside `submit_price` at 1,442; the gap is the `black_box` this stage
    /// needs and the difference between an isolated call and an inlined one.
    pub const PUBLISHED_DECODE: u64 = 1_551;
    pub const CLOCK: u64 = 265;
    /// `price_account_address`: two `compute_pda` calls, each a SHA-256 over its
    /// seeds. The single largest item in a push read, and the reason the claim
    /// that this mode "hashes nothing" was wrong.
    pub const ADDRESS: u64 = 3_656;
    pub const ORDER_WRITE: u64 = 1_022;

    /// How many times a pull read costs what a push read costs.
    ///
    /// The figure M3-08 exists to produce, and the reason P2 asks for costs per
    /// mode rather than once: the two are not variations on one number.
    pub const PULL_IS_THIS_MANY_TIMES_DEARER: u64 = 447;
}

use lez::CYCLE_BUDGET as BUDGET;

/// A read's share of that budget, formatted the way `COSTS.md` prints it.
///
/// Formatted rather than compared as a number so that the published string and
/// the asserted one are the same object: a percentage nobody asserts is a
/// percentage that goes stale silently.
fn budget_share(cycles: u64) -> String {
    let pct = cycles as f64 * 100.0 / BUDGET as f64;
    if pct >= 1.0 {
        format!("{pct:.2}%")
    } else {
        format!("{pct:.4}%")
    }
}

fn vector() -> Vector {
    vectors::named(FEED)
}

fn feed_id(vector: &Vector) -> [u8; 32] {
    let bytes = vector.feed_id.as_bytes();
    assert!(
        bytes.len() <= 32,
        "the capture's feed id {:?} is {} bytes, and a feed id is 32",
        vector.feed_id,
        bytes.len()
    );
    let mut id = [0u8; 32];
    id[..bytes.len()].copy_from_slice(bytes);
    id
}

/// The trust account the authority would have registered for this feed.
fn trust(vector: &Vector) -> Vec<u8> {
    let registered = FeedTrust {
        data_service_id: "redstone-primary-prod".to_owned(),
        feed_id: feed_id(vector),
        base_asset: BASE_ASSET,
        quote_asset: QUOTE_ASSET,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers: vector.signers.iter().map(|s| *s.as_bytes()).collect(),
        threshold: THRESHOLD,
    };
    let mut out = Vec::new();
    registered
        .serialize(&mut out)
        .expect("a trust account encodes");
    out
}

/// An open order against that feed, priced on the same scale.
fn order(vector: &Vector) -> Vec<u8> {
    let open = OrderAccount {
        owner: OWNER,
        feed_id: feed_id(vector),
        base_asset: BASE_ASSET,
        quote_asset: QUOTE_ASSET,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        limit_price_q64: LIMIT_Q64,
        filled: false,
    };
    let mut out = Vec::new();
    open.serialize(&mut out).expect("an order encodes");
    out
}

/// The registration this consumer's authority would have written.
fn price_source(vector: &Vector) -> Vec<u8> {
    let registered = PriceSource {
        feed_id: feed_id(vector),
        aggregator: ProgramId::from(AGGREGATOR_ID),
        base_asset: BASE_ASSET,
        quote_asset: QUOTE_ASSET,
        max_age_ms: MAX_AGE_MS,
    };
    let mut out = Vec::new();
    registered
        .serialize(&mut out)
        .expect("a registration encodes");
    out
}

/// The address that registration publishes to, derived the way the consumer
/// derives it rather than spelled out here.
fn price_address(vector: &Vector) -> [u8; 32] {
    *price_account_address(&ProgramId::from(AGGREGATOR_ID), &feed_id(vector)).value()
}

/// The account the push aggregator would have written for this feed.
///
/// Timestamped at the same instant the pull side's payload is, so both modes are
/// reading a price of the same age and neither figure includes a different
/// distance from the clock.
fn published_price(vector: &Vector) -> Vec<u8> {
    let published = OraclePriceAccount {
        base_asset: AccountId::new(BASE_ASSET),
        quote_asset: AccountId::new(QUOTE_ASSET),
        price: PUBLISHED_Q64,
        timestamp: vector.timestamp_ms,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    };
    let mut out = Vec::new();
    published.serialize(&mut out).expect("a price encodes");
    out
}

/// An open order against that registration, on the same scale.
fn push_order(vector: &Vector) -> Vec<u8> {
    let open = reference_consumer_aggregator_read::order::OrderAccount {
        owner: OWNER,
        feed_id: feed_id(vector),
        base_asset: BASE_ASSET,
        quote_asset: QUOTE_ASSET,
        max_age_ms: MAX_AGE_MS,
        limit_price_q64: LIMIT_Q64,
        filled: false,
    };
    let mut out = Vec::new();
    open.serialize(&mut out).expect("an order encodes");
    out
}

/// The clock account's sixteen bytes: a block id, then the timestamp.
fn clock(now_ms: u64) -> Vec<u8> {
    let mut out = 1u64.to_le_bytes().to_vec();
    out.extend_from_slice(&now_ms.to_le_bytes());
    out
}

/// Which consumer is under measurement.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Mode {
    Pull,
    Push,
}

impl Mode {
    /// How many accounts that mode's settlement hands back. The assertion is on
    /// the count rather than on `Ok`, because a refusal returns no post-states
    /// and would otherwise read as a very cheap settlement.
    const fn post_states(self) -> u32 {
        match self {
            // The order, the trust account and the clock.
            Self::Pull => 3,
            // The order, the source, the price account and the clock.
            Self::Push => 4,
        }
    }
}

/// Cycles for one stage, and what the guest reported about its own run.
///
/// Memoised for the reason `cost.rs` memoises: a zkVM execution is a
/// deterministic function of the ELF and its input, so a repeat is not a check.
fn run(mode: Mode, stage: u8) -> (u64, (u32, u64, u32)) {
    type Slot = Arc<OnceLock<(u64, (u32, u64, u32))>>;
    static MEASURED: OnceLock<Mutex<HashMap<(Mode, u8), Slot>>> = OnceLock::new();

    let slot = MEASURED
        .get_or_init(Mutex::default)
        .lock()
        .expect("not poisoned")
        .entry((mode, stage))
        .or_default()
        .clone();
    *slot.get_or_init(|| execute(mode, stage))
}

fn execute(mode: Mode, stage: u8) -> (u64, (u32, u64, u32)) {
    let vector = vector();
    let env = match mode {
        Mode::Pull => {
            let input = (
                stage,
                vector.payload.clone(),
                order(&vector),
                trust(&vector),
                clock(vector.timestamp_ms),
                CLOCK_ACCOUNT_ID.to_vec(),
                feed_id(&vector).to_vec(),
                PROGRAM_ID,
            );
            ExecutorEnv::builder().write(&input).expect("input").build()
        }
        Mode::Push => {
            let input = (
                stage,
                push_order(&vector),
                price_source(&vector),
                published_price(&vector),
                clock(vector.timestamp_ms),
                CLOCK_ACCOUNT_ID.to_vec(),
                price_address(&vector).to_vec(),
                feed_id(&vector).to_vec(),
                AGGREGATOR_ID,
                PROGRAM_ID,
            );
            ExecutorEnv::builder().write(&input).expect("input").build()
        }
    }
    .expect("env");

    let elf = match mode {
        Mode::Pull => PULL_COST_ELF,
        Mode::Push => READ_COST_ELF,
    };
    let session = default_executor().execute(env, elf).expect("execution");
    let report: (u32, u64, u32) = session.journal.decode().expect("journal");
    (session.cycles(), report)
}

/// Cycles for a stage that is required to have done its work.
fn cycles(mode: Mode, stage: u8) -> u64 {
    let (cycles, (did_work, _value, post_states)) = run(mode, stage);
    if stage != stage::FLOOR {
        assert!(
            did_work > 0,
            "{mode:?} stage {stage} reported no work, so it took an error path; a refusal \
             does less work than a read and would read as a cheap component rather than as \
             a failure"
        );
    }
    if stage == stage::SETTLE {
        assert_eq!(
            post_states,
            mode.post_states(),
            "{mode:?} settlement returned {post_states} post-states, which means it refused"
        );
    }
    cycles
}

#[test]
fn a_pull_read_costs_what_is_published() {
    let (floor, read) = (
        cycles(Mode::Pull, stage::FLOOR),
        cycles(Mode::Pull, stage::READ),
    );
    assert_eq!(floor, pull::FLOOR, "the harness floor moved");
    assert_eq!(
        read - floor,
        pull::READ,
        "a pull read moved. It is a verification, so this figure tracks `verify_feed`: \
         check `cost.rs` before assuming this guest changed"
    );
    assert_eq!(
        budget_share(read - floor),
        pull::BUDGET_SHARE,
        "a pull read's share of the per-transaction budget moved"
    );
}

#[test]
fn the_roster_rebuild_is_not_where_a_pull_read_goes() {
    let (floor, roster, read) = (
        cycles(Mode::Pull, stage::FLOOR),
        cycles(Mode::Pull, stage::PREREQUISITE),
        cycles(Mode::Pull, stage::READ),
    );
    assert_eq!(roster - floor, pull::ROSTER, "the roster rebuild moved");
    assert_eq!(read - roster, pull::VERIFY, "the verification itself moved");
}

#[test]
fn settling_costs_what_is_published_on_top_of_reading() {
    let (floor, read, settle) = (
        cycles(Mode::Pull, stage::FLOOR),
        cycles(Mode::Pull, stage::READ),
        cycles(Mode::Pull, stage::SETTLE),
    );
    assert_eq!(settle - floor, pull::SETTLE, "the `settle` body moved");
    assert_eq!(
        budget_share(settle - floor),
        pull::SETTLE_BUDGET_SHARE,
        "a pull settlement's share of the per-transaction budget moved"
    );
    assert_eq!(
        settle - read,
        pull::BEYOND_READ,
        "what settling adds over reading moved"
    );
}

#[test]
fn the_operations_the_residual_contains_cost_what_is_published() {
    let floor = cycles(Mode::Pull, stage::FLOOR);
    assert_eq!(
        cycles(Mode::Pull, stage::REGISTRATION) - floor,
        pull::REGISTRATION,
        "`trust::read` moved"
    );
    assert_eq!(
        cycles(Mode::Pull, stage::ORDER_DECODE) - floor,
        pull::ORDER_DECODE,
        "the order decode moved"
    );
    assert_eq!(
        cycles(Mode::Pull, stage::ORDER_WRITE) - floor,
        pull::ORDER_WRITE,
        "filling and encoding the order moved"
    );
}

/// Prints the table `COSTS.md` publishes, so a published figure and an asserted
/// figure cannot drift apart.
///
/// ```sh
/// cargo test --release -p kanon-methods --test read_cost -- --nocapture the_read_table
/// ```
#[test]
fn the_read_table_is_reproducible() {
    let floor = cycles(Mode::Pull, stage::FLOOR);
    let rows = [
        (
            "pull: the mode's read",
            cycles(Mode::Pull, stage::READ) - floor,
        ),
        (
            "pull:   of which the roster rebuild",
            cycles(Mode::Pull, stage::PREREQUISITE) - floor,
        ),
        (
            "pull:   of which verify_price",
            cycles(Mode::Pull, stage::READ) - cycles(Mode::Pull, stage::PREREQUISITE),
        ),
        (
            "pull: settling, beyond the read",
            cycles(Mode::Pull, stage::SETTLE) - cycles(Mode::Pull, stage::READ),
        ),
        (
            "pull: the whole settle body",
            cycles(Mode::Pull, stage::SETTLE) - floor,
        ),
        (
            "pull: trust::read, isolated",
            cycles(Mode::Pull, stage::REGISTRATION) - floor,
        ),
        (
            "pull: the order decode, isolated",
            cycles(Mode::Pull, stage::ORDER_DECODE) - floor,
        ),
        (
            "pull: fill and encode the order, isolated",
            cycles(Mode::Pull, stage::ORDER_WRITE) - floor,
        ),
        ("pull: harness floor, subtracted out", floor),
    ];
    let floor_push = cycles(Mode::Push, stage::FLOOR);
    let push = [
        (
            "push: the mode's read",
            cycles(Mode::Push, stage::READ) - floor_push,
        ),
        (
            "push:   of which the clock decode",
            cycles(Mode::Push, stage::CLOCK) - floor_push,
        ),
        (
            "push:   of which the published decode",
            cycles(Mode::Push, stage::PREREQUISITE) - floor_push,
        ),
        (
            "push:   of which the address derivation",
            cycles(Mode::Push, stage::ADDRESS) - floor_push,
        ),
        (
            "push: settling, beyond the read",
            cycles(Mode::Push, stage::SETTLE) - cycles(Mode::Push, stage::READ),
        ),
        (
            "push: the whole settle body",
            cycles(Mode::Push, stage::SETTLE) - floor_push,
        ),
        (
            "push: source::read, isolated",
            cycles(Mode::Push, stage::REGISTRATION) - floor_push,
        ),
        (
            "push: the order decode, isolated",
            cycles(Mode::Push, stage::ORDER_DECODE) - floor_push,
        ),
        (
            "push: fill and encode the order, isolated",
            cycles(Mode::Push, stage::ORDER_WRITE) - floor_push,
        ),
        ("push: harness floor, subtracted out", floor_push),
    ];
    println!();
    for (name, value) in rows.iter().chain(push.iter()) {
        println!("| {name:42} | {value:>11} |");
    }
    // The two percentages `COSTS.md` publishes, printed by the code that asserts
    // them so the table is reproducible in full rather than in most of its rows.
    for (name, share) in [
        (
            "pull: the read, against LEZ's budget",
            budget_share(cycles(Mode::Pull, stage::READ) - floor),
        ),
        (
            "pull: the settle body, against LEZ's budget",
            budget_share(cycles(Mode::Pull, stage::SETTLE) - floor),
        ),
        (
            "push: the read, against LEZ's budget",
            budget_share(cycles(Mode::Push, stage::READ) - floor_push),
        ),
        (
            "push: the settle body, against LEZ's budget",
            budget_share(cycles(Mode::Push, stage::SETTLE) - floor_push),
        ),
    ] {
        println!("| {name:42} | {share:>11} |");
    }
    println!();
}

#[test]
fn a_push_mode_read_costs_what_is_published() {
    let (floor, read) = (
        cycles(Mode::Push, stage::FLOOR),
        cycles(Mode::Push, stage::READ),
    );
    assert_eq!(floor, push::FLOOR, "the harness floor moved");
    assert_eq!(read - floor, push::READ, "a push-mode read moved");
    assert_eq!(
        budget_share(read - floor),
        push::BUDGET_SHARE,
        "a push read's share of the per-transaction budget moved"
    );
}

#[test]
fn the_operations_a_push_read_cannot_skip_cost_what_is_published() {
    let floor = cycles(Mode::Push, stage::FLOOR);
    assert_eq!(
        cycles(Mode::Push, stage::CLOCK) - floor,
        push::CLOCK,
        "the clock decode moved"
    );
    assert_eq!(
        cycles(Mode::Push, stage::PREREQUISITE) - floor,
        push::PUBLISHED_DECODE,
        "the published account's decode moved"
    );
    assert_eq!(
        cycles(Mode::Push, stage::ADDRESS) - floor,
        push::ADDRESS,
        "deriving the price account's address moved"
    );
}

#[test]
fn settling_on_the_push_side_costs_what_is_published_on_top_of_reading() {
    let (floor, read, settle) = (
        cycles(Mode::Push, stage::FLOOR),
        cycles(Mode::Push, stage::READ),
        cycles(Mode::Push, stage::SETTLE),
    );
    assert_eq!(settle - floor, push::SETTLE, "the `settle` body moved");
    assert_eq!(
        budget_share(settle - floor),
        push::SETTLE_BUDGET_SHARE,
        "a push settlement's share of the per-transaction budget moved"
    );
    assert_eq!(
        settle - read,
        push::BEYOND_READ,
        "what settling adds over reading moved"
    );
}

#[test]
fn the_push_sides_isolated_operations_cost_what_is_published() {
    let floor = cycles(Mode::Push, stage::FLOOR);
    assert_eq!(
        cycles(Mode::Push, stage::REGISTRATION) - floor,
        push::REGISTRATION,
        "`source::read` moved"
    );
    assert_eq!(
        cycles(Mode::Push, stage::ORDER_DECODE) - floor,
        push::ORDER_DECODE,
        "the order decode moved"
    );
    assert_eq!(
        cycles(Mode::Push, stage::ORDER_WRITE) - floor,
        push::ORDER_WRITE,
        "filling and encoding the order moved"
    );
}

/// The requirement's own question: what does the mode cost a consumer?
///
/// Asserted rather than left to the reader's arithmetic, because it is the one
/// figure a reader takes away and the two modes' rows are far apart in the table.
#[test]
fn a_pull_read_costs_hundreds_of_times_more_than_a_push_read() {
    let pull_read = cycles(Mode::Pull, stage::READ) - cycles(Mode::Pull, stage::FLOOR);
    let push_read = cycles(Mode::Push, stage::READ) - cycles(Mode::Push, stage::FLOOR);
    assert_eq!(
        pull_read / push_read,
        push::PULL_IS_THIS_MANY_TIMES_DEARER,
        "the ratio between the two modes' reads moved: {pull_read} against {push_read}"
    );
}

/// Almost every cycle of the difference is recovery, and nothing else comes close.
///
/// The claim `COSTS.md` makes and the one a reader is most likely to doubt: that
/// the gap is not a pile of small differences but one component. It is 96% of the
/// gap rather than all of it, and the remaining 4% is stated as such in both
/// places rather than rounded away.
#[test]
fn the_gap_between_the_modes_is_signature_recovery() {
    let pull_read = cycles(Mode::Pull, stage::READ) - cycles(Mode::Pull, stage::FLOOR);
    let push_read = cycles(Mode::Push, stage::READ) - cycles(Mode::Push, stage::FLOOR);
    let gap = pull_read - push_read;
    // In thousandths rather than whole percent. Integer-flooring a percentage
    // let the gap drift by up to about 1% -- some 30,000 cycles -- without the
    // assertion noticing, which is a wide enough band to hide a real change.
    let share = precompile::FIVE_SIGNER_RECOVERY * 1_000 / gap;
    assert_eq!(
        share, 963,
        "recovery is {share} thousandths of the {gap}-cycle gap between the modes, not \
         963; either `cost.rs`'s recovery row moved or something else grew"
    );
}

/// What a native ECDSA and keccak256 precompile would be worth to a reader
/// (M3-09, P3).
///
/// The other half of the delta; `cost.rs` carries the update path and the
/// component arithmetic. Two terms are assumed, and both are properties of a
/// primitive nobody has built: what a call into it costs, and what it hands back,
/// which decides how many calls there are. So the figures below are published at
/// both ends of a band and at zero. `[M3-09:01]` records
/// why they are not collapsed into one number.
mod precompile {
    /// The two removable rows of `cost.rs`'s five-signer component table.
    ///
    /// Copied, because each file in `tests/` is its own crate and there is
    /// nothing to import. The copies are not self-checking: if those rows move,
    /// these constants do not follow and nothing here notices, so the two have
    /// to be changed together. `per_component_cycles_are_unchanged` is what
    /// stops the originals moving unobserved.
    ///
    /// **They are measured in `verify_cost` and subtracted from `pull_cost`**, so
    /// on this side the removable term is imported rather than measured in the
    /// guest it is taken out of. What bounds the import is
    /// `the_roster_rebuild_is_not_where_a_pull_read_goes`: `pull::VERIFY` is
    /// 3,040,512 against `cost.rs`'s 3,039,790 for the same payload, 722 cycles
    /// apart, so the two guests are doing the same cryptographic work to about a
    /// part in four thousand. The residuals below inherit that, which is fine for
    /// a reduction factor and not fine for reading small differences between
    /// them.
    pub use super::lez::{FIVE_SIGNER_RECOVERY, REMOVABLE};

    /// One keccak256 and one recovery per package, five packages.
    pub const CALLS: u64 = 10;

    /// A free syscall, then `m0`'s band. The first is not achievable and is not
    /// meant to be: it is the bound that holds whatever a real precompile costs.
    pub const SYSCALL: [u64; 3] = [0, 1_000, 10_000];

    /// What would remain of a pull read at each of those, and the reduction in
    /// tenths: 97.7x, 74.0x, 23.2x.
    ///
    /// Rounded rather than floored. Integer division truncates, which published
    /// 73.97 as 73.9 — a figure that presents as 74.0 anywhere else, so the
    /// convention read as an arithmetic slip rather than as a convention. It is
    /// the same objection this file already makes about flooring the recovery
    /// share, one table over.
    pub const PULL_READ_REMAINS: [u64; 3] = [31_115, 41_115, 131_115];
    pub const PULL_READ_REDUCTION_TENTHS: [u64; 3] = [977, 740, 232];

    /// The same for the whole `settle` body. It lands 199 cycles from what
    /// `cost.rs` computes for a push update, which is not a finding: those are
    /// residuals from different guests, disagreeing by 42 and 722 cycles on
    /// identical verification, and a settlement is not a submission. Both being
    /// small is the claim; their being equal is not.
    pub const PULL_SETTLE_REMAINS: [u64; 3] = [38_343, 48_343, 138_343];
}

/// A reduction factor in tenths, rounded to the nearest rather than truncated.
fn tenths(before: u64, after: u64) -> u64 {
    (before * 10 + after / 2) / after
}

/// The pull side of P3's per-mode delta.
///
/// A pull consumer pays verification on every read, so this is what a precompile
/// would be worth to it *per read* rather than once per update.
#[test]
fn what_a_precompile_would_leave_of_a_pull_read() {
    let floor = cycles(Mode::Pull, stage::FLOOR);
    let read = cycles(Mode::Pull, stage::READ) - floor;
    let settle = cycles(Mode::Pull, stage::SETTLE) - floor;

    for (i, syscall) in precompile::SYSCALL.into_iter().enumerate() {
        // Checked, because `REMOVABLE` is a constant and `read` is measured: if the
        // guests ever take risc0's keccak coprocessor -- which `COSTS.md` discusses
        // as a live option worth about a sevenfold cut to that row -- `read` falls
        // and the constant does not, and an overflow backtrace is a worse way to
        // learn that than the message below.
        let remains = read.checked_sub(precompile::REMOVABLE).unwrap_or_else(|| {
            panic!(
                "a pull read is {read} cycles, less than the {} a precompile was \
                     going to remove: the read no longer contains all of what \
                     `cost.rs` measures as removable, so `REMOVABLE` describes a \
                     different build",
                precompile::REMOVABLE
            )
        }) + precompile::CALLS * syscall;
        assert_eq!(
            remains,
            precompile::PULL_READ_REMAINS[i],
            "a pull read with a precompile at {syscall} cycles a call moved"
        );
        assert_eq!(
            tenths(read, remains),
            precompile::PULL_READ_REDUCTION_TENTHS[i],
            "the reduction at {syscall} cycles a call moved"
        );
        let settle_remains = settle
            .checked_sub(precompile::REMOVABLE)
            .expect("a settlement contains its own read, and the read contains the removable rows")
            + precompile::CALLS * syscall;
        assert_eq!(
            settle_remains,
            precompile::PULL_SETTLE_REMAINS[i],
            "a pull settlement with a precompile at {syscall} cycles a call moved"
        );
    }
}

/// The push side: a precompile is worth nothing to a consumer's read.
///
/// Not an arithmetic identity dressed as a test. What it asserts is that a push
/// read is cheaper than a *single* signature recovery, which is only possible if
/// it performs none — so the mode has nothing for an ECDSA precompile to remove,
/// and the saving lands on the aggregator's update instead, once per update
/// rather than once per read.
#[test]
fn a_precompile_is_worth_nothing_to_a_push_read() {
    let read = cycles(Mode::Push, stage::READ) - cycles(Mode::Push, stage::FLOOR);
    let one_recovery = precompile::FIVE_SIGNER_RECOVERY / 5;
    assert!(
        read < one_recovery,
        "a push read is {read} cycles against {one_recovery} for one recovery, so it may \
         now contain cryptography an ECDSA precompile would address"
    );
    // What hashing it does do is SHA-256, for the PDA derivation, and neither
    // primitive RFP-020 names reaches SHA-256. So the majority of a push read
    // survives every precompile in question -- which is the claim `COSTS.md`
    // makes, and this is what would fail if the derivation stopped dominating.
    // The measured derivation rather than `push::ADDRESS`, which is a pinned
    // constant and so cannot fall when the thing it describes does: comparing it
    // against a fresh `read` would keep passing while the claim stopped being true.
    // The margin is thin enough for that to matter -- 3,656 of 6,803 is 53.7%.
    let derivation = cycles(Mode::Push, stage::ADDRESS) - cycles(Mode::Push, stage::FLOOR);
    assert!(
        derivation * 2 > read,
        "the address derivation is {derivation} cycles of a {read}-cycle read, no longer \
         the majority of it"
    );
}

/// Prints the read half of `COSTS.md`'s precompile table.
///
/// ```sh
/// cargo test --release -p kanon-methods --test read_cost \
///     -- --nocapture the_precompile_read_table
/// ```
#[test]
fn the_precompile_read_table_is_reproducible() {
    let pull = cycles(Mode::Pull, stage::READ) - cycles(Mode::Pull, stage::FLOOR);
    let push = cycles(Mode::Push, stage::READ) - cycles(Mode::Push, stage::FLOOR);

    println!("\n| a read | pull | push |");
    println!("| --- | ---: | ---: |");
    println!("| today | {pull} | {push} |");
    println!("| removable | {} | 0 |", precompile::REMOVABLE);
    for syscall in precompile::SYSCALL {
        let remains = pull - precompile::REMOVABLE + precompile::CALLS * syscall;
        println!(
            "| with a precompile at {syscall} cycles a call | {remains} ({}.{}x) | {push} (1x) |",
            tenths(pull, remains) / 10,
            tenths(pull, remains) % 10
        );
    }
    // The settlement rows too, because `COSTS.md` cites the pull settlement's 38,343
    // when it puts the two residuals beside each other, and a figure nothing prints
    // is a figure a reader cannot check against the code that asserts it.
    let settle = cycles(Mode::Pull, stage::SETTLE) - cycles(Mode::Pull, stage::FLOOR);
    println!("\n| a pull `settle` body | cycles |");
    println!("| --- | ---: |");
    println!("| today | {settle} |");
    for syscall in precompile::SYSCALL {
        println!(
            "| with a precompile at {syscall} cycles a call | {} |",
            settle - precompile::REMOVABLE + precompile::CALLS * syscall
        );
    }
    println!();
}
