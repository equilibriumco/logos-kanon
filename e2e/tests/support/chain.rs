//! The chain plumbing both end-to-end suites need, and nothing about either
//! program.
//!
//! Nothing ships from here. The crate exists so `tests/` has a target to hang
//! off — `Cargo.toml` records why the workspace is its own — and this module is
//! what stops the second suite copying the first's transport. What is generic is
//! here; what a program means by an instruction stays in its own test file.
//!
//! The rule every helper is shaped by: **assert on account state read back from
//! the chain, never on what `sendTransaction` returned.** `Ok` from that call
//! means the transaction was accepted into the mempool, not that it executed. A
//! failing transaction still returns a hash, leaves every account untouched, and
//! reports its reason only in `target/lez-sequencer/sequencer.log`. That is why
//! [`account_when`] exists and why nothing here returns a transaction hash.

#![allow(dead_code)] // Each suite drives a different subset of the chain.

use kanon_clock::{LezClock, CLOCK_ACCOUNT_ID};
use lee_core::account::{Account, Data};
use lee_core::program::ProgramId;
use sequencer_service_rpc::{RpcClient as _, SequencerClient, SequencerClientBuilder};
use serde::Serialize;
use verifier_core::time::TimeSource as _;

/// Where the harness puts it. `KANON_SEQUENCER_PORT` overrides, as the script does.
pub fn sequencer_url() -> String {
    let port = std::env::var("KANON_SEQUENCER_PORT").unwrap_or_else(|_| "3055".to_owned());
    format!("http://127.0.0.1:{port}")
}

pub fn client() -> SequencerClient {
    SequencerClientBuilder::default()
        .build(sequencer_url())
        .expect("a client for the sequencer URL")
}

/// How long to wait for a transaction to show up in state.
///
/// The sequencer collects the mempool on an interval — about fifteen seconds
/// against this harness — so a submission takes a block or two to appear. Sixty
/// seconds is several of those.
pub const SETTLE_ATTEMPTS: u32 = 60;

