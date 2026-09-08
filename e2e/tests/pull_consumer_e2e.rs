//! The pull path across a real standalone LEZ sequencer (M3-07).
//!
//! The other half of what `sequencer_e2e.rs` does for the push path, and the
//! same discipline: every assertion is on account state read back from the
//! chain, never on what `sendTransaction` returned. `tests/support/chain.rs`
//! holds the plumbing both suites share and says why it is shaped that way.
//!
//! # What is different about this half
//!
//! **No aggregator anywhere.** Reference consumer B verifies a payload itself,
//! against a roster it registered under its own authority, so nothing here
//! deploys the push program, reads a price account, or names `aggregator-program`
//! at all. That is F9 on a running chain rather than in a dependency walk: the
//! closure job proves a pull consumer *cannot* reach the aggregator, and this
//! proves it does not *need* to. Nothing enforces the absence here: the closure
//! walk covers `pull-lib`, the consumer and its guest workspace, not `e2e`, where
//! `aggregator-program` is a dev-dependency the push suite legitimately uses. So
//! this paragraph is the only thing holding it — do not add the aggregator to this
//! file to reuse a fixture.
//!
//! **The roster moves.** A compiled roster would have made
//! `rotate_signers` unnecessary and a rotation impossible without a rebuild.
//! `[M3-06:02]` records why the roster is state instead, and the second half of
//! this test is that decision working on a chain: the authority rotates to a
//! disjoint signer set, and an order opened afterwards settles against a payload
//! only the new signers signed.
//!
//! # Re-running against a chain that already has state
//!
//! `lez-sequencer.sh start` gives a clean chain, which is what CI gets. Locally
//! a second run meets the accounts the first one created, so each setup step is
//! conditional on the state it would create. Order ids carry the host clock's
//! nanoseconds, so no previous run can have opened them and every run has to
//! open and fill its own orders or fail. The chain's own clock would not do:
//! it advances once per block, so two runs started inside the same fifteen
//! seconds would derive the same ids and meet each other's filled orders.

#[path = "support/chain.rs"]
mod chain;
use chain::{
    account_now, account_when, chain_now_ms, client, decode, deploy_guest,
    ensure_the_account_is_owned, send_to_program,
};
use lee_core::program::ProgramId;
use reference_consumer_pull::{
    authority::config_address, order::ORDER_ACCOUNT_SEED, trust::trust_address, ConfigAccount,
    FeedTrust, Instruction, OrderAccount,
};
use sequencer_service_rpc::SequencerClient;
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::test_support::{address_of, signing_key, PayloadBuilder};

/// The authority key this suite holds.
///
/// Deliberately not the push suite's `[0x11; 32]`. Cargo runs the two test
/// binaries one after another against the same chain, so the second meets
/// whatever the first left behind; a key each keeps a suite's accounts its own
/// and stops a failure in one being explained by the other's state.
///
/// Fixed rather than random, because the guest compiles the *account id* of this
/// key in as `KANON_PULL_GENESIS_AUTHORITY` and a fresh key each run would mean
/// a fresh image id and a rebuilt guest each run. It signs for one account on a
/// throwaway local sequencer and controls nothing anywhere else.
const TEST_AUTHORITY_KEY: [u8; 32] = [0x22; 32];

/// The account id the guest must carry as `KANON_PULL_GENESIS_AUTHORITY`.
///
/// A `const` because the CI job needs the same value as an env var and a `const`
/// is what a shell line can hold. `require_a_configured_build` derives it from
/// [`TEST_AUTHORITY_KEY`] and refuses to run if the two disagree, so the literal
/// cannot drift from the key it claims to describe.
const EXPECTED_GENESIS: &str = "8536b4cd1b24c4af1df73f6b509eadc2d26bebc1f94d10253658419fe643e197";

/// The owner that opens orders here.
///
/// A second key, because an order's owner and the program's authority are
/// different parties and the test should not be able to confuse them. It signs
/// its own opens; the authority never does.
const TEST_OWNER_KEY: [u8; 32] = [0x23; 32];

