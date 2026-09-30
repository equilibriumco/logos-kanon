# [M2-16:01]. Where a payload nobody captured comes from

- **Status**: accepted, unnumbered
- **Milestone**: M2 (`M2-16`)
- **Requirements**: S3
- **Supersedes**: [21](0021-property-tests-for-the-invariants-examples-cannot-reach.md)
  and [23](0023-the-accept-and-reject-suite-is-shaped-by-the-contract.md), on the
  rejected alternative only — both refused a feature-gated `test_support`, and
  their reason turned out to be right about the risk
- **Artefacts**: `verifier-core/Cargo.toml`, `verifier-core/src/lib.rs`,
  `verifier-core/src/test_support.rs`, `aggregator-program/tests/submit_price.rs`

## Context

S3 asks for a test per named dimension per mode, and one of its dimensions cannot
be reached with the payloads this repository committed. "Invalid value" is
`VerifyError::ValueOutOfRange`, raised for a package carrying a zero, negative or
over-wide value (`feed.rs:411`). **The production captures committed here contain
none**, so a test for this dimension has to sign its own deterministic fixture.

Narrower than "RedStone does not sign such values", which is what an earlier draft
of this record said and is not true: RedStone's own connector repository carries
zero-value fixtures and code that filters them. What holds is the claim about
*these* captures.

The captures are otherwise the right fixture and ADR 19 says why: what is
RedStone's in them is the packages and the signatures over them, which is the part
nobody here could have produced. A test that wants a value RedStone would not
publish has to sign its own.

`verifier-core` already has a builder that does this correctly.
`test_support::PayloadBuilder` assembles the framing `decode`'s tests need and
signs the exact span the decoder hands to keccak256, and `decode`, `feed`,
`accept_and_reject` and `properties` all use it. It was `#[cfg(test)]`, so it was
reachable only from `verifier-core`'s own unit tests — and the test that needs it
is an `aggregator-program` integration test, because the dimension is about the
*push path* and not about the verifier.

## Decision

**The builder is exposed behind a `test-fixtures` feature, off by default.**

```toml
[features]
test-fixtures = []
```

```rust
#[cfg(any(test, feature = "test-fixtures"))]
pub mod test_support;
```

`cfg(test)` stays, so `verifier-core`'s own tests do not need the feature. The
consumer enables it as a dev-dependency:

```toml
verifier-core = { workspace = true, features = ["test-fixtures"] }
```

### ADR 21 and ADR 23 rejected this, and they were right about the risk

Both records considered a feature-gated `test_support` and refused it, for the same
reason: *"fixture code does not belong in the published surface of a crate a
consumer program links"* (ADR 23, on ADR 21's grounds). This record supersedes that
rejection, and only because it can hold the line mechanically — the first draft
could not, and said so wrongly.

**What the first draft claimed, and why it was wrong.** It named three guards: the
feature being off by default, the `no_std` job, and M3-02's closure walk. Review
showed the middle one does not hold and the third never could:

- the `no_std` job builds `-p verifier-core -p pull-lib -p kanon-clock`. It never
  builds `aggregator-program`, so a feature enabled *by* `aggregator-program` is
  outside what it compiles;
- and reaching it would not help. The product guest targets
  `riscv32im-risc0-zkvm-elf`, and risc0's toolchain **ships `std` for that
  target** — `extern crate std` is no barrier where the guest actually builds.
  `cargo build -p kanon-methods` with the feature on a normal edge succeeds and
  compiles the fixtures into the ELF;
- the closure walk compares package *names*, and a feature adds no package.

So all that separated the fixtures from a shipped guest was one word:
`[dev-dependencies]` rather than `[dependencies]`.

**`verifier-core/tests/feature_edges.rs` is what makes it a rule.** It walks each
product guest's `--edges normal` tree and refuses `test-fixtures` anywhere in it,
and it discovers the guest workspaces from the root manifest's `exclude` list so a
guest added later is covered without anyone remembering. Its second test enables
the feature deliberately and requires it to appear, because an assertion about an
absence proves nothing until something can make it present. Verified by moving the
dependency to a normal edge: the guard fails with the offending line.

The feature is still off by default, and it is still reached only from `tests/`.
Those remain true; they are simply no longer the whole argument.

