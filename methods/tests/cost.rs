//! Per-component cycle costs of the shipped verification path, measured and
//! pinned.
//!
//! `m0/cost-baseline` measured the two primitives in isolation and settled the
//! accelerator configuration. This measures `verifier_core::verify_feed` itself,
//! in the product guest, over payloads RedStone published, and answers a
//! different question: where does an update's cost actually go, component by
//! component.
//!
//! # Method
//!
//! `methods/guest/src/bin/verify_cost.rs` runs a **prefix** of the pipeline,
//! selected by a stage number, with identical setup and identical input in every
//! stage. Subtracting two stages gives the component between them, with zkVM
//! startup, input deserialization and the journal commit cancelling out.
//!
//! ```text
//! decode                  = stage 1 - stage 0
//! keccak256               = stage 2 - stage 1
//! recovery                = stage 3 - stage 2
//! membership              = stage 4 - stage 3
//! the rest of verify_feed = stage 5 - stage 4
//! whole update            = stage 5 - stage 0
//! ```
//!
//! The fifth row names the remainder rather than leaving it implicit: the
//! median, value sanity, the staleness window, the threshold ladder and the
//! Q64.64 conversion are real work, and a table whose rows do not add up to its
//! total is a table with somewhere to hide. Stage 6 is outside that chain — it
//! is stage 4 plus one Q64.64 conversion, which is the largest single item in
//! the remainder.
//!
//! # Why these assertions
//!
//! Cycle counts are a deterministic function of the guest ELF and its input, so
//! they are pinned by exact equality rather than by a ceiling, for the reason
//! `m0`'s guardrails give: a ceiling passes quietly when a figure improves for a
//! reason nobody has understood yet. A failure here is a prompt to re-measure and
//! re-publish `COSTS.md`, not necessarily a defect.
//!
//! Two of them are ADR 6's, which `methods/guest/Cargo.toml` records as
//! outstanding. The mixed configuration is a `[patch.crates-io]` section, and
//! such a section fails silently — a moved tag or a dropped line changes the cost
//! profile with a green build. Once keccak256 and recovery are separate rows,
//! both failure directions are visible in cycles alone:
//!
//! - losing the secp256k1 patch multiplies the recovery row by about 20;
//! - gaining the keccak patch divides the keccak row by about 7.
//!
//! The second is the one that needed a proof-size ceiling while the workload was
//! a single opaque number: the coprocessor's work is invisible to the cycle
//! counter, so gaining it makes the *total* cheaper. Per component it is
//! unmistakable.
//!
//! ```sh
//! cargo test --release -p kanon-methods
//! cargo test --release -p kanon-methods -- --nocapture the_cost_table   # regenerate COSTS.md
//! ```

use kanon_methods::VERIFY_COST_ELF;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use risc0_zkvm::{default_executor, ExecutorEnv};
use verifier_core::decode::{EMPTY_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES, REDSTONE_MARKER};
use verifier_core::feed::MAX_MAX_AGE_MS;
use verifier_core::feed::MAX_RECOVERIES;

/// The M1-21 capture. Real packages, real signatures, one data point each, so a
/// package's signable span is the 77 bytes `m0` measured keccak256 over. Read
/// through the same module `verifier-core`'s conformance suite reads it with,
/// so the two cannot disagree about the file's shape.
#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

/// RedStone's scale for `redstone-primary-prod`.
const DECIMALS: u8 = 8;

/// The feed the published table is measured on. BTC is the one RFP-020 names
/// first, and `the_table_is_not_particular_to_one_feed` checks the choice does
/// not matter.
const FEED: &str = "BTC";

/// Signer counts. 3 is RFP-020's default threshold and the row `m0`'s 3-of-N
/// figure compares against; 1 is the unit cost; 5 is what the capture holds.
const SIGNER_COUNTS: [usize; 3] = [1, 3, 5];

mod stage {
    pub const FLOOR: u8 = 0;
    pub const DECODE: u8 = 1;
    pub const HASH: u8 = 2;
    pub const RECOVER: u8 = 3;
    pub const MEMBERSHIP: u8 = 4;
    pub const VERIFY: u8 = 5;
    /// Not part of the prefix chain: stage 4 plus one Q64.64 conversion.
    pub const SCALE: u8 = 6;
}

/// Measured on risc0-zkvm 3.0.5, guest rustc 1.97.0. Update together with
/// `COSTS.md`.
///
/// `adr/0008` records that cycle counts are identical on 3.0.5 and 3.0.6, so a
/// local run on either reproduces these.
mod expected {
    /// Input, setup and journal, with no verification at all.
    ///
    /// Larger than `m0`'s 2,940-cycle empty guest, and growing with the signer
    /// count, because the payload and the signer set arrive through
    /// `env::read()` and RISC Zero's deserializer walks them a word at a time.
    /// It is the harness's cost, not the verifier's: it is subtracted out of
    /// every component, and a program reading a payload from an account rather
    /// than from the guest's input stream would pay something different.
    pub const FLOOR: [(usize, u64); 3] = [(1, 25_324), (3, 62_295), (5, 99_491)];

    /// Per signer count: decode, keccak256, recovery, membership, then
    /// everything else `verify_feed` does.
    pub const COMPONENTS: [(usize, [u64; 5]); 3] = [
        (1, [582, 17_476, 585_276, 162, 17_773]),
        (3, [1_495, 52_428, 1_755_580, 534, 21_601]),
        (5, [2_408, 87_380, 2_922_890, 970, 26_142]),
    ];

    /// The whole update, floor subtracted.
    pub const TOTAL: [(usize, u64); 3] = [(1, 621_269), (3, 1_831_638), (5, 3_039_790)];

    /// One Q64.64 conversion: the largest single item in the remainder, and the
    /// only one worth naming separately.
    pub const SCALING: u64 = 11_294;

