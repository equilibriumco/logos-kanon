//! The SPEL program: reference consumer A's entry point, as LEZ executes it.
//!
//! Eight instructions, all thin, and the same eight reference consumer B
//! declares. The logic is in `reference-consumer-aggregator-read`, which the host
//! workspace can test; this file is the seam between that and LEZ.
//!
//! **The clock's bytes are bound to the account they came from.** `clock` in
//! `settle` is one `AccountWithMetadata` the dispatcher supplied, and the logic
//! reads its id and its data out of that one struct.
//!
//! **The price account carries no `pda` constraint, and that is deliberate.**
//! Every other derived account here is this program's own, so the constraint
//! both enforces the address and publishes it in the IDL for a client to
//! reproduce. The price account is the *aggregator's*, derived under a program
//! id this program holds in state, and SPEL's `pda` constraint derives under
//! `self_program_id` — it cannot express another program's address. So the check
//! moves into Rust, in `read::read_price`, which is the first thing it does. The
//! split is the one `aggregator-program` states for its owner constraint:
//! declare what a client must derive, check in Rust what only the program can
//! explain.
//!
//! Accounts are declared subject first: the order on the two order instructions,
//! the source account on the three that change a feed, the config on the three
//! that change the authority. The order is positional on the wire, so a caller
//! building several has otherwise to remember which.

#![cfg_attr(not(test), no_main)]

use nssa_core::account::AccountWithMetadata;
use spel_framework::context::ProgramContext;
use spel_framework::prelude::*;

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

/// This build's genesis authority: the only signer that may establish the config
/// account, and nothing afterwards.
///
/// A build input rather than a committed key, so a devnet deployment and a real
/// one do not share one. All zeros is a build nobody configured, and `establish`
/// refuses it — which is what stops a forgotten key becoming an open first write,
/// the race whoever watches for a deployment would otherwise win.
///
/// Set it at build time:
///
/// ```sh
/// KANON_READ_GENESIS_AUTHORITY=<64 hex characters> cargo build
/// ```
const GENESIS_AUTHORITY: [u8; 32] = match option_env!("KANON_READ_GENESIS_AUTHORITY") {
    Some(hex) => genesis_from_hex(hex),
    None => [0u8; 32],
};

