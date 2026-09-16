//! The price account conforms to the published standard.
//!
//! `OraclePriceAccount` is re-exported rather than redefined, so the type
//! cannot drift from the crate. What can drift is the *published artefact*: the
//! IDL other teams integrate against lives in `logos-blockchain/lez-programs`
//! and is vendored here at a named commit. If Logos changes the standard, the
//! crate and the artefact move together upstream and the vendored copy goes
//! stale — silently, because this crate would still compile.
//!
//! These tests make that noisy. They come in two halves: the declared shape,
//! read out of the vendored IDL, and the bytes an account actually encodes to.
//! The second half is the one a foreign consumer depends on. Nothing outside
//! this repository links our `OraclePriceAccount` — a consumer written in
//! another language, or against another crate, decodes 136 bytes at fixed
//! offsets. Agreeing on field names would not save it if the widths or the
//! order moved.

use kanon_idl::{AccountId, OraclePriceAccount, PRICE_ACCOUNT_FIELDS, REFERENCE_TWAP_ORACLE_IDL};
use lee_core::account::Data;
use serde_json::Value;

/// The account entry for the price account in the vendored IDL.
fn price_account_fields() -> Vec<(String, String)> {
    let idl: Value =
        serde_json::from_str(REFERENCE_TWAP_ORACLE_IDL).expect("the vendored IDL is valid JSON");

    let account = idl["accounts"]
        .as_array()
        .expect("the IDL declares an `accounts` array")
        .iter()
        .find(|account| account["name"] == "OraclePriceAccount")
        .expect("the IDL declares OraclePriceAccount");

    account["type"]["fields"]
        .as_array()
        .expect("OraclePriceAccount is a struct with fields")
        .iter()
        .map(|field| {
            (
                field["name"].as_str().expect("field name").to_owned(),
                field["type"].as_str().expect("field type").to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_vendored_idl_declares_the_fields_we_expect() {
    let actual = price_account_fields();
    let expected: Vec<(String, String)> = PRICE_ACCOUNT_FIELDS
        .iter()
        .map(|(name, ty)| ((*name).to_owned(), (*ty).to_owned()))
        .collect();

    assert_eq!(
        actual, expected,
        "the vendored IDL no longer matches the field list this crate expects. \
         Either the standard changed upstream, in which case refresh \
         `reference/twap_oracle-idl.json` and this list together and read the diff \
         carefully, or the vendored file was edited by hand, which it should not be. \
         See `reference/README.md`."
    );
}

#[test]
fn field_order_is_part_of_the_standard() {
    // Borsh encodes struct fields positionally, so a reordering is a wire break
    // even though every field is still present. Asserting the names as a set
    // would miss exactly the change that would corrupt every consumer's decode.
    let names: Vec<String> = price_account_fields()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(
        names.first().map(String::as_str),
        Some("base_asset"),
        "base_asset must stay first: consumers decode positionally"
    );
    assert_eq!(
        names.last().map(String::as_str),
        Some("confidence_interval"),
        "confidence_interval must stay last. SVM account layouts are \
         append-friendly by design -- RFP-019 chose the shape for that reason -- \
         so a new field belongs after it, and anything else moves the offsets \
         every consumer decodes at"
    );
}

/// The bytes Borsh spends on each type the IDL names.
///
/// All three are fixed-width and carry no length prefix, which is what makes
/// the account a constant 136 bytes and lets a consumer slice a field out
/// without decoding the ones before it.
fn encoded_width(idl_type: &str) -> usize {
    match idl_type {
        "account_id" => 32,
        "u64" => 8,
        "u128" => 16,
        other => panic!("the vendored IDL declares `{other}`, which this test has no width for"),
    }
}

const BASE_ASSET: [u8; 32] = [0x11; 32];
const QUOTE_ASSET: [u8; 32] = [0x22; 32];
const SOURCE: [u8; 32] = [0x33; 32];
const PRICE: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
const TIMESTAMP: u64 = 0x2021_2223_2425_2627;
// Non-zero, unlike every account Kanon writes, because a zero here would encode
// as sixteen zero bytes and so be indistinguishable from the field being absent.
const CONFIDENCE_INTERVAL: u128 = 0x3031_3233_3435_3637_3839_3a3b_3c3d_3e3f;

fn fixture() -> OraclePriceAccount {
    OraclePriceAccount {
        base_asset: AccountId::new(BASE_ASSET),
        quote_asset: AccountId::new(QUOTE_ASSET),
        price: PRICE,
        timestamp: TIMESTAMP,
        source_id: AccountId::new(SOURCE),
        confidence_interval: CONFIDENCE_INTERVAL,
    }
}

#[test]
fn the_account_encodes_to_the_bytes_a_foreign_decoder_expects() {
    let mut expected = Vec::new();
    expected.extend_from_slice(&BASE_ASSET);
    expected.extend_from_slice(&QUOTE_ASSET);
    expected.extend_from_slice(&PRICE.to_le_bytes());
    expected.extend_from_slice(&TIMESTAMP.to_le_bytes());
    expected.extend_from_slice(&SOURCE);
    expected.extend_from_slice(&CONFIDENCE_INTERVAL.to_le_bytes());

    assert_eq!(
        Data::from(&fixture()).as_ref(),
        expected.as_slice(),
        "the encoding moved. This one assertion carries four separate promises: \
         the fields are in the declared order, the integers are little-endian, \
         the account ids are 32 raw bytes with no length prefix, and nothing \
         precedes the first field -- no discriminator, no framing"
    );
}

#[test]
fn the_encoded_length_is_what_the_idl_field_types_add_up_to() {
    let declared: usize = price_account_fields()
        .iter()
        .map(|(_, ty)| encoded_width(ty))
        .sum();

    assert_eq!(
        declared, 136,
        "the vendored IDL no longer adds up to 136 bytes, so a field was added, \
         removed, or widened upstream"
    );
    assert_eq!(
        Data::from(&fixture()).len(),
        declared,
        "the type we encode and the type the IDL declares are no longer the \
         same size, so one of them changed without the other"
    );
}

#[test]
fn a_round_trip_through_the_on_chain_container_recovers_every_field() {
    let account = fixture();

    let decoded = OraclePriceAccount::try_from(&Data::from(&account))
        .expect("what `Data::from` wrote is what `try_from` reads");

    assert_eq!(decoded, account);
}

#[test]
fn a_seventh_field_would_be_a_break_rather_than_something_consumers_ignore() {
    // Borsh refuses a buffer with bytes left over, so appending is where a new
    // field has to go but does not make the change compatible: a consumer on
    // this decode path gets an error, not a price it can use. Worth knowing
    // before anyone reads "append-friendly" above as "safe to extend".
    let mut bytes = Data::from(&fixture()).to_vec();
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    let longer = Data::try_from(bytes).expect("far under the account data limit");

    assert!(
        OraclePriceAccount::try_from(&longer).is_err(),
        "trailing bytes decoded cleanly, so this decode path tolerates a longer \
         account and the note about appending being a break is now wrong"
    );
}

#[test]
fn the_reexported_type_is_constructible_with_the_documented_shape() {
    // Cheap, but it is the assertion that the re-export is the real type rather
    // than a coincidentally matching name in scope: this fails to compile if a
    // field is renamed, removed, or changes type upstream.
    let account = OraclePriceAccount {
        base_asset: Default::default(),
        quote_asset: Default::default(),
        price: 1,
        timestamp: 2,
        source_id: Default::default(),
        confidence_interval: 0,
    };

    assert_eq!(account.price, 1);
    assert_eq!(account.timestamp, 2);
    assert_eq!(
        account.confidence_interval, 0,
        "RedStone provides no confidence interval, so this field is always zero"
    );
}
