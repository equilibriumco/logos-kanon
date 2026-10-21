//! Whether the cycle saving survives into real proving time.
//!
//! Cycles are what the cost table measures, and a cycle count is not a wall
//! clock: RISC Zero proves in segments, so time moves in steps as the executed
//! cycles cross a power of two rather than in proportion to them. Halving the
//! cycles below a boundary buys nothing, and halving them across one buys a
//! segment. The table reports cycles and segments beside the seconds so the
//! steps are visible rather than inferred.
//!
//! What this measures is the signature checks and the zkVM floor, in a bare
//! guest. It is not a private LEZ transaction, where the privacy circuit runs
//! on top of all of it.
//!
//! One sample per row. Cycles, segments and receipt bytes are exact and
//! repeat; the seconds have shown about 15% run-to-run spread on an idle
//! laptop, so read a single figure as a magnitude and a ratio between two
//! rows of the same run rather than as a constant.
//!
//! Ignored by default: real proving takes minutes.
//!
//! ```sh
//! cargo test --release -p bip340-bench --test proving -- --ignored --nocapture
//! ```

mod support;

use std::time::Instant;

use risc0_zkvm::{default_executor, default_prover, ExecutorEnv};

/// The unit, the RFP's required threshold, and twice it.
const THRESHOLDS: &[usize] = &[1, 5, 10];

/// What one ELF costs over one case set.
struct Measurement {
    cycles: u64,
    segments: usize,
    seconds: f64,
    receipt_bytes: usize,
}

fn measure(elf: &[u8], input: &[(Vec<u8>, Vec<u8>, Vec<u8>)]) -> Measurement {
    // Rebuilt per use: executing and proving each consume the environment.
    let build = || {
        ExecutorEnv::builder()
            .write(&input.to_vec())
            .expect("the cases serialise")
            .build()
            .expect("an executor environment")
    };

    let session = default_executor()
        .execute(build(), elf)
        .expect("the guest runs to completion");

    let started = Instant::now();
    let info = default_prover()
        .prove(build(), elf)
        .expect("the guest proves");
    let seconds = started.elapsed().as_secs_f64();

    // A dev-mode run returns a stub instead of proving, and would report a
    // time that measures nothing. The receipt says which happened.
    assert!(
        !matches!(info.receipt.inner, risc0_zkvm::InnerReceipt::Fake(_)),
        "dev mode produced a stub receipt, so this timing is meaningless"
    );

    Measurement {
        cycles: session.cycles(),
        segments: session.segments.len(),
        seconds,
        receipt_bytes: bincode::serialize(&info.receipt)
            .expect("the receipt serialises")
            .len(),
    }
}

#[test]
#[ignore = "real proving, minutes"]
fn the_cycle_saving_in_wall_time() {
    println!();
    println!("| M | implementation | cycles | segments | prove (s) | receipt (B) |");
    println!("| ---: | --- | ---: | ---: | ---: | ---: |");

    for &count in THRESHOLDS {
        let input = support::valid_cases(count);
        for (name, elf) in [
            ("k256", bip340_bench::PROVE_K256_ELF),
            ("fast", bip340_bench::PROVE_FAST_ELF),
        ] {
            let m = measure(elf, &input);
            println!(
                "| {count} | {name} | {} | {} | {:.1} | {} |",
                m.cycles, m.segments, m.seconds, m.receipt_bytes
            );
        }
    }
    println!();
}