fn key_and_id(raw: [u8; 32]) -> (lee::PrivateKey, lee::AccountId) {
    let key = lee::PrivateKey::try_new(raw).expect("a valid secp256k1 key");
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
    let (_, authority_id) = key_and_id(TEST_AUTHORITY_KEY);
    let derived = hex::encode(authority_id.value());
    assert_eq!(
        EXPECTED_GENESIS, derived,
        "EXPECTED_GENESIS is not the account id of TEST_AUTHORITY_KEY. One of the two \
         was edited without the other; the derived value is the correct one, and it \
         also has to be set in the `sequencer` CI job"
    );

    let configured = option_env!("KANON_PULL_GENESIS_AUTHORITY").unwrap_or("");
    assert_eq!(
        configured, EXPECTED_GENESIS,
        "this test needs a guest built with the test genesis authority. Run it as:\n    \
         KANON_PULL_GENESIS_AUTHORITY={EXPECTED_GENESIS} cargo test --manifest-path e2e/Cargo.toml"
    );
}

// ---------------------------------------------------------------------------
// The feed this consumer registers, and the rosters it registers for it.
// ---------------------------------------------------------------------------

/// The roster registered first, and the disjoint one it rotates to.
///
/// Disjoint on purpose: an overlapping set would let a payload signed by the
/// first roster still meet a threshold under the second, so the rotation would
/// prove nothing about which roster was in force.
const FIRST_ROSTER: [u8; 3] = [1, 2, 3];
const SECOND_ROSTER: [u8; 3] = [4, 5, 6];
const THRESHOLD: u8 = 3;

const DATA_SERVICE_ID: &str = "redstone-primary-prod";
const DECIMALS: u8 = 8;

/// Five minutes. Well inside `MAX_MAX_AGE_MS`, and wide enough that a payload
/// stays current across the blocks between signing it and settling with it.
const MAX_AGE_MS: u64 = 300_000;

const BASE_ASSET: [u8; 32] = [0xB7; 32];
const QUOTE_ASSET: [u8; 32] = [0x05; 32];

/// `65_000.00000000` at eight decimals, as RedStone would scale it.
const MARKET_VALUE: u64 = 6_500_000_000_000;

/// A price on the `Q64.64` scale the consumer compares limits on.
///
/// Computed here rather than through the crate under test, and the orders below
/// are opened at exactly this value rather than under it. A settlement fills on
/// `price >= limit`, so a limit at half the market clears whatever the verifier
/// scaled the payload to -- `<< 63`, `<< 65` or a registration at the wrong
/// `decimals` would all still fill, and the only price-sensitive outcome this
/// suite has is whether an order filled at all. At the exact value, a scale that
/// moved in either direction stops clearing and the settlement never lands.
fn q64(value_scaled: u64) -> u128 {
    (u128::from(value_scaled) << 64) / 10u128.pow(u32::from(DECIMALS))
}

fn feed_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..3].copy_from_slice(b"BTC");
    id
}

/// The payload `roster` would publish for `timestamp_ms`.
///
/// Full-width big-endian values, which is what RedStone puts on the wire.
fn signed_payload(roster: [u8; 3], timestamp_ms: u64, value_scaled: u64) -> Vec<u8> {
    let mut value = [0u8; 32];
    value[24..].copy_from_slice(&value_scaled.to_be_bytes());

    let mut builder = PayloadBuilder::default();
    for seed in roster {
        builder = builder.signed_package(&signing_key(seed), &[(b"BTC", &value)], timestamp_ms);
    }
    builder.build()
}

fn addresses_of(roster: [u8; 3]) -> Vec<[u8; 20]> {
    roster
        .iter()
        .map(|seed| address_of(&signing_key(*seed)).0)
        .collect()
}

// ---------------------------------------------------------------------------
// Addresses, derived the way a client has to derive them.
// ---------------------------------------------------------------------------

/// Through the crate's own helpers, which exist so that a client's derivation
/// and the guest's declared constraint cannot drift apart. Spelling either out
/// here would make this file one of the places that drift.
fn config_id(program: ProgramId) -> lee::AccountId {
    lee::AccountId::new(*config_address(&program).value())
}

fn trust_id(program: ProgramId) -> lee::AccountId {
    lee::AccountId::new(*trust_address(&program, &feed_id()).value())
}

