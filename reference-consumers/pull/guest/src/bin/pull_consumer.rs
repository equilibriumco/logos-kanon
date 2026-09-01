//! The SPEL program: reference consumer B's entry point, as LEZ executes it.
//!
//! Eight instructions, all thin. The logic is in `reference-consumer-pull`, which
//! the host workspace can test; this file is the seam between that and LEZ, and
//! the seam is where the two things `pull-lib` cannot check are settled.
//!
//! **The clock's bytes are bound to the account they came from.** `clock` in
//! `settle` is one `AccountWithMetadata` the dispatcher supplied, and the logic
//! reads its id and its data out of that one struct. A consumer that took a
//! timestamp as an argument, or paired the pinned account id with bytes of its
//! own, would pass every check in `pull-lib` and be lying to its own users about
//! how fresh a price is.
//!
//! **Nothing a caller sends reaches the signer set.** The roster lives in a trust
//! account this program owns, and the two instructions that write one stand
//! behind the authority gate. `settle` takes a payload and an order; it writes
//! neither the trust account nor the config.
//!
//! Accounts are declared subject first: the order on the two order instructions,
//! the trust account on the two that change a feed, the config on the three that
//! change the authority. The rule is worth stating because the order is
//! positional on the wire and a caller building several has otherwise to
//! remember which.

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
/// KANON_PULL_GENESIS_AUTHORITY=<64 hex characters> cargo build
/// ```
///
/// It recurs per build and not per deployment: a program's id is its image id, so
/// a rebuild from unchanged inputs lands on the same config account while a build
/// with a changed input presents a fresh one. The aggregator's `[M2-06:01]` works
/// through the same mechanism.
const GENESIS_AUTHORITY: [u8; 32] = match option_env!("KANON_PULL_GENESIS_AUTHORITY") {
    Some(hex) => genesis_from_hex(hex),
    None => [0u8; 32],
};

/// Decodes the genesis key at compile time, so a malformed one is a build failure
/// rather than a deployment that cannot be established.
const fn genesis_from_hex(hex: &str) -> [u8; 32] {
    let bytes = hex.as_bytes();
    assert!(
        bytes.len() == 64,
        "KANON_PULL_GENESIS_AUTHORITY must be 64 hex characters"
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
        _ => panic!("KANON_PULL_GENESIS_AUTHORITY is not hexadecimal"),
    }
}

#[lez_program]
mod kanon_pull_consumer {
    #[allow(unused_imports)]
    use super::*;

