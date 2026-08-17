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
use risc0_zkvm::{default_executor, ExecutorEnv};
use verifier_core::{backend::SignerAddress, decode::REDSTONE_MARKER};

/// The M1-21 capture. Real packages, real signatures, one data point each, so a
/// package's signable span is the 77 bytes `m0` measured keccak256 over.
const VECTORS: &str = include_str!("../../verifier-core/tests/vectors/redstone-primary-prod.json");

/// RedStone's scale for `redstone-primary-prod`.
const DECIMALS: u8 = 8;

/// The feed the published table is measured on. BTC is the one RFP-020 names
/// first, and `the_table_is_not_particular_to_one_feed` checks the choice does
/// not matter.
const FEED: &str = "BTC";

/// Signer counts. 3 is RFP-020's default threshold and the row `m0`'s 3-of-N
/// figure compares against; 1 is the unit cost; 5 is what the capture holds.
const SIGNER_COUNTS: [usize; 3] = [1, 3, 5];

/// Bytes after the last package: count, unsigned metadata size, marker. The
/// captured payloads carry no unsigned metadata.
const ENVELOPE_BYTES: usize = 2 + 3 + REDSTONE_MARKER.len();

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
    pub const FLOOR: [(usize, u64); 3] = [(1, 25_323), (3, 62_293), (5, 99_493)];

    /// Per signer count: decode, keccak256, recovery, membership, then
    /// everything else `verify_feed` does.
    pub const COMPONENTS: [(usize, [u64; 5]); 3] = [
        (1, [551, 17_475, 585_274, 165, 17_939]),
        (3, [1_435, 52_425, 1_755_574, 546, 20_962]),
        (5, [2_319, 87_375, 2_922_880, 995, 24_681]),
    ];

    /// The whole update, floor subtracted.
    pub const TOTAL: [(usize, u64); 3] = [(1, 621_404), (3, 1_830_942), (5, 3_038_250)];

    /// One Q64.64 conversion: the largest single item in the remainder, and the
    /// only one worth naming separately.
    pub const SCALING: u64 = 11_270;

    /// What the same rows would read if the wrong `[patch.crates-io]` were in
    /// force, from `m0`'s software and accelerated measurements: one software
    /// recovery, and one accelerated keccak256 over the 77-byte signable span.
    pub const SOFTWARE_RECOVERY: u64 = 11_074_701;
    pub const ACCELERATED_KECCAK: u64 = 2_527;
}

struct Vector {
    feed_id: String,
    timestamp_ms: u64,
    signers: Vec<SignerAddress>,
    payload: Vec<u8>,
}

fn vector() -> Vector {
    named(FEED)
}

fn named(feed: &str) -> Vector {
    let parsed: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors parse");
    let found = parsed["vectors"]
        .as_array()
        .expect("a vectors array")
        .iter()
        .find(|v| v["feed_id"].as_str() == Some(feed))
        .unwrap_or_else(|| panic!("{feed} is not in the capture"));

    Vector {
        feed_id: feed.to_owned(),
        timestamp_ms: found["timestamp_ms"].as_u64().expect("timestamp"),
        signers: found["signers"]
            .as_array()
            .expect("signers")
            .iter()
            .map(|s| {
                let bytes = hex::decode(s.as_str().expect("signer").trim_start_matches("0x"))
                    .expect("signer hex");
                SignerAddress(bytes.try_into().expect("twenty bytes"))
            })
            .collect(),
        payload: hex::decode(found["payload_hex"].as_str().expect("payload")).expect("payload hex"),
    }
}

/// The captured payload cut down to its first `n` packages.
///
/// Every package in the capture carries one data point of the same width, so
/// they are a fixed stride apart and the first `n` of them are a valid payload
/// once the count is re-emitted. Signatures are untouched, so the shortened
/// payload still recovers to `signers[..n]`.
fn payload_with(vector: &Vector, n: usize) -> Vec<u8> {
    let body_len = vector.payload.len() - ENVELOPE_BYTES;
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
fn run(vector: &Vector, stage: u8, n: usize) -> (u64, u32) {
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
        let other = components(&named(feed), 5);
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