/// Hand-rolled because an order's address has no published counterpart: the
/// order module exports the seed, not a derivation over it.
fn order_id_address(program: ProgramId, order_id: [u8; 32]) -> lee::AccountId {
    lee::AccountId::new(
        *compute_pda(&program, &[&order_id, &seed_from_str(ORDER_ACCOUNT_SEED)]).value(),
    )
}

// ---------------------------------------------------------------------------
// The steps.
// ---------------------------------------------------------------------------

async fn ensure_the_authority_is_established(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    authority_id: lee::AccountId,
) {
    let config = config_id(program);
    let established = decode::<ConfigAccount>(&account_now(client, config).await.data)
        .is_some_and(|held| held.authority == *authority_id.value());

    if !established {
        send_to_program(
            client,
            program,
            vec![config, authority_id],
            key,
            authority_id,
            &Instruction::EstablishAuthority,
        )
        .await;
    }

    let held: ConfigAccount = account_when(client, config, "the authority is established", |a| {
        decode(&a.data)
    })
    .await;

    assert_eq!(
        held,
        ConfigAccount {
            authority: *authority_id.value(),
            pending: None,
        },
        "the config account holds an authority other than the genesis key this build carries"
    );
}

/// Registers the feed's trust with `roster`, or rotates to it if it is already
/// registered with another.
///
/// Both paths end in the same assertion, so a run that skipped the write still
/// has to find the roster it asked for.
async fn ensure_the_roster_is(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    authority_id: lee::AccountId,
    roster: [u8; 3],
) {
    let trust = trust_id(program);
    let signers = addresses_of(roster);
    let stored = decode::<FeedTrust>(&account_now(client, trust).await.data);

    match &stored {
        Some(held) if held.signers == signers => {}
        Some(_) => {
            send_to_program(
                client,
                program,
                vec![trust, authority_id, config_id(program)],
                key,
                authority_id,
                &Instruction::RotateSigners {
                    feed_id: feed_id(),
                    signers: signers.clone(),
                    threshold: THRESHOLD,
                },
            )
            .await;
        }
        None => {
            send_to_program(
                client,
                program,
                vec![trust, authority_id, config_id(program)],
                key,
                authority_id,
                &Instruction::RegisterFeedTrust {
                    data_service_id: DATA_SERVICE_ID.to_owned(),
                    feed_id: feed_id(),
                    base_asset: BASE_ASSET,
                    quote_asset: QUOTE_ASSET,
                    decimals: DECIMALS,
                    max_age_ms: MAX_AGE_MS,
                    signers: signers.clone(),
                    threshold: THRESHOLD,
                },
            )
            .await;
        }
    }

    let held: FeedTrust = account_when(client, trust, "the feed's roster is in force", |a| {
        decode::<FeedTrust>(&a.data).filter(|held| held.signers == signers)
    })
    .await;

    assert_eq!(held.threshold, THRESHOLD);
    assert_eq!(held.feed_id, feed_id());
    assert_eq!(
        account_now(client, trust).await.program_owner,
        program,
        "the trust account is not owned by the consumer, so the registration's claim did not land"
    );
}

/// Opens an order at `limit`, and returns the account it was written to.
async fn open_an_order(
    client: &SequencerClient,
    program: ProgramId,
    key: &lee::PrivateKey,
    owner_id: lee::AccountId,
    order_id: [u8; 32],
    limit: u128,
) -> lee::AccountId {
    let address = order_id_address(program, order_id);

    send_to_program(
        client,
        program,
        vec![address, owner_id, trust_id(program)],
        key,
        owner_id,
        &Instruction::OpenOrder {
            order_id,
            feed_id: feed_id(),
            base_asset: BASE_ASSET,
            quote_asset: QUOTE_ASSET,
            limit_price_q64: limit,
        },
    )
    .await;

    let opened: OrderAccount =
        account_when(client, address, "the order is open", |a| decode(&a.data)).await;

    assert_eq!(opened.owner, *owner_id.value());
    assert_eq!(opened.feed_id, feed_id());
    assert_eq!(opened.limit_price_q64, limit);
    assert!(!opened.filled, "a new order is not filled");
    assert_eq!(
        opened.max_age_ms, MAX_AGE_MS,
        "the order did not capture the registration's window. `OpenOrder` carries no \
         `max_age_ms`, so this catches the field being dropped rather than a window \
         sourced from the wrong party -- an owner has no way to supply one"
    );

    address
}

