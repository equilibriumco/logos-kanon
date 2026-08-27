//! The SPEL program: the aggregator's entry point, as LEZ executes it.
//!
//! Each instruction's surface is settled here -- which accounts it takes, in
//! which order, and with which arguments -- and each body refuses with the task
//! that fills it. The surface is the part later tasks must not quietly change,
//! because the SDK, the relayer and the IDL are all generated from it.
//!
//! The logic lives in `aggregator-program`, which the host workspace can test.
//! This file is the seam between that and LEZ.

#![cfg_attr(not(test), no_main)]

use aggregator_program::admin::AdminAccount;
use aggregator_program::FeedAccount;
use nssa_core::account::AccountWithMetadata;
use spel_framework::context::ProgramContext;
use spel_framework::prelude::*;

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

/// This build's genesis authority: the only signer that may establish the admin
/// account, and nothing afterwards.
///
/// A build input rather than a committed key, so devnet and mainnet do not share
/// one. All zeros is a build nobody configured, and `initialise` refuses it —
/// which is what stops a forgotten key becoming an open first write, the race
/// `[M2-06:01]` records as recurring once per deployment.
///
/// Set it at build time:
///
/// ```sh
/// KANON_GENESIS_ADMIN=<64 hex characters> cargo build
/// ```
const GENESIS_ADMIN: [u8; 32] = match option_env!("KANON_GENESIS_ADMIN") {
    Some(hex) => genesis_from_hex(hex),
    None => [0u8; 32],
};

/// Decode the genesis key at compile time, so a malformed one is a build failure
/// rather than a program that refuses every administrator at run time.
const fn genesis_from_hex(hex: &str) -> [u8; 32] {
    let bytes = hex.as_bytes();
    assert!(
        bytes.len() == 64,
        "KANON_GENESIS_ADMIN must be exactly 64 hexadecimal characters"
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
        _ => panic!("KANON_GENESIS_ADMIN is not hexadecimal"),
    }
}

/// What an instruction returns until the task that implements it lands.
///
/// A refusal rather than a panic: a program that aborts tells a caller nothing,
/// and the error carries the task id so an integrator reading a failed
/// transaction knows this is unbuilt rather than broken.
fn not_yet(instruction: &str, task: &str) -> SpelError {
    SpelError::custom(0, format!("{instruction} is not implemented yet ({task})"))
}

#[lez_program(instruction = "aggregator_program::Instruction")]
mod kanon_aggregator {
    #[allow(unused_imports)]
    use super::*;

