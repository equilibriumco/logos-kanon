//! CLI for the LEZ cost probe, on LEZ v0.3.

use anyhow::Result;
use clap::Parser;
use lez_probe::{
    accelerated, make_packages, measure, mixed, Run, Variant, LEZ_GAS_LIMIT, V02_CYCLE_BUDGET,
};

#[derive(Parser)]
struct Cli {
    /// Signer counts to verify. The RFP's default threshold is 3.
    #[arg(long, value_delimiter = ',', default_value = "1,3,5")]
    signers: Vec<usize>,

    #[arg(long)]
    prove: bool,

    #[arg(long, default_value_t = 1)]
    repeat: usize,

    #[arg(long)]
    only: Option<String>,

    /// Prove with `ProverOpts::succinct()`, as LEZ's private path proves its
    /// second stage, instead of the composite receipt every other figure uses.
    /// This is the size that would matter if proof bytes were ever priced for
    /// real. Slow: recursion costs more than the proof it compresses.
    #[arg(long)]
    succinct: bool,

    /// Split a recovery into the curve arithmetic a precompile cannot remove
    /// and the surrounding work it could. Seconds.
    #[arg(long)]
    ec: bool,

    /// Measure the whole LEZ program with recovery done by `risc0-crypto-evm`
    /// instead of `k256`, rather than inferring it by substitution.
    #[arg(long)]
    r0crypto: bool,

    /// Measure the multi-point round: BTC/ETH/SOL through one `___FAST___`-shaped
    /// payload (9-point signed spans, 5 signers) plus XMR and ZEC as single-point
    /// payloads, against five single-point payloads. Both recovery backends.
    #[arg(long)]
    multifeed: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    println!(
        "LEZ v0.3 action-phase gas limit: {LEZ_GAS_LIMIT} (cycles, one for one). \
         v0.2 allowed {V02_CYCLE_BUDGET} cycles."
    );

    if cli.r0crypto {
        println!("\n-- the whole LEZ program, k256 against risc0-crypto-evm --");
        for n in [1usize, 3, 5] {
            let packages = make_packages(n)?;
            let base = measure(mixed().verify, &packages, false, 1)?;
            let alt = measure(lez_probe_methods_mixed::LEZ_VERIFY_R0CRYPTO_ELF, &packages, false, 1)?;
            println!(
                "  {n} signer(s): k256 {:>9} ({:>5.2}%)   risc0-crypto {:>9} ({:>5.2}%)   {:.2}x",
                base.cycles(), base.gas_pct(), alt.cycles(), alt.gas_pct(),
                base.cycles() as f64 / alt.cycles() as f64
            );
        }
        return Ok(());
    }

    if cli.multifeed {
        use lez_probe::{make_packages_spanned, FAST_SIGNED_BYTES, REDSTONE_SIGNED_BYTES};
        println!(
            "\n-- the multi-point round: ___FAST___ (9-point spans) for BTC/ETH/SOL, \
             singles for XMR/ZEC, 5 signers --"
        );
        for (name, elf) in [
            ("k256", mixed().verify),
            ("k256 + keccak coprocessor", accelerated().verify),
            ("risc0-crypto", lez_probe_methods_mixed::LEZ_VERIFY_R0CRYPTO_ELF),
        ] {
            let single = measure(elf, &make_packages_spanned(5, REDSTONE_SIGNED_BYTES)?, false, 1)?;
            let fast = measure(elf, &make_packages_spanned(5, FAST_SIGNED_BYTES)?, false, 1)?;
            let round_single = 5 * single.cycles();
            let round_fast = fast.cycles() + 2 * single.cycles();
            println!("  {name}:");
            println!(
                "    one single-point update, 5 signers:      {:>10} gas",
                single.cycles()
            );
            println!(
                "    one 9-point FAST update, 5 signers:      {:>10} gas  (+{} for the longer spans)",
                fast.cycles(),
                fast.cycles().saturating_sub(single.cycles())
            );
            println!(
                "    round, five single-point updates:        {:>10} gas = {:>6.2}% of a block",
                round_single,
                100.0 * round_single as f64 / lez_probe::LEZ_GAS_LIMIT as f64
            );
            println!(
                "    round, FAST + XMR + ZEC:                 {:>10} gas = {:>6.2}% of a block  ({:.2}x cheaper)",
                round_fast,
                100.0 * round_fast as f64 / lez_probe::LEZ_GAS_LIMIT as f64,
                round_single as f64 / round_fast as f64
            );
        }
        return Ok(());
    }

    if cli.ec {
        let (per_call, raw, lincomb, recover, r0crypto, rounds) = lez_probe::ec_breakdown()?;
        let per = |x: u64| x / u64::from(rounds);
        println!("\n-- a recovery, split by what a native ecrecover could remove --");
        println!("  bigint2 circuits alone (2 muls + add):   {:>9}   irreducible", per(raw));
        println!("  the same through k256:                   {:>9}   +{} k256 overhead", per(lincomb), per(lincomb.saturating_sub(raw)));
        println!("  full ECDSA recovery:                     {:>9}   +{} recovery logic", per(recover), per(recover.saturating_sub(lincomb)));
        println!();
        println!("  via risc0-crypto-evm ecrecover:          {:>9}   {:.2}x cheaper", per(r0crypto), recover as f64 / r0crypto as f64);
        println!();
        println!("  one bigint2 accelerator round trip:      {per_call:>9}");
        println!("  so a 256-bit scalar mul (~384 calls):    {:>9}   as loop overhead", per_call * 384);
        println!();
        println!("  irreducible share of a recovery:          {:>8.1}%", 100.0 * raw as f64 / recover as f64);
        println!("  removable at best:                        {:>9}  ({:.1}%)",
            per(recover.saturating_sub(raw)), 100.0 * (recover - raw) as f64 / recover as f64);
        return Ok(());
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

    if cli.succinct {
        for &n in &cli.signers {
            let packages = make_packages(n)?;
            let (secs, bytes) = lez_probe::prove_succinct(v.verify, &packages)?;
            println!(
                "  {n} signer(s), succinct: {bytes} byte proof in {secs:.2} s"
            );
        }
        return Ok(());
    }

    let floor = measure(v.noop, &Vec::<u8>::new(), cli.prove, cli.repeat)?;
    println!("\n-- LEZ framework floor (decode the call, name one shard, write it) --");
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
    let applies: Vec<String> = r.applies.iter().map(ToString::to_string).collect();
    print!(
        "  {label:>14}: {:>10} gas = plan {} + apply {} | {:>5.2}% of the v0.3 limit \
         ({:>5.2}% of v0.2's)",
        r.cycles(),
        r.plan,
        applies.join(" + "),
        r.gas_pct(),
        r.v02_budget_pct()
    );
    if let Some(f) = floor {
        print!(", above the floor {}", r.cycles().saturating_sub(f.cycles()));
    }
    if let Some(p) = plumbing {
        print!(
            ", crypto alone {} (instruction handling {})",
            r.cycles().saturating_sub(p.cycles()),
            floor.map_or(0, |f| p.cycles().saturating_sub(f.cycles()))
        );
    }
    println!();
    if let (Some(secs), Some(bytes)) = (r.prove_secs(), r.proof_bytes) {
        let spread = r
            .prove_spread_pct()
            .map(|p| format!(" (best of {}, spread {p:.1}%)", r.prove_samples.len()))
            .unwrap_or_default();
        println!("                  plan proved in {secs:.2} s{spread}, {bytes} byte proof");
    }
}