// ---------------------------------------------------------------------------
// The test.
// ---------------------------------------------------------------------------

/// M3-07: the pull path verifying and settling across a standalone sequencer.
///
/// One test rather than several, for the reason the push suite gives: the steps
/// are ordered and share one chain and one signer, so as separate tests they
/// would interleave transactions from the same account and take each other's
/// nonces.
#[tokio::test]
async fn the_pull_path_verifies_and_settles_across_a_real_sequencer() {
    require_a_configured_build();
    let client = client();
    let (authority_key, authority_id) = key_and_id(TEST_AUTHORITY_KEY);
    let (owner_key, owner_id) = key_and_id(TEST_OWNER_KEY);

    let program = deploy_guest(
        &client,
        kanon_methods::PULL_CONSUMER_ELF,
        kanon_methods::PULL_CONSUMER_ID,
    )
    .await;
    ensure_the_account_is_owned(&client, &authority_key, authority_id).await;
    ensure_the_account_is_owned(&client, &owner_key, owner_id).await;
    ensure_the_authority_is_established(&client, program, &authority_key, authority_id).await;
    ensure_the_roster_is(&client, program, &authority_key, authority_id, FIRST_ROSTER).await;

    // Nanoseconds from the host clock, so no previous local run can have opened
    // these and every run has to open and fill its own. Not the chain's clock:
    // that advances once per block, so two runs inside the same fifteen seconds
    // would collide and each would meet the other's already-filled orders.
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the host clock is past the epoch")
        .as_nanos();
    let mut first_id = [0u8; 32];
    first_id[..16].copy_from_slice(&run.to_be_bytes());
    let mut second_id = first_id;
    second_id[31] = 1;

    let first = open_an_order(
        &client,
        program,
        &owner_key,
        owner_id,
        first_id,
        q64(MARKET_VALUE),
    )
    .await;

    // A payload this consumer verifies itself, signed by the roster it
    // registered, timestamped at the chain's own clock. Nothing the aggregator
    // wrote is read, and no account of the aggregator's is named.
    let round = chain_now_ms(&client).await;
    send_to_program(
        &client,
        program,
        vec![
            first,
            trust_id(program),
            lee::AccountId::new(kanon_clock::CLOCK_ACCOUNT_ID),
        ],
        &owner_key,
        owner_id,
        &Instruction::Settle {
            order_id: first_id,
            feed_id: feed_id(),
            payload: signed_payload(FIRST_ROSTER, round, MARKET_VALUE),
        },
    )
    .await;

    let filled: OrderAccount = account_when(&client, first, "the order is filled", |a| {
        decode::<OrderAccount>(&a.data).filter(|order| order.filled)
    })
    .await;
    assert_eq!(
        filled.limit_price_q64,
        q64(MARKET_VALUE),
        "a settlement moved something other than `filled`"
    );

    // The roster is the consumer's own and it moves. A disjoint set means a
    // payload the first roster signed can no longer meet the threshold, so an
    // order settled after this could only have been filled under the new one.
    ensure_the_roster_is(
        &client,
        program,
        &authority_key,
        authority_id,
        SECOND_ROSTER,
    )
    .await;

    let second = open_an_order(
        &client,
        program,
        &owner_key,
        owner_id,
        second_id,
        q64(MARKET_VALUE),
    )
    .await;

    let later = chain_now_ms(&client).await;
    send_to_program(
        &client,
        program,
        vec![
            second,
            trust_id(program),
            lee::AccountId::new(kanon_clock::CLOCK_ACCOUNT_ID),
        ],
        &owner_key,
        owner_id,
        &Instruction::Settle {
            order_id: second_id,
            feed_id: feed_id(),
            payload: signed_payload(SECOND_ROSTER, later, MARKET_VALUE),
        },
    )
    .await;

    account_when(
        &client,
        second,
        "the order settles under the rotated roster",
        |a| decode::<OrderAccount>(&a.data).filter(|order| order.filled),
    )
    .await;
}
