//! CLI for the LEZ cost probe. The measurement core lives in `lib.rs`, shared with
//! the guardrail tests in `tests/`.

use anyhow::Result;
use clap::Parser;
use lez_probe::{accelerated, make_packages, measure, mixed, Run, Variant, LEZ_CYCLE_BUDGET};

#[derive(Parser)]
struct Cli {
    /// Signer counts to verify. The RFP's default threshold is 3.
    #[arg(long, value_delimiter = ',', default_value = "1,3,5")]
    signers: Vec<usize>,

    /// Also prove each program, as LEZ's private path does, reporting wall-clock
    /// time and proof size. Slow: a 3-of-N program is roughly 1.9M cycles.
    #[arg(long)]
    prove: bool,

    /// Prove this many times and report the fastest with the spread. Proving
    /// wall-clock is noisy enough that a single run can order two measurements
    /// wrongly.
    #[arg(long, default_value_t = 1)]
    repeat: usize,

    /// Only measure this guest configuration: "mixed" or "accelerated" (prefixes
    /// accepted). Default: both.
    #[arg(long)]
    only: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    println!(
        "LEZ public-execution budget: {LEZ_CYCLE_BUDGET} cycles ({} MiB-cycles)",
        LEZ_CYCLE_BUDGET / (1024 * 1024)
    );
    if cli.prove {
        println!(
            "Proving mirrors LEZ's private path: no session limit, as \
             execute_and_prove_program sets none."
        );
    }

    for v in [mixed(), accelerated()] {
        if let Some(only) = &cli.only {
            if !v.name.starts_with(only.as_str()) {
                continue;
            }
        }
        report_variant(&v, &cli)?;
    }
    Ok(())
}

fn report_variant(v: &Variant, cli: &Cli) -> Result<()> {
    println!("\n=== {} configuration ===", v.name);

    let floor = measure(v.noop, &Vec::<u8>::new(), cli.prove, cli.repeat)?;
    println!("\n-- LEZ framework floor (read inputs, echo post state) --");
    report("framework only", &floor, None, None);

    println!("\n-- M-of-N RedStone verification inside a LEZ program --");
    println!(
        "   'crypto alone' subtracts the same program without the crypto, so the \
         framework's\n   per-package instruction handling is excluded rather than \
         counted as recovery."
    );
    for &n in &cli.signers {
        let packages = make_packages(n)?;
        let plumbing = measure(v.plumbing, &packages, false, 1)?;
        let r = measure(v.verify, &packages, cli.prove, cli.repeat)?;
        report(&format!("{n} signer(s)"), &r, Some(&floor), Some(&plumbing));
    }
    Ok(())
}

fn report(label: &str, r: &Run, floor: Option<&Run>, plumbing: Option<&Run>) {
    print!(
        "  {label:>14}: {:>10} cycles, {:>5.2}% of budget",
        r.cycles,
        r.budget_pct()
    );
    if let Some(f) = floor {
        print!(", above the floor {}", r.cycles.saturating_sub(f.cycles));
    }
    if let Some(p) = plumbing {
        print!(
            ", crypto alone {} (instruction handling {})",
            r.cycles.saturating_sub(p.cycles),
            floor.map_or(0, |f| p.cycles.saturating_sub(f.cycles))
        );
    }
    println!();
    if let (Some(secs), Some(bytes)) = (r.prove_secs(), r.proof_bytes) {
        let spread = r
            .prove_spread_pct()
            .map(|p| format!(" (best of {}, spread {p:.1}%)", r.prove_samples.len()))
            .unwrap_or_default();
        println!("                  proved in {secs:.2} s{spread}, {bytes} byte proof");
    }
}
