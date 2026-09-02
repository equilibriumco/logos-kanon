//! The three constants this consumer copies, held against their originals.
//!
//! `src/read.rs` declares the aggregator's two address seeds and its source
//! constant itself, so that a reader can see the whole integration surface in
//! one file and so that a Logos module in another repository could reproduce it
//! without linking `aggregator-program`. A copy is only safe if something fails
//! when it drifts, and that is this file.
//!
//! `aggregator-program` is a dev-dependency, so none of this reaches the crate a
//! consumer links.

use aggregator_program::publish::REDSTONE_SOURCE_ID as AGGREGATORS_SOURCE_ID;
use aggregator_program::register::FEED_ACCOUNT_SEED as AGGREGATORS_FEED_SEED;
use aggregator_program::submit::PRICE_ACCOUNT_SEED as AGGREGATORS_PRICE_SEED;
use lee_core::program::ProgramId;
use reference_consumer_aggregator_read::read::{
    price_account_address, FEED_ACCOUNT_SEED, MAX_AHEAD_MS, PRICE_ACCOUNT_SEED, REDSTONE_SOURCE_ID,
};
use reference_consumer_aggregator_read::source::MAX_MAX_AGE_MS;
use spel_framework::pda::{compute_pda, seed_from_str};

const AGGREGATOR: ProgramId = [11u32; 8];
const FEED_ID: [u8; 32] = {
    let mut id = [0u8; 32];
    id[0] = b'B';
    id[1] = b'T';
    id[2] = b'C';
    id
};

#[test]
fn the_seeds_this_consumer_copies_are_the_aggregators_own() {
    assert_eq!(FEED_ACCOUNT_SEED, AGGREGATORS_FEED_SEED);
    assert_eq!(PRICE_ACCOUNT_SEED, AGGREGATORS_PRICE_SEED);
}

#[test]
fn the_source_constant_this_consumer_copies_is_the_one_the_aggregator_writes() {
    assert_eq!(REDSTONE_SOURCE_ID, AGGREGATORS_SOURCE_ID);
}

/// The address is what ties a consumer to an aggregator, so it is asserted
/// against a derivation built from the aggregator's own constants rather than
/// against a recomputation of this crate's.
#[test]
fn a_consumer_derives_the_address_the_aggregator_writes_to() {
    let feed = compute_pda(
        &AGGREGATOR,
        &[&FEED_ID, &seed_from_str(AGGREGATORS_FEED_SEED)],
    );
    let expected = compute_pda(
        &AGGREGATOR,
        &[feed.value(), &seed_from_str(AGGREGATORS_PRICE_SEED)],
    );

    assert_eq!(price_account_address(&AGGREGATOR, &FEED_ID), expected);
}

/// The property `update_aggregator` exists to serve, stated as an assertion: a
/// different build is a different set of addresses, so a consumer that could not
/// be told about the new one could not follow.
#[test]
fn another_aggregator_build_puts_the_same_feed_at_another_address() {
    let rebuilt: ProgramId = [12u32; 8];

    assert_ne!(
        price_account_address(&AGGREGATOR, &FEED_ID),
        price_account_address(&rebuilt, &FEED_ID)
    );
}

#[test]
fn two_feeds_under_one_aggregator_are_two_price_accounts() {
    let mut other = FEED_ID;
    other[0] = b'E';

    assert_ne!(
        price_account_address(&AGGREGATOR, &FEED_ID),
        price_account_address(&AGGREGATOR, &other)
    );
}

/// The skew allowance is a constant rather than a registered value, so this one
/// really is a statement about every read: a consumer that tolerated less skew
/// than the verifier would refuse observations the aggregator was entitled to
/// publish, and neither operator would have chosen that.
#[test]
fn the_skew_allowance_is_the_verifiers() {
    assert_eq!(MAX_AHEAD_MS, verifier_core::feed::MAX_AHEAD_MS);
}

/// A ceiling, and only a ceiling.
///
/// It says an authority *may* register a window at least as wide as the
/// verifier's, not that any particular source has one: `register` accepts
/// anything from one millisecond up, and a narrow window refusing a price the
/// aggregator published is the consumer exercising the policy U7 asks it to
/// have. What this rules out is the ceiling itself forcing that choice.
#[test]
fn the_registrable_window_reaches_at_least_as_wide_as_the_verifiers() {
    const { assert!(MAX_MAX_AGE_MS >= verifier_core::feed::MAX_MAX_AGE_MS) };
}

/// The authority module is reference consumer B's, copied.
///
/// The two consumers are deliberately parallel so a reader can diff them, and
/// the governance half is where that is literal: strip the comments and the
/// test module and the two files are the same 199 lines, differing only in the
/// seed each derives its config address from. Nothing failed if they drifted.
///
/// The same arrangement as the constants above, for the same reason: a copy is
/// only safe when something fails on divergence. Extracting a shared crate
/// would work too and is the alternative, at the cost of the property that
/// makes these two consumers readable side by side.
#[test]
fn the_authority_module_is_still_reference_consumer_bs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the repository root");

    // Everything above `#[cfg(test)]`, without comments or blank lines: the two
    // crates' test modules build their fixtures differently and are not the
    // claim being made here.
    let logic = |relative: &str, seed: &str| -> String {
        let text = std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|_| panic!("{relative} is on disk"));
        text.split("#[cfg(test)]")
            .next()
            .expect("a logic half")
            .replace(seed, "KANON_CONFIG_SEED")
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim_start().starts_with("//") && !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };

    assert_eq!(
        logic(
            "reference-consumers/aggregator-read/src/authority.rs",
            "KANON_READ_CONFIG"
        ),
        logic(
            "reference-consumers/pull/src/authority.rs",
            "KANON_PULL_CONFIG"
        ),
        "the two consumers' authority modules have diverged. If that is intended, \
         this test is where to record what differs and why; if it is not, one of \
         them has drifted from a review the other already had"
    );
}
