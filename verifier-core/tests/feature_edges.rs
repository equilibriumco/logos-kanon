//! `test-fixtures` reaches no shipped guest.
//!
//! The feature exposes `test_support`, which carries a signing key and a payload
//! builder and turns on `extern crate std` in a `no_std` library. It is meant for
//! another crate's *tests* and nothing else, and `[M2-16:01]` records why it
//! exists at all.
//!
//! **This file exists because the argument that was supposed to hold that line
//! does not.** The claim was that the `no_std` job would catch a shipped build
//! enabling the feature, since it builds the guest-reachable crates for
//! `riscv32im-unknown-none-elf` where there is no `std`. Two things wrong with it,
//! both measured:
//!
//! 1. That job builds `-p verifier-core -p pull-lib -p kanon-clock`. It never
//!    builds `aggregator-program`, so a feature enabled *by* `aggregator-program`
//!    is outside what it compiles.
//! 2. Even reaching it would not help. The product guest targets
//!    `riscv32im-risc0-zkvm-elf`, and risc0's toolchain **ships `std` for that
//!    target** -- `extern crate std` is no barrier where the guest actually
//!    builds. `cargo build -p kanon-methods` with the feature on a normal edge
//!    succeeds and compiles the fixtures straight into the ELF.
//!
//! Nor does M3-02's closure walk see it: that compares package *names*, and a
//! feature adds no package. So all three of the things named as guards were blind
//! to the one move that matters -- writing `features = ["test-fixtures"]` under
//! `[dependencies]` instead of `[dev-dependencies]`.
//!
//! ADR 21 and ADR 23 both rejected exposing `test_support` behind a feature, for
//! the reason this file is about: fixture code does not belong in the published
//! surface of a crate a consumer program links. They were right about the risk.
//! This is what makes the narrower thing they did not consider -- a dev-only
//! feature -- hold mechanically rather than by intention.

use std::process::Command;

/// Every package reachable from `root` by `normal` edges, with its enabled
/// features, as `cargo tree` reports them.
///
/// `--edges normal` is the whole point: it excludes `dev` and `build`, so a
/// dev-dependency's features do not appear and a normal dependency's do.
fn normal_edge_features(manifest: &str, root: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--color",
            "never",
            "--target",
            "all",
            "--manifest-path",
            manifest,
            "-p",
            root,
            "--edges",
            "normal",
            "--format",
            "{p} {f}",
            "--prefix",
            "none",
        ])
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree failed for {root}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo tree output is utf-8")
}

/// The guest workspaces, read from the root manifest's `exclude` list.
///
/// A guest workspace is excluded because it cross-compiles under its own
/// resolution and its own lockfile, which is exactly why the root walk cannot see
/// it. `m0` is excluded too and is a measurement workspace rather than a guest, so
/// it is filtered on having a guest's shape: a `[patch.crates-io]` section.
fn excluded_guest_workspaces() -> Vec<String> {
    let root = std::fs::read_to_string("../Cargo.toml").expect("the root manifest");
    let excludes = root
        .lines()
        .skip_while(|line| line.trim() != "exclude = [")
        .skip(1)
        .take_while(|line| !line.trim().starts_with(']'))
        .filter_map(|line| line.split('"').nth(1))
        .map(str::to_owned);

    excludes
        .filter_map(|dir| {
            let manifest = format!("../{dir}/Cargo.toml");
            let text = std::fs::read_to_string(&manifest).ok()?;
            text.lines()
                .any(|line| line.trim() == "[patch.crates-io]")
                .then_some(manifest)
        })
        .collect()
}

/// The `name` a manifest declares.
fn package_name(manifest: &str) -> String {
    std::fs::read_to_string(manifest)
        .unwrap_or_else(|_| panic!("{manifest} is on disk"))
        .lines()
        .find_map(|line| {
            line.strip_prefix("name = ")
                .and_then(|rest| rest.split('"').nth(1))
        })
        .unwrap_or_else(|| panic!("{manifest} declares a package name"))
        .to_owned()
}

#[test]
fn the_product_guests_reach_no_fixture_feature() {
    // The root workspace's own guest-reachable crates, then every excluded guest
    // workspace. Discovered from the root manifest's `exclude` list rather than
    // written out, so a guest workspace added later is covered without anyone
    // remembering this file -- which is the failure mode a hardcoded list has.
    let mut targets: Vec<(String, String)> = vec![
        ("../Cargo.toml".to_owned(), "aggregator-program".to_owned()),
        ("../Cargo.toml".to_owned(), "pull-lib".to_owned()),
    ];
    for manifest in excluded_guest_workspaces() {
        let root = package_name(&manifest);
        targets.push((manifest, root));
    }
    assert!(
        targets.len() >= 3,
        "found no guest workspaces to check, so this test guards nothing: {targets:#?}"
    );

    for (manifest, root) in &targets {
        let tree = normal_edge_features(manifest, root);
        let offenders: Vec<&str> = tree
            .lines()
            .filter(|line| line.contains("test-fixtures"))
            .collect();
        assert!(
            offenders.is_empty(),
            "`test-fixtures` is enabled on a normal edge of {root}, so it compiles \
             into a shipped guest -- risc0's target has `std`, so nothing else will \
             refuse it. Move the dependency to [dev-dependencies]: {offenders:#?}"
        );
    }
}

#[test]
fn the_feature_is_off_unless_someone_asks() {
    // The other half: that the walk above would notice. A test asserting an
    // absence proves nothing until something can make it present -- so enable the
    // feature explicitly and require it to show up.
    let tree = normal_edge_features("Cargo.toml", "verifier-core");
    assert!(
        !tree.contains("test-fixtures"),
        "verifier-core enables its own fixture feature by default: {tree}"
    );

    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--color",
            "never",
            "-p",
            "verifier-core",
            "--features",
            "test-fixtures",
            "--edges",
            "normal",
            "--format",
            "{p} {f}",
            "--prefix",
            "none",
        ])
        .output()
        .expect("cargo tree runs");
    let asked = String::from_utf8(output.stdout).expect("utf-8");
    assert!(
        asked.contains("test-fixtures"),
        "asking for the feature did not enable it, so the assertion above is \
         vacuous and this file guards nothing: {asked}"
    );
}
