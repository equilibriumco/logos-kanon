//! Where "now" comes from, and why a caller may not choose it.
//!
//! A staleness check is only as trustworthy as its clock. A timestamp the
//! caller passes in, or an account the caller picks, makes an old package look
//! current — which defeats the check and, with it, the main defence against
//! replaying a signed package that was valid an hour ago.
//!
//! So the clock is a trait, implemented by something that reads a pinned
//! on-chain account, and never a parameter. The trait lives here for the same
//! reason [`VerifierBackend`] does: this crate has no chain dependency, so the
//! implementation lives in a crate that does, and both modes share the one
//! implementation rather than each carrying its own.
//!
//! [`VerifierBackend`]: crate::backend::VerifierBackend

/// A clock a caller cannot choose.
pub trait TimeSource {
    /// Milliseconds since the Unix epoch.
    ///
    /// Milliseconds because that is what the LEZ sequencer writes and what
    /// RedStone puts on the wire; the two need no conversion between them, and
    /// stating the unit in the signature is what keeps it that way.
    ///
    /// # Errors
    ///
    /// [`TimeError`] when no trustworthy time is available. Refusing is the
    /// only safe answer: a clock that guesses makes every staleness check a
    /// formality.
    fn now_ms(&self) -> Result<u64, TimeError>;
}

/// Why a clock could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// The clock account was not supplied.
    Missing,
    /// The clock account is not the pinned one.
    ///
    /// Separate from [`Self::Missing`] because it is the security-relevant case:
    /// a caller that supplies a stale or self-controlled clock account is
    /// attempting exactly what pinning exists to stop, where a caller that
    /// supplies none has merely wired it up wrong.
    WrongAccount,
    /// The account data is not a clock, or is truncated.
    Undecodable,
    /// The clock reads zero, which upstream uses as "no valid time".
    ///
    /// Reported rather than believed. A zero clock makes every package look
    /// impossibly far in the future, so trusting it would reject a whole feed
    /// for a reason that has nothing to do with the feed.
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock whose answer the test chooses.
    struct FixedClock(Result<u64, TimeError>);

    impl TimeSource for FixedClock {
        fn now_ms(&self) -> Result<u64, TimeError> {
            self.0
        }
    }

    #[test]
    fn a_clock_can_refuse_and_the_refusal_says_which_way_it_failed() {
        // The variants exist to be told apart: a missing account is a wiring
        // mistake and a wrong one is an attempt at the thing pinning prevents.
        assert_eq!(
            FixedClock(Ok(1_770_000_000_000)).now_ms(),
            Ok(1_770_000_000_000)
        );
        assert_eq!(
            FixedClock(Err(TimeError::Missing)).now_ms(),
            Err(TimeError::Missing)
        );
        assert_ne!(TimeError::Missing, TimeError::WrongAccount);
        assert_ne!(TimeError::Undecodable, TimeError::Unavailable);
    }

    #[test]
    fn the_trait_is_object_safe() {
        // Same property `VerifierBackend` needs: a caller may hold one behind a
        // reference without knowing which implementation it has.
        let clock = FixedClock(Ok(7));
        let erased: &dyn TimeSource = &clock;
        assert_eq!(erased.now_ms(), Ok(7));
    }
}
