//! The SPEL program: reference consumer B's entry point, as LEZ executes it.
//!
//! Two instructions, both thin. The logic is in `reference-consumer-pull`, which
//! the host workspace can test; this file is the seam between that and LEZ, and
//! the seam is where the two things `pull-lib` cannot check are settled.
//!
//! **The clock's bytes are bound to the account they came from.** `clock` below
//! is one `AccountWithMetadata` the dispatcher supplied, and `settle` reads its
//! id and its data out of that one struct. A consumer that took a timestamp as
//! an argument, or paired the pinned account id with bytes of its own, would pass
//! every check in `pull-lib` and be lying to its own users about how fresh a
//! price is.
//!
//! **Nothing a caller sends reaches the signer set.** The instruction data below
//! is an order id, a feed index, a limit price and a payload. There is no
//! parameter for a roster, a threshold or a window, because those are constants
//! in `reference-consumer-pull` — see its module header on why that is the whole
//! of pull mode's security model.
//!
//! Accounts are declared subject first, the order account before the clock, the
//! way the aggregator declares the feed before the accounts it reads. The order
//! matters because it is positional on the wire.

#![cfg_attr(not(test), no_main)]

use nssa_core::account::AccountWithMetadata;
use spel_framework::context::ProgramContext;
use spel_framework::prelude::*;

#[cfg(not(test))]
risc0_zkvm::guest::entry!(main);

#[lez_program]
mod kanon_pull_consumer {
    #[allow(unused_imports)]
    use super::*;

    /// Opens an order at the price its owner is prepared to trade at.
    ///
    /// Expected accounts:
    /// 1. `order` — the order's account, at
    ///    `for_public_pda(program, sha256(order_id || zero_pad_32("KANON_PULL_ORDER")))`.
    ///    Declared rather than checked in code, because the derivation is what a
    ///    client has to reproduce and the constraint is what publishes it in the
    ///    IDL. `mut` and not `init`: `init` emits its own
    ///    `AccountAlreadyInitialized` in the dispatcher, which would hide the
    ///    difference between an id already in use and an address somebody
    ///    squatted, and those are two different pieces of advice.
    /// 2. `owner` — the signer the order belongs to.
    #[instruction]
    pub fn open_order(
        ctx: ProgramContext,
        // `r#const` and not `const`: the seed is parsed as a `syn::Expr` and a
        // bare keyword is not one, so the documented spelling fails to parse
        // before the seed parser sees it.
        #[account(mut, pda = [arg("order_id"), r#const("KANON_PULL_ORDER")])]
        order: AccountWithMetadata,
        #[account(signer)] owner: AccountWithMetadata,
        order_id: [u8; 32],
        feed: u8,
        limit_price_q64: u128,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::open_order(
            order,
            owner,
            feed,
            limit_price_q64,
            order_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Fills an order if a payload verified here says the market reached its
    /// limit.
    ///
    /// Permissionless, like the aggregator's `submit_price` and for the same
    /// reason: the sender attests to nothing about the bytes. Authenticity comes
    /// from the signatures in `payload`, checked against the consumer's own
    /// signer set.
    ///
    /// Expected accounts:
    /// 1. `order` — the order to fill. Owned by this program and holding an
    ///    order, both checked in `settle`: an account a caller owns would decode
    ///    as an order with a limit price of the caller's choosing. Checked there
    ///    rather than by an `owner` constraint because the IDL generator parses
    ///    `owner` and discards it, so a constraint no client can see is worse
    ///    than one that carries its own error.
    ///
    ///    No `pda` constraint here, unlike `open_order`. It would buy nothing:
    ///    every account this program owns is an order account, so a caller
    ///    passing one at an underived address is passing an account that fails
    ///    the ownership check, and a caller passing a real order is settling the
    ///    order it named. The derivation is published on `open_order`, which is
    ///    where a client needs it.
    /// 2. `clock` — the LEZ clock program's every-block account, read-only, and
    ///    the only admissible source of "now" (ADR 13). Passed whole, which is
    ///    what binds its bytes to its id.
    #[instruction]
    pub fn settle(
        ctx: ProgramContext,
        #[account(mut)] order: AccountWithMetadata,
        clock: AccountWithMetadata,
        payload: Vec<u8>,
    ) -> SpelResult {
        let post_states =
            reference_consumer_pull::settle(order, clock, &payload, ctx.self_program_id)?;
        // Fully qualified deliberately. `#[lez_program]` rewrites a call whose
        // path is exactly the two segments `SpelOutput::execute` into
        // `execute_with_claims`, which takes `&[Account]` and a generated claims
        // helper with no way to reach a runtime seed. Three segments are left
        // alone, which is the form Logos's own programs use wherever the
        // delegated logic already decided the post-states.
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }
}

#[cfg(test)]
mod tests {
    use nssa_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
    use nssa_core::program::{InstructionData, ProgramId};
    use reference_consumer_pull::ORDER_ACCOUNT_SEED;
    use spel_framework::error::SpelError;
    use spel_framework::pda::{compute_pda, seed_from_str};

    const OURS: ProgramId = [7u32; 8];
    const ORDER_ID: [u8; 32] = [0x0D; 32];

    fn account(id: [u8; 32], signs: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: OURS,
                balance: 0,
                data: Data::default(),
                nonce: Nonce(0),
            },
            is_authorized: signs,
            account_id: AccountId::new(id),
        }
    }

    /// The address the constraint should accept, derived the way a client would.
    fn order_account_id() -> AccountId {
        compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
    }

    /// The generated validator takes only the arguments its constraints name,
    /// so `order_id` is here and the feed index and limit price are not.
    fn validate_open(order_id: AccountId, owner_signs: bool) -> Result<(), SpelError> {
        let accounts = [
            account(*order_id.value(), false),
            account([0xA0; 32], owner_signs),
        ];
        let instruction: InstructionData = Vec::new();
        super::kanon_pull_consumer::__validate_open_order(&accounts, &OURS, &instruction, &ORDER_ID)
    }

    #[test]
    fn the_declared_order_accounts_pass_the_generated_validator() {
        // Also the assertion that this file and `reference-consumer-pull` derive
        // the same address: the constraint hashes `r#const("KANON_PULL_ORDER")`
        // and this hashes `ORDER_ACCOUNT_SEED`, and only one of the two is a
        // literal here.
        validate_open(order_account_id(), true).expect("the derived address is the declared one");
    }

    #[test]
    fn an_order_account_at_another_address_is_refused_by_the_generated_validator() {
        // Without the constraint an order could be opened at an address nobody
        // can derive, which is an order no client could ever find again.
        assert!(
            matches!(
                validate_open(AccountId::new([0x11; 32]), true),
                Err(SpelError::PdaMismatch { .. })
            ),
            "an address nobody derived has to be refused"
        );
    }

    #[test]
    fn an_owner_that_did_not_sign_is_refused_by_the_generated_validator() {
        // The dispatcher's half of the gate. `open_order` checks the flag too,
        // so a missing annotation here would not be an authorisation hole
        // today -- but it would be a silent change to the IDL's `signer`
        // metadata, which is what a client builds a transaction from.
        assert!(
            matches!(
                validate_open(order_account_id(), false),
                Err(SpelError::Unauthorized { .. })
            ),
            "an unsigned owner has to be refused"
        );
    }
}
