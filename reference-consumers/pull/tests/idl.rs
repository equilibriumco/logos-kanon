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

#[test]
fn nothing_in_settles_instruction_data_can_reach_the_signer_set() {
    // SEC2, read off the published surface. The security model of pull mode is
    // that the roster is the consumer's, and a reader who wants to check that
    // claim without reading the program can check it here: `settle` takes a feed
    // id and a payload. No signer list, no threshold, no window, no pair.
    //
    // Asserted against the IDL rather than the source because the IDL is what a
    // caller builds against, and because an added parameter shows up here as a
    // failure rather than as a diff nobody read.
    assert_eq!(
        instruction("settle")["args"],
        serde_json::json!([
            { "name": "feed_id", "type": { "array": ["u8", 32] } },
            { "name": "payload", "type": { "vec": "u8" } },
        ]),
        "a settlement takes the bytes to verify and the feed to verify them for"
    );

    // And the accounts it reads for configuration are not writable by it, so a
    // settlement cannot move what the program trusts even for the feed it names.
    let accounts = instruction("settle")["accounts"].clone();
    assert_eq!(accounts[0]["name"], "order");
    assert_eq!(accounts[0]["writable"], true);
    assert_eq!(accounts[1]["name"], "trust");
    assert_eq!(
        accounts[1]["writable"], false,
        "a settlement reads what the program trusts and never writes it"
    );
    assert_eq!(accounts[2]["name"], "clock");
    assert_eq!(accounts[2]["writable"], false);

    for account in accounts.as_array().expect("accounts") {
        assert_eq!(
            account["signer"], false,
            "settling is permissionless: authenticity comes from the payload"
        );
    }
}

#[test]
fn every_instruction_that_moves_what_the_program_trusts_declares_a_signer() {
    // The other half of SEC2, and the half a compiled roster did not have. The
    // roster is state now, so what protects it is the authority gate -- and a
    // client reading this IDL has to be told that these four need a signature,
    // because `signer` metadata is what a transaction builder acts on.
    //
    // The gate is enforced in Rust as well: the dispatcher checks the flag and
    // `authority::authorise` checks the key. A missing annotation here would not
    // be an authorisation hole, but it would be a silent change to what every
    // generated client believes.
    for name in [
        "establish_authority",
        "nominate_authority",
        "accept_authority",
        "register_feed_trust",
        "rotate_signers",
        "deregister_feed_trust",
    ] {
        let accounts = instruction(name)["accounts"].clone();
        assert!(
            accounts
                .as_array()
                .expect("accounts")
                .iter()
                .any(|a| a["signer"] == true),
            "{name} changes what this program trusts and declares no signer"
        );
    }
}

#[test]
fn the_idl_declares_every_derivation_a_client_has_to_reproduce() {
    // None of these addresses is announced anywhere: a client computes them or it
    // cannot build a transaction at all. The seeds are the contract.
    assert_eq!(
        instruction("establish_authority")["accounts"][0]["pda"]["seeds"],
        serde_json::json!([
            { "kind": "const", "value": reference_consumer_pull::CONFIG_ACCOUNT_SEED },
        ]),
        "one config account per program"
    );
    assert_eq!(
        instruction("register_feed_trust")["accounts"][0]["pda"]["seeds"],
        serde_json::json!([
            { "kind": "arg", "path": "feed_id" },
            { "kind": "const", "value": reference_consumer_pull::TRUST_ACCOUNT_SEED },
        ]),
        "one trust account per feed, at the address its id derives"
    );
    // `open_order` publishes the owner's expected pair as instruction data, which
    // a client has to fill in rather than leave to the program. That is the
    // independent record `settle` compares against the registration, so a client
    // that omitted it would be asking the program to check a value against
    // itself.
    assert_eq!(
        instruction("open_order")["args"],
        serde_json::json!([
            { "name": "order_id", "type": { "array": ["u8", 32] } },
            { "name": "feed_id", "type": { "array": ["u8", 32] } },
            { "name": "base_asset", "type": { "array": ["u8", 32] } },
            { "name": "quote_asset", "type": { "array": ["u8", 32] } },
            { "name": "limit_price_q64", "type": "u128" },
        ])
    );
    assert_eq!(
        instruction("open_order")["accounts"][0]["pda"]["seeds"],
        serde_json::json!([
            { "kind": "arg", "path": "order_id" },
            { "kind": "const", "value": reference_consumer_pull::ORDER_ACCOUNT_SEED },
        ]),
        "one order per id"
    );
    // `settle` constrains the trust account it reads to the feed it names, which
    // is what stops a caller substituting another feed's roster.
    assert_eq!(
        instruction("settle")["accounts"][1]["pda"]["seeds"],
        serde_json::json!([
            { "kind": "arg", "path": "feed_id" },
            { "kind": "const", "value": reference_consumer_pull::TRUST_ACCOUNT_SEED },
        ])
    );
}