    /// Establishes this program's authority from the key it was built with.
    ///
    /// Expected accounts:
    /// 1. `config` — the authority's account, at
    ///    `for_public_pda(program, sha256(zero_pad_32("KANON_PULL_CONFIG")))`.
    ///    `mut` and not `init`: `init` emits its own `AccountAlreadyInitialized`
    ///    in the dispatcher, which would hide the difference between an
    ///    authority already established and an address somebody squatted, and
    ///    those are two different pieces of advice.
    /// 2. `authority` — the signer claiming to be this build's genesis key.
    #[instruction]
    pub fn establish_authority(
        ctx: ProgramContext,
        // `r#const` and not `const`: the seed is parsed as a `syn::Expr` and a
        // bare keyword is not one, so the documented spelling fails to parse
        // before the seed parser sees it.
        #[account(mut, pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::authority::establish(
            config,
            authority,
            &GENESIS_AUTHORITY,
            ctx.self_program_id,
        )?;
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
        #[account(mut, pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        nominee: [u8; 32],
    ) -> SpelResult {
        let post_states = reference_consumer_pull::authority::nominate(
            config,
            authority,
            nominee,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Takes the authority over, as the nominee.
    ///
    /// Two steps rather than one so a mistyped nomination costs a second
    /// nomination rather than the program's whole administrative surface.
    ///
    /// Expected accounts:
    /// 1. `config` — the authority's account.
    /// 2. `nominee` — the nominated key, signing.
    #[instruction]
    pub fn accept_authority(
        ctx: ProgramContext,
        #[account(mut, pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        #[account(signer)] nominee: AccountWithMetadata,
    ) -> SpelResult {
        let post_states =
            reference_consumer_pull::authority::accept(config, nominee, ctx.self_program_id)?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Registers what this consumer trusts for one feed.
    ///
    /// Expected accounts:
    /// 1. `trust` — the feed's trust account, at
    ///    `for_public_pda(program, sha256(feed_id || zero_pad_32("KANON_PULL_TRUST")))`.
    ///    Declared rather than checked in code, because the derivation is what a
    ///    client has to reproduce and the constraint is what publishes it in the
    ///    IDL.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[expect(
        clippy::too_many_arguments,
        reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
    )]
    #[instruction]
    pub fn register_feed_trust(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_PULL_TRUST")])]
        trust: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        data_service_id: String,
        feed_id: [u8; 32],
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        decimals: u8,
        max_age_ms: u64,
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::trust::register(
            trust,
            authority,
            config,
            data_service_id,
            feed_id,
            base_asset,
            quote_asset,
            decimals,
            max_age_ms,
            signers,
            threshold,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Replaces one feed's signer set and threshold, together.
    ///
    /// The instruction RedStone's rotation schedule is the reason for. A rotation
    /// names its feed as well as addressing it, so an operator rotating five
    /// feeds cannot move the wrong one by transposing two accounts.
    ///
    /// Expected accounts:
    /// 1. `trust` — the feed's trust account.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[instruction]
    pub fn rotate_signers(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_PULL_TRUST")])]
        trust: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::trust::rotate_signers(
            trust,
            authority,
            config,
            feed_id,
            signers,
            threshold,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Retires what this consumer trusts for one feed.
    ///
    /// The recovery path for a registration that was wrong. Only the roster and
    /// its threshold rotate, so a mistyped pair, exponent or window has no
    /// in-place correction — and without this the feed id would be spent for the
    /// life of the build, because its address derives from the id and LEZ never
    /// releases ownership.
    ///
    /// Orders against a retired feed cannot settle until it is registered again,
    /// and if it returns under a different pair they never can. That is the
    /// intended outcome and the reason this is the authority's instruction.
    ///
    /// Expected accounts:
    /// 1. `trust` — the feed's trust account.
    /// 2. `authority` — the signer claiming to be the authority.
    /// 3. `config` — the account holding the authority it is checked against.
    #[instruction]
    pub fn deregister_feed_trust(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("feed_id"), r#const("KANON_PULL_TRUST")])]
        trust: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        #[account(pda = [r#const("KANON_PULL_CONFIG")])] config: AccountWithMetadata,
        feed_id: [u8; 32],
    ) -> SpelResult {
        let post_states = reference_consumer_pull::trust::deregister(
            trust,
            authority,
            config,
            feed_id,
            ctx.self_program_id,
        )?;
        Ok(spel_framework::SpelOutput::execute(post_states, vec![]))
    }

    /// Opens an order at the price its owner is prepared to trade at.
    ///
    /// Expected accounts:
    /// 1. `order` — the order's account, at
    ///    `for_public_pda(program, sha256(order_id || zero_pad_32("KANON_PULL_ORDER")))`.
    /// 2. `owner` — the signer the order belongs to.
    /// 3. `trust` — the feed's trust account, read and never written. An order
    ///    against a feed nobody registered could never settle, so the
    ///    registration is required at the open where the owner can still act on
    ///    it.
    ///
    /// `base_asset` and `quote_asset` are the owner's own expectation, not a
    /// convenience. Nothing on the wire says which assets a feed prices, so a
    /// registration that labelled RedStone's `BTC` feed as ETH/USD would verify
    /// real BTC packages and fill an order its owner believed was for ether. The
    /// owner names the pair it is signing for, this refuses the order if the
    /// registration disagrees, and the order carries the pair afterwards so a
    /// settlement compares two independently written records rather than one
    /// against itself.
    #[instruction]
    pub fn open_order(
        ctx: ProgramContext,
        #[account(mut, pda = [arg("order_id"), r#const("KANON_PULL_ORDER")])]
        order: AccountWithMetadata,
        #[account(signer)] owner: AccountWithMetadata,
        #[account(pda = [arg("feed_id"), r#const("KANON_PULL_TRUST")])] trust: AccountWithMetadata,
        order_id: [u8; 32],
        feed_id: [u8; 32],
        base_asset: [u8; 32],
        quote_asset: [u8; 32],
        limit_price_q64: u128,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::open_order(
            order,
            owner,
            trust,
            feed_id,
            base_asset,
            quote_asset,
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
    /// from the signatures in `payload`, checked against the roster `trust`
    /// holds.
    ///
    /// Expected accounts:
    /// 1. `order` — the order to fill. Owned by this program and holding an
    ///    order, both checked in `settle`: an account a caller owns would decode
    ///    as an order with a limit price of the caller's choosing. Checked there
    ///    rather than by an `owner` constraint because the IDL generator parses
    ///    `owner` and discards it, so a constraint no client can see is worse
    ///    than one that carries its own error.
    ///
    ///    No `pda` constraint, unlike `open_order`. It would buy nothing: the
    ///    order's own contents decide which feed is verified, and an account at
    ///    an underived address fails the ownership check. The derivation is
    ///    published on `open_order`, which is where a client needs it.
    /// 2. `trust` — what this program trusts for `feed_id`. Read and never
    ///    written, and constrained to the named feed's address so a caller cannot
    ///    substitute another feed's roster.
    /// 3. `clock` — the LEZ clock program's every-block account, read-only, and
    ///    the only admissible source of "now" (ADR 13). Passed whole, which is
    ///    what binds its bytes to its id.
    #[instruction]
    pub fn settle(
        ctx: ProgramContext,
        #[account(mut)] order: AccountWithMetadata,
        #[account(pda = [arg("feed_id"), r#const("KANON_PULL_TRUST")])] trust: AccountWithMetadata,
        clock: AccountWithMetadata,
        feed_id: [u8; 32],
        payload: Vec<u8>,
    ) -> SpelResult {
        let post_states = reference_consumer_pull::settle(
            order,
            trust,
            clock,
            feed_id,
            &payload,
            ctx.self_program_id,
        )?;
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
    use reference_consumer_pull::{CONFIG_ACCOUNT_SEED, ORDER_ACCOUNT_SEED, TRUST_ACCOUNT_SEED};
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

    /// A generated validator takes only the arguments its own constraints name,
    /// so a `feed_id` seed appears and a signer list does not.
    ///
    /// The addresses the constraints should accept, derived the way a client
    /// would. Each hashes the seed constant the logic crate exports, while the
    /// constraint hashes an `r#const` literal in this file — so these are also
    /// the assertion that the two spellings agree.
    fn config_id() -> AccountId {
        compute_pda(&OURS, &[&seed_from_str(CONFIG_ACCOUNT_SEED)])
    }

    fn trust_id() -> AccountId {
        compute_pda(&OURS, &[&FEED_ID, &seed_from_str(TRUST_ACCOUNT_SEED)])
    }

    fn order_id() -> AccountId {
        compute_pda(&OURS, &[&ORDER_ID, &seed_from_str(ORDER_ACCOUNT_SEED)])
    }

    #[test]
    fn the_declared_accounts_pass_every_generated_validator() {
        let empty: InstructionData = Vec::new();

        super::kanon_pull_consumer::__validate_establish_authority(
            &[account(config_id(), false), account(id([0xA1; 32]), true)],
            &OURS,
            &empty,
        )
        .expect("the config derivation is the declared one");

        super::kanon_pull_consumer::__validate_rotate_signers(
            &[
                account(trust_id(), false),
                account(id([0xA1; 32]), true),
                account(config_id(), false),
            ],
            &OURS,
            &empty,
            &FEED_ID,
        )
        .expect("the trust derivation is the declared one");

        super::kanon_pull_consumer::__validate_open_order(
            &[
                account(order_id(), false),
                account(id([0xA0; 32]), true),
                account(trust_id(), false),
            ],
            &OURS,
            &empty,
            &ORDER_ID,
            &FEED_ID,
        )
        .expect("the order derivation is the declared one");

        super::kanon_pull_consumer::__validate_settle(
            &[
                account(order_id(), false),
                account(trust_id(), false),
                account(id(*b"/LEZ/ClockProgramAccount/0000001"), false),
            ],
            &OURS,
            &empty,
            &FEED_ID,
        )
        .expect("settle's trust constraint accepts the named feed's address");
    }

    #[test]
    fn an_unconfigured_build_refuses_to_establish_an_authority() {
        // What this covers that the host suite cannot: that the handler reads
        // `GENESIS_AUTHORITY` rather than a genesis of its own, and that a build
        // nobody configured refuses everyone instead of accepting the first
        // caller. The host tests pass a genesis in as an argument, so only here is
        // the constant itself -- and `genesis_from_hex` and `nibble` beneath it --
        // on a test path at all.
        //
        // It asserts the unconfigured case because that is the one CI can build. A
        // configured build is a deployment concern, and `[M3-06:02]` records that
        // the key is a build input. The aggregator's
        // `an_unconfigured_build_refuses_to_establish_an_authority` is the same
        // test for the same reason; this is the half of that pairing the consumer
        // was missing.
        assert_eq!(
            super::GENESIS_AUTHORITY,
            [0u8; 32],
            "CI builds carry no genesis key"
        );

        let config = AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: config_id(),
        };
        let mut authority = account(id([0xA1; 32]), true);
        authority.account.program_owner = [42u32; 8];

        let ctx = super::ProgramContext::new(OURS, [0u32; 8]);
        let refused = super::kanon_pull_consumer::establish_authority(ctx, config, authority)
            .expect_err("a build with no genesis authority must refuse");

        // By code rather than by variant: `SpelError` carries no `PartialEq`, and
        // the code is the part a caller acts on.
        assert_eq!(
            refused.error_code(),
            SpelError::from(reference_consumer_pull::AuthorityError::NoGenesisAuthority)
                .error_code()
        );
    }

    #[test]
    fn the_genesis_key_is_decoded_from_its_hex_exactly() {
        // The test above asserts what an unconfigured build does, and that is all
        // it can assert: `option_env!` takes the `None` branch in CI, so
        // `genesis_from_hex` and `nibble` are never called and the constant is
        // `[0; 32]` either way. Measured -- breaking `nibble`'s `a..f` arm, or
        // replacing `&GENESIS_AUTHORITY` at the call site with a literal
        // `&[0u8; 32]`, leaves that test green.
        //
        // So the decode is exercised directly. It is the half of a build input
        // that a build can get wrong silently: a mis-decoded key produces a
        // deployment whose authority nobody holds, on the one instruction that
        // cannot be retried.
        const KEY: [u8; 32] = super::genesis_from_hex(
            "0123456789abcdefABCDEF0000000000000000000000000000000000000000ff",
        );

        assert_eq!(KEY[0], 0x01, "0 and 1");
        assert_eq!(KEY[1], 0x23);
        assert_eq!(KEY[4], 0x89, "the digit and letter boundary");
        assert_eq!(KEY[5], 0xab, "lower-case a-f");
        assert_eq!(KEY[7], 0xef);
        assert_eq!(KEY[8], 0xAB, "upper-case A-F decodes the same");
        assert_eq!(KEY[10], 0xEF);
        assert_eq!(KEY[31], 0xff, "the last byte, so the walk covers the width");

        // Every nibble arm, so a shifted offset in one of them is caught rather
        // than averaged away by the spot checks above.
        const ALL_ZERO: [u8; 32] = super::genesis_from_hex(
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert_eq!(ALL_ZERO, [0u8; 32]);
    }

    #[test]
    fn a_trust_account_for_another_feed_is_refused_by_settles_validator() {
        // The dispatcher's half of the rule the body also checks. Without the
        // constraint a caller could hand `settle` any trust account this program
        // owns -- another feed's roster, with another feed's threshold -- and the
        // body's own comparison would be the only thing standing in the way.
        let empty: InstructionData = Vec::new();
        let other = compute_pda(&OURS, &[&[0xAB; 32], &seed_from_str(TRUST_ACCOUNT_SEED)]);
        assert!(
            matches!(
                super::kanon_pull_consumer::__validate_settle(
                    &[
                        account(order_id(), false),
                        account(other, false),
                        account(id(*b"/LEZ/ClockProgramAccount/0000001"), false),
                    ],
                    &OURS,
                    &empty,
                    &FEED_ID,
                ),
                Err(SpelError::PdaMismatch { .. })
            ),
            "another feed's trust account has to be refused"
        );
    }

    #[test]
    fn an_account_at_an_underived_address_is_refused_by_the_generated_validator() {
        let empty: InstructionData = Vec::new();
        assert!(matches!(
            super::kanon_pull_consumer::__validate_establish_authority(
                &[
                    account(id([0x11; 32]), false),
                    account(id([0xA1; 32]), true)
                ],
                &OURS,
                &empty,
            ),
            Err(SpelError::PdaMismatch { .. })
        ));
        assert!(matches!(
            super::kanon_pull_consumer::__validate_open_order(
                &[
                    account(id([0x11; 32]), false),
                    account(id([0xA0; 32]), true),
                    account(trust_id(), false),
                ],
                &OURS,
                &empty,
                &ORDER_ID,
                &FEED_ID,
            ),
            Err(SpelError::PdaMismatch { .. })
        ));
    }

    #[test]
    fn every_instruction_that_moves_what_the_program_trusts_refuses_an_unsigned_caller() {
        // Six handlers, and `#[account(signer)]` is repeated by hand on each, so
        // a missing annotation is a per-handler mistake and testing one proves
        // nothing about the other five. The logic checks the flag too, which is
        // why a gap would not be an authorisation hole today -- but it would be an
        // unnoticed change to the IDL's `signer` metadata, which is what a client
        // builds a transaction from.
        let empty: InstructionData = Vec::new();
        let unsigned = account(id([0xA1; 32]), false);

        let outcomes = [
            super::kanon_pull_consumer::__validate_establish_authority(
                &[account(config_id(), false), unsigned.clone()],
                &OURS,
                &empty,
            ),
            super::kanon_pull_consumer::__validate_nominate_authority(
                &[account(config_id(), false), unsigned.clone()],
                &OURS,
                &empty,
            ),
            super::kanon_pull_consumer::__validate_accept_authority(
                &[account(config_id(), false), unsigned.clone()],
                &OURS,
                &empty,
            ),
            super::kanon_pull_consumer::__validate_register_feed_trust(
                &[
                    account(trust_id(), false),
                    unsigned.clone(),
                    account(config_id(), false),
                ],
                &OURS,
                &empty,
                &FEED_ID,
            ),
            super::kanon_pull_consumer::__validate_rotate_signers(
                &[
                    account(trust_id(), false),
                    unsigned.clone(),
                    account(config_id(), false),
                ],
                &OURS,
                &empty,
                &FEED_ID,
            ),
            super::kanon_pull_consumer::__validate_deregister_feed_trust(
                &[
                    account(trust_id(), false),
                    unsigned,
                    account(config_id(), false),
                ],
                &OURS,
                &empty,
                &FEED_ID,
            ),
        ];

        for outcome in outcomes {
            assert!(
                matches!(outcome, Err(SpelError::Unauthorized { .. })),
                "an unsigned caller has to be refused, got {outcome:?}"
            );
        }
    }
}
