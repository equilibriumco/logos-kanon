//! The IDL's instruction order and the wire enum's must agree, argument by
//! argument.
//!
//! Two independent declarations describe one instruction set. The IDL is
//! generated from the guest's function order; the transaction is decoded by
//! `read_lee_inputs::<Instruction>()`, which uses this crate's enum. A caller
//! encodes the discriminant from the IDL -- `spel-cli` takes
//! `idl.instructions.iter().position(|i| i.name == ix.name)` -- and fills each
//! argument positionally.
//!
//! Nothing in the build ties those together, and until the instruction enum
//! moved into this crate nothing had to: `#[lez_program]` generated it from the
//! handlers, so a handler and its variant could not disagree. Now both orders
//! are a human's to keep, at two levels. Reordering two variants sends a
//! caller's `rotate_signers` to `deregister_feed_trust`. Reordering two
//! same-typed fields inside one variant -- `order_id` and `feed_id` in
//! `OpenOrder`, both `[u8; 32]` -- shifts values between arguments with no type
//! error anywhere and no other test failing.
//!
//! So the agreement is asserted against the encoding the wire uses rather than
//! against a restatement of it. The helpers are `aggregator-program`'s, included
//! by path: they are generic over anything serde serialises and know nothing
//! about either program, and a path include adds no package edge, so F9 is
//! untouched.

use reference_consumer_pull::Instruction;

use std::collections::BTreeSet;

#[path = "../../../aggregator-program/tests/support/idl_parity.rs"]
mod idl_parity;
use idl_parity::{idl_at, name_and_fields, snake_case, wire_discriminant};

/// One value per variant. Order is deliberately not meaningful: each variant's
/// discriminant is read out of its own encoding, so this list only has to be
/// complete, and `the_enum_and_the_idl_describe_the_same_number_of_instructions`
/// is what checks that it is.
fn every_variant() -> Vec<Instruction> {
    vec![
        Instruction::EstablishAuthority,
        Instruction::NominateAuthority { nominee: [1u8; 32] },
        Instruction::AcceptAuthority,
        Instruction::RegisterFeedTrust {
            data_service_id: "redstone-primary-prod".to_owned(),
            feed_id: [2u8; 32],
            base_asset: [3u8; 32],
            quote_asset: [4u8; 32],
            decimals: 8,
            max_age_ms: 60_000,
            signers: vec![[5u8; 20]],
            threshold: 1,
        },
        Instruction::RotateSigners {
            feed_id: [6u8; 32],
            signers: vec![[7u8; 20]],
            threshold: 2,
        },
        Instruction::DeregisterFeedTrust { feed_id: [8u8; 32] },
        Instruction::OpenOrder {
            order_id: [9u8; 32],
            feed_id: [10u8; 32],
            base_asset: [11u8; 32],
            quote_asset: [12u8; 32],
            limit_price_q64: 13,
        },
        Instruction::Settle {
            feed_id: [14u8; 32],
            payload: vec![15, 16, 17],
        },
    ]
}

fn idl() -> serde_json::Value {
    idl_at("pull-consumer-idl.json")
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
             `guest/src/bin/pull_consumer.rs` or the variants in `src/instruction.rs` \
             so the two agree, then regenerate the IDL.",
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

    // The *set* of discriminants rather than the count, because length is not
    // completeness: a copy-pasted entry in `every_variant` would leave the list
    // the right length with one variant listed twice and another not at all, and
    // the two tests above only ever look at what the list contains. The missing
    // instruction's discriminant and field order would then be asserted by
    // nothing.
    let covered: BTreeSet<u32> = every_variant().iter().map(wire_discriminant).collect();
    let expected: BTreeSet<u32> = (0..u32::try_from(published).expect("a small enum")).collect();

    assert_eq!(
        covered,
        expected,
        "every discriminant the IDL publishes needs exactly one value in \
         `every_variant`. Missing: {:?}. Not published: {:?}. A new instruction \
         needs a guest function, an `Instruction` variant, and a value here.",
        expected.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&expected).collect::<Vec<_>>(),
    );
}
