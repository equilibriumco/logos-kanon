//! Prints the aggregator's IDL, read off the guest program's own source.
//!
//! The IDL is generated rather than written, so the instruction surface a client
//! builds against cannot drift from the one the program dispatches on.
//!
//! ```sh
//! cargo run -p aggregator-program --example generate_idl > kanon-idl/aggregator-idl.json
//! ```

spel_framework_macros::generate_idl!("../methods/guest/src/bin/aggregator.rs");
