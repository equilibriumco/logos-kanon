//! The committed IDL is what the guest source currently says.
//!
//! The artefact under `kanon-idl/` is a cache of a surface defined elsewhere, so
//! the only thing that keeps it true is regenerating it and comparing. A changed
//! instruction with a stale artefact would ship an SDK and a CLI built against a
//! surface the program no longer has, and nothing else in the build would notice.

use std::path::Path;
use std::process::Command;

#[test]
fn the_committed_idl_matches_the_guest_source() {
    let generated = Command::new(env!("CARGO_BIN_EXE_generate-idl"))
        .output()
        .expect("the generator builds and runs");
    assert!(
        generated.status.success(),
        "the generator failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let committed_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kanon-idl")
        .join("aggregator-idl.json");
    let committed = std::fs::read_to_string(&committed_path).expect("the artefact is committed");

    let generated = String::from_utf8(generated.stdout).expect("the IDL is UTF-8");

    assert_eq!(
        generated.trim(),
        committed.trim(),
        "`kanon-idl/aggregator-idl.json` is stale. Regenerate it:\n    \
         cargo run -p aggregator-program --bin generate-idl > kanon-idl/aggregator-idl.json"
    );
}

/// The committed artefact, parsed.
fn committed_idl() -> serde_json::Value {
    let committed_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kanon-idl")
        .join("aggregator-idl.json");
    serde_json::from_str(&std::fs::read_to_string(committed_path).expect("committed"))
        .expect("the IDL is JSON")
}

/// The `submit_price` accounts the committed artefact declares, by name.
fn submit_price_accounts() -> serde_json::Value {
    committed_idl()["instructions"]
        .as_array()
        .expect("an instructions array")
        .iter()
        .find(|ix| ix["name"] == "submit_price")
        .expect("submit_price is in the IDL")["accounts"]
        .clone()
}

#[test]
fn only_a_submission_may_write_a_price() {
    // R1: a price read is read-only and never modifies adaptor state.
    //
    // In push mode a read is an account read -- a consumer decodes the canonical
    // account and sends no transaction to this program at all, so there is no
    // instruction to make read-only. What can be asserted is the converse, and
    // the IDL is where it is legible to a caller rather than only true: of the ten
    // instructions, exactly one declares a price account it may write.
    //
    // `register_feed` is the case that makes this worth a test rather than a
    // glance. It *does* take the price account -- it reads the pair a feed id has
    // already published, so a re-registration cannot repoint the id at another
    // pair (ADR 33) -- and it takes it read-only. An instruction that acquired a
    // writable price account for a read would be invisible in every test that
    // checks what a body returns, because a post-state that happens to equal its
    // pre-state passes them all. It is visible here.
    let idl = committed_idl();
    let instructions = idl["instructions"].as_array().expect("an array");

    let mut writers = Vec::new();
    let mut readers = Vec::new();
    for ix in instructions {
        let name = ix["name"].as_str().expect("a name");
        for account in ix["accounts"].as_array().expect("accounts") {
            if account["name"] == "price_account" {
                if account["writable"] == serde_json::Value::Bool(true) {
                    writers.push(name);
                } else {
                    readers.push(name);
                }
            }
        }
    }

    assert_eq!(
        writers,
        vec!["submit_price"],
        "exactly one instruction may write a price, and it is the submission"
    );
    assert_eq!(
        readers,
        vec!["register_feed"],
        "the only other instruction that sees a price account reads it, for the \
         pair check ADR 33 records"
    );
}

#[test]
fn the_idl_declares_the_seeds_and_writability_the_program_relies_on() {
    // The test above proves the generator and the artefact agree. It cannot
    // prove either says what was intended, and three of these are load-bearing
    // for somebody outside this repository: a relayer derives the price
    // account's address from these seeds, and a caller marks accounts writable
    // from these flags.
    let accounts = submit_price_accounts();

    let feed = &accounts[0];
    assert_eq!(feed["name"], "feed");
    assert_eq!(
        feed["writable"], false,
        "`submit_price` reads the feed and never writes it"
    );

    let price = &accounts[1];
    assert_eq!(price["name"], "price_account");
    assert_eq!(price["writable"], true);
    assert_eq!(
        price["pda"]["seeds"],
        serde_json::json!([
            { "kind": "account", "path": "feed" },
            { "kind": "const", "value": aggregator_program::submit::PRICE_ACCOUNT_SEED },
        ]),
        "one price account per feed, at an address a client has to be able to derive"
    );

    assert_eq!(accounts[2]["name"], "clock");
    assert_eq!(
        accounts[2]["writable"], false,
        "the clock is read, and ADR 13 is the reason it is read from that account"
    );
}

#[test]
fn the_feeds_owner_constraint_is_deliberately_absent_from_the_idl() {
    // Recorded as a test because it is surprising, and because the natural fix
    // for the surprise is wrong. `submit_price` requires the feed account to be
    // owned by this program, and that requirement is enforced in
    // `aggregator_program::submit` rather than by `#[account(owner = ...)]`:
    // SPEL's IDL generator parses `owner` and discards it, so a declarative
    // constraint would be invisible to every generated client while looking, in
    // the source, as though it were published. If a later SPEL starts emitting
    // it, this test fails and the choice is worth revisiting.
    let accounts = submit_price_accounts();
    for account in accounts.as_array().expect("accounts") {
        assert!(
            account.get("owner").is_none(),
            "SPEL began publishing `owner`: move the feed's ownership check back \
             to a constraint and delete this test"
        );
    }
}
