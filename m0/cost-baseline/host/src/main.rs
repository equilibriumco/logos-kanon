//! RFP-020 M0 cost baseline: the CLI that produces the reported figures.
//!
//! Measures what keccak256 and secp256k1 ECDSA public-key recovery cost inside
//! the RISC Zero zkVM. These are the two primitives a RedStone price update must
//! run on-chain, and RFP-020 makes their in-zkVM cost the primary deliverable.
//!
//! Three configurations are compared, differing only in which of RISC Zero's
//! accelerated crate forks they use: none (`software`), both (`accelerated`), and
//! the secp256k1 one alone (`mixed`).
//!
//! The measurement itself lives in the library crate, so the guardrail tests
//! assert on the same code path that prints these tables. Method and caveats are
//! documented there and in the README; this file is presentation only.
//!
//! Cycles are cheap to obtain, since they need execution only, and they are
//! deterministic across machines. But they are **not** a sound cost metric across
//! configurations: the keccak accelerator moves work into a coprocessor that the
//! cycle count cannot see, so it looks cheaper in cycles while proving more
//! slowly and producing a larger proof. Use `--prove` for any cross-configuration
//! claim.

use anyhow::Result;
use clap::Parser;
use kanon_cost_baseline::{
    keccak_delta, recover_delta, variants, verify_run, Elfs, Opts, Run, KECCAK_ITERS,
    KECCAK_MSG_LENS, RECOVER_SIGNER_COUNTS, REDSTONE_SIGNED_BYTES, VERIFY_SIGNERS,
};

#[derive(Parser)]
struct Cli {
    /// Also generate real proofs, to report proving wall-clock time and proof
    /// size. Much slower than cycle counting.
    #[arg(long)]
    prove: bool,

    /// Only measure this variant: "software", "accelerated", or "mixed"
    /// (prefixes accepted). Default: all.
    #[arg(long)]
    only: Option<String>,

    /// Cap the cycles per segment at 2^N. Lowering this cuts the prover's peak
    /// memory, at the cost of more segments, which is how GPU proving fits on a
    /// card whose VRAM cannot hold a full-size segment. Comparisons are only
    /// valid between runs that used the same value.
    #[arg(long)]
    segment_po2: Option<u32>,

    /// Prove this many times per measurement and report the fastest along with
    /// the spread. Proving wall-clock is noisy enough that a single run can order
    /// two configurations wrongly: a one-shot measurement once showed a strictly
    /// larger workload proving faster than a smaller one. Only meaningful with
    /// --prove.
    #[arg(long, default_value_t = 1)]
    repeat: usize,

    /// Only measure this workload: "noop", "keccak", "recover", or "verify"
    /// (prefixes accepted). Default: all. Useful with --prove, where the software
    /// recovery workload is slow enough to dominate a run. "verify" is the full
    /// 3-of-N update shape and the one the headline figures come from.
    #[arg(long)]
    workload: Option<String>,
}