#[test]
fn the_owner_constraint_is_deliberately_absent_from_the_idl() {
    // Recorded as a test because it is surprising and the natural fix is wrong.
    // Ownership of the order, the trust and the config is checked in Rust rather
    // than by `#[account(owner = ...)]`, because SPEL's IDL generator parses
    // `owner` and discards it — a declarative constraint would be invisible to
    // every generated client while looking, in the source, as though it were
    // published. If a later SPEL starts emitting it, this fails and the choice is
    // worth revisiting.
    for instructions in idl()["instructions"].as_array().expect("instructions") {
        for account in instructions["accounts"].as_array().expect("accounts") {
            assert!(
                account.get("owner").is_none(),
                "SPEL began publishing `owner`: move the ownership checks back to \
                 constraints and delete this test"
            );
        }
    }
}

/// The enum's variants are the guest's handlers, in the same order.
///
/// This is the invariant `#[lez_program]` used to hold for free. While it
/// generated the enum, a handler and its variant could not disagree; pointing it
/// at `reference_consumer_pull::Instruction` moved the enum somewhere a
/// host-side caller can reach, and moved the ordering into a human's hands with
/// it.
///
/// The order is the whole encoding. The dispatcher deserialises with
/// `risc0_zkvm::serde`, which writes a variant's *position*, so swapping two
/// declarations here sends every caller's `RotateSigners` to `RegisterFeedTrust`
/// and compiles cleanly on both sides. Nothing else in this repository would
/// notice: the guest tests call handlers by name, and the IDL is read off the
/// handlers rather than off the enum.
///
/// Read from the source text rather than from the type, because declaration
/// order is what is being asserted and a value carries no trace of it. The same
/// technique `generate_idl!` uses on the guest.
#[test]
fn the_instruction_variants_are_the_guests_handlers_in_order() {
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/instruction.rs"))
            .expect("the instruction module is on disk");

    // A variant is a line indented four spaces starting with a capital, inside
    // the one enum this file declares.
    let variants: Vec<String> = source
        .lines()
        .skip_while(|line| !line.starts_with("pub enum Instruction {"))
        .filter_map(|line| {
            let name = line.strip_prefix("    ")?;
            let name = name.strip_suffix(" {").or_else(|| name.strip_suffix(','))?;
            name.chars()
                .next()
                .is_some_and(char::is_uppercase)
                .then(|| name.to_owned())
        })
        .collect();

    let handlers: Vec<String> = idl()["instructions"]
        .as_array()
        .expect("an instructions array")
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_owned())
        .collect();

    assert_eq!(
        variants.len(),
        handlers.len(),
        "the enum declares {} variants and the guest {} handlers: {variants:?} against {handlers:?}",
        variants.len(),
        handlers.len()
    );

    for (position, (variant, handler)) in variants.iter().zip(&handlers).enumerate() {
        assert_eq!(
            &pascal_case(handler),
            variant,
            "position {position} is `{handler}` in the guest and `{variant}` in the enum. \
             A caller encodes the position, so a transaction built for one would execute \
             the other"
        );
    }
}

/// `register_feed_trust` to `RegisterFeedTrust`.
fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
