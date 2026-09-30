//! The SPEL program: the aggregator's entry point, as LEZ executes it.
//!
//! Each instruction's surface is settled here -- which accounts it takes, in
//! which order, and with which arguments -- and each body refuses with the task
//! that fills it. The surface is the part later tasks must not quietly change,
//! because the SDK, the relayer and the IDL are all generated from it.
//!
//! The logic lives in `aggregator-program`, which the host workspace can test.
//! This file is the seam between that and LEZ.
//!
//! Accounts are declared subject first: the feed on the instructions that change
//! a feed, the config account on the ones that change the authority. So `config`
//! is third on the five feed instructions and first on the four admin ones. The
//! rule is worth stating because the order is positional on the wire and a
//! caller building both has otherwise to remember which.

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
/// `[M2-06:01]` records as recurring once per build.
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
    /// 1. `feed` — default or emptied-and-ours, at
    ///    `for_public_pda(program, sha256(feed_id || zero_pad_32("KANON_FEED_ACCOUNT")))`.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked
    ///    against, at the address its constraint derives.
    /// 4. `price_account` — read, never written. It is what remembers a feed
    ///    id's pair across a retirement, since a retirement empties the feed
    ///    account and leaves this one alone.
    #[expect(
        clippy::too_many_arguments,
        reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
    )]
    #[instruction]
    pub fn register_feed(
        ctx: ProgramContext,
        // The feed id is the seed, so a client finds a feed without being told
        // where it is, and one feed id has one account. Declared rather than
        // checked in code for the reason ADR 32 gives about the price account:
        // the derivation is what a client reproduces, and the constraint is what
        // publishes it in the IDL.
        //
        // `mut` and not `init`, which it was until M2-09 gave the account a third
        // pre-state. `init` emits `accounts[0] != Account::default()` in the
        // dispatcher, and a deregistered account is ours and empty rather than
        // default -- so the constraint would refuse every re-registration with
        // `AccountAlreadyInitialized` before the body could accept it, and a feed
        // id would be spent by its first retirement after all. The body decides
        // the pre-state, on one discriminator, because it is the only layer that
        // can tell the three apart.
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]
        feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        // Read-only, and declared rather than checked so the IDL publishes the
        // derivation a client has to reproduce -- the same seeds `submit_price`
        // declares, because it is the same account.
        #[account(pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]
        price_account: AccountWithMetadata,
        feed_id: [u8; 32],
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        decimals: u8,
        max_age_ms: u64,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let post_states = aggregator_program::register::register_feed(
            feed,
            admin,
            config,
            price_account,
            feed_id,
            base_asset,
            quote_asset,
            decimals,
            max_age_ms,
            signers,
            threshold,
            ctx.self_program_id,
        )?;
        // Three segments, as `submit_price` uses: the delegated logic already
        // built the claim, so the generated claims helper must not build a
        // second one.
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Replaces one feed's signer set and threshold.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed, at the address `feed_id` derives.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked
    ///    against, at the address its constraint derives.
    #[instruction]
    pub fn update_signer_set(
        ctx: ProgramContext,
        // Constrained to the address its id derives, as `register_feed` claims
        // it. Without this an admin could hand any account this program owns
        // whose bytes decode as a feed.
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]
        feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let post_states = aggregator_program::manage::update_signer_set(
            feed,
            admin,
            config,
            feed_id,
            signers,
            threshold,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Removes a feed's registration.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked
    ///    against, at the address its constraint derives.
    #[instruction]
    pub fn deregister_feed(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]
        feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let post_states = aggregator_program::manage::deregister_feed(
            feed,
            admin,
            config,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Stops a feed accepting submissions, leaving its registration intact.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked
    ///    against, at the address its constraint derives.
    #[instruction]
    pub fn pause_feed(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]
        feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let post_states = aggregator_program::manage::pause_feed(
            feed,
            admin,
            config,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Lets a paused feed accept submissions again.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority `admin` is checked
    ///    against, at the address its constraint derives.
    #[instruction]
    pub fn unpause_feed(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]
        feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        #[account(pda = [r#const("KANON_ADMIN_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        aggregator_program::admin::authorise(&config, &admin, ctx.self_program_id)?;
        let post_states = aggregator_program::manage::unpause_feed(
            feed,
            admin,
            config,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
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
/// either. They exist only after macro expansion, which is why these tests
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

    /// A feed id used only to satisfy the feed constraint, so these tests are
    /// about the *config* account and nothing else.
    const SOME_FEED: [u8; 32] = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

    /// An administrative instruction's accounts, in the order it declares them.
    ///
    /// The feed is at its derived address (ADR 33), because otherwise the feed
    /// constraint would refuse before the config account was ever looked at and
    /// these tests would pass for the wrong reason.
    fn validate_pause(config_id: AccountId) -> Result<(), SpelError> {
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let accounts = [
            account(*feed_account_id(&SOME_FEED).value()),
            admin,
            account(*config_id.value()),
        ];
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_pause_feed(&accounts, &OURS, &instruction, &SOME_FEED)
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
    fn every_administrative_instruction_refuses_an_admin_that_did_not_sign() {
        // The dispatcher's half of the gate, over all five handlers rather than
        // one. `#[account(signer)]` is repeated by hand on each of them, so a
        // missing annotation is a per-handler mistake and testing one handler
        // proves nothing about the other four. `authorise` checks the flag too,
        // which is why a gap here would not be an auth hole today -- but it
        // would be an unnoticed change to the IDL's `signer` metadata, which is
        // what a client builds a transaction from.
        type Validator = fn(
            &[AccountWithMetadata],
            &ProgramId,
            &InstructionData,
            &[u8; 32],
        ) -> Result<(), SpelError>;

        let handlers: [(&str, Validator); 5] = [
            (
                "register_feed",
                super::kanon_aggregator::__validate_register_feed,
            ),
            (
                "update_signer_set",
                super::kanon_aggregator::__validate_update_signer_set,
            ),
            (
                "deregister_feed",
                super::kanon_aggregator::__validate_deregister_feed,
            ),
            ("pause_feed", super::kanon_aggregator::__validate_pause_feed),
            (
                "unpause_feed",
                super::kanon_aggregator::__validate_unpause_feed,
            ),
        ];

        // The admin is index 1 and is not authorised. The signer check runs
        // before the pda check, so the feed address is not what these refuse on.
        let feed_at = feed_account_id(&SOME_FEED);
        let accounts = [
            account(*feed_at.value()),
            account([0xAD; 32]),
            account(*admin_config_id().value()),
            // Only `register_feed` declares a fourth; the others ignore it, and
            // passing it means none of these refuse merely for being handed a
            // short account list.
            account(*price_account_for(&feed_at).value()),
        ];
        let instruction: InstructionData = Vec::new();

        for (name, validate) in handlers {
            let refused = validate(&accounts, &OURS, &instruction, &SOME_FEED);
            assert!(
                matches!(refused, Err(SpelError::Unauthorized { .. })),
                "{name} admitted an admin that did not sign: {refused:?}"
            );
        }
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
            SpelError::from(aggregator_program::admin::AdminError::NoGenesisAuthority).error_code()
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

    /// The price account's address for a given feed account, derived the way a
    /// client would.
    fn price_account_for(feed_at: &AccountId) -> AccountId {
        compute_pda(
            &OURS,
            &[feed_at.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
        )
    }

    /// The feed account's address, derived the way a client would.
    fn feed_account_id(feed_id: &[u8; 32]) -> AccountId {
        compute_pda(
            &OURS,
            &[
                feed_id,
                &seed_from_str(aggregator_program::register::FEED_ACCOUNT_SEED),
            ],
        )
    }

    /// `register_feed`'s accounts, in the order it declares them, with the
    /// instruction data the `arg("feed_id")` seed is read from.
    fn validate_register(feed_id: [u8; 32], feed_at: AccountId) -> Result<(), SpelError> {
        validate_register_feed_account(feed_id, feed_at, Account::default())
    }

    /// The same, with the feed account's pre-state chosen: what the dispatcher
    /// does with an address that is right and an account that is not pristine.
    fn validate_register_feed_account(
        feed_id: [u8; 32],
        feed_at: AccountId,
        pre: Account,
    ) -> Result<(), SpelError> {
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let mut feed = account(*feed_at.value());
        feed.account = pre;
        // Fourth account: the price account, at the address derived from the
        // feed account's own id. A registration reads it for the pair it may not
        // change, so the constraint has to accept the same address
        // `submit_price` writes.
        let price = account(*price_account_for(&feed_at).value());
        let accounts = [feed, admin, account(*admin_config_id().value()), price];
        // The seed argument is handed to the validator directly rather than
        // decoded from the instruction: the generated claims function and the
        // validator take the same `&[u8; 32]`, which is what keeps the address
        // one thing.
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_register_feed(&accounts, &OURS, &instruction, &feed_id)
    }

    #[test]
    fn the_feed_address_the_constraint_accepts_is_the_one_its_id_derives() {
        // The seed is an instruction argument rather than a constant, so the
        // validator has to read it out of the instruction data to derive the
        // address. That is the part worth pinning: a client computing the
        // address off the IDL and this program deriving it must agree.
        let feed_id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        validate_register(feed_id, feed_account_id(&feed_id))
            .expect("the derived address is the declared one");
    }

    #[test]
    fn a_feed_account_at_another_address_is_refused_by_the_generated_validator() {
        // Without the constraint a caller could register a feed anywhere, and
        // two accounts could both claim to be BTC/USD with different signer
        // sets. The address is what makes one feed id one feed.
        let feed_id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        assert!(
            matches!(
                validate_register(feed_id, AccountId::new([0x11; 32])),
                Err(SpelError::PdaMismatch { .. })
            ),
            "an address nobody derived has to be refused"
        );
    }

    #[test]
    fn a_feed_address_derived_from_another_id_is_refused() {
        // The sharper case: a well-formed address for the wrong feed. Registering
        // ETH at BTC's address would let one id's registration occupy another's.
        let btc = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let eth = *b"ETH\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        assert!(
            matches!(
                validate_register(eth, feed_account_id(&btc)),
                Err(SpelError::PdaMismatch { .. })
            ),
            "the address has to follow the id in the instruction"
        );
    }

    #[test]
    fn a_deregistered_feed_account_reaches_the_body() {
        // What `#[account(mut)]` buys over `#[account(init)]`, and the reason the
        // declaration changed in M2-09. `init` emits
        // `accounts[0] != Account::default()` in the dispatcher; a deregistered
        // account is ours and empty rather than default, so under `init` every
        // re-registration died as `AccountAlreadyInitialized` before
        // `register_feed` could accept it -- and a feed id would be spent by its
        // first retirement, which is the outcome M2-09 exists to avoid.
        let feed_id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let emptied = Account {
            program_owner: OURS,
            balance: 0,
            data: Data::default(),
            nonce: Nonce(3),
        };

        validate_register_feed_account(feed_id, feed_account_id(&feed_id), emptied)
            .expect("a deregistered feed account has to reach the body");
    }

    #[test]
    fn a_squatted_feed_account_passes_the_validator_and_the_body_refuses_it() {
        // The squat from `m0/versions.md` question 4, and where its refusal lives
        // now. Dropping `init` moved it from the dispatcher into the body, which
        // is the only layer that can tell a squat (not ours, not default) from a
        // deregistration (ours, empty). The refusal is unchanged in effect and
        // still permanent -- `AlreadyRegistered`, pinned by
        // `tests/register_feed.rs::a_derived_address_can_be_squatted_and_this_pins_the_refusal`.
        let feed_id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let squatted = Account {
            balance: 1,
            ..Account::default()
        };

        validate_register_feed_account(feed_id, feed_account_id(&feed_id), squatted)
            .expect("the constraint checks the address, not the pre-state");
    }

    #[test]
    fn a_squatted_admin_config_is_refused_by_the_generated_validator() {
        // The worst case of the same attack, recorded here because the config
        // address takes no input an attacker cannot predict -- the program id and
        // a compile-time constant, both readable from this repository -- so it
        // can be squatted before the operator ever runs the bootstrap.
        // `initialise_admin` still declares `#[account(init, ...)] config` -- the
        // config account has no third pre-state, so it keeps the constraint
        // `register_feed` had to give up -- and the dispatcher therefore answers
        // first, with `AdminError::AlreadyInitialised` only the pure function's
        // answer. Not M2-07's to fix; `m0/versions.md` question 4.
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let mut config = account(*admin_config_id().value());
        config.account = Account {
            balance: 1,
            ..Account::default()
        };
        let accounts = [config, admin];
        let instruction: InstructionData = Vec::new();

        let refused =
            super::kanon_aggregator::__validate_initialise_admin(&accounts, &OURS, &instruction)
                .expect_err("a squatted config cannot be initialised");

        assert!(
            matches!(
                refused,
                SpelError::AccountAlreadyInitialized { account_index: 0 }
            ),
            "expected the init check to refuse it, got {refused:?}"
        );
    }

    /// `update_signer_set`'s accounts, in the order it declares them.
    fn validate_rotation(feed_id: [u8; 32], feed_at: AccountId) -> Result<(), SpelError> {
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let accounts = [
            account(*feed_at.value()),
            admin,
            account(*admin_config_id().value()),
        ];
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_update_signer_set(
            &accounts,
            &OURS,
            &instruction,
            &feed_id,
        )
    }

    #[test]
    fn a_rotation_accepts_the_feed_at_the_address_its_id_derives() {
        // The same constraint `register_feed` claims under, so the account a
        // registration created is the only one a rotation reaches.
        let id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        validate_rotation(id, feed_account_id(&id))
            .expect("the derived address is the declared one");
    }

    #[test]
    fn a_rotation_refuses_a_feed_account_at_another_address() {
        // Without the constraint an admin could hand any account this program
        // owns whose bytes decode as a feed. `update_signer_set` checks the
        // stored id too, and this is the half the constraint carries.
        let id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        assert!(
            matches!(
                validate_rotation(id, AccountId::new([0x11; 32])),
                Err(SpelError::PdaMismatch { .. })
            ),
            "an address nobody derived has to be refused"
        );
    }

    #[test]
    fn a_rotation_refuses_one_feeds_account_under_another_feeds_id() {
        // The address follows the id in the instruction, so naming ETH while
        // handing BTC's account is refused before the body runs -- and the body
        // refuses it again on the stored id, which is the check that does not
        // depend on this constraint being right.
        let btc = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let eth = *b"ETH\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        assert!(
            matches!(
                validate_rotation(eth, feed_account_id(&btc)),
                Err(SpelError::PdaMismatch { .. })
            ),
            "the address has to follow the id"
        );
    }

    #[test]
    fn a_deregistration_accepts_only_the_feed_at_the_derived_address() {
        // Same constraint as the other two, so every operation on a feed reaches
        // the account `register_feed` claimed and nothing else.
        let id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let ok = [
            account(*feed_account_id(&id).value()),
            admin.clone(),
            account(*admin_config_id().value()),
        ];
        let instruction: InstructionData = Vec::new();
        super::kanon_aggregator::__validate_deregister_feed(&ok, &OURS, &instruction, &id)
            .expect("the derived address is the declared one");

        let wrong = [
            account([0x11; 32]),
            admin,
            account(*admin_config_id().value()),
        ];
        assert!(matches!(
            super::kanon_aggregator::__validate_deregister_feed(&wrong, &OURS, &instruction, &id),
            Err(SpelError::PdaMismatch { .. })
        ));
    }

    #[test]
    fn pausing_and_resuming_accept_only_the_feed_at_the_derived_address() {
        // The last two of the five feed operations, under the same constraint, so
        // no administrative instruction can reach an account `register_feed` did
        // not claim.
        let id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let mut admin = account([0xAD; 32]);
        admin.is_authorized = true;
        let ok = [
            account(*feed_account_id(&id).value()),
            admin.clone(),
            account(*admin_config_id().value()),
        ];
        let wrong = [
            account([0x11; 32]),
            admin,
            account(*admin_config_id().value()),
        ];
        let instruction: InstructionData = Vec::new();

        super::kanon_aggregator::__validate_pause_feed(&ok, &OURS, &instruction, &id)
            .expect("pause accepts the derived address");
        super::kanon_aggregator::__validate_unpause_feed(&ok, &OURS, &instruction, &id)
            .expect("unpause accepts the derived address");
        assert!(matches!(
            super::kanon_aggregator::__validate_pause_feed(&wrong, &OURS, &instruction, &id),
            Err(SpelError::PdaMismatch { .. })
        ));
        assert!(matches!(
            super::kanon_aggregator::__validate_unpause_feed(&wrong, &OURS, &instruction, &id),
            Err(SpelError::PdaMismatch { .. })
        ));
    }
    /// A config account holding `admin_key` as the established authority.
    ///
    /// Encoded by hand rather than with `borsh::to_vec` to avoid another direct
    /// test dependency and a change to the guest's pinned lockfile. `AdminAccount`
    /// is two `Option<[u8; 32]>`, and Borsh writes an option as a one-byte tag then
    /// the payload. `Some(key)` is `1` and the key, and `None` is a lone `0`. A
    /// change to the struct breaks this loudly: the bytes stop decoding and the
    /// gate answers `NoAuthority`.
    fn config_holding(admin_key: [u8; 32]) -> AccountWithMetadata {
        let mut data = Vec::with_capacity(34);
        data.push(1);
        data.extend_from_slice(&admin_key);
        data.push(0);
        let mut config = account(*admin_config_id().value());
        config.account.data = Data::try_from(data).expect("fits");
        config
    }

    #[test]
    fn only_the_stored_authority_can_rotate_a_signer_set() {
        // SEC2 end to end rather than per function. That `authorise` refuses a
        // stranger, and that `update_signer_set` rotates a feed, are each covered
        // on their own -- and neither says the handler runs them in that order.
        // The gate is a Rust call in the body rather than a constraint the
        // dispatcher applies, so this layer is the only one it is observable
        // from: a handler that dropped the call would pass every host test and
        // every generated-validator test above.
        const AUTHORITY: [u8; 32] = [0xA1; 32];
        const STRANGER: [u8; 32] = [0x5A; 32];
        let feed_id = *b"BTC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

        let rotate = |who: [u8; 32]| {
            let mut admin = account(who);
            admin.is_authorized = true;
            super::kanon_aggregator::update_signer_set(
                super::ProgramContext::new(OURS, [0u32; 8]),
                account(*feed_account_id(&feed_id).value()),
                admin,
                config_holding(AUTHORITY),
                feed_id,
                (10..=12u8).map(|i| [i; 20]).collect(),
                2,
            )
        };

        // A signer who is not the stored authority never reaches the rotation.
        let refused = rotate(STRANGER).expect_err("a stranger must not rotate a signer set");
        assert_eq!(
            refused.error_code(),
            SpelError::from(aggregator_program::admin::AdminError::Unauthorised).error_code(),
            "the refusal has to be the admin gate's"
        );

        // The authority gets past the gate. It stops at the *next* check rather
        // than succeeding, because the feed account here carries no feed -- and
        // that is the assertion: what changed between these two calls is the
        // signer, and the gate is what noticed.
        let onwards = rotate(AUTHORITY).expect_err("no feed is stored at that address");
        assert_eq!(
            onwards.error_code(),
            SpelError::from(aggregator_program::manage::ManageError::FeedDeregistered).error_code(),
            "the authority has to get past the gate and be stopped by the feed instead"
        );
    }
}
