//! One number per leaf cause, across the whole program.
//!
//! The per-module tests each check their own block. This one is what catches a
//! collision *between* blocks, and it is where the number a caller actually sees
//! is pinned — `SpelError::error_code`, offset included.
//!
//! Leaf causes rather than constructible values: `SettleError::Source` and
//! `ReadError::Clock` are wrappers, and a wrapper answering with its wrapped
//! cause's number is the intent rather than a clash.

use kanon_clock::TimeError;
use reference_consumer_aggregator_read::authority::AuthorityError;
use reference_consumer_aggregator_read::clock_code;
use reference_consumer_aggregator_read::order::{OpenError, SettleError};
use reference_consumer_aggregator_read::read::ReadError;
use reference_consumer_aggregator_read::source::SourceError;

fn every_authority_cause() -> Vec<AuthorityError> {
    vec![
        AuthorityError::ConfigNotOurs,
        AuthorityError::ConfigUndecodable,
        AuthorityError::NotSigned,
        AuthorityError::Unauthorised,
        AuthorityError::NoGenesisAuthority,
        AuthorityError::AlreadyEstablished,
        AuthorityError::ConfigAccountUnusable,
        AuthorityError::NotGenesisKey,
        AuthorityError::NomineeIsZero,
        AuthorityError::NoNomination,
        AuthorityError::NotTheNominee,
        AuthorityError::KeyUnowned,
    ]
}

fn every_source_cause() -> Vec<SourceError> {
    vec![
        SourceError::NotOurs,
        SourceError::Undecodable,
        SourceError::FeedMismatch,
        SourceError::AlreadyRegistered,
        SourceError::AccountUnusable,
        SourceError::NotRegistered,
        SourceError::FeedIsZero,
        SourceError::AggregatorIsZero,
        SourceError::WindowIsZero,
        SourceError::WindowTooWide,
    ]
}

fn every_read_cause() -> Vec<ReadError> {
    vec![
        ReadError::WrongAccount,
        ReadError::Unavailable,
        ReadError::NotTheAggregators,
        ReadError::Undecodable,
        ReadError::NotRedStone,
        ReadError::AssetMismatch,
        ReadError::Stale {
            published_ms: 0,
            now_ms: 0,
            max_age_ms: 0,
        },
        ReadError::AheadOfClock {
            published_ms: 0,
            now_ms: 0,
        },
    ]
}

fn every_clock_cause() -> Vec<TimeError> {
    vec![
        TimeError::Missing,
        TimeError::WrongAccount,
        TimeError::Undecodable,
        TimeError::Unavailable,
    ]
}

/// Every number this program can answer with, once per leaf cause.
fn every_number() -> Vec<u32> {
    let mut numbers = Vec::new();
    numbers.extend(every_authority_cause().iter().map(AuthorityError::code));
    numbers.extend(every_source_cause().iter().map(SourceError::code));
    numbers.extend(every_read_cause().iter().map(ReadError::code));
    numbers.extend(every_clock_cause().into_iter().map(clock_code));
    numbers.extend(
        [
            OpenError::OwnerDidNotSign,
            OpenError::AlreadyOpen,
            OpenError::OrderAccountUnusable,
            OpenError::PairMismatch,
            OpenError::OrderTooLarge,
        ]
        .iter()
        .map(OpenError::code),
    );
    numbers.extend(
        [
            SettleError::OrderNotOurs,
            SettleError::OrderUndecodable,
            SettleError::AlreadyFilled,
            SettleError::OrderIsForAnotherFeed,
            SettleError::OrderTooLarge,
            SettleError::LimitNotReached { price: 0, limit: 0 },
            SettleError::WindowChanged { was: 0, now: 0 },
            SettleError::PairChanged,
        ]
        .iter()
        .map(SettleError::code),
    );
    numbers
}

#[test]
fn no_two_causes_answer_with_the_same_number() {
    let mut numbers = every_number();
    let total = numbers.len();
    numbers.sort_unstable();
    numbers.dedup();

    assert_eq!(
        numbers.len(),
        total,
        "two leaf causes in different modules share a number"
    );
}

/// What reaches a caller is `SpelError::error_code`, not `code`. The offset
/// belongs to the framework and this is where it is pinned, so a caller reading
/// 8102 can look 2102 up.
#[test]
fn the_number_a_caller_sees_is_the_frameworks_offset_one() {
    for cause in every_read_cause() {
        let spel: spel_framework::error::SpelError = cause.into();
        assert_eq!(spel.error_code(), 6000 + cause.code());
    }
    for cause in every_source_cause() {
        let spel: spel_framework::error::SpelError = cause.into();
        assert_eq!(spel.error_code(), 6000 + cause.code());
    }
}

/// A wrapper is not a cause. Reaching the same fault through two layers has to
/// produce one number, or a caller distinguishing "the source is unreadable"
/// from "the source is unreadable, seen from a settlement" would be
/// distinguishing nothing.
#[test]
fn a_wrapped_cause_keeps_its_own_number_by_every_route() {
    for cause in every_source_cause() {
        assert_eq!(SettleError::Source(cause).code(), cause.code());
        assert_eq!(OpenError::Source(cause).code(), cause.code());
        assert_eq!(ReadError::Source(cause).code(), cause.code());
        assert_eq!(
            SettleError::Read(ReadError::Source(cause)).code(),
            cause.code()
        );
    }
    for cause in every_clock_cause() {
        assert_eq!(ReadError::Clock(cause).code(), clock_code(cause));
        assert_eq!(
            SettleError::Read(ReadError::Clock(cause)).code(),
            clock_code(cause)
        );
    }
    for cause in every_authority_cause() {
        assert_eq!(SourceError::Authority(cause).code(), cause.code());
    }
}
