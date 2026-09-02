//! The push path across a real standalone LEZ sequencer (M2-19).
//!
//! `scripts/lez-sequencer.sh` stands one up; the CI job of the same name runs it
//! and this alongside. Locally, `start` it first — with nothing listening these
//! tests fail rather than skip, because a skipped end-to-end test reports the
//! same green as a passing one and is the reason this task exists at all.
//!
//! # What this adds over the host suite
//!
//! The host tests call the pure functions with accounts they built themselves.
//! They are the ones that cover the failure modes, and they cover them better
//! than a chain can: every rejection is reachable by construction. What they
//! cannot do is establish that LEZ agrees — that the post-states the program
//! returns are ones the state machine accepts, that the addresses the generated
//! validators check are the ones a caller derives, and that an instruction the
//! host accepts is one the block builder executes rather than drops.
//!
//! So the assertion here is always on **account state read back from the chain**,
//! never on what `sendTransaction` returned. `Ok` from that call means the
//! transaction was accepted into the mempool, not that it executed: a failing
//! transaction still returns a hash, leaves every account untouched, and reports
//! its reason only in `target/lez-sequencer/sequencer.log`. A test asserting the
//! send is inert, and an inert end-to-end test is worse than none.
//!
//! # Re-running against a chain that already has state
//!
//! `lez-sequencer.sh start` gives a clean chain, which is what CI gets. Locally a
//! second run meets the accounts the first one created, so each setup step here
//! is conditional on the state it would create — `initialise_admin` and the
//! account claim each succeed exactly once. The final assertion is not
//! conditional on anything: every run signs a payload timestamped at the chain's
//! own clock, which is strictly newer than whatever is stored, so every run has
//! to move the price account or fail.

use aggregator_program::{
    admin::{AdminAccount, ADMIN_CONFIG_SEED},
    feed_account::FeedAccount,
    kanon_idl::{self, OraclePriceAccount},
    publish::REDSTONE_SOURCE_ID,
    register::FEED_ACCOUNT_SEED,
    submit::PRICE_ACCOUNT_SEED,
    Instruction,
};
use kanon_clock::{LezClock, CLOCK_ACCOUNT_ID};
use lee_core::{
    account::{Account, Data},
    program::ProgramId,
};
use sequencer_service_rpc::{RpcClient as _, SequencerClient, SequencerClientBuilder};
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::{
    test_support::{address_of, signing_key, PayloadBuilder},
    time::TimeSource as _,
};

/// Where the harness puts it. `KANON_SEQUENCER_PORT` overrides, as the script does.
fn sequencer_url() -> String {
    let port = std::env::var("KANON_SEQUENCER_PORT").unwrap_or_else(|_| "3055".to_owned());
    format!("http://127.0.0.1:{port}")
}

fn client() -> SequencerClient {
    SequencerClientBuilder::default()
        .build(sequencer_url())
        .expect("a client for the sequencer URL")
}

/// The admin key this end-to-end test holds.
///
/// Fixed rather than random, because the guest compiles the *account id* of this
/// key in as `KANON_GENESIS_ADMIN` and a fresh key each run would mean a fresh
/// image id and a rebuilt guest each run. Worthless: it signs for one account on
/// a throwaway local sequencer and controls nothing anywhere else.
const TEST_ADMIN_KEY: [u8; 32] = [0x11; 32];

/// The account id the guest must carry as `KANON_GENESIS_ADMIN`.
///
/// Written out because it has to appear in a CI job's `env` and on a command
/// line, neither of which can call a function. `require_a_configured_build`
/// derives it from [`TEST_ADMIN_KEY`] and refuses to run if the two disagree,
/// so it is a cache of that derivation rather than a second source of truth.
const EXPECTED_GENESIS: &str = "5b0e4f6dddea8b9ef3118d6002a25c09ab379653ffb60586f4604f1fa6a0b392";

fn test_admin() -> (lee::PrivateKey, lee::AccountId) {
    let key = lee::PrivateKey::try_new(TEST_ADMIN_KEY).expect("a valid secp256k1 key");
    let id = lee::AccountId::from(&lee::PublicKey::new_from_private_key(&key));
    (key, id)
}

