# 9. Re-export the canonical price account, and vendor its IDL as a conformance oracle

- **Status**: accepted
- **Milestone**: M1 (`M1-09`; written by `M1-20`, conformed by `M1-26`)
- **Requirements**: F5, U4
- **Artefacts**: `kanon-idl/src/lib.rs`, `kanon-idl/reference/twap_oracle-idl.json`, `kanon-idl/reference/README.md`, `kanon-idl/tests/price_account_conformance.rs`

## Context

RFP-020 Functionality 5 requires the verified price to be published into a **canonical
RFP-019 price account** — base and quote asset, price, timestamp, source identifier and
a zero confidence interval. Usability 4 asks for the IDL to be "re-exported rather than
forked".

M1-09 went looking for that account and found two facts pointing in opposite
directions.

**The RFP-019 grant is not running.** Its proposals were closed on 2026-08-10
(`logos-co/rfp#70`, `#86`) and, unlike RFP-001 and RFP-020, it has no `[MILESTONE]`
issues. No team is delivering it.

**Logos built the canonical account anyway**, in `logos-blockchain/lez-programs` at
`programs/twap_oracle/core/src/lib.rs`. `OraclePriceAccount` carries exactly the six
fields Functionality 5 names, and its own doc comment names external adaptors as
intended writers:
*"intentionally generic so that any oracle type (TWAP, external adaptor, aggregator) can
use the same interface"*.

The second fact wins. But taking the dependency was not free — it forced the pin
realignment in ADR 8.

## Decision

**`kanon-idl` re-exports `OraclePriceAccount` from the crate that defines it.** Nothing
in this repository redefines a field. That is what "re-exported, not forked" asks for,
and it is preferable to paraphrasing a third-party standard.

**And the published IDL artefact is vendored anyway**, at a named commit
(`4363f139…`, 2026-08-05), with `tests/price_account_conformance.rs` asserting that the
type and the artefact still agree on field names, types and **order**.

The two are not redundant. The *type* and the *published artefact* can drift apart
without the build noticing. The artefact is what other teams integrate against; if
Logos changes the standard, the crate and the artefact move together upstream while the
vendored copy goes stale — and this repository still compiles.
The test turns that silence into a failure.

Field **order** is checked deliberately. Borsh encodes positionally, so a reordering is
a wire break even when every field is still present, and a set comparison would miss
exactly the change that corrupts every consumer's decode. A field appended at the end is
the expected, compatible change — SVM account layouts are append-friendly by design,
which is why RFP-019 chose this shape.

`PRICE_ACCOUNT_FIELDS` restates the field list in Rust rather than deriving it, for the
same reason: if the list and the artefact disagree, one of them changed, and that is
what the test is for.

## Consequences

- M1-20 writes the real struct. The "forward-compatible fallback" branch of that task is
  dead scope.
- The dependency dragged the product onto risc0 3.0.5 and LEZ v0.2.0 (ADR 8), and made
  `methods/guest` link `kanon-idl` so the whole graph — `twap_oracle_core`, `lee_core`,
  `uniswap_v3_math`, `alloy_primitives` — is proven to cross-compile to riscv32 on every
  push. M2's aggregator guest depends on that being true.
- `twap_oracle_core`'s pins are inherited, and they are stricter than they need to be.
  Two questions went to Logos and are tracked in `m0/versions.md` (*Open: questions
  outstanding with Logos*, items 3 and 4): an account-type crate
  needs neither risc0 nor `uniswap_v3_math` to carry six Borsh fields, so splitting
  `OraclePriceAccount` into a standalone crate — or at minimum loosening `=3.0.5` to
  `^3.0.5` — would make the canonical account usable by the external adaptors it was
  explicitly written for.
- `twap_oracle_core` ships with no `license` field, which is why `deny.toml` carries a
  `[[licenses.clarify]]` for it (ADR 1).
- Refreshing the vendored artefact is a deliberate act: re-download, record the new
  commit in `kanon-idl/reference/README.md`, run `cargo test -p kanon-idl`. A change
  anywhere but the end of the struct is a break to be reviewed before it is absorbed.

## Alternatives considered

- **Define a local price account.** Rejected: it forks a standard and defeats the purpose
  of a canonical account other programs can read.
- **Vendor the struct definition without the dependency.** Was the contingency while the
  crate looked unresolvable; dropped once ADR 8 made it resolvable.
- **Depend on the crate and skip the vendored artefact.** Rejected: it removes the only
  mechanism that notices the *published standard* drifting away from the linked type.
