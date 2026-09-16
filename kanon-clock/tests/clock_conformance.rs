//! The oracle for `kanon-clock`'s declared copies of upstream's clock account.
//!
//! `kanon-clock` is `no_std` and `clock_core` is not, so the account id and the
//! sixteen-byte layout are declared there rather than imported. That is only
//! safe while something checks them, and this is that something: it links the
//! real `clock_core` and fails if either has moved.
//!
//! A test rather than a doc note, for the same reason `kanon-idl` conforms the
//! price account's IDL against the type that defines it: a comment cannot fail
//! CI.

use clock_core::{ClockAccountData, CLOCK_01_PROGRAM_ACCOUNT_ID};
use kanon_clock::{LezClock, CLOCK_ACCOUNT_ID};
use verifier_core::time::TimeSource;

#[test]
fn the_declared_account_id_is_the_every_block_clocks() {
    assert_eq!(
        CLOCK_ACCOUNT_ID,
        *CLOCK_01_PROGRAM_ACCOUNT_ID.as_ref(),
        "the pinned clock account moved upstream"
    );
}

#[test]
fn the_declared_layout_decodes_what_upstream_encodes() {
    // The whole of the layout assumption in one assertion: borsh writes
    // `block_id` then `timestamp`, both little-endian, sixteen bytes in total.
    let upstream = ClockAccountData {
        block_id: 4_242,
        timestamp: 1_770_000_000_000,
    };
    let encoded = borsh::to_vec(&upstream).expect("clock data serialises");

    assert_eq!(encoded.len(), 16, "the clock account is two u64s");

    let clock = LezClock::from_account(&CLOCK_ACCOUNT_ID, &encoded)
        .expect("upstream's own encoding must decode here");

    assert_eq!(clock.now_ms(), Ok(upstream.timestamp));
    assert_eq!(clock.block_id(), upstream.block_id);
}

#[test]
fn the_fields_are_not_read_in_the_wrong_order() {
    // Two u64s of the same width swap silently if the order is wrong, so the
    // test uses values that cannot be mistaken for one another.
    let upstream = ClockAccountData {
        block_id: 1,
        timestamp: u64::from(u32::MAX),
    };
    let encoded = borsh::to_vec(&upstream).expect("clock data serialises");
    let clock = LezClock::from_account(&CLOCK_ACCOUNT_ID, &encoded).expect("decodes");

    assert_eq!(clock.now_ms(), Ok(u64::from(u32::MAX)));
    assert_eq!(clock.block_id(), 1);
}
