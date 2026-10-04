//! Reading a SPEL program's instruction ABI off its own encoding.
//!
//! Shared by `aggregator-program/tests/instruction_parity.rs` and
//! `reference-consumers/pull/tests/instruction_parity.rs`, included by path the
//! way `verifier-core/tests/support/vectors.rs` is. Nothing here knows about
//! either program: the helpers are generic over anything serde can serialise,
//! which is what lets one implementation answer for both instruction sets.
//!
//! A path include is textual and adds no package edge, so the pull consumer
//! reaching this file is not a dependency on the aggregator — F9's closure walk
//! is over `--edges normal,build`, and dev-dependencies are outside it in any
//! case. What crosses here is a JSON scanner, not an instruction set.
//!
//! It does mean `reference-consumers/pull/` no longer carries a test suite that
//! compiles once lifted out of this repository. That is the same trade
//! `verifier-core/tests/support/vectors.rs` already makes for six crates, and it
//! costs nothing a reader needs: `[M3-06:01]` records that the artefact somebody
//! copies is the guest program, which depends on none of this.

use serde::Serialize;
use serde_json::Value;

/// The discriminant the wire carries, which is the first word risc0's serde
/// writes for an enum.
pub fn wire_discriminant<T: Serialize>(instruction: &T) -> u32 {
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
pub fn name_and_fields<T: Serialize>(instruction: &T) -> (String, Vec<String>) {
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
/// A container stack rather than a depth counter, and a key/value position flag
/// rather than "every string at depth two". Both matter: `[` and `{` nest alike
/// but only an object has keys, and inside an object a string appears in both
/// positions. Counting depth alone reads a `String` field's *value* as another
/// key — which no variant of the aggregator's own instruction set happens to
/// have, so the pull consumer's `data_service_id` is what found it.
pub fn keys_of_the_inner_object(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut current = String::new();
    // True while the next string would be a key: set by `{` and by a comma
    // inside an object, cleared by `:` and by anything that opens an array.
    let mut at_key_position = false;

    for character in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
                current.push(character);
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
                if at_key_position && stack.len() == 2 && stack.last() == Some(&'{') {
                    keys.push(std::mem::take(&mut current));
                }
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                current.clear();
            }
            '{' => {
                stack.push('{');
                at_key_position = true;
            }
            '[' => {
                stack.push('[');
                at_key_position = false;
            }
            '}' | ']' => {
                stack.pop();
                at_key_position = false;
            }
            ',' => at_key_position = stack.last() == Some(&'{'),
            ':' => at_key_position = false,
            _ => {}
        }
    }
    keys
}

/// `SubmitPrice` and `submit_price` are the same instruction under two
/// conventions: the enum is Rust's, the IDL takes the guest function's.
pub fn snake_case(camel: &str) -> String {
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

/// The committed IDL at `path`, relative to the including crate's manifest.
pub fn idl_at(path: &str) -> Value {
    let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let text = std::fs::read_to_string(&full)
        .unwrap_or_else(|_| panic!("{} is committed", full.display()));
    serde_json::from_str(&text).expect("the artefact is JSON")
}