    /// Verifies a signed RedStone payload and publishes the price it carries.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed, read for its configuration. Read-only,
    ///    and this program's: an account a caller owns would decode as a feed
    ///    with a signer set of the caller's choosing. That check is in
    ///    `submit_price` rather than an `owner` constraint, because the IDL
    ///    generator drops `owner` and a constraint no client can see is worse
    ///    than one that carries its own error.
    /// 2. `price_account` — the canonical RFP-019 account the price is written
    ///    to, one per feed at an address derived from the feed's own. Declared
    ///    rather than checked in code, because the derivation is what a relayer,
    ///    an SDK and a consumer each have to reproduce, and the constraint is
    ///    what publishes it to them.
    /// 3. `clock` — the LEZ clock program's every-block account. Read-only, and
    ///    the only admissible source of "now" (ADR 13).
    #[instruction]
    pub fn submit_price(
        ctx: ProgramContext,
        feed: AccountWithMetadata,
        // `r#const` and not `const`: the seed is parsed as a `syn::Expr`, and a
        // bare keyword is not one, so the documented spelling fails to parse
        // before the seed parser sees it. Both SPEL's parsers accept the raw
        // identifier.
        #[account(mut, pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]
        price_account: AccountWithMetadata,
        clock: AccountWithMetadata,
        payload: Vec<u8>,
    ) -> SpelResult {
        let post_states = aggregator_program::submit::submit_price(
            feed,
            price_account,
            clock,
            &payload,
            ctx.self_program_id,
        )?;
        // Fully qualified deliberately. `#[lez_program]` rewrites a call whose
        // path is exactly the two segments `SpelOutput::execute` into
        // `execute_with_claims`, which takes `&[Account]` and a generated claims
        // helper that has no way to reach the runtime feed seed -- and could not
        // build a conditional claim in any case, since `price_account` is `mut`
        // rather than `init`. Three segments are left alone, which is the form
        // Logos's own programs use wherever the delegated logic already decided
        // the post-states.
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Registers a feed with the parameters later submissions are checked against.
    ///
    /// Expected accounts:
    /// 1. `feed` — uninitialised, claimed by this program.
    /// 2. `admin` — the RFP-001 authority.
    #[expect(
        clippy::too_many_arguments,
        reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
    )]
    #[instruction]
    pub fn register_feed(
        ctx: ProgramContext,
        #[account(init)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        feed_id: Vec<u8>,
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        decimals: u8,
        max_age_ms: u64,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let _ = (
            feed,
            admin,
            config,
            feed_id,
            base_asset,
            quote_asset,
            decimals,
            max_age_ms,
            signers,
            threshold,
        );
        Err(not_yet("register_feed", "M2-07"))
    }

    /// Replaces one feed's signer set and threshold.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn update_signer_set(
        ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let _ = (feed, admin, config, signers, threshold);
        Err(not_yet("update_signer_set", "M2-08"))
    }

    /// Removes a feed's registration.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn deregister_feed(
        ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let _ = (feed, admin, config);
        Err(not_yet("deregister_feed", "M2-09"))
    }

    /// Stops a feed accepting submissions, leaving its registration intact.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn pause_feed(
        ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let _ = (feed, admin, config);
        Err(not_yet("pause_feed", "M2-10"))
    }

    /// Lets a paused feed accept submissions again.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn unpause_feed(
        ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let _ = (feed, admin, config);
        Err(not_yet("unpause_feed", "M2-10"))
    }

    /// Establishes this build's admin authority, once.
    ///
    /// Declared last so the two additions are appended: a variant's position in
    /// the instruction enum is the discriminant a caller encodes, and the parity
    /// test ties this order to that one.
    ///
    /// Expected accounts:
    /// 1. `config` — uninitialised, claimed by this program at the address the
    ///    constraint publishes, so a client derives it rather than being told it.
    /// 2. `admin` — the genesis authority, authorising itself. Which key that is
    ///    comes from the build and never from the transaction.
    #[instruction]
    pub fn initialise_admin(
        _ctx: ProgramContext,
        #[account(init, pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let post_states =
            aggregator_program::admin::initialise_admin(config, admin, &GENESIS_ADMIN)?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Nominates a key to take the admin authority. It does not take it yet.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the current authority.
    #[instruction]
    pub fn nominate_admin(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        new_admin: [u8; 32],
    ) -> SpelResult {
        let post_states = aggregator_program::admin::nominate_admin(
            config,
            admin,
            new_admin,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Takes the admin authority, as the nominated key.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the nominated key, signing for itself. This is the signature
    ///    that makes a mistyped nomination cost a second nomination rather than
    ///    the program's whole administrative surface.
    #[instruction]
    pub fn accept_admin(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let post_states =
            aggregator_program::admin::accept_admin(config, admin, ctx.self_program_id)?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Gives up the admin authority permanently, cancelling any nomination.
    ///
    /// Expected accounts:
    /// 1. `config` — the account holding the authority.
    /// 2. `admin` — the current authority.
    #[instruction]
    pub fn revoke_admin(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let post_states =
            aggregator_program::admin::revoke_admin(config, admin, ctx.self_program_id)?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }
}

/// Linked so the feed state the instructions read cross-compiles with them.
///
/// The IDL generator publishes [`FeedAccount`] from the crate that defines it;
/// naming it here is what makes a change to that type a build failure in the
/// guest rather than a surprise at deployment.
const _: Option<FeedAccount> = None;
/// The authority state, for the same reason: a client that cannot decode the
/// config account cannot tell who may administer a feed.
const _: Option<AdminAccount> = None;

/// The checks SPEL generates from the account attributes above.
///
/// They run in the dispatcher, before any handler, so nothing in
/// `aggregator-program` can reach them and no host test in that crate can
/// either. They exist only after macro expansion, which is why these two tests
/// live in the guest workspace — and why `cargo test --workspace` never runs
/// them.
#[cfg(test)]
mod tests {
    use aggregator_program::admin::ADMIN_CONFIG_SEED;
    use aggregator_program::submit::PRICE_ACCOUNT_SEED;
    use nssa_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
    use nssa_core::program::{InstructionData, ProgramId};
    use spel_framework::error::SpelError;
    use spel_framework::pda::{compute_pda, seed_from_str};

    const OURS: ProgramId = [7u32; 8];
    const FEED_ACCOUNT_ID: [u8; 32] = [0xFE; 32];

    fn account(id: [u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: OURS,
                balance: 0,
                data: Data::default(),
                nonce: Nonce(0),
            },
            is_authorized: false,
            account_id: AccountId::new(id),
        }
    }

    /// The address the constraint should accept, derived the way a client would.
    fn price_account_id() -> AccountId {
        compute_pda(
            &OURS,
            &[&FEED_ACCOUNT_ID, &seed_from_str(PRICE_ACCOUNT_SEED)],
        )
    }

    fn validate(price_account_id: AccountId) -> Result<(), SpelError> {
        let accounts = [
            account(FEED_ACCOUNT_ID),
            account(*price_account_id.value()),
            account(*b"/LEZ/ClockProgramAccount/0000001"),
        ];
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_submit_price(&accounts, &OURS, &instruction)
    }

    #[test]
    fn the_declared_accounts_pass_the_generated_validator() {
        // Also the assertion that this file and `aggregator-program` derive the
        // same address: the constraint hashes `r#const("KANON_PRICE_ACCOUNT")`
        // and the test hashes `PRICE_ACCOUNT_SEED`, and only one of the two is a
        // literal here.
        validate(price_account_id()).expect("the derived address is the declared one");
    }

    /// The config account's address, derived the way a client would.
    fn admin_config_id() -> AccountId {
        compute_pda(&OURS, &[&seed_from_str(ADMIN_CONFIG_SEED)])
    }

    /// An administrative instruction's accounts, in the order it declares them.
    fn validate_pause(config_id: AccountId) -> Result<(), SpelError> {
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let accounts = [account(FEED_ACCOUNT_ID), admin, account(*config_id.value())];
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_pause_feed(&accounts, &OURS, &instruction)
    }

    #[test]
    fn the_admin_config_address_the_constraint_accepts_is_the_derived_one() {
        // The same double-derivation check the price account gets: the
        // constraint hashes `r#const("KANON_ADMIN_CONFIG")` and this hashes
        // `ADMIN_CONFIG_SEED`, and only one of the two is a literal here.
        validate_pause(admin_config_id()).expect("the derived address is the declared one");
    }

    #[test]
    fn an_account_that_is_not_the_admin_config_is_refused_by_the_generated_validator() {
        // Without the constraint, any account this program owns whose bytes
        // decode as an authority would do -- including one a caller had this
        // program write for it. `authorise` checks ownership and cannot check
        // which of our accounts it was handed.
        assert!(
            matches!(
                validate_pause(AccountId::new([0x11; 32])),
                Err(SpelError::PdaMismatch { .. })
            ),
            "an account nobody derived has to be refused"
        );
    }

    #[test]
    fn an_administrative_instruction_refuses_an_admin_that_did_not_sign() {
        // The dispatcher's half of the gate. `authorise` checks the flag too,
        // and this is the only place the generated check itself is reachable.
        let accounts = [
            account(FEED_ACCOUNT_ID),
            account([0xAD; 32]),
            account(*admin_config_id().value()),
        ];
        let instruction: InstructionData = Vec::new();
        assert!(matches!(
            super::kanon_aggregator::__validate_pause_feed(&accounts, &OURS, &instruction),
            Err(SpelError::Unauthorized { .. })
        ));
    }

    #[test]
    fn an_unconfigured_build_refuses_to_establish_an_authority() {
        // What this covers that the host suite cannot: that the handler reads
        // `GENESIS_ADMIN` rather than a genesis of its own, and that a build
        // nobody configured refuses everyone instead of accepting the first
        // caller. The host tests pass a genesis in as an argument, so only here
        // is the constant itself on the path.
        //
        // It asserts the unconfigured case because that is the one CI can build.
        // A configured build is a deployment concern, and `[M2-06:01]` records
        // that the key is a build input.
        assert_eq!(
            super::GENESIS_ADMIN,
            [0u8; 32],
            "CI builds carry no genesis key"
        );

        let config = AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: admin_config_id(),
        };
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;

        let ctx = super::ProgramContext::new(OURS, [0u32; 8]);
        let refused = super::kanon_aggregator::initialise_admin(ctx, config, admin)
            .expect_err("a build with no genesis authority must refuse");

        // By code rather than by variant: `SpelError` carries no `PartialEq`, and
        // the code is the part a caller acts on.
        assert_eq!(
            refused.error_code(),
            SpelError::from(aggregator_program::admin::AdminError::AuthorityIsZero).error_code()
        );
    }

    #[test]
    fn a_wrong_price_account_pda_is_refused_by_the_generated_validator() {
        // What stops feed A writing feed B's price account. Every account here
        // is owned by this program, so ownership alone would admit it.
        let elsewhere = compute_pda(&OURS, &[&[0xAB; 32], &seed_from_str(PRICE_ACCOUNT_SEED)]);
        assert!(
            matches!(validate(elsewhere), Err(SpelError::PdaMismatch { .. })),
            "an account derived from another feed has to be refused"
        );

        // And a plain account nobody derived, which is the simpler mistake.
        assert!(matches!(
            validate(AccountId::new([0x11; 32])),
            Err(SpelError::PdaMismatch { .. })
        ));
    }
}