/// Polls `id` until `read` answers, and fails naming the log if it never does.
///
/// This is the shape every step takes, and the reason is the one in the module
/// docs: a transaction that failed is indistinguishable from one still in the
/// mempool by looking at the send. Only state answers, and only the sequencer's
/// log says why it never moved.
pub async fn account_when<T>(
    client: &SequencerClient,
    id: lee::AccountId,
    what: &str,
    read: impl Fn(&Account) -> Option<T>,
) -> T {
    for _ in 0..SETTLE_ATTEMPTS {
        let account = client
            .get_account(id)
            .await
            .expect("the sequencer answers getAccount");
        if let Some(value) = read(&account) {
            return value;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    panic!(
        "{what}: state never moved within {SETTLE_ATTEMPTS}s. The transaction was accepted -- \
         `sendTransaction` returning a hash says only that -- so the reason it did not execute is \
         in target/lez-sequencer/sequencer.log"
    );
}

/// One read, with no waiting.
pub async fn account_now(client: &SequencerClient, id: lee::AccountId) -> Account {
    client
        .get_account(id)
        .await
        .expect("the sequencer answers getAccount")
}

pub fn decode<T: borsh::BorshDeserialize>(data: &Data) -> Option<T> {
    borsh::from_slice(data.as_ref()).ok()
}

/// Sends an instruction to `program`, signed by `key`.
///
/// Generic over the instruction type because each program declares its own, and
/// both declare it in a library precisely so a caller like this one can name it.
///
/// Nonces are fetched per send and are one per **signer**, not one per account:
/// a nonce for every account gets `InvalidInput("Mismatch between number of
/// nonces and signatures/public keys")` out of the block builder, which -- being
/// the block builder -- is invisible at the send.
pub async fn send_to_program<I: Serialize>(
    client: &SequencerClient,
    program: ProgramId,
    accounts: Vec<lee::AccountId>,
    key: &lee::PrivateKey,
    signer: lee::AccountId,
    instruction: &I,
) {
    let nonces = client
        .get_accounts_nonces(vec![signer])
        .await
        .expect("the sequencer answers getAccountsNonces");

    let message = lee::public_transaction::Message::try_new(program, accounts, nonces, instruction)
        .expect("the instruction encodes");
    let witness = lee::public_transaction::WitnessSet::for_message(&message, &[key]);

    client
        .send_transaction(common::transaction::LeeTransaction::Public(
            lee::PublicTransaction::new(message, witness),
        ))
        .await
        .expect("the sequencer accepts the transaction");
}

/// The chain's own time, from the account a program is required to read.
pub async fn chain_now_ms(client: &SequencerClient) -> u64 {
    let clock = account_now(client, lee::AccountId::new(CLOCK_ACCOUNT_ID)).await;
    LezClock::from_account(&CLOCK_ACCOUNT_ID, clock.data.as_ref())
        .expect("the pinned clock account decodes")
        .now_ms()
        .expect("the clock has a time")
}

/// The chain's clock, once it is strictly past `round`.
///
/// Waited for rather than dated forward: a package timestamped ahead of the
/// chain would be testing the forward tolerance instead of the update.
pub async fn chain_now_past(client: &SequencerClient, round: u64) -> u64 {
    for _ in 0..SETTLE_ATTEMPTS {
        let now = chain_now_ms(client).await;
        if now > round {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("the chain clock never passed {round}, so no newer round was available");
}

/// Deploys `bytecode` and returns `id`, the image id it was built with.
///
/// Unconditional, because a deployment cannot be read back: `getProgramIds`
/// answers with the sequencer's built-ins only. On a clean chain it lands. On a
/// chain that already has this program — a second local run — the same bytecode
/// gives the same transaction hash and the sequencer refuses it with a
/// `failed execution check` line in its log. That line is expected there and
/// nothing here depends on the send: what establishes the program is deployed is
/// that the instructions after it execute.
pub async fn deploy_guest(client: &SequencerClient, bytecode: &[u8], id: ProgramId) -> ProgramId {
    assert!(
        !bytecode.is_empty(),
        "the guest ELF is empty, which means RISC0_SKIP_BUILD stubbed it out -- \
         this test needs a real build"
    );

    client
        .send_transaction(common::transaction::LeeTransaction::ProgramDeployment(
            lee::ProgramDeploymentTransaction::new(
                lee::program_deployment_transaction::Message::new(bytecode.to_vec()),
            ),
        ))
        .await
        .expect("the sequencer accepts a program deployment");

    id
}

/// Claims `account` under `authenticated_transfer`, so a program owns it.
///
/// Both programs refuse a default-owned key as their authority -- LEZ strands
/// such an account after one transaction -- so the key has to be an account that
/// already exists on chain. `[M2-06:01]` states that as a deployment property;
/// this is what satisfies it on a fresh chain, and it is the check firing for
/// real rather than in a host fixture.
///
/// `authenticated_transfer::Instruction::Initialize` is a unit variant at index
/// 1 and claims a *default* account for itself, needing only that the account
/// signs. No funding and no genesis entitlement: the account claims itself. The
/// instruction is encoded by index rather than by depending on
/// `authenticated_transfer_core`, because one `u32` is a smaller thing to keep
/// true than a git dependency, and `getProgramIds` is what pins the program it
/// is sent to.
pub async fn ensure_the_account_is_owned(
    client: &SequencerClient,
    key: &lee::PrivateKey,
    account: lee::AccountId,
) {
    if account_now(client, account).await.program_owner != ProgramId::default() {
        return;
    }

    let program = *client
        .get_program_ids()
        .await
        .expect("the sequencer answers getProgramIds")
        .get("authenticated_transfer")
        .expect("the sequencer has authenticated_transfer built in");

    let nonces = client
        .get_accounts_nonces(vec![account])
        .await
        .expect("nonces");
    let message = lee::public_transaction::Message::new_preserialized(
        program,
        vec![account],
        nonces,
        vec![1u32],
    );
    let witness = lee::public_transaction::WitnessSet::for_message(&message, &[key]);
    client
        .send_transaction(common::transaction::LeeTransaction::Public(
            lee::PublicTransaction::new(message, witness),
        ))
        .await
        .expect("the sequencer accepts the claim");

    account_when(client, account, "the key account is claimed", |account| {
        (account.program_owner != ProgramId::default()).then_some(())
    })
    .await;
}
