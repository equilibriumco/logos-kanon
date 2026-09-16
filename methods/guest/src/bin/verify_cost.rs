//! Runs a prefix of the verification pipeline, so that differencing two runs
//! gives the cost of one component.
//!
//! Measurement code, not product code. It ships in the product guest workspace
//! because that is the only place the product's `[patch.crates-io]` applies, and
//! a cost figure measured under different patches is a figure for a different
//! program.
//!
//! # The stages
//!
//! Each stage runs everything the stage below it runs, plus one component:
//!
//! | stage | adds |
//! | --- | --- |
//! | 0 | nothing: input, setup, journal |
//! | 1 | `Payload::decode` and a full package walk |
//! | 2 | keccak256 over each package's signable span |
//! | 3 | signer recovery from each package's signature |
//! | 4 | looking each recovered address up in the configured set |
//! | 5 | the real [`verify_feed`], whole |
//!
//! Stage 6 is not part of that chain. It is stage 4 plus one Q64.64 conversion,
//! which isolates the largest single item inside what stage 5 adds over stage 4.
//!
//! Setup is identical in every stage and happens before the branch, so zkVM
//! startup, input deserialization, the signer vector and the journal commit
//! cancel when two stages are subtracted. Stages 1 to 4 call exactly the
//! functions `verify_feed` calls, in the same order, rather than reimplementing
//! them.
//!
//! # Panics
//!
//! None. A guest panic aborts the transaction rather than reporting a failure,
//! and while nothing here is on a transaction path, a measurement that aborts is
//! not a measurement. Every fallible step reports through the journal instead,
//! and the host asserts on what it finds there: a run that took an error path
//! did not measure the work.

use risc0_zkvm::guest::env;

use verifier_core::{
    backend::{InProgramBackend, SignerAddress, VerifierBackend},
    decode::Payload,
    feed::{verify_feed, AssetPair, FeedConfig},
    time::{TimeError, TimeSource},
    value::Value,
};

/// A realistic eight-decimal price, for the stage that isolates the Q64.64
/// conversion. Fixed rather than taken from the payload, so the figure does not
/// move when the vectors are re-captured.
const SAMPLE_PRICE: u128 = 300_012_345_678;

/// The clock the LEZ clock program would supply. Fixed by the host so the
/// measurement does not depend on when it ran.
struct FixedClock(u64);

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.0)
    }
}

/// What the host reads back: how many packages the stage got through, and a
/// checksum keeping the work live.
///
/// The count is the assertion that matters. A stage that silently took an early
/// exit — a payload that did not decode, a signature that did not recover —
/// costs a fraction of the work it claims to measure, and would report as a
/// suspiciously cheap component rather than as a failure.
type Report = (u32, u64);

fn main() {
    let (stage, payload_bytes, feed_id, signer_bytes, threshold, decimals, now_ms, max_age_ms): (
        u8,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        u8,
        u8,
        u64,
        u64,
    ) = env::read();

    // Everything below this line and above the match is setup, and runs
    // identically in every stage.
    let signers: Vec<SignerAddress> = signer_bytes
        .chunks_exact(SignerAddress::LEN)
        .filter_map(|chunk| chunk.try_into().ok().map(SignerAddress))
        .collect();
    let backend = InProgramBackend::new();
    let clock = FixedClock(now_ms);
    // Arbitrary but fixed: the pair is a registration claim (ADR 16), and what
    // it costs to compare is 64 bytes whatever it holds.
    let assets = AssetPair::new([1; AssetPair::ID_LEN], [2; AssetPair::ID_LEN]);
    let config = FeedConfig::try_new(&feed_id, assets, decimals, max_age_ms, &signers, threshold);
    // Built here rather than at stage 6, so that only the conversion itself
    // falls inside the difference between stage 6 and stage 4.
    let sample = Value::from_be_slice(&SAMPLE_PRICE.to_be_bytes());

    let report: Report = match (config, sample) {
        (Ok(config), Some(sample)) => run(
            stage,
            &payload_bytes,
            &config,
            &assets,
            &backend,
            &clock,
            sample,
        ),
        _ => (0, 0),
    };

    env::commit(&report);
}

#[allow(clippy::too_many_arguments)]
fn run(
    stage: u8,
    payload_bytes: &[u8],
    config: &FeedConfig<'_>,
    expected: &AssetPair,
    backend: &InProgramBackend,
    clock: &FixedClock,
    sample: Value,
) -> Report {
    if stage == 0 {
        return (0, 0);
    }

    let Ok(payload) = Payload::decode(payload_bytes) else {
        return (0, 0);
    };

    if stage == 5 {
        return match verify_feed(&payload, config, expected, backend, clock) {
            Ok(verified) => (u32::from(verified.signers), verified.price as u64),
            Err(_) => (0, 0),
        };
    }

    let mut visited = 0u32;
    let mut checksum = 0u64;
    let walked = payload.for_each_package(|package| {
        if stage >= 2 {
            let digest = backend.keccak256(package.signable());
            checksum ^= u64::from(digest[0]);

            if stage >= 3 {
                let Ok(signer) = backend.recover_signer(&digest, &package.signature) else {
                    // Not a skip the way `verify_feed`'s is: there, an
                    // unrecoverable signature is one package's problem. Here it
                    // means the measurement is of something other than five
                    // recoveries, so the count the host checks must not include
                    // it.
                    return Ok(());
                };
                checksum ^= u64::from(signer.as_bytes()[0]);

                if stage >= 4 {
                    // The membership check, exactly as `verify_feed` spells it.
                    let found = config.signers().iter().position(|s| *s == signer);
                    checksum ^= found.map_or(u64::MAX, |index| index as u64);
                }
            }
        }
        visited += 1;
        Ok::<(), ()>(())
    });

    // One conversion, not one per package: `verify_feed` converts the median
    // once, at the end, so that is what stage 6 has to add to stage 4 for the
    // difference to be a figure about the real path.
    if stage >= 6 {
        checksum ^= sample.to_q64_64(config.decimals()).unwrap_or(0) as u64;
    }

    match walked {
        Ok(Ok(())) => (visited, checksum),
        _ => (0, 0),
    }
}