    /// `MAX_NUM_CYCLES_PUBLIC_EXECUTION`, the cycles a LEZ public transaction
    /// gets. Recorded in `m0/lez-probe/README.md` and the figure P1 is measured
    /// against.
    pub const LEZ_CYCLE_BUDGET: u64 = 33_554_432;

    /// What the same rows would read if the wrong `[patch.crates-io]` were in
    /// force, from `m0`'s software and accelerated measurements: one software
    /// recovery, and one accelerated keccak256 over the 77-byte signable span.
    pub const SOFTWARE_RECOVERY: u64 = 11_074_701;
    pub const ACCELERATED_KECCAK: u64 = 2_527;
}

fn vector() -> Vector {
    vectors::named(FEED)
}

/// The captured payload cut down to its first `n` packages.
///
/// Every package in the capture carries one data point of the same width, so
/// they are a fixed stride apart and the first `n` of them are a valid payload
/// once the count is re-emitted. Signatures are untouched, so the shortened
/// payload still recovers to `signers[..n]`.
fn payload_with(vector: &Vector, n: usize) -> Vec<u8> {
    let body_len = vector.payload.len() - EMPTY_ENVELOPE_BYTES;
    let count = vector.signers.len();
    assert_eq!(body_len % count, 0, "packages are not a fixed stride apart");
    let stride = body_len / count;

    let mut out = vector.payload[..n * stride].to_vec();
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&REDSTONE_MARKER);
    out
}

/// Cycles for one stage over an `n`-package payload, and what the guest reported.
///
/// Memoised. A zkVM execution is a deterministic function of the guest ELF and
/// its input, and the input here is fully described by the feed, the stage and
/// the signer count -- so the eight tests between them asked for 45 distinct
/// executions 144 times. Nothing about a repeat is a check: it is
/// the same ELF over the same bytes, and it would have to return the same
/// number for the arithmetic in `components` to mean anything at all.
fn run(vector: &Vector, stage: u8, n: usize) -> (u64, u32) {
    type Key = (String, u8, usize);
    type Slot = Arc<OnceLock<(u64, u32)>>;
    static MEASURED: OnceLock<Mutex<HashMap<Key, Slot>>> = OnceLock::new();

    // A cell per input, taken under the lock; the execution happens outside it.
    // Two threads asking for the same measurement share one execution, because
    // the second blocks in `get_or_init` until the first has filled the cell,
    // while threads asking for different ones still run at the same time.
    let slot = MEASURED
        .get_or_init(Mutex::default)
        .lock()
        .expect("not poisoned")
        .entry((vector.feed_id.clone(), stage, n))
        .or_default()
        .clone();
    *slot.get_or_init(|| execute(vector, stage, n))
}

fn execute(vector: &Vector, stage: u8, n: usize) -> (u64, u32) {
    let signer_bytes: Vec<u8> = vector.signers[..n]
        .iter()
        .flat_map(|s| *s.as_bytes())
        .collect();

    let input = (
        stage,
        payload_with(vector, n),
        vector.feed_id.as_bytes().to_vec(),
        signer_bytes,
        n as u8,
        DECIMALS,
        vector.timestamp_ms,
        // The clock above is the vector's own timestamp, so every package is
        // current whatever the bound. The widest legal one, because what is
        // under measurement is the work and not the clock.
        MAX_MAX_AGE_MS,
    );

    let env = ExecutorEnv::builder()
        .write(&input)
        .expect("input")
        .build()
        .expect("env");
    let session = default_executor()
        .execute(env, VERIFY_COST_ELF)
        .expect("execution");
    let (reported, _checksum): (u32, u64) = session.journal.decode().expect("journal");
    (session.cycles(), reported)
}

fn cycles(vector: &Vector, stage: u8, n: usize) -> u64 {
    let (cycles, reported) = run(vector, stage, n);
    if stage != stage::FLOOR {
        assert_eq!(
            reported as usize, n,
            "stage {stage} got through {reported} of {n} packages; \
             a run that took an early exit is not a measurement of the work"
        );
    }
    cycles
}

/// `[decode, keccak256, recovery, membership, the rest of verify_feed]`.
fn components(vector: &Vector, n: usize) -> [u64; 5] {
    let at = |s| cycles(vector, s, n);
    let (floor, decode, hash, recover, membership, verify) = (
        at(stage::FLOOR),
        at(stage::DECODE),
        at(stage::HASH),
        at(stage::RECOVER),
        at(stage::MEMBERSHIP),
        at(stage::VERIFY),
    );
    [
        decode - floor,
        hash - decode,
        recover - hash,
        membership - recover,
        verify - membership,
    ]
}

#[test]
fn the_zkvm_floor_is_unchanged() {
    let vector = vector();
    for (n, want) in expected::FLOOR {
        assert_eq!(cycles(&vector, stage::FLOOR, n), want, "{n} signers");
    }
}

#[test]
fn per_component_cycles_are_unchanged() {
    let vector = vector();
    for (n, want) in expected::COMPONENTS {
        assert_eq!(components(&vector, n), want, "{n} signers");
    }
    for (n, want) in expected::TOTAL {
        assert_eq!(
            cycles(&vector, stage::VERIFY, n) - cycles(&vector, stage::FLOOR, n),
            want,
            "{n} signers, whole update"
        );
    }
}

/// ADR 6's first ceiling. Losing the `k256` patch would multiply this row by
/// about twenty, and nothing else in CI would notice.
///
/// A bound rather than an exact figure: the absolute numbers are already pinned
/// by `per_component_cycles_are_unchanged`, and what this test says is which
/// implementation is in force. Recovery is not exactly linear in the signer
/// count — its cost depends on the scalar — so an exact per-signer figure would
/// be three different constants saying one thing.
#[test]
fn recovery_is_still_the_accelerated_implementation() {
    let vector = vector();
    for n in SIGNER_COUNTS {
        let per_signer = components(&vector, n)[2] / n as u64;
        assert!(
            per_signer * 10 < expected::SOFTWARE_RECOVERY,
            "{n} signers: recovery is {per_signer} cycles per signer, which is not \
             the twenty-fold saving the secp256k1 patch gives; software recovery \
             is {} cycles",
            expected::SOFTWARE_RECOVERY
        );
    }
}

