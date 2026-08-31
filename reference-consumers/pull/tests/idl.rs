//! The committed IDL is what the guest source currently says.
//!
//! The artefact beside this crate is a cache of a surface defined in the guest,
//! so the only thing that keeps it true is regenerating it and comparing. It sits
//! here rather than under `kanon-idl/` because that crate is where the canonical
//! price account arrives, and a pull consumer must not depend on it (F9); a JSON
//! file next to the program it describes needs no dependency at all.

use std::path::Path;
use std::process::Command;

fn committed_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("pull-consumer-idl.json")
}

#[test]
fn the_committed_idl_matches_the_guest_source() {
    let generated = Command::new(env!("CARGO_BIN_EXE_generate-pull-consumer-idl"))
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
        "`reference-consumers/pull/pull-consumer-idl.json` is stale. Regenerate it:\n    \
         cargo run -p reference-consumer-pull --bin generate-pull-consumer-idl \
         > reference-consumers/pull/pull-consumer-idl.json"
    );
}

fn instruction(name: &str) -> serde_json::Value {
    let idl: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(committed_path()).expect("committed"))
            .expect("the IDL is JSON");
    idl["instructions"]
        .as_array()
        .expect("an instructions array")
        .iter()
        .find(|ix| ix["name"] == name)
        .unwrap_or_else(|| panic!("{name} is in the IDL"))
        .clone()
}

#[test]
fn the_idl_declares_the_order_derivation_a_client_has_to_reproduce() {
    // The test above proves the generator and the artefact agree. It cannot prove
    // either says what was intended, and this is the part somebody outside this
    // repository depends on: an order's address is derived rather than announced,
    // so a client that cannot compute it cannot open an order or find one again.
    let accounts = instruction("open_order")["accounts"].clone();

    let order = &accounts[0];
    assert_eq!(order["name"], "order");
    assert_eq!(order["writable"], true);
    assert_eq!(
        order["pda"]["seeds"],
        serde_json::json!([
            { "kind": "arg", "path": "order_id" },
            { "kind": "const", "value": reference_consumer_pull::ORDER_ACCOUNT_SEED },
        ]),
        "one order per id, at an address a client has to be able to derive"
    );

    let owner = &accounts[1];
    assert_eq!(owner["name"], "owner");
    assert_eq!(
        owner["signer"], true,
        "opening an order is a statement about its owner"
    );
    assert_eq!(owner["writable"], false);
}

#[test]
fn the_idl_declares_settle_as_permissionless_and_the_clock_as_read_only() {
    // Both halves matter to a caller building a transaction, and both are
    // decisions rather than defaults. No signer, because the sender attests to
    // nothing about a payload; the clock read-only, because ADR 13 makes it the
    // only admissible source of "now" and nothing writes it.
    let accounts = instruction("settle")["accounts"].clone();

    assert_eq!(accounts[0]["name"], "order");
    assert_eq!(accounts[0]["writable"], true);

    assert_eq!(accounts[1]["name"], "clock");
    assert_eq!(accounts[1]["writable"], false);

    for account in accounts.as_array().expect("accounts") {
        assert_eq!(
            account["signer"], false,
            "settling is permissionless: authenticity comes from the payload"
        );
    }
}

#[test]
fn nothing_in_settles_instruction_data_can_reach_the_signer_set() {
    // SEC2, read off the published surface. The security model of pull mode is
    // that the roster is the consumer's, and a reader who wants to check that
    // claim without reading the program can check it here: `settle` takes a
    // payload and nothing else. No signer list, no threshold, no window, no pair.
    //
    // Asserted against the IDL rather than the source because the IDL is what a
    // caller builds against, and because an added parameter would show up here as
    // a failure rather than as a diff nobody read.
    let args = instruction("settle")["args"].clone();
    assert_eq!(
        args,
        serde_json::json!([{ "name": "payload", "type": { "vec": "u8" } }]),
        "a settlement takes the bytes to verify and no configuration"
    );

    // `open_order` chooses which compiled feed an order is against, and that is
    // the whole of what instruction data may influence. An index is not a roster.
    let args = instruction("open_order")["args"].clone();
    assert_eq!(
        args,
        serde_json::json!([
            { "name": "order_id", "type": { "array": ["u8", 32] } },
            { "name": "feed", "type": "u8" },
            { "name": "limit_price_q64", "type": "u128" },
        ])
    );
}
