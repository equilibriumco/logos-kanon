//! How a consumer should read the canonical price account.
//!
//! The same limit order reference consumer B holds, priced from the other side.
//! [`order::open_order`] records a price somebody is prepared to trade at;
//! [`order::settle`] fills it if the price the push aggregator published says
//! the market has reached it. The domain is deliberately the one B uses, so the
//! two crates differ in exactly one thing — where the price comes from — and a
//! reader can put them side by side.
//!
//! What B does in `pull_lib::verify_price`, this crate does in
//! [`read::read_price`]: no payload, no signatures, no transaction sent to the
//! aggregator. A price account is an account.
//!
//! # The four things RFP-020 asks a reading consumer to get right
//!
//! **The pair is checked against something the consumer wrote down.** The price
//! account carries a pair, but it is the aggregator's claim, and comparing a
//! value against itself is not a check. A [`source::PriceSource`] records what
//! this consumer's authority believes the feed prices, [`read::read_price`]
//! compares the account against that, and an order compares its owner's
//! expectation against the source. Three parties, two comparisons. Reference
//! consumer B made the mistake of doing this with one and `[M3-06:02]` records
//! it.
//!
//! **Staleness is the consumer's own window.** [`source::PriceSource::max_age_ms`]
//! is what the authority registered, and it bounds how old a *published price*
//! may be — which keeps running after the aggregator's own window closed.
//!
//! **Every cause has its own number.** [`read::ReadError`] and
//! [`order::SettleError`] have no wildcard arm, so a variant added to a layer
//! below stops this crate compiling until somebody decides what to tell its
//! callers. Wrappers dispatch into the wrapped layer's block rather than
//! reporting a layer as one number.
//!
//! | block | causes |
//! |---|---|
//! | 1200 | the authority gate |
//! | 1300 | opening an order |
//! | 1400 | settling one |
//! | 1900 | the clock |
//! | 2000 | a feed's price source |
//! | 2100 | reading the price account |
//!
//! **Refusing is the only thing this program does when it cannot read a price.**
//! Not a zero, not the last value, not `Ok` with an unchanged account — a caller
//! has to be able to tell "the market has not reached your limit" from "nobody
//! could tell me the price", because those need different next moves.
//!
//! # What this consumer depends on, and what it does not
//!
//! The aggregator's program id, two seed strings and one source constant. No
//! aggregator crate: [`read`] carries its own copies of the three, and
//! `tests/derivation.rs` fails if a copy and its original disagree. That is not
//! frugality — a Logos module in another repository could not link
//! `aggregator-program` either, so a reference that did would be demonstrating
//! something no reader could copy.
//!
//! The aggregator's id lives in state rather than in the build, because a
//! program's id is its RISC0 image id: an aggregator build whose inputs changed
//! moves every price account it writes. A rebuild from unchanged inputs
//! reproduces the id and moves nothing, which `[M2-06:01]` measured.
//! `[M3-05:01]` records that decision, and
//! [`source::update_aggregator`] is what it buys.
#![forbid(unsafe_code)]

pub mod authority;
pub mod order;
pub mod read;
pub mod source;

/// The clock this consumer reads its freshness from, and the account it will
/// accept one from.
///
/// Re-exported because [`read::read_price`] takes a `&LezClock` in its public
/// signature: without this a caller cannot name the type it has to pass, which
/// makes the public API unusable from anywhere but this crate. `pull-lib`
/// re-exports `verifier_core` for the same reason.
pub use kanon_clock;

/// The account the push aggregator publishes, and the six fields this consumer
/// reads out of it.
///
/// Re-exported for the same reason: what a push-mode read *is* is a decode of
/// this type, so a consumer built from this crate as a template needs it, and
/// reaching it through a second dependency invites pinning two versions of one
/// account layout. `[M3-08:01]` records the other caller -- the cost
/// measurement, which prices the decode directly and could not reach it from
/// this consumer's guest workspace without adding an edge there that would move
/// `AGGREGATOR_READ_CONSUMER_ID`, and with it every account a deployed consumer
/// owns.
pub use kanon_idl;

use kanon_clock::TimeError;

/// One number per clock fault, in the 1900 block.
///
/// The same four causes reference consumer B numbers in its own space. An
/// integer is a program's interface to its own callers, so two programs
/// answering one cause with one number is a convenience rather than a contract;
/// what U6 asks to be shared is the typed value, and both crates report
/// `TimeError` itself.
#[must_use]
pub const fn clock_code(err: TimeError) -> u32 {
    match err {
        TimeError::Missing => 1901,
        TimeError::WrongAccount => 1902,
        TimeError::Undecodable => 1903,
        TimeError::Unavailable => 1904,
    }
}