/// ADR 6's second ceiling, and the one that used to need a proof to see.
/// Gaining the keccak coprocessor would divide this row by about seven and make
/// the *total* cheaper, which no ceiling on the total could catch.
#[test]
fn the_keccak_accelerator_is_still_declined() {
    let vector = vector();
    for n in SIGNER_COUNTS {
        let per_package = components(&vector, n)[1] / n as u64;
        assert!(
            per_package > expected::ACCELERATED_KECCAK * 4,
            "{n} packages: keccak256 is {per_package} cycles per package, near the \
             {} of the accelerated implementation rather than the software one \
             this configuration declares",
            expected::ACCELERATED_KECCAK
        );
    }
}

/// The Q64.64 conversion, isolated out of the remainder row.
///
/// It runs once per update rather than once per package, which is why that row
/// barely grows with the signer count, and it is worth its own figure because
/// `COSTS.md` names it: a published number that nothing asserts is the drift
/// this file exists to prevent.
#[test]
fn the_price_conversion_costs_what_is_published() {
    let vector = vector();
    for n in SIGNER_COUNTS {
        assert_eq!(
            cycles(&vector, stage::SCALE, n) - cycles(&vector, stage::MEMBERSHIP, n),
            expected::SCALING,
            "{n} signers"
        );
    }
}

/// The table describes the format, not the feed it was measured on.
///
/// Decoding, hashing and the membership check are byte-count work, so the four
/// other captured feeds must agree with BTC exactly. Recovery cannot: its cost
/// depends on the scalar, which is why `m0` reports it as scalar-dependent and
/// why this bounds it instead. If a published row turned out to be a property of
/// one feed's bytes, the table would be about BTC rather than about an update.
#[test]
fn the_table_is_not_particular_to_one_feed() {
    let btc = components(&vector(), 5);

    for feed in ["ETH", "SOL", "XMR", "ZEC"] {
        let other = components(&vectors::named(feed), 5);
        assert_eq!(
            [other[0], other[1], other[3]],
            [btc[0], btc[1], btc[3]],
            "{feed}"
        );

        let drift = 100.0 * (other[2] as f64 - btc[2] as f64).abs() / btc[2] as f64;
        assert!(drift < 1.0, "{feed}: recovery drifted {drift:.2}% from BTC");
    }
}

/// Widening the threshold is linearly priced, with no engineering remedy.
///
/// `m0` asserts this of bare recovery. Here it is the two components that are
/// 99.5% of an update, measured through `VerifierBackend` on a real payload, so
/// it is a statement about what an operator actually pays for a wider signer set
/// rather than about a primitive.
#[test]
fn the_dominant_cost_is_linear_in_the_signer_count() {
    let vector = vector();
    let crypto = |n| {
        let c = components(&vector, n);
        (c[1] + c[2]) as f64 / n as f64
    };
    let unit = crypto(1);

    for n in [3, 5] {
        let drift = 100.0 * (crypto(n) - unit).abs() / unit;
        assert!(
            drift < 1.0,
            "hashing and recovery drifted {drift:.2}% per signer between 1 and {n}; \
             they may now share work"
        );
    }
}

/// The published table, printed by the code that asserts it so the two cannot
/// drift apart.
#[test]
fn the_cost_table_is_reproducible() {
    let vector = vector();
    let names = [
        "decode",
        "keccak256",
        "recovery",
        "signer-set membership",
        "the rest of `verify_feed`",
    ];

    println!("\n| component | 1 signer | 3 signers | 5 signers |");
    println!("| --- | ---: | ---: | ---: |");
    let measured: Vec<[u64; 5]> = SIGNER_COUNTS
        .iter()
        .map(|&n| components(&vector, n))
        .collect();
    for (index, name) in names.iter().enumerate() {
        let row: Vec<String> = measured
            .iter()
            .map(|c| format!("{:>9}", thousands(c[index])))
            .collect();
        println!("| {name} | {} |", row.join(" | "));
    }
    let totals: Vec<String> = SIGNER_COUNTS
        .iter()
        .map(|&n| thousands(cycles(&vector, stage::VERIFY, n) - cycles(&vector, stage::FLOOR, n)))
        .collect();
    println!("| **whole update** | {} |", totals.join(" | "));
    let floors: Vec<String> = SIGNER_COUNTS
        .iter()
        .map(|&n| thousands(cycles(&vector, stage::FLOOR, n)))
        .collect();
    println!(
        "| _harness floor, subtracted out_ | {} |\n",
        floors.join(" | ")
    );
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The captured payload with its packages repeated until there are `n`.
///
/// Every package is a fixed stride and carries the same feed, so repeating the
/// body and re-emitting the count gives a payload that is well formed, entirely
/// valid, and as expensive as `n` packages of this feed can be.
fn payload_repeating(vector: &Vector, n: usize) -> Vec<u8> {
    let body_len = vector.payload.len() - EMPTY_ENVELOPE_BYTES;
    let mut out = Vec::with_capacity(n * body_len);
    while out.len() < n * (body_len / vector.signers.len()) {
        out.extend_from_slice(&vector.payload[..body_len]);
    }
    out.truncate(n * (body_len / vector.signers.len()));
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&REDSTONE_MARKER);
    out
}