### What the guard does not do, and the cost this record accepts

**The public surface is real and stays.** `test-fixtures` is opt-in API and
`test_support` is a public module, so any third-party program linking
`verifier-core` can enable it and ship the fixtures. `feature_edges.rs` walks
*Kanon's* product guests: it prevents the accidental case here, and says nothing
about a consumer who enables the feature deliberately in their own build.

That is what ADR 21 and ADR 23 objected to — not the risk of an accident, but
fixture code existing in the published surface of a crate a consumer program links.
**This record accepts that cost knowingly**, and it is worth saying so plainly
rather than letting a mechanical guard read as though it answered the objection:

- the two dimensions that need it (`ValueOutOfRange` here, a fresh timestamp for
  M2-19) cannot be reached any other way with the fixtures this repository has;
- a consumer who enables it gets a signing key and a payload builder, which are
  useless to a production program and harmless to one that wants them for its own
  tests, so the failure mode is bloat rather than a wrong answer;
- and it is off by default, so the surface has to be asked for by name.

If the surface itself is judged too high a price, the last alternative below is the
way out and it is a contained change.

## Consequences

- **S3's last push-mode dimension closes.**
  `a_package_carrying_an_unusable_value_is_refused_by_the_submission_path` signs a
  payload over keys the feed is registered against and drives a zero and a negative
  through `submit_price`, so every check before the value passes and the value is
  what refuses.
- **Negative means the top bit of the full 32-byte value**, which only writing the
  test established. `Value::is_negative` reads `self.0[0] & 0x80` and
  `from_be_slice` right-aligns what it is given, so four bytes of `0xFF` become a
  large positive number and publish. The first version of this test asserted a
  refusal and got a successful write.
- **`ValueOutOfRange` and `ScalingOutOfRange` are different causes** and now have
  different tests. The second is the conversion overflowing rather than the value
  being unusable, and it is unreachable from the captures for a different reason —
  arithmetic rather than the contents of the captures.
- **M2-19 needs the same builder for a different reason.** The chain clock on a
  standalone sequencer is live wall time, the committed captures are weeks old, and
  `MAX_MAX_AGE_MS` is fifteen minutes — so no captured payload can pass freshness
  against a real sequencer either. An end-to-end push test has to sign a payload
  timestamped now, which is this builder. Exposing it was on that task's path as
  much as this one's.

## Alternatives considered

- **A second builder in the consumer's tests.** No change to `verifier-core`, and
  it is how two implementations of one wire format start disagreeing. The framing
  is not obvious — SPEL widens seeds, the signature covers a specific span, the
  envelope carries a package count — and a copy that drifted would produce a test
  passing against a payload the decoder would refuse.
- **`#[cfg(feature = "test-fixtures")]` without `cfg(test)`.** One condition
  instead of two, and every `cargo test -p verifier-core` would then need the
  feature spelled out. The crate's own tests should not depend on a flag to run.
- **Moving the builder to `verifier-core/tests/support/`.** Where `vectors.rs`
  already lives, included by path — the established pattern for sharing a fixture
  reader. It does not work here: the builder is used by `src/` modules' own
  `cfg(test)` tests, which cannot include from `tests/`, so it would have to be
  duplicated or those tests rewritten.
- **Fetching a fresh payload from RedStone's gateway at test time.** Reaches both
  dimensions and puts CI on an external service, which is the thing the committed
  captures exist to avoid. It also cannot produce a negative value, since the
  gateway will not serve one.
- **A separate dev-only crate holding the builder.** The one alternative that
  removes the objection rather than accepting it: no feature on `verifier-core`, no
  public `test_support`, nothing a third party can switch on. It works because
  everything the builder touches is already public — `decode::REDSTONE_MARKER`,
  `decode::FEED_ID_BYTES`, `backend::Signature`, `backend::SignerAddress` — so it
  needs no privileged access. The cost is a cycle in the dev graph
  (`verifier-core` dev-depends on it, it depends on `verifier-core`), which cargo
  permits but which makes this crate's own tests harder to reason about, plus a
  manifest, a lockfile entry and a `cargo deny` consideration. Not taken now
  because the feature is off by default and asked for by name; the right change if
  the published surface is judged the deciding cost.