/// Decodes the genesis key at compile time, so a malformed one is a build failure
/// rather than a deployment that cannot be established.
const fn genesis_from_hex(hex: &str) -> [u8; 32] {
    let bytes = hex.as_bytes();
    assert!(
        bytes.len() == 64,
        "KANON_READ_GENESIS_AUTHORITY must be 64 hex characters"
    );
    let mut key = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        key[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    key
}

const fn nibble(character: u8) -> u8 {
    match character {
        b'0'..=b'9' => character - b'0',
        b'a'..=b'f' => character - b'a' + 10,
        b'A'..=b'F' => character - b'A' + 10,
        _ => panic!("KANON_READ_GENESIS_AUTHORITY is not hexadecimal"),
    }
}

#[lez_program]
mod kanon_aggregator_read_consumer {
    #[allow(unused_imports)]
    use super::*;

    /// Establishes this program's authority from the key it was built with.
    ///
    /// Expected accounts:
    /// 1. `config` — the authority's account, at
    ///    `for_public_pda(program, sha256(zero_pad_32("KANON_READ_CONFIG")))`.
    ///    `mut` and not `init`: `init` emits its own `AccountAlreadyInitialized`
    ///    in the dispatcher, which would hide the difference between an
    ///    authority already established and an address somebody squatted.
    /// 2. `authority` — the signer claiming to be this build's genesis key.
    #[instruction]
    pub fn establish_authority(
        ctx: ProgramContext,
        // `r#const` and not `const`: the seed is parsed as a `syn::Expr` and a
        // bare keyword is not one, so the documented spelling fails to parse
        // before the seed parser sees it.
        #[account(mut, pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::authority::establish(
            config,
            authority,
            &GENESIS_AUTHORITY,
            ctx.self_program_id,
        )?;
        // Fully qualified deliberately, here and in every instruction below.
        // `#[lez_program]` rewrites a call whose path is exactly the two segments
        // `SpelOutput::execute` into `execute_with_claims`, which takes
        // `&[Account]` and a generated claims helper with no way to reach a
        // runtime seed. Three segments are left alone, which is the form Logos's
        // own programs use wherever the delegated logic already decided the
        // post-states.
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Records a key that may take the authority over once it accepts.
    ///
    /// Expected accounts:
    /// 1. `config` — the authority's account.
    /// 2. `authority` — the current authority, signing.
    #[instruction]
    pub fn nominate_authority(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        nominee: [u8; 32],
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::authority::nominate(
            config,
            authority,
            nominee,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Takes the authority over, as the nominee.
    ///
    /// Expected accounts:
    /// 1. `config` — the authority's account.
    /// 2. `nominee` — the nominated key, signing.
    #[instruction]
    pub fn accept_authority(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] nominee: AccountWithMetadata,
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::authority::accept(
            config,
            nominee,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Registers which price account this consumer believes for one feed.
    ///
    /// Expected accounts:
    /// 1. `source` — the feed's source account, at
    ///    `for_public_pda(program, sha256(feed_id || zero_pad_32("KANON_READ_SOURCE")))`.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[expect(
        clippy::too_many_arguments,
        reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
    )]
    #[instruction]
    pub fn register_price_source(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_READ_SOURCE")])]
        source: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
        aggregator: [u32; 8],
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        max_age_ms: u64,
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::source::register(
            source,
            authority,
            config,
            feed_id,
            aggregator,
            base_asset,
            quote_asset,
            max_age_ms,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Points one feed at another aggregator build.
    ///
    /// The instruction the aggregator's own rebuild schedule is the reason for:
    /// a program's id is its image id, so an aggregator build whose inputs
    /// changed moves every price account it writes, and a consumer with the id
    /// compiled in would answer that with a redeployment of its own. A rebuild
    /// from unchanged inputs reproduces the id and needs nothing
    /// (`[M2-06:01]`).
    ///
    /// Expected accounts:
    /// 1. `source` — the feed's source account.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[instruction]
    pub fn update_aggregator(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_READ_SOURCE")])]
        source: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
        aggregator: [u32; 8],
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::source::update_aggregator(
            source,
            authority,
            config,
            feed_id,
            aggregator,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Retires one feed's price source, leaving the account empty and still ours.
    ///
    /// Expected accounts:
    /// 1. `source` — the feed's source account.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[instruction]
    pub fn deregister_price_source(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_READ_SOURCE")])]
        source: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_READ_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::source::deregister(
            source,
            authority,
            config,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Opens an order at a price its owner is prepared to trade at.
    ///
    /// `base_asset` and `quote_asset` are the owner's own expectation and are
    /// compared against the registration here. They are not decoration: nothing
    /// the aggregator publishes attests to which assets a feed prices, so a
    /// mislabelled registration is only catchable by the party that signed for a
    /// pair.
    ///
    /// Expected accounts:
    /// 1. `order` — the new order account, at
    ///    `for_public_pda(program, sha256(order_id || zero_pad_32("KANON_READ_ORDER")))`.
    /// 2. `owner` — the signer the order belongs to.
    /// 3. `source` — the feed's source account.
    #[expect(
        clippy::too_many_arguments,
        reason = "the owner's expected pair is what makes the settlement's pair check real, and it has to be named separately from the feed it is checked against"
    )]
    #[instruction]
    pub fn open_order(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("order_id"), r#const("KANON_READ_ORDER")])]
        order: AccountWithMetadata,
        #[account(signer)] owner: AccountWithMetadata,
        #[account(pda = [arg("feed_id"), r#const("KANON_READ_SOURCE")])]
        source: AccountWithMetadata,
        order_id: [u8; 32],
        feed_id: [u8; 32],
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        limit_price_q64: u128,
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::order::open_order(
            order,
            owner,
            source,
            feed_id,
            base_asset,
            quote_asset,
            limit_price_q64,
            order_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Fills an order if the price the aggregator published has reached its limit.
    ///
    /// Permissionless: the sender is not checked, because the sender attests to
    /// nothing. What makes the price trustworthy is the account it was read from.
    ///
    /// Expected accounts:
    /// 1. `order` — the order account to fill. No `pda` constraint, because the
    ///    order id is not an argument here: the account is identified by being
    ///    this program's and decoding as an order. What keeps that sound is that
    ///    borsh refuses trailing bytes and this program's three account types
    ///    encode to different lengths, which `tests/settle.rs`'s
    ///    `the_account_types_this_program_owns_have_distinct_encoded_lengths`
    ///    asserts, because nothing else would notice a field added to one of them.
    /// 2. `source` — the feed's source account, which names the aggregator.
    /// 3. `price` — the aggregator's price account. No `pda` constraint: it is
    ///    derived under another program's id, which the constraint cannot
    ///    express. `read::read_price` checks the address first instead.
    /// 4. `clock` — the LEZ clock account.
    #[instruction]
    pub fn settle(
        ctx: ProgramContext,
        #[account(mut)] order: AccountWithMetadata,
        #[account(pda = [arg("feed_id"), r#const("KANON_READ_SOURCE")])]
        source: AccountWithMetadata,
        price: AccountWithMetadata,
        clock: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        let post_states = reference_consumer_aggregator_read::order::settle(
            order,
            source,
            price,
            clock,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }
}

#[cfg(test)]
mod tests {
    use nssa_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
    use nssa_core::program::{InstructionData, ProgramId};
    use reference_consumer_aggregator_read::authority::CONFIG_ACCOUNT_SEED;
    use reference_consumer_aggregator_read::order::ORDER_ACCOUNT_SEED;
    use reference_consumer_aggregator_read::source::SOURCE_ACCOUNT_SEED;
    use spel_framework::error::SpelError;
    use spel_framework::pda::{compute_pda, seed_from_str};

    const OURS: ProgramId = [7u32; 8];
    const ORDER_ID: [u8; 32] = [0x0D; 32];
    const FEED_ID: [u8; 32] = [0xFE; 32];

    fn account(id: AccountId, signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: OURS,
                balance: 0,
                data: Data::default(),
                nonce: Nonce(0),
            },
            is_authorized: signs,
            account_id: id,
        }
    }

    fn id(raw: [u8; 32]) -> AccountId {
        AccountId::new(raw)
    }

    /// The addresses the constraints should accept, derived the way a client
    /// would. Each hashes the seed constant the logic crate exports, while the
    /// constraint hashes an `r#const` literal in this file — so these are also
    /// the assertion that the two spellings agree.
    fn config_id() -> AccountId {
        compute_pda(&OURS, &[&seed_from_str(CONFIG_ACCOUNT_SEED)])
    }

    fn source_id() -> AccountId {
        compute_pda(&OURS, &[&FEED_ID, &seed_from_str(SOURCE_ACCOUNT_SEED)])
    }

    fn order_id() -> AccountId {
        compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
    }

    #[test]
    fn the_declared_accounts_pass_every_generated_validator() {
        let empty: InstructionData = Vec::new();

        super::kanon_aggregator_read_consumer::__validate_establish_authority(
            &[account(config_id(), false), account(id([0xA1; 32]), true)],
            &OURS,
            &empty,
        )
        .expect("the config derivation is the declared one");

        super::kanon_aggregator_read_consumer::__validate_update_aggregator(
            &[
                account(source_id(), false),
                account(id([0xA1; 32]), true),
                account(config_id(), false),
            ],
            &OURS,
            &empty,
            &FEED_ID,
        )
        .expect("the source derivation is the declared one");

        super::kanon_aggregator_read_consumer::__validate_open_order(
            &[
                account(order_id(), false),
                account(id([0xA0; 32]), true),
                account(source_id(), false),
            ],
            &OURS,
            &empty,
            &ORDER_ID,
            &FEED_ID,
        )
        .expect("the order derivation is the declared one");

        super::kanon_aggregator_read_consumer::__validate_settle(
            &[
                account(id([0x0E; 32]), false),
                account(source_id(), false),
                account(id([0xAB; 32]), false),
                account(id(*b"/LEZ/ClockProgramAccount/0000001"), false),
            ],
            &OURS,
            &empty,
            &FEED_ID,
        )
        .expect("settle's source constraint accepts the named feed's address");
    }

    /// The property that makes the price account's *unconstrained* declaration
    /// safe: an arbitrary account passes the dispatcher, so the check that
    /// matters has to be the Rust one in `read::read_price`.
    #[test]
    fn the_generated_validator_does_not_check_the_price_account() {
        let empty: InstructionData = Vec::new();

        super::kanon_aggregator_read_consumer::__validate_settle(
            &[
                account(id([0x0E; 32]), false),
                account(source_id(), false),
                // Any address at all, which is exactly the point.
                account(id([0x99; 32]), false),
                account(id(*b"/LEZ/ClockProgramAccount/0000001"), false),
            ],
            &OURS,
            &empty,
            &FEED_ID,
        )
        .expect("the dispatcher has no opinion about another program's PDA");
    }

    #[test]
    fn a_source_account_for_another_feed_is_refused_by_settles_validator() {
        let empty: InstructionData = Vec::new();
        let another = [0xEE; 32];

        super::kanon_aggregator_read_consumer::__validate_settle(
            &[
                account(id([0x0E; 32]), false),
                account(source_id(), false),
                account(id([0xAB; 32]), false),
                account(id(*b"/LEZ/ClockProgramAccount/0000001"), false),
            ],
            &OURS,
            &empty,
            &another,
        )
        .expect_err("the source account must be the named feed's");
    }

    #[test]
    fn an_account_at_an_underived_address_is_refused_by_the_generated_validator() {
        let empty: InstructionData = Vec::new();

        super::kanon_aggregator_read_consumer::__validate_establish_authority(
            &[
                account(id([0x11; 32]), false),
                account(id([0xA1; 32]), true),
            ],
            &OURS,
            &empty,
        )
        .expect_err("the config address is the constraint's, not the caller's");
    }

    /// What this covers that the host suite cannot: that the *handler* reads
    /// `GENESIS_AUTHORITY` rather than a genesis of its own, and that a build
    /// nobody configured refuses rather than admitting whoever arrives first.
    ///
    /// So it calls the generated handler and not the library behind it. An
    /// earlier version called `authority::establish` with the constant passed in
    /// explicitly, which cannot see what the handler passes: replacing
    /// `&GENESIS_AUTHORITY` at the call site with `&[0x11u8; 32]` -- a build
    /// handing the authority to whoever holds that key -- left all six guest
    /// tests green. Measured again with this version, and it fails.
    #[test]
    fn an_unconfigured_build_refuses_to_establish_an_authority() {
        assert_eq!(
            super::GENESIS_AUTHORITY,
            [0u8; 32],
            "this test only means anything in a build with no key set"
        );

        let config = AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: config_id(),
        };
        let mut authority = account(id([0xA1; 32]), true);
        authority.account.program_owner = [42u32; 8];

        let ctx = super::ProgramContext::new(OURS, [0u32; 8]);
        let refused =
            super::kanon_aggregator_read_consumer::establish_authority(ctx, config, authority)
                .expect_err("a build with no genesis authority must refuse");

        // By code rather than by variant: `SpelError` carries no `PartialEq`,
        // and the code is the part a caller acts on.
        assert_eq!(
            refused.error_code(),
            SpelError::from(
                reference_consumer_aggregator_read::authority::AuthorityError::NoGenesisAuthority
            )
            .error_code()
        );
    }

    /// The handler hands each of its four `[u8; 32]` arguments to the parameter
    /// it names.
    ///
    /// `open_order` takes `order_id`, `feed_id`, `base_asset` and `quote_asset`
    /// adjacently and passes them to the library in a different order, so any
    /// two of them can be transposed and still compile. The IDL test cannot see
    /// it either, because the IDL is generated from the signature rather than
    /// from the call.
    ///
    /// The four ids here are pairwise distinct and the registration agrees with
    /// exactly one assignment of them, so every transposition refuses: swapping
    /// `feed_id` with anything reaches `FeedMismatch`, swapping the pair reaches
    /// `PairMismatch`. Asserting the success is therefore the whole check.
    #[test]
    fn the_open_order_handler_passes_each_id_to_the_argument_it_names() {
        use reference_consumer_aggregator_read::source::{source_address, PriceSource};

        const BASE: [u8; 32] = [0xBA; 32];
        const QUOTE: [u8; 32] = [0x9C; 32];

        let stored = PriceSource {
            feed_id: FEED_ID,
            aggregator: [11u32; 8],
            base_asset: BASE,
            quote_asset: QUOTE,
            max_age_ms: 60_000,
        };
        let mut source = account(id(*source_address(&OURS, &FEED_ID).value()), false);
        source.account.data =
            Data::try_from(borsh::to_vec(&stored).expect("serialises")).expect("a source fits");

        let mut order = account(order_id(), false);
        order.account.program_owner = [0u32; 8];

        let ctx = super::ProgramContext::new(OURS, [0u32; 8]);
        super::kanon_aggregator_read_consumer::open_order(
            ctx,
            order,
            account(id([0xA0; 32]), true),
            source,
            ORDER_ID,
            FEED_ID,
            BASE,
            QUOTE,
            1,
        )
        .expect("the handler passes the ids the way its own signature names them");
    }

    /// The hex decoder runs at compile time, so a build with a key set proves
    /// itself and a build without one proves nothing. Exercising it here is what
    /// keeps `nibble` and the digit arithmetic covered either way.
    #[test]
    fn the_genesis_key_is_decoded_from_its_hex_exactly() {
        const KEY: [u8; 32] = super::genesis_from_hex(
            "0123456789abcdef0123456789abcdef0123456789ABCDEF0123456789ABCDEF",
        );

        assert_eq!(KEY[0], 0x01);
        assert_eq!(KEY[7], 0xef);
        assert_eq!(KEY[16], 0x01);
        assert_eq!(KEY[31], 0xef);
        assert_eq!(KEY[..16], KEY[16..], "the two halves are the same bytes");
    }
}
