//! The committed IDL is what the guest source currently says.
//!
//! The artefact beside this crate is a cache of a surface defined in the guest,
//! so the only thing that keeps it true is regenerating it and comparing.

use std::path::Path;
use std::process::Command;

fn committed_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("aggregator-read-consumer-idl.json")
}

fn idl() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(committed_path()).expect("committed"))
        .expect("the IDL is JSON")
}

fn instruction(name: &str) -> serde_json::Value {
    idl()["instructions"]
        .as_array()
        .expect("an instructions array")
        .iter()
        .find(|ix| ix["name"] == name)
        .unwrap_or_else(|| panic!("{name} is in the IDL"))
        .clone()
}

fn accounts(name: &str) -> Vec<serde_json::Value> {
    instruction(name)["accounts"]
        .as_array()
        .expect("an accounts array")
        .clone()
}

fn account(name: &str, account: &str) -> serde_json::Value {
    accounts(name)
        .into_iter()
        .find(|a| a["name"] == account)
        .unwrap_or_else(|| panic!("{name} takes {account}"))
}

fn seeds(name: &str, account_name: &str) -> Vec<(String, String)> {
    account(name, account_name)["pda"]["seeds"]
        .as_array()
        .unwrap_or_else(|| panic!("{account_name} in {name} declares seeds"))
        .iter()
        .map(|seed| {
            let kind = seed["kind"].as_str().expect("a seed kind").to_owned();
            let value = seed
                .get("path")
                .or_else(|| seed.get("value"))
                .and_then(serde_json::Value::as_str)
                .expect("a seed value")
                .to_owned();
            (kind, value)
        })
        .collect()
}

#[test]
fn the_committed_idl_matches_the_guest_source() {
    let generated = Command::new(env!("CARGO_BIN_EXE_generate-aggregator-read-idl"))
        .output()
        .expect("the generator builds and runs");
    assert!(
        generated.status.success(),
        "the generator failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let committed = std::fs::read_to_string(committed_path()).expect("the artefact is committed");
    let generated = String::from_utf8(generated.stdout).expect("the IDL is UTF-8");

    assert_eq!(
        generated.trim(),
        committed.trim(),
        "`reference-consumers/aggregator-read/aggregator-read-consumer-idl.json` is stale. \
         Regenerate it:\n    cargo run -p reference-consumer-aggregator-read \
         --bin generate-aggregator-read-idl \
         > reference-consumers/aggregator-read/aggregator-read-consumer-idl.json"
    );
}

/// The security model of this mode is that which account holds the price is the
/// authority's decision, not the caller's. A reader who wants to check that
/// without reading the program can check it here: `settle` takes a feed id and
/// four accounts. No aggregator id, no pair, no window, no timestamp.
#[test]
fn nothing_in_settles_instruction_data_can_reach_the_price_source() {
    let settle = instruction("settle");
    let args: Vec<&str> = settle["args"]
        .as_array()
        .expect("an args array")
        .iter()
        .map(|arg| arg["name"].as_str().expect("a name"))
        .collect();

    assert_eq!(args, ["feed_id"]);
}

/// The gate, read off the published surface. Every instruction that changes what
/// this program believes about a feed declares a signer and touches the config
/// account; the two that act on a price declare neither.
#[test]
fn every_instruction_that_moves_what_the_program_trusts_declares_a_signer() {
    for name in [
        "establish_authority",
        "nominate_authority",
        "accept_authority",
        "register_price_source",
        "update_aggregator",
        "deregister_price_source",
    ] {
        assert!(
            accounts(name).iter().any(|a| a["signer"] == true),
            "{name} must declare a signer"
        );
    }

    assert!(
        !accounts("settle").iter().any(|a| a["signer"] == true),
        "settling is permissionless: the sender attests to nothing"
    );
}

/// None of these addresses is announced anywhere: a client computes them or it
/// cannot build a transaction at all. The seeds are the contract.
#[test]
fn the_idl_declares_every_derivation_a_client_has_to_reproduce() {
    for name in [
        "establish_authority",
        "nominate_authority",
        "accept_authority",
    ] {
        assert_eq!(
            seeds(name, "config"),
            [("const".to_owned(), "KANON_READ_CONFIG".to_owned())]
        );
    }

    for name in [
        "register_price_source",
        "update_aggregator",
        "deregister_price_source",
    ] {
        assert_eq!(
            seeds(name, "source"),
            [
                ("arg".to_owned(), "feed_id".to_owned()),
                ("const".to_owned(), "KANON_READ_SOURCE".to_owned()),
            ]
        );
    }

    assert_eq!(
        seeds("open_order", "order"),
        [
            ("arg".to_owned(), "order_id".to_owned()),
            ("const".to_owned(), "KANON_READ_ORDER".to_owned()),
        ]
    );
}

/// The one address a client has to derive that the IDL cannot publish, and the
/// reason is worth a test rather than only a comment: it is derived under the
/// *aggregator's* program id, and SPEL's `pda` constraint derives under
/// `self_program_id`. A future author who "fixed" the omission by adding a
/// constraint would produce an address no aggregator ever writes to, and every
/// settlement would refuse.
///
/// Two other accounts carry no constraint either and neither is this case:
/// `settle`'s `order` is identified by being this program's and decoding as an
/// order rather than by an id in the instruction data, and `clock` is at a fixed
/// address that no seed derives.
#[test]
fn the_price_account_carries_no_pda_constraint() {
    assert!(
        account("settle", "price").get("pda").is_none(),
        "the price account is another program's PDA and the constraint cannot express one"
    );
}

/// Only the accounts an instruction really writes are writable. The price
/// account is read and never written by this program, and never could be: it
/// belongs to the aggregator.
#[test]
fn this_program_never_asks_to_write_a_price_account() {
    assert_eq!(account("settle", "price")["writable"], false);
    assert_eq!(account("settle", "clock")["writable"], false);
    assert_eq!(account("settle", "source")["writable"], false);
    assert_eq!(account("settle", "order")["writable"], true);
}
