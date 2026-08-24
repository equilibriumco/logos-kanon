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
    pub const FLOOR: [(usize, u64); 3] = [(1, 25_315), (3, 62_283), (5, 99_482)];

    /// Per signer count: decode, keccak256, recovery, membership, then
    /// everything else `verify_feed` does.
    pub const COMPONENTS: [(usize, [u64; 5]); 3] = [
        (1, [587, 17_476, 585_276, 160, 17_783]),
        (3, [1_506, 52_428, 1_755_580, 528, 21_629]),
        (5, [2_425, 87_380, 2_922_890, 960, 26_188]),
    ];

    /// The whole update, floor subtracted.
    pub const TOTAL: [(usize, u64); 3] = [(1, 621_282), (3, 1_831_671), (5, 3_039_843)];

    /// One Q64.64 conversion: the largest single item in the remainder, and the
    /// only one worth naming separately.
    pub const SCALING: u64 = 11_263;

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
        // The vectors age from the moment they were captured, and what is under
        // measurement is the work, not the clock.
        u64::MAX,
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
        u64::MAX,
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
