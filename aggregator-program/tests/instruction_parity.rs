//! The IDL's instruction order and the wire enum's must agree.
//!
//! Two independent declarations describe one instruction set. The IDL is
//! generated from the guest function order; the transaction is decoded by
//! `read_lee_inputs::<Instruction>()`, which uses this crate's enum. A caller
//! encodes the discriminant from the IDL -- `spel-cli` takes
//! `idl.instructions.iter().position(|i| i.name == ix.name)` -- and the guest
//! reads it as the enum's declaration index.
//!
//! Nothing in the build ties those together. Reordering two variants with the
//! same argument and account shapes still compiles, still generates a valid IDL,
//! and sends a caller's `deregister_feed` to `pause_feed`. Same for reordering
//! two same-typed fields inside one variant, which shifts values between
//! arguments with no type error anywhere.
//!
//! So the agreement is asserted here, against the encoding the wire uses rather
//! than against a restatement of it.

use aggregator_program::Instruction;
use serde_json::Value;

/// One value per variant. Order is deliberately not meaningful: each variant's
/// discriminant is read out of its own encoding, so this list only has to be
/// complete, and the count assertion is what checks that it is.
fn every_variant() -> Vec<Instruction> {
    vec![
        Instruction::SubmitPrice {
            payload: vec![1, 2, 3],
        },
        Instruction::RegisterFeed {
            feed_id: b"BTC".to_vec(),
            base_asset: [1u8; 32],
            quote_asset: [2u8; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: vec![[3u8; 20]],
            threshold: 1,
        },
        Instruction::UpdateSignerSet {
            signers: vec![[4u8; 20]],
            threshold: 2,
        },
        Instruction::DeregisterFeed,
        Instruction::PauseFeed,
        Instruction::UnpauseFeed,
        Instruction::InitialiseAdmin,
        Instruction::NominateAdmin {
            new_admin: [5u8; 32],
        },
        Instruction::AcceptAdmin,
        Instruction::RevokeAdmin,
    ]
}

/// The discriminant the wire carries, which is the first word risc0's serde
/// writes for an enum.
fn wire_discriminant(instruction: &Instruction) -> u32 {
    let words = risc0_zkvm::serde::to_vec(instruction).expect("the instruction encodes");
    *words.first().expect("an encoded enum has a discriminant")
}

/// The variant's name and its field names, in declaration order.
///
/// Read off the serialised text rather than out of a `serde_json::Value`. A
/// `Value` holds its object in a `BTreeMap`, which sorts the keys and so destroys
/// the one property being asserted; serialising writes each key as it is reached,
/// which is declaration order. The `preserve_order` feature would give the same
/// answer, and would also reorder what the IDL generator emits, because the
/// feature is unified across the graph.
///
/// Serde's external tagging puts the name at the only key of the outer object. A
/// unit variant serialises to a bare string and has no fields.
fn name_and_fields(instruction: &Instruction) -> (String, Vec<String>) {
    let text = serde_json::to_string(instruction).expect("the instruction serialises");
    let name = match serde_json::from_str::<Value>(&text).expect("it is JSON") {
        Value::String(name) => return (name, Vec::new()),
        Value::Object(map) => map.keys().next().expect("one variant per value").clone(),
        other => panic!("expected an externally tagged variant, got {other:?}"),
    };
    (name, keys_of_the_inner_object(&text))
}

/// The keys of the one object nested inside the outer one, in the order written.
///
/// A depth-aware scan rather than a search for `"key":`, so a field whose own
/// value is an object cannot contribute its keys to the answer.
fn keys_of_the_inner_object(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut current = String::new();
    let mut expecting_key = false;

    for character in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
                if expecting_key && depth == 2 {
                    keys.push(std::mem::take(&mut current));
                }
                expecting_key = false;
                continue;
            }
            if expecting_key {
                current.push(character);
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                expecting_key = true;
                current.clear();
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    keys
}

/// `SubmitPrice` and `submit_price` are the same instruction under two
/// conventions: the enum is Rust's, the IDL takes the guest function's.
fn snake_case(camel: &str) -> String {
    let mut out = String::new();
    for (index, character) in camel.char_indices() {
        if character.is_ascii_uppercase() {
            if index != 0 {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

fn idl() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kanon-idl")
        .join("aggregator-idl.json");
    let text = std::fs::read_to_string(path).expect("the artefact is committed");
    serde_json::from_str(&text).expect("the artefact is JSON")
}

#[test]
fn each_variant_decodes_to_the_instruction_the_idl_names_at_its_index() {
    let idl = idl();
    let instructions = idl["instructions"]
        .as_array()
        .expect("the IDL lists instructions");

    for variant in every_variant() {
        let (name, _) = name_and_fields(&variant);
        let expected = snake_case(&name);
        let index = wire_discriminant(&variant) as usize;

        let at_index = instructions
            .get(index)
            .unwrap_or_else(|| panic!("the IDL has no instruction at index {index} for {name}"));

        assert_eq!(
            at_index["name"].as_str(),
            Some(expected.as_str()),
            "`Instruction::{name}` encodes discriminant {index}, where the IDL names \
             `{}`. A caller building `{expected}` from the IDL would reach the wrong \
             handler. Reorder the guest functions in \
             `methods/guest/src/bin/aggregator.rs` or the variants in \
             `src/instruction.rs` so the two agree, then regenerate the IDL.",
            at_index["name"].as_str().unwrap_or("<unnamed>"),
        );
    }
}

#[test]
fn each_variants_fields_are_the_arguments_the_idl_lists_in_that_order() {
    let idl = idl();
    let instructions = idl["instructions"]
        .as_array()
        .expect("the IDL lists instructions");

    for variant in every_variant() {
        let (name, fields) = name_and_fields(&variant);
        let index = wire_discriminant(&variant) as usize;
        let published: Vec<String> = instructions[index]["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .map(|arg| arg["name"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default();

        assert_eq!(
            fields, published,
            "`Instruction::{name}` declares its fields in a different order than the \
             IDL lists its arguments. Values are encoded positionally, so a caller \
             would fill each argument from the wrong one, and same-typed fields would \
             do it without any error.",
        );
    }
}

#[test]
fn the_enum_and_the_idl_describe_the_same_number_of_instructions() {
    let idl = idl();
    let published = idl["instructions"]
        .as_array()
        .expect("the IDL lists instructions")
        .len();

    // What this catches is a variant added to one side only. Without it the two
    // tests above pass while a whole instruction goes unchecked, because they
    // only ever look at the variants this file remembers to list.
    assert_eq!(
        every_variant().len(),
        published,
        "the IDL publishes {published} instructions and this test covers {}. \
         A new instruction needs a guest function, an `Instruction` variant, and a \
         value in `every_variant`.",
        every_variant().len(),
    );
}