/// P1's ceiling, not its typical case.
///
/// The package count lives in the envelope, outside every signature, so anyone
/// handling a payload can raise it. Before ADR 25 that decided how much of the
/// transaction's budget verification spent: 56 repeated packages, an 8 KB
/// payload, measured 34,746,599 cycles and overran the budget, so the
/// transaction aborted rather than publishing. `MAX_RECOVERIES` is what bounds
/// it, and this is the assertion that the bound is set low enough to matter.
#[test]
fn the_most_a_payload_can_cost_still_fits_in_one_transaction() {
    let vector = vector();
    let signer_bytes: Vec<u8> = vector.signers.iter().flat_map(|s| *s.as_bytes()).collect();

    let cost = |n: usize| {
        let bytes = payload_repeating(&vector, n);
        let whole = raw_cycles(&vector, stage::VERIFY, bytes.clone(), &signer_bytes);
        // The harness reads the payload through `env::read()`, which costs more
        // for a longer one whatever verification then does with it. Subtracted
        // out here as it is everywhere else in this file.
        (
            whole,
            whole - raw_cycles(&vector, stage::FLOOR, bytes, &signer_bytes),
        )
    };

    let (at_ceiling, _) = cost(MAX_RECOVERIES);
    assert!(
        at_ceiling < expected::LEZ_CYCLE_BUDGET,
        "a payload at the ceiling costs {at_ceiling} of {} cycles, {:.1}%",
        expected::LEZ_CYCLE_BUDGET,
        100.0 * at_ceiling as f64 / expected::LEZ_CYCLE_BUDGET as f64
    );

    // And past it the work stops growing, which is the part that makes the
    // ceiling a ceiling. Two payloads well over it, one three times the other,
    // do the same verification: recover up to the ceiling, then refuse. A run
    // past the ceiling is slightly cheaper than one at it because it never
    // reaches the median and the conversion.
    let (_, verified_at) = cost(MAX_RECOVERIES);
    for n in [MAX_RECOVERIES + 8, MAX_RECOVERIES * 3, MAX_RECOVERIES * 8] {
        let (whole, verified) = cost(n);
        assert!(
            verified <= verified_at,
            "{n} packages did {verified} cycles of verification, more than the \
             {verified_at} a payload at the ceiling does"
        );
        assert!(
            whole < expected::LEZ_CYCLE_BUDGET,
            "{n} packages cost {whole} of {} cycles",
            expected::LEZ_CYCLE_BUDGET
        );
    }
}

/// Cycles for a whole `verify_feed`, on a payload this test built rather than
/// one `payload_with` cut down. No journal assertion: these payloads are
/// supposed to fail verification, and what is under measurement is the cost.
fn raw_cycles(vector: &Vector, stage: u8, payload: Vec<u8>, signer_bytes: &[u8]) -> u64 {
    let input = (
        stage,
        payload,
        vector.feed_id.as_bytes().to_vec(),
        signer_bytes.to_vec(),
        3u8,
        DECIMALS,
        vector.timestamp_ms,
        MAX_MAX_AGE_MS,
    );
    let env = ExecutorEnv::builder()
        .write(&input)
        .expect("input")
        .build()
        .expect("env");
    default_executor()
        .execute(env, VERIFY_COST_ELF)
        .expect("execution")
        .cycles()
}

/// The whole of P1's ceiling, read included.
///
/// `the_most_a_payload_can_cost_still_fits_in_one_transaction` bounds what
/// verification spends. It does not bound what getting the payload into guest
/// memory spends, and that is not verification's to bound: LEZ reads a program's
/// entire instruction data before the program's first instruction, so the cycles
/// are gone before `Payload::decode` can object. At about 113 cycles a byte a
/// 900-package payload -- 127,814 bytes, well formed, and refused by the
/// recovery ceiling -- measured 33,792,622 cycles and overran the budget on the
/// read alone.
///
/// `MAX_PAYLOAD_BYTES` is what makes the pair statable, and this is the
/// assertion that the pair fits: the largest payload the decoder will accept,
/// every package of it carrying the requested feed, read and verified.
#[test]
fn the_largest_payload_the_decoder_accepts_is_read_and_verified_inside_the_budget() {
    let vector = vector();
    let signer_bytes: Vec<u8> = vector.signers.iter().flat_map(|s| *s.as_bytes()).collect();
    let stride = (vector.payload.len() - EMPTY_ENVELOPE_BYTES) / vector.signers.len();

    let n = (MAX_PAYLOAD_BYTES - EMPTY_ENVELOPE_BYTES) / stride;
    let bytes = payload_repeating(&vector, n);
    assert!(
        bytes.len() <= MAX_PAYLOAD_BYTES && MAX_PAYLOAD_BYTES - bytes.len() < stride,
        "the test is only meaningful at the limit; this payload is {} of {MAX_PAYLOAD_BYTES} bytes",
        bytes.len()
    );

    let whole = raw_cycles(&vector, stage::VERIFY, bytes, &signer_bytes);
    assert!(
        whole < expected::LEZ_CYCLE_BUDGET,
        "{n} packages in {MAX_PAYLOAD_BYTES} bytes cost {whole} of {} cycles, {:.1}%",
        expected::LEZ_CYCLE_BUDGET,
        100.0 * whole as f64 / expected::LEZ_CYCLE_BUDGET as f64
    );

    // The headroom is worth asserting too, because it is what the program doing
    // the verifying gets to spend: the account write, the framework's own floor,
    // whatever the caller does with the price.
    let headroom = expected::LEZ_CYCLE_BUDGET - whole;
    assert!(
        headroom > expected::LEZ_CYCLE_BUDGET / 4,
        "only {headroom} cycles left for the rest of the program"
    );
}

// ---------------------------------------------------------------------------
// The push write (M2-18).
//
// Everything above measures verification. What P1 was missing is the other half
// of a submission: what the instruction spends *besides* verifying, which is the
// part that reads the accounts and writes the price.
// ---------------------------------------------------------------------------

