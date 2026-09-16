//! How a consumer should verify a RedStone payload inline.
//!
//! The pattern the RFP asks this to demonstrate: pass the expected
//! `(base_asset, quote_asset)`, the signer set, the M-of-N threshold and
//! `maxAge` **from the consumer's own configuration**, handle each typed error
//! code, and refuse the action when verification fails rather than falling back
//! to an unsafe default.
//!
//! # Not yet implemented
//!
//! Nothing here yet. It will be built over [`pull_lib`], with its tests run
//! against the standalone sequencer. `TRACEABILITY.md` maps the work to its
//! task.
#![forbid(unsafe_code)]
