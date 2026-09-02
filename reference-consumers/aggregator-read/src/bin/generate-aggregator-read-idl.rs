//! Prints the reference consumer's IDL, read off the guest program's own source.
//!
//! Generated rather than written, so the instruction surface a client builds
//! against cannot drift from the one the program dispatches on. `tests/idl.rs`
//! runs this and compares, which is what makes the committed artefact a cache of
//! the guest source rather than a second definition of it.
//!
//! ```sh
//! cargo run -p reference-consumer-aggregator-read --bin generate-aggregator-read-idl \
//!   > reference-consumers/aggregator-read/aggregator-read-consumer-idl.json
//! ```
//!
//! The path is relative to this crate's manifest directory, not to this file.

spel_framework_macros::generate_idl!("guest/src/bin/aggregator_read_consumer.rs");
