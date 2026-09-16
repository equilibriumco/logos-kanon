# Reference artefacts

Third-party artefacts this repository conforms to, vendored so that conformance is
checked against a fixed file rather than against whatever an upstream default branch
says today.

## `twap_oracle-idl.json`

The canonical LEZ oracle price account, as published by Logos.

| | |
| --- | --- |
| source | `logos-blockchain/lez-programs`, `artifacts/twap_oracle-idl.json` |
| commit | `4363f13912e0ba8627e70575161862ab337c1169` (2026-08-05) |
| defined in | `programs/twap_oracle/core/src/lib.rs` |

RFP-019 requires the canonical price account struct to be "specified as a SPEL IDL
and published as a standalone artefact that other programs (including external oracle
adaptors in RFP-020 ...) can import without depending on the TWAP program itself".
This is that artefact. Its `OraclePriceAccount` is the account RFP-020 Functionality 5
requires the aggregator to populate:

```json
{"name": "OraclePriceAccount", "type": {"kind": "struct", "fields": [
  {"name": "base_asset",          "type": "account_id"},
  {"name": "quote_asset",         "type": "account_id"},
  {"name": "price",               "type": "u128"},
  {"name": "timestamp",           "type": "u64"},
  {"name": "source_id",           "type": "account_id"},
  {"name": "confidence_interval", "type": "u128"}
]}}
```

The struct's own doc comment names external adaptors as intended writers:
*"intentionally generic so that any oracle type (TWAP, external adaptor, aggregator)
can use the same interface"*. Producers own how the account is written; consumers only read and
validate.

### Why a vendored file *as well as* the dependency

`kanon-idl` depends on `twap_oracle_core` and re-exports `OraclePriceAccount`, so the
type is imported rather than restated — Usability 4 asks for "re-exported, not
forked", and this honours it. Nothing in this repository redefines a field.

So why keep the file? Because the *type* and the *published artefact* can drift apart
without the build noticing. The artefact is what other teams integrate against; if
Logos changes the standard, the crate and the artefact move together upstream while
the vendored copy goes stale, and this crate still compiles. `tests/price_account_conformance.rs`
turns that into a failing test: the field names, their types and their order must
match what this file declares.

Field *order* is checked deliberately. Borsh encodes positionally, so a reordering is
a wire break even when every field is still present, and a set comparison would miss
exactly the change that corrupts every consumer's decode. A new field appended at the
end is the expected, compatible change — SVM account layouts are append-friendly by
design, which is why RFP-019 chose this shape.

What it cost to take the dependency is recorded in `adr/0009-re-export-the-canonical-price-account-and-vendor-its-idl.md`
and `adr/0008-exact-pins-and-tracking-the-estate.md`: the product moved to risc0 3.0.5
and LEZ v0.2.0 to match, and the cycle counts on both were re-measured there.

### Refreshing it

Re-download from the path above, record the new commit in the table, and run
`cargo test -p kanon-idl`. A field appearing at the end of the struct is the expected
kind of change — SVM account layouts are append-friendly by design, which is the
reason RFP-019 chose this shape — and a change anywhere else is a break worth a
conversation before it is absorbed.