/// Fails the test rather than letting it fail obscurely downstream.
///
/// The guest compiles the genesis key in, so the *build* has to carry it -- the
/// test cannot set it for itself. Read through `option_env!` here because this
/// file is compiled by the same cargo invocation as the guest, so it sees the
/// same environment the build did.
fn require_a_configured_build() {
    // Derived first, because the constant is a copied string and the env var can
    // agree with a wrong one. That case is not hypothetical in its consequences:
    // the guest would carry a genesis authority nobody here holds, every
    // administrative instruction would be refused, and the first symptom would be
    // `the authority is established` timing out sixty seconds later -- exactly the
    // unclear downstream failure this function exists to pre-empt. @frenzox
    // raised it on #58.
    let (_, admin_id) = test_admin();
    let derived = hex::encode(admin_id.value());
    assert_eq!(
        EXPECTED_GENESIS, derived,
        "EXPECTED_GENESIS is not the account id of TEST_ADMIN_KEY. One of the two \
         was edited without the other; the derived value is the correct one, and it \
         also has to be updated in the `sequencer` CI job's KANON_GENESIS_ADMIN"
    );

    let configured = option_env!("KANON_GENESIS_ADMIN").unwrap_or("");
    assert_eq!(
        configured, EXPECTED_GENESIS,
        "this test needs a guest built with the test genesis key. Run it as:\n    \
         KANON_GENESIS_ADMIN={EXPECTED_GENESIS} cargo test --manifest-path e2e/Cargo.toml"
    );
}

// ---------------------------------------------------------------------------
// The feed this test registers.
// ---------------------------------------------------------------------------

/// Three signers and a threshold of three, so every configured signer has to
/// report: the point here is the round trip, and a partially met threshold is
/// the host suite's ground.
const SIGNER_SEEDS: [u8; 3] = [1, 2, 3];
const THRESHOLD: u8 = 3;

const DECIMALS: u8 = 8;

/// Five minutes. Well inside `MAX_MAX_AGE_MS`, and wide enough that the payload
/// stays current across the fifteen seconds or so between the clock reading and
/// the block that executes the submission.
const MAX_AGE_MS: u64 = 300_000;

const BASE_ASSET: [u8; 32] = [0xB7; 32];
const QUOTE_ASSET: [u8; 32] = [0x05; 32];

/// `65_000.00000000` at eight decimals, as RedStone would scale it, and the
/// second round's `66_000.00000000`. Two different values because an update has
/// to be seen to move the price, and a second submission of the same number
/// would move only the timestamp.
const FIRST_VALUE: u64 = 6_500_000_000_000;
const SECOND_VALUE: u64 = 6_600_000_000_000;

fn feed_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..3].copy_from_slice(b"BTC");
    id
}

/// The payload a signer set would publish for `timestamp_ms`.
///
/// Full-width big-endian values, which is what RedStone puts on the wire.
/// `Value::from_be_slice` right-aligns a shorter one, so a narrow value is
/// readable but is not what a real package carries.
fn signed_payload(timestamp_ms: u64, value_scaled: u64) -> Vec<u8> {
    let mut value = [0u8; 32];
    value[24..].copy_from_slice(&value_scaled.to_be_bytes());

    let mut builder = PayloadBuilder::default();
    for seed in SIGNER_SEEDS {
        builder = builder.signed_package(&signing_key(seed), &[(b"BTC", &value)], timestamp_ms);
    }
    builder.build()
}

/// What `publish` must have written for a scaled value, computed here rather
/// than read from the crate under test.
///
/// `to_q64_64` is `value << 64 / 10^decimals`, and this is that arithmetic done
/// independently: asserting against the program's own conversion would only
/// establish that it agrees with itself.
fn expected_price(value_scaled: u64) -> u128 {
    (u128::from(value_scaled) << 64) / 10u128.pow(u32::from(DECIMALS))
}

// ---------------------------------------------------------------------------
// Reading the chain.
// ---------------------------------------------------------------------------

/// How long to wait for a transaction to show up in state.
///
/// The sequencer collects the mempool on an interval — about fifteen seconds
/// against this harness — so a submission takes a block or two to appear. Sixty
/// seconds is several of those.
const SETTLE_ATTEMPTS: u32 = 60;