/// Stages of `submit_cost.rs`, which brackets the `submit_price` body rather than
/// the verification pipeline.
mod push_stage {
    /// Input, setup and journal: the three accounts built, nothing run.
    pub const FLOOR: u8 = 0;
    /// `Payload::decode` and `verify_feed`, called as `submit_price` calls them.
    pub const VERIFY: u8 = 1;
    /// The real `submit_price`, whole.
    pub const SUBMIT: u8 = 2;

    // Outside the prefix chain: one thing each, to take the residual apart.
    /// The PDA derivation SPEL's generated validator performs before the body.
    pub const VALIDATOR_PDA: u8 = 3;
    /// `OraclePriceAccount::try_from` -- the read only an update does.
    pub const PUBLISHED_READ: u8 = 4;
    /// `AutoClaim::pda_from_seeds` -- the claim only a first write does.
    pub const AUTO_CLAIM: u8 = 5;
    /// `publish::price_account` -- building the account, which only a first write does.
    pub const BUILD: u8 = 6;
    /// `publish::publish` -- the three checks, which only an update makes.
    pub const PUBLISH: u8 = 7;
}

/// Which of the instruction's two paths is under measurement.
///
/// The same instruction, branching on whether the price account already exists
/// (`submit.rs`: `let create = price_account.account == Account::default()`).
/// They are separate figures because only one of them reads an account:
///
/// - [`Case::Create`] is the first submission a feed ever takes, once per feed;
/// - [`Case::Update`] is every one after it, so it is the cost an operator pays
///   on every heartbeat and every deviation trigger for the life of the feed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Case {
    Create,
    Update,
}

