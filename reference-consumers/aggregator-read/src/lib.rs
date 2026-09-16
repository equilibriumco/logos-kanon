//! How a consumer should read the canonical price account.
//!
//! The pattern the RFP asks this to demonstrate: check that
//! `(base_asset, quote_asset)` matches what the consumer expects, read price
//! and timestamp, reject anything older than the consumer's own `maxAge`, and
//! **refuse the action** when no price is available rather than falling back to
//! an unsafe default.
//!
//! # Not yet implemented
//!
//! Nothing here yet. It will be built over the account the push aggregator
//! writes. `TRACEABILITY.md` maps the work to its task.
#![forbid(unsafe_code)]
