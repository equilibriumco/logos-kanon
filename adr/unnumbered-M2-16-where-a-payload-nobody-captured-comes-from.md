# [M2-16:01]. Where a payload nobody captured comes from

- **Status**: accepted, unnumbered
- **Milestone**: M2 (`M2-16`)
- **Requirements**: S3
- **Artefacts**: `verifier-core/Cargo.toml`, `verifier-core/src/lib.rs`,
  `verifier-core/src/test_support.rs`, `aggregator-program/tests/submit_price.rs`

## Context

S3 asks for a test per named dimension per mode, and one of its dimensions cannot
be reached with the payloads this repository committed. "Invalid value" is
`VerifyError::ValueOutOfRange`, raised for a package carrying a zero, negative or
over-wide value (`feed.rs:411`). **RedStone does not sign such values**, so no
capture contains one and none ever will.

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

**It is not a shipped surface, and three things keep it that way.** The feature is
off by default, so `cargo build -p verifier-core` does not compile it. The `no_std`
job builds the guest-reachable crates with default features for
`riscv32im-unknown-none-elf`, so a shipped build that enabled it would fail there —
`test_support` declares `extern crate std` and pulls in a signing key, neither of
which belongs in a guest. And it is reached from `tests/`, so it sits outside
`--edges normal,build` and cannot enter the closure M3-02's `pull-independence` job
pins; measured, both walks are byte-identical with the feature present.

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
  arithmetic rather than what RedStone will sign.
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