/// Polls `id` until `read` answers, and fails naming the log if it never does.
///
/// This is the shape every step here takes, and the reason is the one in the
/// module docs: a transaction that failed is indistinguishable from one still in
/// the mempool by looking at the send. Only state answers, and only the
/// sequencer's log says why it never moved.
async fn account_when<T>(
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
async fn account_now(client: &SequencerClient, id: lee::AccountId) -> Account {
    client
        .get_account(id)
        .await
        .expect("the sequencer answers getAccount")
}

fn decode<T: borsh::BorshDeserialize>(data: &Data) -> Option<T> {
    borsh::from_slice(data.as_ref()).ok()
}

/// Sends an instruction to the aggregator, signed by `key`.
///
/// Nonces are fetched per send and are one per **signer**, not one per account:
/// a nonce for every account gets `InvalidInput("Mismatch between number of
/// nonces and signatures/public keys")` out of the block builder, which -- being
/// the block builder -- is invisible at the send.
async fn send_to_program(
    client: &SequencerClient,
    program: ProgramId,
    accounts: Vec<lee::AccountId>,
    key: &lee::PrivateKey,
    signer: lee::AccountId,
    instruction: &Instruction,
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

/// The chain's own time, from the account the program is required to read.
async fn chain_now_ms(client: &SequencerClient) -> u64 {
    let clock = account_now(client, lee::AccountId::new(CLOCK_ACCOUNT_ID)).await;
    LezClock::from_account(&CLOCK_ACCOUNT_ID, clock.data.as_ref())
        .expect("the pinned clock account decodes")
        .now_ms()
        .expect("the clock has a time")
}

// ---------------------------------------------------------------------------
// The steps.
// ---------------------------------------------------------------------------

/// Deploys this build's guest, and returns its program id.
///
/// Unconditional, because a deployment cannot be read back: `getProgramIds`
/// answers with the sequencer's built-ins only. On a clean chain it lands. On a
/// chain that already has this program — a second local run — the same bytecode
/// gives the same transaction hash and the sequencer refuses it with a
/// `failed execution check` line in its log. That line is expected there and
/// nothing here depends on the send: what establishes the program is deployed is
/// that the instructions after this one execute.
async fn deploy_the_guest(client: &SequencerClient) -> ProgramId {
    let bytecode = kanon_methods::AGGREGATOR_ELF.to_vec();
    assert!(
        !bytecode.is_empty(),
        "the guest ELF is empty, which means RISC0_SKIP_BUILD stubbed it out -- \
         this test needs a real build"
    );

    client
        .send_transaction(common::transaction::LeeTransaction::ProgramDeployment(
            lee::ProgramDeploymentTransaction::new(
                lee::program_deployment_transaction::Message::new(bytecode),
            ),
        ))
        .await
        .expect("the sequencer accepts a program deployment");

    kanon_methods::AGGREGATOR_ID
}

/// Claims `admin` under `authenticated_transfer`, so it is owned by a program.
///
/// `initialise_admin` refuses a default-owned key -- error 812, "owned by no
/// program, and LEZ strands such an account after one transaction" -- so the
/// authority has to be an account that already exists on chain. `[M2-06:01]`
/// states that as a deployment property; this is what satisfies it on a fresh
/// chain, and it is the check firing for real rather than in a host fixture.
///
/// `authenticated_transfer::Instruction::Initialize` is a unit variant at index
/// 1 and claims a *default* account for itself, needing only that the account
/// signs. No funding and no genesis entitlement: the account claims itself. The
/// instruction is encoded by index rather than by depending on
/// `authenticated_transfer_core`, because one `u32` is a smaller thing to keep
/// true than a git dependency, and `getProgramIds` is what pins the program it
/// is sent to.
async fn ensure_the_admin_account_is_owned(
    client: &SequencerClient,
    key: &lee::PrivateKey,
    admin_id: lee::AccountId,
) {
    if account_now(client, admin_id).await.program_owner != ProgramId::default() {
        return;
    }

    let program = *client
        .get_program_ids()
        .await
        .expect("the sequencer answers getProgramIds")
        .get("authenticated_transfer")
        .expect("the sequencer has authenticated_transfer built in");

    let nonces = client
        .get_accounts_nonces(vec![admin_id])
        .await
        .expect("nonces");
    let message = lee::public_transaction::Message::new_preserialized(
        program,
        vec![admin_id],
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

    account_when(
        client,
        admin_id,
        "the admin account is claimed",
        |account| (account.program_owner != ProgramId::default()).then_some(()),
    )
    .await;
}

/// Establishes the authority, and returns the config account's address.
async fn ensure_the_authority_is_established(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    admin_id: lee::AccountId,
) -> lee::AccountId {
    let config_id =
        lee::AccountId::new(*compute_pda(&program, &[&seed_from_str(ADMIN_CONFIG_SEED)]).value());

    let established = decode::<AdminAccount>(&account_now(client, config_id).await.data)
        .is_some_and(|config| config.admin == Some(*admin_id.value()));

    if !established {
        send_to_program(
            client,
            program,
            vec![config_id, admin_id],
            key,
            admin_id,
            &Instruction::InitialiseAdmin,
        )
        .await;
    }

    // Asserted whether or not this run wrote it: the point is the state, and a
    // run that skipped the write still has to find the authority there.
    let config: AdminAccount = account_when(
        client,
        config_id,
        "the authority is established",
        |account| decode(&account.data),
    )
    .await;

    assert_eq!(
        config,
        AdminAccount {
            admin: Some(*admin_id.value()),
            pending: None,
        },
        "the config account holds an authority other than the genesis key this build carries"
    );

    config_id
}

/// Registers the feed, and returns its account address.
async fn ensure_the_feed_is_registered(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    admin_id: lee::AccountId,
    config_id: lee::AccountId,
) -> lee::AccountId {
    let id = feed_id();
    let feed_account_id = lee::AccountId::new(
        *compute_pda(&program, &[&id, &seed_from_str(FEED_ACCOUNT_SEED)]).value(),
    );
    let price_account_id = price_account_id(program, feed_account_id);

    let signers: Vec<[u8; 20]> = SIGNER_SEEDS
        .iter()
        .map(|seed| address_of(&signing_key(*seed)).0)
        .collect();

    let expected = FeedAccount {
        feed_id: id,
        base_asset: BASE_ASSET,
        quote_asset: QUOTE_ASSET,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers: signers.clone(),
        threshold: THRESHOLD,
        paused: false,
    };

    let registered = decode::<FeedAccount>(&account_now(client, feed_account_id).await.data)
        .as_ref()
        == Some(&expected);

    if !registered {
        send_to_program(
            client,
            program,
            vec![feed_account_id, admin_id, config_id, price_account_id],
            key,
            admin_id,
            &Instruction::RegisterFeed {
                feed_id: id,
                base_asset: BASE_ASSET,
                quote_asset: QUOTE_ASSET,
                decimals: DECIMALS,
                max_age_ms: MAX_AGE_MS,
                signers,
                threshold: THRESHOLD,
            },
        )
        .await;
    }

    let stored: FeedAccount = account_when(
        client,
        feed_account_id,
        "the feed is registered",
        |account| decode(&account.data),
    )
    .await;

    assert_eq!(
        stored, expected,
        "the registered feed is not the configuration this test asked for"
    );

    // The feed account is the program's own, and a submission is required to
    // refuse one it does not own -- an account a caller owns would decode as a
    // feed with a signer set of the caller's choosing.
    assert_eq!(
        account_now(client, feed_account_id).await.program_owner,
        program,
        "the feed account is not owned by the aggregator, so the claim in the registration did not land"
    );

    feed_account_id
}

/// `for_public_pda(program, sha256(feed_account_id || zero_pad_32(seed)))`.
///
/// Derived here rather than read back from anywhere, because that is what a
/// caller has to do: the generated validator refuses any other address, and
/// getting the 32-byte seed padding wrong derives a different one.
fn price_account_id(program: ProgramId, feed_account_id: lee::AccountId) -> lee::AccountId {
    lee::AccountId::new(
        *compute_pda(
            &program,
            &[feed_account_id.value(), &seed_from_str(PRICE_ACCOUNT_SEED)],
        )
        .value(),
    )
}

/// Submits a payload signed for `round`, and returns the account it wrote.
///
/// Waits on `timestamp == round`, which is the part that makes this a real
/// observation rather than a re-read: no previous run can have written this
/// round, because it comes from the chain's clock during this one.
async fn submit_and_read_back(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    admin_id: lee::AccountId,
    feed_account_id: lee::AccountId,
    round: u64,
    value_scaled: u64,
) -> OraclePriceAccount {
    let price_id = price_account_id(program, feed_account_id);

    send_to_program(
        client,
        program,
        vec![
            feed_account_id,
            price_id,
            lee::AccountId::new(CLOCK_ACCOUNT_ID),
        ],
        key,
        admin_id,
        &Instruction::SubmitPrice {
            payload: signed_payload(round, value_scaled),
        },
    )
    .await;

    account_when(client, price_id, "the submission is published", |account| {
        decode::<OraclePriceAccount>(&account.data).filter(|price| price.timestamp == round)
    })
    .await
}

/// The chain's clock, once it is strictly past `round`.
///
/// Waited for rather than dated forward: a package timestamped ahead of the
/// chain would be testing the forward tolerance instead of the update.
async fn chain_now_past(client: &SequencerClient, round: u64) -> u64 {
    for _ in 0..SETTLE_ATTEMPTS {
        let now = chain_now_ms(client).await;
        if now > round {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("the chain clock never passed {round}, so no newer round was available");
}

// ---------------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_sequencer_is_there_and_producing_blocks() {
    // The precondition every other test here rests on, asserted first so a
    // failure downstream is about the adaptor rather than about the harness.
    let client = client();

    client
        .check_health()
        .await
        .expect("the sequencer answers checkHealth");

    let block = client
        .get_last_block_id()
        .await
        .expect("the sequencer answers getLastBlockId");
    assert!(
        block > 0,
        "the sequencer has produced no blocks, so nothing submitted here could land"
    );
}

/// M2's done-gate line: verify and publish, end to end, on a standalone sequencer.
///
/// One test rather than several, and that is not a stylistic choice: cargo runs
/// tests in a thread pool, the steps here are ordered, and they share one chain
/// and one signer. As separate tests they would interleave transactions from the
/// same account and take each other's nonces.
///
/// There is no separate "the guest deploys" test either, because it would be
/// inert -- `getProgramIds` answers with the sequencer's built-ins only, so a
/// deployment cannot be read back, and what establishes that it landed is that
/// the instructions below execute at all.
#[tokio::test]
async fn the_push_path_verifies_and_publishes_across_a_real_sequencer() {
    require_a_configured_build();
    let client = client();
    let (key, admin_id) = test_admin();

    let program = deploy_the_guest(&client).await;
    ensure_the_admin_account_is_owned(&client, &key, admin_id).await;
    let config_id = ensure_the_authority_is_established(&client, program, &key, admin_id).await;
    let feed_account_id =
        ensure_the_feed_is_registered(&client, program, &key, admin_id, config_id).await;
    let price_id = price_account_id(program, feed_account_id);

    // Timestamped at the chain's own clock, which is what makes the run
    // unconditional: whatever a previous run left behind, this round is newer.
    let stored_before = decode::<OraclePriceAccount>(&account_now(&client, price_id).await.data);
    let round = chain_now_ms(&client).await;
    if let Some(before) = &stored_before {
        assert!(
            round > before.timestamp,
            "the chain clock ({round}) is not ahead of the stored observation ({}), so this \
             submission could not be an update",
            before.timestamp
        );
    }

    let published = submit_and_read_back(
        &client,
        program,
        &key,
        admin_id,
        feed_account_id,
        round,
        FIRST_VALUE,
    )
    .await;

    assert_eq!(
        published,
        OraclePriceAccount {
            base_asset: kanon_idl::AccountId::new(BASE_ASSET),
            quote_asset: kanon_idl::AccountId::new(QUOTE_ASSET),
            price: expected_price(FIRST_VALUE),
            timestamp: round,
            source_id: REDSTONE_SOURCE_ID,
            confidence_interval: 0,
        },
        "the published account is not the one this payload and this registration describe"
    );

    // The account is the program's own, which is what makes it the canonical one
    // rather than something a caller could have written.
    assert_eq!(
        account_now(&client, price_id).await.program_owner,
        program,
        "the price account is not owned by the aggregator"
    );

    // The other half of the write path: an update at a newer round, moving the
    // price and the timestamp and nothing else.
    let next_round = chain_now_past(&client, round).await;
    let updated = submit_and_read_back(
        &client,
        program,
        &key,
        admin_id,
        feed_account_id,
        next_round,
        SECOND_VALUE,
    )
    .await;

    assert_eq!(
        updated,
        OraclePriceAccount {
            price: expected_price(SECOND_VALUE),
            timestamp: next_round,
            ..published
        },
        "an update moved something other than the price and the timestamp"
    );
    assert_ne!(
        updated.price, published.price,
        "the second round carried a different value, so a price that did not move means the \
         update was not applied"
    );
}