impl Case {
    /// The price account's data, which is what distinguishes the two paths.
    ///
    /// Empty means a fully default account, which is how `submit_price` decides
    /// it is creating rather than updating.
    fn price_data(self, vector: &Vector) -> Vec<u8> {
        match self {
            Self::Create => Vec::new(),
            Self::Update => {
                use aggregator_program::kanon_idl::{AccountId, OraclePriceAccount};
                // One round older than the payload, so the submission is an
                // update rather than a `NotNewer` refusal, and carrying the same
                // pair, because a mismatch would refuse before the write.
                let account = OraclePriceAccount {
                    base_asset: AccountId::new(PUSH_BASE),
                    quote_asset: AccountId::new(PUSH_QUOTE),
                    price: 1,
                    timestamp: vector.timestamp_ms - 1_000,
                    source_id: aggregator_program::publish::REDSTONE_SOURCE_ID,
                    confidence_interval: 0,
                };
                borsh::to_vec(&account).expect("the canonical account encodes")
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Create => "first write",
            Self::Update => "update",
        }
    }
}

/// Arbitrary but fixed. The pair is a registration claim (ADR 16) and comparing
/// it costs 64 bytes whatever it holds; it has to match what the published
/// account carries, or the update refuses before reaching the write.
const PUSH_BASE: [u8; 32] = [0xB7; 32];
const PUSH_QUOTE: [u8; 32] = [0x05; 32];

/// The program that owns the accounts. Fixed and arbitrary: `submit_price`
/// compares program ids, and comparing two `[u32; 8]` costs the same whatever
/// they hold.
const PUSH_OURS: [u32; 8] = [7; 8];

/// The feed as it is stored on chain, which is what the instruction decodes.
fn push_feed_state(vector: &Vector) -> aggregator_program::FeedAccount {
    let mut feed_id = [0u8; 32];
    feed_id[..vector.feed_id.len()].copy_from_slice(vector.feed_id.as_bytes());
    aggregator_program::FeedAccount {
        feed_id,
        base_asset: PUSH_BASE,
        quote_asset: PUSH_QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_MAX_AGE_MS,
        signers: vector.signers.iter().map(|s| s.0).collect(),
        threshold: vector.signers.len() as u8,
        paused: false,
    }
}

/// The clock account's sixteen bytes: block id, then the timestamp.
fn push_clock_data(now_ms: u64) -> Vec<u8> {
    let mut out = 1u64.to_le_bytes().to_vec();
    out.extend_from_slice(&now_ms.to_le_bytes());
    out
}

/// Cycles for one stage of one case, and what the guest reported.
///
/// Memoised for the reason [`run`] is: a zkVM execution is a deterministic
/// function of the ELF and its input, so a repeat is not a check.
fn push_run(vector: &Vector, stage: u8, case: Case) -> (u64, (u32, u64, u32)) {
    type Key = (String, u8, Case);
    type Slot = Arc<OnceLock<(u64, (u32, u64, u32))>>;
    static MEASURED: OnceLock<Mutex<HashMap<Key, Slot>>> = OnceLock::new();

    let slot = MEASURED
        .get_or_init(Mutex::default)
        .lock()
        .expect("not poisoned")
        .entry((vector.feed_id.clone(), stage, case))
        .or_default()
        .clone();
    *slot.get_or_init(|| push_execute(vector, stage, case))
}

fn push_execute(vector: &Vector, stage: u8, case: Case) -> (u64, (u32, u64, u32)) {
    let signer_bytes: Vec<u8> = vector.signers.iter().flat_map(|s| *s.as_bytes()).collect();

    #[allow(clippy::type_complexity)]
    let input = (
        stage,
        vector.payload.clone(),
        borsh::to_vec(&push_feed_state(vector)).expect("the feed account encodes"),
        case.price_data(vector),
        push_clock_data(vector.timestamp_ms),
        kanon_clock::CLOCK_ACCOUNT_ID.to_vec(),
        vector.feed_id.as_bytes().to_vec(),
        PUSH_BASE.to_vec(),
        PUSH_QUOTE.to_vec(),
        signer_bytes,
        vector.signers.len() as u8,
        DECIMALS,
        vector.timestamp_ms,
        MAX_MAX_AGE_MS,
        PUSH_OURS,
    );

    let env = ExecutorEnv::builder()
        .write(&input)
        .expect("input")
        .build()
        .expect("env");
    let session = default_executor()
        .execute(env, kanon_methods::SUBMIT_COST_ELF)
        .expect("execution");
    let report: (u32, u64, u32) = session.journal.decode().expect("journal");
    (session.cycles(), report)
}

/// Cycles for a stage, having first checked the run did the work it claims.
///
/// The journal is the guard against the failure mode that matters here: a
/// submission that refused returns an error and no post-states, and would report
/// as a very cheap write rather than as a failure.
fn push_cycles(vector: &Vector, stage: u8, case: Case) -> u64 {
    let (cycles, (signers, _price, post_states)) = push_run(vector, stage, case);

    if stage == push_stage::VERIFY {
        assert_eq!(
            signers as usize,
            vector.signers.len(),
            "{}: verification counted {signers} signers, not {}; a run that took \
             an early exit is not a measurement of the work",
            case.label(),
            vector.signers.len()
        );
    }
    if stage == push_stage::SUBMIT {
        assert_eq!(
            post_states,
            3,
            "{}: the submission produced {post_states} post-states, not three. It \
             refused, so this is not a measurement of a write",
            case.label()
        );
    }

    // The isolating stages report 1 when they did their work. Without this a
    // stage that folded to a constant, or a decode that failed, reads as a very
    // cheap component -- the same trap the post-state count closes for stage 2.
    if matches!(
        stage,
        push_stage::VALIDATOR_PDA
            | push_stage::PUBLISHED_READ
            | push_stage::AUTO_CLAIM
            | push_stage::BUILD
            | push_stage::PUBLISH
    ) {
        assert_eq!(
            signers,
            1,
            "{}: stage {stage} reported no work, so it is measuring nothing",
            case.label()
        );
    }
    cycles
}

/// What the whole `submit_price` body costs, and what the write costs inside it.
///
/// Per case, because the price account is built during setup: the 136 bytes an
/// update carries only cancel out of the subtraction if every stage of that case
/// pays for them. Measuring the update's write against the *create*'s
/// verification inflates it by 15,410 cycles of setup, which is how the first
/// version of this measurement was wrong.
fn push_components(vector: &Vector, case: Case) -> (u64, u64, u64) {
    let floor = push_cycles(vector, push_stage::FLOOR, case);
    let verify = push_cycles(vector, push_stage::VERIFY, case);
    let submit = push_cycles(vector, push_stage::SUBMIT, case);

    (verify - floor, submit - verify, submit - floor)
}

/// The push write costs what `COSTS.md` publishes.
///
/// Exact equality, for the reason every figure here is pinned exactly: a ceiling
/// passes quietly when a number moves for a reason nobody has understood yet.
/// A failure is a prompt to re-measure and re-publish, not necessarily a defect.
#[test]
fn the_push_write_costs_what_is_published() {
    let vector = vector();

    let (verify_create, write_create, body_create) = push_components(&vector, Case::Create);
    let (verify_update, write_update, body_update) = push_components(&vector, Case::Update);

    // The floors are in here because `COSTS.md` publishes them, and a published
    // figure that nothing asserts is the drift this file exists to prevent. They
    // are also the one pair that a change to *setup* would move while leaving
    // every difference above intact, so without them that change is invisible.
    // @frenzox asked for this on #60.
    let floor_create = push_cycles(&vector, push_stage::FLOOR, Case::Create);
    let floor_update = push_cycles(&vector, push_stage::FLOOR, Case::Update);

    assert_eq!(
        [floor_create, verify_create, write_create, body_create],
        [138_475, 3_039_832, 7_386, 3_047_218],
        "the first-write figures moved"
    );
    assert_eq!(
        [floor_update, verify_update, write_update, body_update],
        [153_882, 3_039_832, 8_582, 3_048_414],
        "the update figures moved"
    );
}

/// Verification costs the same whichever path the write takes.
///
/// Structural rather than a measurement: `verify_feed` is handed the payload, the
/// registration and the clock, and none of those is the price account. If these
/// two ever differ, the staging is measuring something it should not be --
/// setup that failed to cancel, most likely -- and the write figures either side
/// of it are wrong by that difference.
#[test]
fn verification_does_not_depend_on_the_price_account() {
    let vector = vector();

    assert_eq!(
        push_components(&vector, Case::Create).0,
        push_components(&vector, Case::Update).0,
        "verification differs between a first write and an update, so the two \
         cases are not subtracting identical setup"
    );
}

/// Both harnesses measure the same verification, to within the ELF.
///
/// `verify_cost.rs` and `submit_cost.rs` are separate guests reaching
/// `verify_feed` by different routes, so this is the check that neither harness
/// is measuring something other than the shipped path -- the strongest evidence
/// available that the staging in either is faithful.
///
/// A tolerance rather than exact equality, and the reason is not slack: a cycle
/// count is a property of a *guest ELF*, not of the logical work, so two
/// independently compiled binaries running identical source are not obliged to
/// agree to the cycle. They agree to 42 out of 3,039,790, which is 0.0014%, and
/// the gap moved from 60 to 42 when stages were added to `submit_cost.rs` --
/// which is the property in action rather than a contradiction of it. The bound
/// is a tenth of a percent, far tighter than any real regression. Each guest's
/// own figures stay pinned exactly; this is the only comparison here that cannot
/// be.
#[test]
fn both_harnesses_agree_about_what_verification_costs() {
    let vector = vector();
    let n = vector.signers.len();

    let through_verify_cost = cycles(&vector, stage::VERIFY, n) - cycles(&vector, stage::FLOOR, n);
    let through_submit_cost = push_components(&vector, Case::Create).0;

    let gap = through_verify_cost.abs_diff(through_submit_cost);
    let drift = 100.0 * gap as f64 / through_verify_cost as f64;
    assert!(
        drift < 0.1,
        "the two harnesses disagree about verification by {gap} cycles ({drift:.4}%): \
         {through_verify_cost} through verify_cost, {through_submit_cost} through \
         submit_cost. Beyond code layout, so one of them is no longer measuring \
         the shipped path"
    );
}

/// The published push-write table, printed by the code that asserts it.
#[test]
fn the_push_write_cost_table_is_reproducible() {
    let vector = vector();

    println!("\n| | first write | update |");
    println!("| --- | ---: | ---: |");

    let create = push_components(&vector, Case::Create);
    let update = push_components(&vector, Case::Update);

    println!(
        "| verification | {:>9} | {:>9} |",
        thousands(create.0),
        thousands(update.0)
    );
    // No padding inside the emphasis markers: `**    7,386**` would render the
    // spaces literally.
    println!(
        "| **the write** | **{}** | **{}** |",
        thousands(create.1),
        thousands(update.1)
    );
    println!(
        "| whole `submit_price` body | {:>9} | {:>9} |",
        thousands(create.2),
        thousands(update.2)
    );
    println!(
        "| _the write, as a share of the body_ | _{:.3}%_ | _{:.3}%_ |",
        100.0 * create.1 as f64 / create.2 as f64,
        100.0 * update.1 as f64 / update.2 as f64
    );
    println!(
        "| _harness floor, subtracted out_ | {:>9} | {:>9} |",
        thousands(push_cycles(&vector, push_stage::FLOOR, Case::Create)),
        thousands(push_cycles(&vector, push_stage::FLOOR, Case::Update))
    );
    // Against the constant the assertions use, not a rounded "32M". The two
    // differ by 4.9%, which is the difference between 0.0255% and the 0.027% an
    // earlier draft published.
    println!(
        "| _the write, against LEZ's per-transaction budget_ | _{:.4}%_ | _{:.4}%_ |\n",
        100.0 * create.1 as f64 / expected::LEZ_CYCLE_BUDGET as f64,
        100.0 * update.1 as f64 / expected::LEZ_CYCLE_BUDGET as f64
    );
}

/// What one isolating stage costs: itself, less the floor of the same case.
fn push_isolated(vector: &Vector, stage: u8, case: Case) -> u64 {
    push_cycles(vector, stage, case) - push_cycles(vector, push_stage::FLOOR, case)
}

/// What the generated validator costs, and what each case's exclusive work costs.
///
/// Prints rather than asserts; the assertions are below. Useful on its own when
/// a figure moves and the question is which of the three moved.
#[test]
fn the_isolated_components_are_reproducible() {
    let vector = vector();

    let validator = push_isolated(&vector, push_stage::VALIDATOR_PDA, Case::Create);
    let read = push_isolated(&vector, push_stage::PUBLISHED_READ, Case::Update);
    let claim = push_isolated(&vector, push_stage::AUTO_CLAIM, Case::Create);
    let build = push_isolated(&vector, push_stage::BUILD, Case::Create);
    let publish = push_isolated(&vector, push_stage::PUBLISH, Case::Update);

    println!("build and encode (first write only): {}", thousands(build));
    println!(
        "publish's checks, incl. the read (update only): {}",
        thousands(publish)
    );
    println!("\nvalidator PDA derivation: {}", thousands(validator));
    println!("published-account read (update only): {}", thousands(read));
    println!("auto-claim (first write only): {}", thousands(claim));

    // Each measured on its own, so each is pinned on its own. They are not
    // components of the write and do not sum to it -- see
    // `an_update_costs_more_than_a_first_write` for why that was tried and
    // abandoned.
    // All five, because `COSTS.md` publishes all five. Build and publish were
    // printed and unpinned in the first version, which @frenzox flagged on #60
    // alongside the deeper problem that build was measuring nothing at all.
    assert_eq!(
        [validator, read, claim, build, publish],
        [1_722, 1_442, 1_304, 1_782, 2_381],
        "an isolated figure moved"
    );
}

/// An update costs more than a first write, and the gap is a net between paths.
///
/// The direction is the operationally relevant fact: an update is what every
/// heartbeat and every deviation trigger runs, so the recurring cost is the
/// higher of the two.
///
/// What this deliberately does *not* claim is a cause. The first version of this
/// measurement called the gap "the account read", and @frenzox pointed out on #60
/// that the first-write path does create-only work of its own -- the claim in
/// `post_states` -- so the gap is a net, not a component.
///
/// Two attempts to decompose it into isolated pieces both failed, and the reason
/// is worth recording rather than retrying. `read - claim` is 138 cycles against
/// a gap of 1,196. Taking all four operations -- what only an update does, less
/// what only a first write does -- gives 2,381 - (1,304 + 1,782) = **-705**, the
/// wrong sign. Isolated calls do not cost what inlined ones do, so the terms are
/// under no obligation to sum, and the second attempt got further from the answer
/// than the first. The isolated figures are each real and each pinned; their sum
/// is not this gap and this test does not pretend otherwise.
#[test]
fn an_update_costs_more_than_a_first_write() {
    let vector = vector();

    let create = push_components(&vector, Case::Create).1;
    let update = push_components(&vector, Case::Update).1;

    assert!(
        update > create,
        "an update ({update}) is no dearer than a first write ({create}), so \
         either the published account is no longer being decoded or the two \
         cases have become the same path"
    );
    assert_eq!(
        update - create,
        1_196,
        "the gap between the two paths moved"
    );
}

/// The validator SPEL generates is not free, and is not in the body figures.
///
/// `price_account` is declared `#[account(mut, pda = [account("feed"),
/// r#const("KANON_PRICE_ACCOUNT")])]`, so the derivation runs before
/// `submit_price` is entered. Everything else here measures the body, so this is
/// the figure that has to be added to it -- `COSTS.md` publishes them separately
/// rather than pretending either is the instruction.
#[test]
fn the_generated_validator_costs_what_is_published() {
    let vector = vector();

    assert_eq!(
        push_isolated(&vector, push_stage::VALIDATOR_PDA, Case::Create),
        1_722,
        "the validator's PDA derivation moved"
    );
}

/// Every push figure at once, for re-pinning after a guest change.
///
/// Cycle counts are a property of the ELF, so adding a stage to `submit_cost.rs`
/// moves all of them by a little. This prints the whole set so re-pinning is one
/// step rather than six failures read one at a time.
#[test]
fn the_push_figures_for_repinning() {
    let vector = vector();
    for case in [Case::Create, Case::Update] {
        let (verify, write, body) = push_components(&vector, case);
        println!(
            "\nREPIN {:>11}: floor {} verify {} write {} body {}",
            case.label(),
            push_cycles(&vector, push_stage::FLOOR, case),
            verify,
            write,
            body
        );
    }
    println!(
        "REPIN isolated: validator {} read {} claim {} build {} publish {}",
        push_isolated(&vector, push_stage::VALIDATOR_PDA, Case::Create),
        push_isolated(&vector, push_stage::PUBLISHED_READ, Case::Update),
        push_isolated(&vector, push_stage::AUTO_CLAIM, Case::Create),
        push_isolated(&vector, push_stage::BUILD, Case::Create),
        push_isolated(&vector, push_stage::PUBLISH, Case::Update)
    );
    let update_write = push_components(&vector, Case::Update).1;
    println!(
        "REPIN gap {} | budget share create {:.4}% update {:.4}%",
        update_write - push_components(&vector, Case::Create).1,
        100.0 * push_components(&vector, Case::Create).1 as f64 / expected::LEZ_CYCLE_BUDGET as f64,
        100.0 * update_write as f64 / expected::LEZ_CYCLE_BUDGET as f64
    );
}

// ---------------------------------------------------------------------------
// Registration (M2-18, P2's registration clause).
//
// P2 asks for the push side's "account write and registration". The write is
// above; this is the other half. @frenzox noticed on #60 that the requirement
// names it and nothing measured it.
// ---------------------------------------------------------------------------

/// What a first registration costs.
///
/// Measured on risc0-zkvm 3.0.5, guest rustc 1.97.0. Update together with
/// `COSTS.md`, and note that this figure lives in its own guest so it does not
/// move when a stage is added to `submit_cost.rs`.
const REGISTRATION_CYCLES: u64 = 5_270;

/// The admin whose signature authorises a registration.
const REGISTER_ADMIN_KEY: [u8; 32] = [0x44; 32];

fn register_run(vector: &Vector, stage: u8) -> (u64, (u32, u32)) {
    type Slot = Arc<OnceLock<(u64, (u32, u32))>>;
    static MEASURED: OnceLock<Mutex<HashMap<(String, u8), Slot>>> = OnceLock::new();

    let slot = MEASURED
        .get_or_init(Mutex::default)
        .lock()
        .expect("not poisoned")
        .entry((vector.feed_id.clone(), stage))
        .or_default()
        .clone();
    *slot.get_or_init(|| {
        let config = aggregator_program::admin::AdminAccount {
            admin: Some(REGISTER_ADMIN_KEY),
            pending: None,
        };
        let input = (
            stage,
            borsh::to_vec(&config).expect("the admin account encodes"),
            REGISTER_ADMIN_KEY.to_vec(),
            vector.feed_id.as_bytes().to_vec(),
            PUSH_BASE.to_vec(),
            PUSH_QUOTE.to_vec(),
            vector
                .signers
                .iter()
                .flat_map(|s| *s.as_bytes())
                .collect::<Vec<u8>>(),
            vector.signers.len() as u8,
            DECIMALS,
            MAX_MAX_AGE_MS,
            PUSH_OURS,
        );

        let env = ExecutorEnv::builder()
            .write(&input)
            .expect("input")
            .build()
            .expect("env");
        let session = default_executor()
            .execute(env, kanon_methods::REGISTER_COST_ELF)
            .expect("execution");
        let report: (u32, u32) = session.journal.decode().expect("journal");
        (session.cycles(), report)
    })
}

/// What a first registration costs, having checked it registered.
fn register_cycles(vector: &Vector) -> u64 {
    let (floor, _) = register_run(vector, 0);
    let (registered, (ran, post_states)) = register_run(vector, 1);

    assert_eq!(
        ran, 1,
        "the registration refused, so this is not a measurement of a registration"
    );
    assert_eq!(
        post_states, 4,
        "the registration produced {post_states} post-states, not four"
    );
    registered - floor
}

/// Registering a feed costs what `COSTS.md` publishes.
///
/// P2 names registration alongside the write, and this is the figure. It does no
/// cryptography — Borsh, one PDA derivation and a handful of comparisons — so it
/// is cheap in a way the write is not even in the same conversation with
/// verification about.
#[test]
fn registering_a_feed_costs_what_is_published() {
    let vector = vector();

    assert_eq!(
        register_cycles(&vector),
        REGISTRATION_CYCLES,
        "the registration figure moved"
    );
}

/// Printed by the code that asserts it, like every other figure here.
#[test]
fn the_registration_figure_is_reproducible() {
    let vector = vector();
    let cycles = register_cycles(&vector);

    println!("\n| | cycles |");
    println!("| --- | ---: |");
    println!("| a first registration | {} |", thousands(cycles));
    println!(
        "| _as a share of the per-transaction budget_ | _{:.4}%_ |\n",
        100.0 * cycles as f64 / expected::LEZ_CYCLE_BUDGET as f64
    );
}
