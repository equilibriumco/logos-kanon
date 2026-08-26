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

use aggregator_program::FeedAccount;
use nssa_core::account::AccountWithMetadata;
use spel_framework::context::ProgramContext;
use spel_framework::prelude::*;

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

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
        _ctx: ProgramContext,
        #[account(init)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        feed_id: Vec<u8>,
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        decimals: u8,
        max_age_ms: u64,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        let _ = (
            feed,
            admin,
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
        _ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        let _ = (feed, admin, signers, threshold);
        Err(not_yet("update_signer_set", "M2-08"))
    }

    /// Removes a feed's registration.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn deregister_feed(
        _ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let _ = (feed, admin);
        Err(not_yet("deregister_feed", "M2-09"))
    }

    /// Stops a feed accepting submissions, leaving its registration intact.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn pause_feed(
        _ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let _ = (feed, admin);
        Err(not_yet("pause_feed", "M2-10"))
    }

    /// Lets a paused feed accept submissions again.
    ///
    /// Expected accounts:
    /// 1. `feed` — the registered feed.
    /// 2. `admin` — the RFP-001 authority.
    #[instruction]
    pub fn unpause_feed(
        _ctx: ProgramContext,
        #[account(mut)] feed: AccountWithMetadata,
        #[account(signer)] admin: AccountWithMetadata,
    ) -> SpelResult {
        let _ = (feed, admin);
        Err(not_yet("unpause_feed", "M2-10"))
    }
}

/// Linked so the feed state the instructions read cross-compiles with them.
///
/// The IDL generator publishes [`FeedAccount`] from the crate that defines it;
/// naming it here is what makes a change to that type a build failure in the
/// guest rather than a surprise at deployment.
const _: Option<FeedAccount> = None;

/// The checks SPEL generates from the account attributes above.
///
/// They run in the dispatcher, before any handler, so nothing in
/// `aggregator-program` can reach them and no host test in that crate can
/// either. They exist only after macro expansion, which is why these two tests
/// live in the guest workspace — and why `cargo test --workspace` never runs
/// them.
#[cfg(test)]
mod tests {
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
