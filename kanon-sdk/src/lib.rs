//! Build Logos modules against Kanon, in either mode.
//!
//! Submitting RedStone data packages to the push aggregator, reading verified
//! prices from the canonical price account, and calling the public-mode pull
//! verification library from a consumer program, behind helpers ergonomic
//! enough that switching modes does not change payload handling.
//!
//! # Not yet implemented
//!
//! Nothing here yet. The CLI and the mini-app in `kanon-app` both consume this
//! crate rather than reimplementing its calls, so it lands before either.
//! `TRACEABILITY.md` maps the work to its task.
#![forbid(unsafe_code)]
