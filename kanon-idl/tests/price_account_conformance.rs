//! The price account conforms to the published standard.
//!
//! `OraclePriceAccount` is re-exported rather than redefined, so the type
//! cannot drift from the crate. What can drift is the *published artefact*: the
//! IDL other teams integrate against lives in `logos-blockchain/lez-programs`
//! and is vendored here at a named commit. If Logos changes the standard, the
//! crate and the artefact move together upstream and the vendored copy goes
//! stale — silently, because this crate would still compile.
//!
//! These tests make that noisy. They cover the declared shape; the byte-level
//! half arrives once there is a write path to check a round-trip against.

use kanon_idl::{OraclePriceAccount, PRICE_ACCOUNT_FIELDS, REFERENCE_TWAP_ORACLE_IDL};
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
         so a new field belongs after it, and anything else is a break"
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
