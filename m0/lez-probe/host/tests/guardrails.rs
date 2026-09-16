//! Machine-checked guardrails for the cost figures.
//!
//! RFP-020's M0 asks for cost numbers "reproducible from a benchmark target".
//! These are tests rather than a `cargo bench` target, deliberately: cycle counts
//! are deterministic and take seconds, which suits CI, whereas criterion-style
//! benchmarking wants many iterations of a proof that takes minutes. What M0
//! actually needs is not a timing harness but an assertion that fails a build when
//! the numbers move, and that is what these are.
//!
//! # Why two kinds of assertion
//!
//! The recommended configuration accelerates secp256k1 and leaves keccak256 in
//! software. A `[patch.crates-io]` section fails **silently**: a crate bump, a
//! moved tag, or a dropped section restores a different cost profile with a green
//! build. The two accelerators also fail in opposite directions, so one assertion
//! cannot catch both:
//!
//! - Losing the secp256k1 accelerator makes recovery ~20x more expensive, so
//!   cycles rise sharply.
//! - Gaining the keccak256 accelerator makes cycles *fall* slightly while proving
//!   time and proof size rise sharply.
//!
//! So the fast tests pin exact cycle counts, which catch movement in either
//! direction, and a slow ignored test pins proof size, which is the dimension the
//! keccak coprocessor blows up and cycles cannot see.
//!
//! # Exact equality is intentional
//!
//! Cycle counts are a deterministic function of the guest ELF and its input,
//! reproduced byte-identically across x86_64 and ARM64. Pinning them exactly means
//! any toolchain bump, dependency change, or code change fails here and forces the
//! published figures to be re-tagged deliberately, which is what the README
//! promises. A failure is not necessarily a defect; it is a prompt to re-measure
//! and update both the constants and the documentation together.

use lez_probe::{accelerated, make_packages, measure, mixed, LEZ_CYCLE_BUDGET};

/// Measured on risc0-zkvm 3.0.6, guest rustc 1.97.0-dev, against
/// logos-execution-zone `15144ddb`. Update together with the README.
mod expected {
    pub const FRAMEWORK_FLOOR: u64 = 39_103;
    pub const MIXED_1_SIGNER: u64 = 664_117;
    pub const MIXED_3_SIGNERS: u64 = 1_906_737;
    pub const MIXED_5_SIGNERS: u64 = 3_150_337;
    pub const ACCELERATED_3_SIGNERS: u64 = 1_862_527;

    /// `lez_plumbing` at 1, 3 and 5 signers: the framework's own cost with the
    /// real instruction to carry but no cryptography. Pinned separately from the
    /// floor because the floor is measured on an empty instruction and so cannot
    /// see the part of the framework's work that scales with the signer count.
    pub const MIXED_PLUMBING: [(usize, u64); 3] = [(1, 78_972), (3, 158_290), (5, 237_608)];

    /// Proof sizes are deterministic too, and are the dimension in which the
    /// keccak coprocessor is unmistakable.
    pub const MIXED_3_SIGNERS_PROOF: usize = 564_637;
    pub const ACCELERATED_3_SIGNERS_PROOF: usize = 787_749;
}

fn cycles_of(elf: &[u8], signers: usize) -> u64 {
    let packages = make_packages(signers).expect("building packages");
    measure(elf, &packages, false, 1).expect("execution").cycles
}

#[test]
fn framework_floor_is_unchanged() {
    let run = measure(mixed().noop, &Vec::<u8>::new(), false, 1).expect("execution");
    assert_eq!(
        run.cycles,
        expected::FRAMEWORK_FLOOR,
        "LEZ framework floor moved; re-measure and update the README"
    );
}

/// The framework's per-package instruction handling, which the floor cannot see.
///
/// It is what separates the cryptography from the framework in the published
/// decomposition, so it is pinned in its own right: if it moved and only the
/// totals were pinned, the movement would be silently reattributed to recovery.
/// The accelerated arm must agree exactly, since no cryptography is involved.
#[test]
fn instruction_handling_cost_is_unchanged() {
    for (n, want) in expected::MIXED_PLUMBING {
        assert_eq!(cycles_of(mixed().plumbing, n), want, "mixed, {n} signers");
        assert_eq!(
            cycles_of(accelerated().plumbing, n),
            want,
            "accelerated, {n} signers; this program hashes nothing, so the two \
             configurations cannot differ here"
        );
    }
}

/// Pins the recommended configuration in both directions at once. A sharp rise
/// means the secp256k1 accelerator was lost; a fall of about 44,000 at 3 signers
/// means the keccak256 accelerator was gained.
#[test]
fn mixed_configuration_cycles_are_unchanged() {
    let v = mixed();
    assert_eq!(cycles_of(v.verify, 1), expected::MIXED_1_SIGNER);
    assert_eq!(cycles_of(v.verify, 3), expected::MIXED_3_SIGNERS);
    assert_eq!(cycles_of(v.verify, 5), expected::MIXED_5_SIGNERS);
}

#[test]
fn accelerated_configuration_cycles_are_unchanged() {
    assert_eq!(
        cycles_of(accelerated().verify, 3),
        expected::ACCELERATED_3_SIGNERS
    );
}

/// The secp256k1 accelerator is the one that decides feasibility: without it a
/// 3-of-N update is 33.3M cycles, 99.4% of the budget, and does not fit once
/// decode, median and the account write are added. This is the semantic guard,
/// robust to small drift, that the design still has room.
#[test]
fn a_3_of_n_update_leaves_ample_budget_headroom() {
    let run = measure(mixed().verify, &make_packages(3).unwrap(), false, 1).expect("execution");
    let pct = run.budget_pct();
    assert!(
        pct < 25.0,
        "3-of-N uses {pct:.2}% of the {LEZ_CYCLE_BUDGET}-cycle budget; \
         the secp256k1 accelerator has probably been lost"
    );
}

/// Cost must stay linear in the signer count: nothing is shared between signers,
/// so a departure from linearity means the workload is no longer the one measured.
#[test]
fn per_signer_cost_is_linear() {
    let v = mixed();
    let one = cycles_of(v.verify, 1);
    let three = cycles_of(v.verify, 3);
    let five = cycles_of(v.verify, 5);

    let first = (three - one) / 2;
    let second = (five - three) / 2;
    let drift = (first as f64 - second as f64).abs() / first as f64;
    assert!(
        drift < 0.01,
        "per-signer cost is not linear: {first} then {second} cycles"
    );
}

/// Proof size is where the keccak coprocessor is undeniable: it adds roughly
/// 223 KB while *reducing* cycles, so cycles alone cannot catch it.
///
/// Ignored by default because proving a 1.9M-cycle program takes minutes. Run
/// explicitly:
///
/// ```sh
/// cargo test --release -- --ignored
/// ```
#[test]
#[ignore = "proving takes minutes; run with --ignored"]
fn proof_sizes_distinguish_the_configurations() {
    let packages = make_packages(3).unwrap();

    let m = measure(mixed().verify, &packages, true, 1).expect("proving");
    assert_eq!(
        m.proof_bytes,
        Some(expected::MIXED_3_SIGNERS_PROOF),
        "mixed proof size moved"
    );

    let a = measure(accelerated().verify, &packages, true, 1).expect("proving");
    assert_eq!(
        a.proof_bytes,
        Some(expected::ACCELERATED_3_SIGNERS_PROOF),
        "accelerated proof size moved"
    );

    let penalty = a.proof_bytes.unwrap() - m.proof_bytes.unwrap();
    assert!(
        penalty > 200_000,
        "expected the keccak coprocessor to cost >200 KB of proof, saw {penalty}"
    );
}
