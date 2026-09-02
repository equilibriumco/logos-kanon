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

#[path = "support/idl_parity.rs"]
mod idl_parity;
use idl_parity::{idl_at, name_and_fields, snake_case, wire_discriminant};

/// One value per variant. Order is deliberately not meaningful: each variant's
/// discriminant is read out of its own encoding, so this list only has to be
/// complete, and the count assertion is what checks that it is.
/// A feed id as the wire pads it, which is what the instruction now takes.
fn padded(name: &[u8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..name.len()].copy_from_slice(name);
    id
}

fn every_variant() -> Vec<Instruction> {
    vec![
        Instruction::SubmitPrice {
            payload: vec![1, 2, 3],
        },
        Instruction::RegisterFeed {
            feed_id: padded(b"BTC"),
            base_asset: [1u8; 32],
            quote_asset: [2u8; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: vec![[3u8; 20]],
            threshold: 1,
        },
        Instruction::UpdateSignerSet {
            feed_id: padded(b"BTC"),
            signers: vec![[4u8; 20]],
            threshold: 2,
        },
        Instruction::DeregisterFeed {
            feed_id: padded(b"BTC"),
        },
        Instruction::PauseFeed {
            feed_id: padded(b"BTC"),
        },
        Instruction::UnpauseFeed {
            feed_id: padded(b"BTC"),
        },
        Instruction::InitialiseAdmin,
        Instruction::NominateAdmin {
            new_admin: [5u8; 32],
        },
        Instruction::AcceptAdmin,
        Instruction::RevokeAdmin,
    ]
}

fn idl() -> serde_json::Value {
    idl_at("../kanon-idl/aggregator-idl.json")
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
        let at_index = instructions
            .get(index)
            .unwrap_or_else(|| panic!("the IDL has no instruction at index {index} for {name}"));
        // `args` required rather than defaulted: the generator emits the key
        // even for an instruction that takes none, so a missing one means the
        // artefact is not what this test thinks it is reading. Defaulting would
        // compare equal for the unit variants and hide that.
        let published: Vec<String> = at_index["args"]
            .as_array()
            .unwrap_or_else(|| panic!("the IDL lists no `args` for {name}"))
            .iter()
            .map(|arg| arg["name"].as_str().unwrap_or_default().to_owned())
            .collect();

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