impl Cli {
    fn opts(&self) -> Opts {
        Opts {
            segment_po2: self.segment_po2,
            prove: self.prove,
            repeat: self.repeat,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    for v in &variants() {
        // Prefix match, so "software"/"accelerated" and shorthands like "soft"
        // or "acc" all select correctly.
        if let Some(only) = &cli.only {
            if !v.name.starts_with(only.as_str()) {
                continue;
            }
        }
        println!("\n=== {} path ===", v.name);
        let want = |w: &str| {
            cli.workload
                .as_ref()
                .is_none_or(|f| w.starts_with(f.as_str()))
        };
        if want("noop") {
            report_noop(v, &cli)?;
        }
        if want("keccak") {
            report_keccak(v, &cli)?;
        }
        if want("recover") {
            report_recover(v, &cli)?;
        }
        if want("verify") {
            report_verify(v, &cli)?;
        }
    }
    Ok(())
}

fn report_noop(v: &Elfs, cli: &Cli) -> Result<()> {
    let run = kanon_cost_baseline::execute_with(v.noop, &(), cli.opts())?;
    println!("\n-- zkVM floor (empty guest) --");
    println!("  {:>12} cycles, {} segment(s)", run.cycles, run.segments);
    if let (Some(s), Some(b)) = (run.prove_secs(), run.proof_bytes) {
        println!(
            "  {s:>12.2} s to prove, {b} byte proof, receipt {}",
            run.receipt_kind.as_deref().unwrap_or("?")
        );
    }
    Ok(())
}

fn report_keccak(v: &Elfs, cli: &Cli) -> Result<()> {
    println!("\n-- keccak256, {KECCAK_ITERS} hashes per run --");
    println!(
        "  {:>10}  {:>12}  {:>10}  {:>11}  {:>10}  {:>12}",
        "msg bytes", "work cycles", "gross/hash", "bookkeeping", "net/hash", "net cyc/byte"
    );
    for &len in KECCAK_MSG_LENS {
        let hash = keccak_delta(v.keccak, KECCAK_ITERS, len, cli.opts())?;
        let per_hash = hash.work as f64 / f64::from(KECCAK_ITERS);

        // The loop guards that stop the compiler deleting or hoisting the hash
        // sit inside the measured region, so price them and report the hash net
        // of them rather than claiming they are negligible. Never proved: only
        // its cycle count is subtracted.
        let bookkeeping = keccak_delta(v.keccak_overhead, KECCAK_ITERS, len, Opts::execute_only())?;
        let per_iter_overhead = bookkeeping.work as f64 / f64::from(KECCAK_ITERS);
        let net = per_hash - per_iter_overhead;

        println!(
            "  {len:>10}  {:>12}  {per_hash:>10.0}  {per_iter_overhead:>11.0}  {net:>10.0}  {:>12.1}",
            hash.work,
            net / f64::from(len)
        );
        report_proof("           ", &hash.busy);
    }
    Ok(())
}

fn report_recover(v: &Elfs, cli: &Cli) -> Result<()> {
    println!("\n-- secp256k1 ECDSA public-key recovery --");
    println!(
        "  {:>7}  {:>12}  {:>14}",
        "signers", "work cycles", "per recovery"
    );
    for &n in RECOVER_SIGNER_COUNTS {
        let run = recover_delta(v.recover, n, cli.opts())?;
        println!(
            "  {n:>7}  {:>12}  {:>14.0}",
            run.work,
            run.work as f64 / n as f64
        );
        report_proof("         ", &run.busy);
    }
    Ok(())
}

/// The full 3-of-N update shape, measuring keccak256's marginal cost inside a
/// real verification instead of in a synthetic loop.
///
/// Cycle counts alone cannot answer whether the keccak accelerator is worth using
/// here, because the accelerator's work is invisible to the cycle count. Running
/// this with `--prove` on both variants gives the comparison that can.
fn report_verify(v: &Elfs, cli: &Cli) -> Result<()> {
    let n = VERIFY_SIGNERS;
    println!(
        "\n-- {n}-of-N verification: {n} packages, each a {REDSTONE_SIGNED_BYTES}-byte hash then a recovery --"
    );

    let without = verify_run(v.verify, n, false, cli.opts())?;
    let with = verify_run(v.verify, n, true, cli.opts())?;

    for (label, run) in [("recover only", &without), ("hash + recover", &with)] {
        print!("  {label:>15}: {:>10} cycles", run.cycles);
        match (run.prove_secs(), run.proof_bytes) {
            (Some(secs), Some(bytes)) => {
                let spread = run
                    .prove_spread_pct()
                    .map(|p| format!(" (best of {}, spread {p:.1}%)", run.prove_samples.len()))
                    .unwrap_or_default();
                println!(
                    ", {secs:>7.2} s{spread}, {bytes:>8} byte proof, {} segment(s), receipt {}",
                    run.segments,
                    run.receipt_kind.as_deref().unwrap_or("?")
                )
            }
            _ => println!(", {} segment(s)", run.segments),
        }
    }

    let cycle_delta = with.cycles as i64 - without.cycles as i64;
    println!(
        "  {:>15}: {cycle_delta:>10} cycles ({:.2}% of the update)",
        format!("{n} hashes cost"),
        100.0 * cycle_delta as f64 / with.cycles as f64
    );
    if let (Some(a), Some(b)) = (with.prove_secs(), without.prove_secs()) {
        let pb = with.proof_bytes.unwrap_or(0) as i64 - without.proof_bytes.unwrap_or(0) as i64;
        println!(
            "  {:>15}  {:+.2} s proving ({:+.1}% of the update), {pb:+} proof bytes",
            "",
            a - b,
            100.0 * (a - b) / a
        );
    }
    Ok(())
}

/// The proving line, printed only when there is one.
fn report_proof(indent: &str, run: &Run) {
    if let (Some(s), Some(bytes)) = (run.prove_secs(), run.proof_bytes) {
        println!(
            "{indent}  proved in {s:.2} s, {bytes} byte proof, {} segment(s), receipt {}",
            run.segments,
            run.receipt_kind.as_deref().unwrap_or("?")
        );
    }
}
