//! The LEZ clock, read from the one account a caller may not choose.
//!
//! [`verifier_core::time::TimeSource`] says what verification needs from a
//! clock. This crate is the only implementation of it, shared by both modes,
//! because the pinned-account rejection is the whole of the guarantee: a caller
//! that can point verification at a clock of its choosing can make an hour-old
//! package look current, and every staleness check downstream becomes a
//! formality.
//!
//! # Why the layout is declared here rather than imported
//!
//! `clock_core` and `lee_core` are `std` crates, and this one is reachable from
//! a guest, so it cannot depend on them. The account id and the sixteen bytes of
//! [`ClockAccountData`] are declared below and `tests/clock_conformance.rs`
//! asserts both against the real types — the same arrangement `kanon-idl` uses
//! for the price account's IDL, and for the same reason: a copy that is checked
//! is a copy that cannot drift silently.
//!
//! [`ClockAccountData`]: https://github.com/logos-blockchain/logos-execution-zone
#![no_std]
#![forbid(unsafe_code)]

use verifier_core::time::TimeSource;

/// Why a clock could not be read, re-exported from where verification defines
/// it.
///
/// A consumer that reads the clock and never calls the verifier -- reference
/// consumer A is one -- still has to name the error this crate returns, and
/// making it reach for `verifier_core` to do so would put the verifier in the
/// manifest of a crate that verifies nothing.
pub use verifier_core::time::TimeError;

/// The every-block clock account.
///
/// LEZ's clock program maintains three accounts, refreshed every 1, 10 and 50
/// blocks. This is the every-block one, and the only one accepted here: the
/// other two are a freshness-against-contention trade to settle with a
/// measurement, and until that measurement exists, accepting them would mean
/// accepting a clock up to fifty blocks stale without saying so.
pub const CLOCK_ACCOUNT_ID: [u8; 32] = *b"/LEZ/ClockProgramAccount/0000001";

/// Width of the clock account's data: two little-endian `u64`s, borsh-encoded.
const CLOCK_DATA_LEN: usize = 16;

/// Byte range of the `timestamp` field, which follows `block_id`.
const TIMESTAMP_AT: usize = 8;

/// A timestamp read from the pinned clock account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LezClock {
    now_ms: u64,
    block_id: u64,
}

impl LezClock {
    /// Reads the clock from an account the program was given.
    ///
    /// # Errors
    ///
    /// - [`TimeError::WrongAccount`] for any account but [`CLOCK_ACCOUNT_ID`].
    ///   This is the check the whole crate exists for, so it runs first: a
    ///   caller-chosen clock is the one input that would make staleness
    ///   advisory.
    /// - [`TimeError::Undecodable`] for data that is not the clock's sixteen
    ///   bytes.
    /// - [`TimeError::Unavailable`] for a zero timestamp, which upstream uses as
    ///   "no valid time". Believing it would date every package impossibly far
    ///   in the future and fail a feed for a reason unrelated to the feed.
    pub fn from_account(account_id: &[u8; 32], data: &[u8]) -> Result<Self, TimeError> {
        if *account_id != CLOCK_ACCOUNT_ID {
            return Err(TimeError::WrongAccount);
        }
        if data.len() != CLOCK_DATA_LEN {
            return Err(TimeError::Undecodable);
        }

        let block_id = read_u64(data, 0).ok_or(TimeError::Undecodable)?;
        let now_ms = read_u64(data, TIMESTAMP_AT).ok_or(TimeError::Undecodable)?;
        if now_ms == 0 {
            return Err(TimeError::Unavailable);
        }

        Ok(Self { now_ms, block_id })
    }

    /// The time the chain is reporting, in milliseconds.
    ///
    /// The same value [`TimeSource::now_ms`] yields, as an inherent method so a
    /// caller that only wants the time does not import the verifier's trait to
    /// get at it.
    #[must_use]
    pub const fn timestamp_ms(&self) -> u64 {
        self.now_ms
    }

    /// The block this reading came from.
    ///
    /// Not used by verification, which only asks the time. Carried because a
    /// caller recording why it refused a price wants to name the block it was
    /// looking at.
    #[must_use]
    pub const fn block_id(&self) -> u64 {
        self.block_id
    }
}

impl TimeSource for LezClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.now_ms)
    }
}

fn read_u64(data: &[u8], at: usize) -> Option<u64> {
    let field: [u8; 8] = data.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(field))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(block_id: u64, timestamp: u64) -> [u8; CLOCK_DATA_LEN] {
        let mut data = [0u8; CLOCK_DATA_LEN];
        data[..8].copy_from_slice(&block_id.to_le_bytes());
        data[8..].copy_from_slice(&timestamp.to_le_bytes());
        data
    }

    #[test]
    fn the_pinned_account_reads_its_timestamp() {
        let data = account(42, 1_770_000_000_000);
        let clock = LezClock::from_account(&CLOCK_ACCOUNT_ID, &data).expect("the pinned account");

        assert_eq!(clock.now_ms(), Ok(1_770_000_000_000));
        assert_eq!(clock.block_id(), 42);
    }

    #[test]
    fn any_other_account_is_refused_before_its_contents_are_read() {
        // Including the 10- and 50-block clocks, which are real accounts holding
        // real timestamps — and therefore the plausible mistake, where a random
        // account id is the obvious one.
        let data = account(42, 1_770_000_000_000);
        for id in [
            *b"/LEZ/ClockProgramAccount/0000010",
            *b"/LEZ/ClockProgramAccount/0000050",
            [0u8; 32],
        ] {
            assert_eq!(
                LezClock::from_account(&id, &data),
                Err(TimeError::WrongAccount)
            );
        }
    }

    #[test]
    fn an_account_that_is_not_the_clocks_shape_is_refused() {
        assert_eq!(
            LezClock::from_account(&CLOCK_ACCOUNT_ID, &[]),
            Err(TimeError::Undecodable)
        );
        assert_eq!(
            LezClock::from_account(&CLOCK_ACCOUNT_ID, &[0u8; 15]),
            Err(TimeError::Undecodable)
        );
        assert_eq!(
            LezClock::from_account(&CLOCK_ACCOUNT_ID, &[0u8; 17]),
            Err(TimeError::Undecodable)
        );
    }

    #[test]
    fn a_zero_timestamp_is_reported_rather_than_believed() {
        // Upstream's "no valid time" sentinel. Believing it would put every
        // package impossibly far in the future.
        let data = account(42, 0);
        assert_eq!(
            LezClock::from_account(&CLOCK_ACCOUNT_ID, &data),
            Err(TimeError::Unavailable)
        );

        // A zero block id is not a sentinel: genesis is a real block.
        let genesis = account(0, 1_770_000_000_000);
        assert!(LezClock::from_account(&CLOCK_ACCOUNT_ID, &genesis).is_ok());
    }
}
