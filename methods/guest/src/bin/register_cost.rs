//! What registering a feed costs (M2-18, P2's registration clause).
//!
//! P2 asks for the push side's "account write **and registration**", and the
//! write half is `submit_cost.rs`. This is the other half, and it is a separate
//! binary rather than another stage there for a reason worth stating: a cycle
//! count is a property of a guest ELF, so adding stages to `submit_cost.rs` moves
//! every figure it publishes by a little. Keeping registration here means the
//! published write figures do not have to be re-pinned when this changes, and a
//! sibling binary provably does not perturb them — `AGGREGATOR_ID`,
//! `VERIFY_COST_ID` and `VERIFIER_LINK_ID` were all unchanged when
//! `submit_cost.rs` was added.
//!
//! # The stages
//!
//! | stage | adds |
//! | --- | --- |
//! | 0 | nothing: input, setup, the four accounts built, journal |
//! | 1 | [`authorise`] and then the real [`register_feed`], as the handler runs them |
//!
//! So `1 - 0` is a registration. There is no prefix chain because there is
//! nothing to take apart: registration does no cryptography, and what it costs is
//! Borsh, a PDA derivation and a handful of comparisons.
//!
//! # What this is not
//!
//! The generated validator and the dispatcher, on the same terms as
//! `submit_cost.rs`: the validator checks the accounts before the handler body
//! runs and the dispatcher wraps the result afterwards, and neither is inside
//! this figure. `COSTS.md` says so where it publishes it.
//!
//! What *is* inside it, after #60, is the authority gate: the handler calls
//! `admin::authorise` before delegating, so a figure that skipped it was the cost
//! of the helper and not of registering a feed.
//!
//! Registration is measured on a **first** registration — the feed account fully
//! default, which is the state that claims it. A re-registration after a
//! deregistration takes a different branch, and is not measured: it happens once
//! per retired feed and never on an operating path.
//!
//! # Panics
//!
//! None, for the reason the other cost guests give: a measurement that aborts is
//! not a measurement. Failure reports through the journal and the host asserts on
//! what it finds there.

use aggregator_program::{admin::authorise, register_feed};
use nssa_core::{
    account::{Account, AccountId, AccountWithMetadata, Data, Nonce},
    program::ProgramId,
};
use risc0_zkvm::guest::env;

/// Post-states out, which is how the host knows the registration happened rather
/// than refused: a refusal returns an error and no post-states, and would report
/// as an implausibly cheap registration.
type Report = (u32, u32);

fn main() {
    #[allow(clippy::type_complexity)]
    let (
        stage,
        admin_data,
        admin_key,
        feed_id,
        base_asset,
        quote_asset,
        signer_bytes,
        threshold,
        decimals,
        max_age_ms,
        program_id,
    ): (
        u8,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        u8,
        u8,
        u64,
        [u32; 8],
    ) = env::read();

    // Setup, identical in both stages.
    let ours = ProgramId::from(program_id);
    let signers: Vec<[u8; 20]> = signer_bytes
        .chunks_exact(20)
        .filter_map(|chunk| chunk.try_into().ok())
        .collect();

    let feed_id_array = to_32(&feed_id);
    let base = to_32(&base_asset);
    let quote = to_32(&quote_asset);

    // A first registration: the feed account is fully default, which is the state
    // that gets claimed.
    let feed = AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: AccountId::new([0x11; 32]),
    };
    let admin = owned(ours, Vec::new(), to_32(&admin_key), true);
    let config = owned(ours, admin_data, [0x33; 32], false);
    let price_account = AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: AccountId::new([0x22; 32]),
    };

    let report: Report = if stage == 0 {
        (0, 0)
    } else if authorise(&config, &admin, ours).is_err() {
        // The gate the generated handler runs before the delegated helper. It was
        // missing from the first version of this guest, which made 3,821 the cost
        // of the helper rather than of a registration -- @frenzox found that on
        // #60. Reported as a refusal so a broken gate cannot pass as a cheap
        // registration.
        (0, 0)
    } else {
        match register_feed(
            feed,
            admin,
            config,
            price_account,
            feed_id_array,
            base,
            quote,
            decimals,
            max_age_ms,
            signers,
            threshold,
            ours,
        ) {
            Ok(post_states) => (1, post_states.len() as u32),
            Err(_) => (0, 0),
        }
    };

    env::commit(&report);
}

fn to_32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = bytes.len().min(32);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn owned(
    owner: ProgramId,
    data: Vec<u8>,
    id: [u8; 32],
    is_authorized: bool,
) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance: 0,
            data: Data::try_from(data).unwrap_or_default(),
            nonce: Nonce(0),
        },
        is_authorized,
        account_id: AccountId::new(id),
    }
}
