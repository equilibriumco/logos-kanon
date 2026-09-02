//! The push path across a real standalone LEZ sequencer (M2-19), for all five of
//! F7's feeds (M2-11, M2-12).
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
//! # The one deviation from production: whose keys sign
//!
//! The five feeds are registered with **this test's signing keys**, not
//! RedStone's. That is forced rather than chosen. A live chain reads the clock in
//! real time, `MAX_MAX_AGE_MS` caps a feed's window at fifteen minutes, and the
//! committed captures are weeks old — so RedStone's own signatures cannot be
//! submitted to a running sequencer at all, and no `maxAge` a feed may legally
//! register would admit them.
//!
//! What their real signatures do verify is `verifier-core`'s conformance suite,
//! against those captures, with their keys and their published addresses. So:
//!
//! | | signatures | timestamps | chain |
//! | --- | --- | --- | --- |
//! | conformance (M1-21) | RedStone's | as captured | none |
//! | here (M2-11, M2-12) | this test's | live | real |
//!
//! The combination neither covers is real signatures on a live chain, which needs
//! live gateway data at submit time. That is the relayer's (M4) and testnet's
//! (M5), and it is why F7 stays `partial` on the strength of this file: the five
//! feeds are registered and publishing, but on a standalone sequencer rather than
//! the devnet or testnet the requirement names.
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
use kanon_clock::CLOCK_ACCOUNT_ID;
#[path = "support/chain.rs"]
mod chain;
use chain::{
    account_now, account_when, chain_now_ms, chain_now_past, client, decode, deploy_guest,
    ensure_the_account_is_owned, send_to_program,
};
use lee_core::program::ProgramId;
use sequencer_service_rpc::{RpcClient as _, SequencerClient};
use spel_framework::pda::{compute_pda, seed_from_str};
use verifier_core::test_support::{address_of, signing_key, PayloadBuilder};

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
// The five feeds this test registers (M2-11, M2-12).
// ---------------------------------------------------------------------------

/// Five signers registered, three of them signing.
///
/// The shape production has: `FEEDS.md` records one roster of five signers across
/// all five feeds, and RFP-020's default threshold is three. `signed_payload`
/// signs with the first three, so two registered signers are silent on every
/// submission and the threshold is the binding constraint rather than decoration
/// -- a price published here is a median over a genuine subset.
const SIGNER_SEEDS: [u8; 5] = [1, 2, 3, 4, 5];
const THRESHOLD: u8 = 3;

const DECIMALS: u8 = 8;

/// Five minutes. Well inside `MAX_MAX_AGE_MS`, and wide enough that the payload
/// stays current across the fifteen seconds or so between the clock reading and
/// the block that executes the submission.
const MAX_AGE_MS: u64 = 300_000;

/// One of F7's five feeds.
///
/// The ids are RedStone's own, from `redstone-primary-prod` -- the same five
/// `FEEDS.md` confirmed and the conformance vectors were captured for. The asset
/// pairs are this test's: LEZ account ids for real assets are a deployment input
/// nobody has issued yet, and what matters here is that the five differ, so a
/// feed writing another's account fails rather than passes unnoticed.
struct Feed {
    id: &'static [u8],
    base: [u8; 32],
    quote: [u8; 32],
    /// A plausible price at eight decimals, distinct per feed so that a price
    /// account holding the wrong feed's number is visible.
    value: u64,
}

/// F7's five, in the order `FEEDS.md` lists them.
const FEEDS: [Feed; 5] = [
    Feed {
        id: b"BTC",
        base: [0xB7; 32],
        quote: [0x05; 32],
        value: 6_500_000_000_000,
    },
    Feed {
        id: b"ETH",
        base: [0xE7; 32],
        quote: [0x05; 32],
        value: 320_000_000_000,
    },
    Feed {
        id: b"SOL",
        base: [0x50; 32],
        quote: [0x05; 32],
        value: 18_000_000_000,
    },
    Feed {
        id: b"XMR",
        base: [0x8B; 32],
        quote: [0x05; 32],
        value: 22_000_000_000,
    },
    Feed {
        id: b"ZEC",
        base: [0x2E; 32],
        quote: [0x05; 32],
        value: 5_500_000_000,
    },
];

impl Feed {
    /// The id right-padded to the wire's field width, which is how a feed id is
    /// both stored and used as the account's PDA seed.
    fn padded_id(&self) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[..self.id.len()].copy_from_slice(self.id);
        id
    }

    fn label(&self) -> &'static str {
        core::str::from_utf8(self.id).unwrap_or("??")
    }
}

/// The second round's value for BTC, enough above its first that an update is
/// visible.
const BTC_SECOND_VALUE: u64 = 6_600_000_000_000;

/// The payload `feed`'s signer set would publish for `timestamp_ms`.
///
/// Full-width big-endian values, which is what RedStone puts on the wire.
/// `Value::from_be_slice` right-aligns a shorter one, so a narrow value is
/// readable but is not what a real package carries.
///
/// Signed by keys this test holds rather than by RedStone's. That is the one
/// deviation from production in this file and it is forced: a live chain reads
/// the clock in real time, `MAX_MAX_AGE_MS` caps a feed's window at fifteen
/// minutes, and the committed captures are weeks old, so RedStone's own
/// signatures cannot be submitted here at all. What their real signatures do
/// verify is `verifier-core`'s conformance suite, against those captures, with
/// their keys. The two together leave one untested combination -- real
/// signatures on a live chain -- which needs live gateway data at submit time
/// and is the relayer's (M4) and testnet's (M5).
fn signed_payload(feed: &Feed, timestamp_ms: u64, value_scaled: u64) -> Vec<u8> {
    let mut value = [0u8; 32];
    value[24..].copy_from_slice(&value_scaled.to_be_bytes());

    // The first `THRESHOLD` of the registered five, not all of them. Registering
    // five and then signing with five makes quorum 5-of-5 and leaves `THRESHOLD`
    // never binding -- which is what the first version of this did while claiming
    // a median over a subset. Signing with exactly three makes the claim true and
    // the subset real: two registered signers stay silent and the price is still
    // published.
    let mut builder = PayloadBuilder::default();
    for seed in SIGNER_SEEDS.iter().take(usize::from(THRESHOLD)) {
        builder = builder.signed_package(&signing_key(*seed), &[(feed.id, &value)], timestamp_ms);
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

/// The chain, the program deployed on it, and the key that signs as its admin.
///
/// Bundled because these four travel together through every step and threading
/// them individually put `submit_and_read_back` over clippy's argument limit
/// once a feed joined them. Built after the deployment, which is where the
/// program id comes from.
struct Chain {
    client: SequencerClient,
    program: ProgramId,
    key: lee::PrivateKey,
    admin_id: lee::AccountId,
}

// ---------------------------------------------------------------------------
// The steps.
// ---------------------------------------------------------------------------

/// Establishes the authority, and returns the config account's address.
async fn ensure_the_authority_is_established(chain: &Chain) -> lee::AccountId {
    let Chain {
        client,
        program,
        key,
        admin_id,
    } = chain;
    let (program, admin_id) = (*program, *admin_id);
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

/// Registers one feed, and returns its account address.
async fn ensure_the_feed_is_registered(
    chain: &Chain,
    config_id: lee::AccountId,
    feed: &Feed,
) -> lee::AccountId {
    let Chain {
        client,
        program,
        key,
        admin_id,
    } = chain;
    let (program, admin_id) = (*program, *admin_id);
    let id = feed.padded_id();
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
        base_asset: feed.base,
        quote_asset: feed.quote,
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
                base_asset: feed.base,
                quote_asset: feed.quote,
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
        stored,
        expected,
        "{}: the registered feed is not the configuration this test asked for",
        feed.label()
    );

    // The feed account is the program's own, and a submission is required to
    // refuse one it does not own -- an account a caller owns would decode as a
    // feed with a signer set of the caller's choosing.
    assert_eq!(
        account_now(client, feed_account_id).await.program_owner,
        program,
        "{}: the feed account is not owned by the aggregator, so the claim in the \
         registration did not land",
        feed.label()
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
    chain: &Chain,
    feed: &Feed,
    feed_account_id: lee::AccountId,
    round: u64,
    value_scaled: u64,
) -> OraclePriceAccount {
    let Chain {
        client,
        program,
        key,
        admin_id,
    } = chain;
    let (program, admin_id) = (*program, *admin_id);
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
            payload: signed_payload(feed, round, value_scaled),
        },
    )
    .await;

    account_when(client, price_id, "the submission is published", |account| {
        decode::<OraclePriceAccount>(&account.data).filter(|price| price.timestamp == round)
    })
    .await
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

/// M2's done-gate line, and F7's five feeds: verify and publish, end to end, on a
/// standalone sequencer, for BTC, ETH, SOL, XMR and ZEC.
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

    let program = deploy_guest(
        &client,
        kanon_methods::AGGREGATOR_ELF,
        kanon_methods::AGGREGATOR_ID,
    )
    .await;
    ensure_the_account_is_owned(&client, &key, admin_id).await;

    let chain = Chain {
        client,
        program,
        key,
        admin_id,
    };
    let client = &chain.client;
    let config_id = ensure_the_authority_is_established(&chain).await;

    // M2-11 and M2-12: every one of F7's five feeds, registered and then
    // published, on a chain. One loop rather than five tests, for the reason the
    // doc comment gives -- they share a chain and a signer.
    let mut written: Vec<(lee::AccountId, lee::AccountId, OraclePriceAccount)> = Vec::new();

    for feed in &FEEDS {
        let feed_account_id = ensure_the_feed_is_registered(&chain, config_id, feed).await;
        let price_id = price_account_id(program, feed_account_id);

        // Timestamped at the chain's own clock, which is what makes the run
        // unconditional: whatever a previous run left behind, this round is newer.
        let stored_before = decode::<OraclePriceAccount>(&account_now(client, price_id).await.data);
        let round = chain_now_ms(client).await;
        if let Some(before) = &stored_before {
            assert!(
                round > before.timestamp,
                "{}: the chain clock ({round}) is not ahead of the stored observation ({}), \
                 so this submission could not be an update",
                feed.label(),
                before.timestamp
            );
        }

        let published =
            submit_and_read_back(&chain, feed, feed_account_id, round, feed.value).await;

        assert_eq!(
            published,
            OraclePriceAccount {
                base_asset: kanon_idl::AccountId::new(feed.base),
                quote_asset: kanon_idl::AccountId::new(feed.quote),
                price: expected_price(feed.value),
                timestamp: round,
                source_id: REDSTONE_SOURCE_ID,
                confidence_interval: 0,
            },
            "{}: the published account is not the one this payload and this registration \
             describe",
            feed.label()
        );

        // The account is the program's own, which is what makes it the canonical
        // one rather than something a caller could have written.
        assert_eq!(
            account_now(client, price_id).await.program_owner,
            program,
            "{}: the price account is not owned by the aggregator",
            feed.label()
        );

        written.push((feed_account_id, price_id, published));
    }

    // Five feeds, five accounts, each holding its own feed's price. A derivation
    // that ignored the feed id, or a `publish` that wrote through to the wrong
    // account, fails here rather than in review.
    //
    // Deliberately *not* called R3. R3 is about an upstream error for one feed
    // leaving pushes for the others alone, and this loop is happy-path only --
    // no upstream failure is introduced anywhere in it. @frenzox pointed that
    // out on #62; an earlier version of this comment and of F7's note claimed
    // R3's name for a different property. The failure R3 describes needs a
    // relayer to produce it, so it stays M4's.
    let addresses: std::collections::BTreeSet<_> =
        written.iter().map(|(_, price_id, _)| *price_id).collect();
    assert_eq!(
        addresses.len(),
        FEEDS.len(),
        "the five feeds do not have five distinct price accounts: {addresses:?}"
    );

    // Re-read from the chain rather than re-checking what the loop already
    // asserted. The first version compared the snapshot taken during the loop
    // against the same expression that snapshot had already been asserted equal
    // to, so it could not fail -- and it missed exactly the case it names: if
    // ZEC's submission had also written BTC's account, BTC's snapshot was taken
    // four submissions earlier and still held BTC's price. Only a read after all
    // five have published can see that.
    for (feed, (_, price_id, _)) in FEEDS.iter().zip(&written) {
        let current: OraclePriceAccount = account_when(
            client,
            *price_id,
            "the price account is still readable after all five published",
            |account| decode(&account.data),
        )
        .await;

        assert_eq!(
            current,
            OraclePriceAccount {
                base_asset: kanon_idl::AccountId::new(feed.base),
                quote_asset: kanon_idl::AccountId::new(feed.quote),
                price: expected_price(feed.value),
                timestamp: current.timestamp,
                source_id: REDSTONE_SOURCE_ID,
                confidence_interval: 0,
            },
            "{}'s account no longer holds its own feed's price after all five \
             published, so a submission crossed feeds",
            feed.label()
        );
    }

    // The other half of the write path, on BTC: an update at a newer round,
    // moving the price and the timestamp and nothing else.
    // Found by the literal id, so reordering `FEEDS` cannot point this phase at
    // another feed. Two earlier versions could: `written[0]` was position zero
    // outright, and searching for `BTC.id` was circular, because that constant
    // was itself `&FEEDS[0]` -- moving ETH to the first slot would have made
    // `BTC` mean ETH and the search find ETH. @frenzox caught both on #62.
    let (btc, (btc_feed_account, _, btc_published)) = FEEDS
        .iter()
        .zip(&written)
        .find(|(feed, _)| feed.id == b"BTC")
        .expect("BTC is one of the five feeds");
    let next_round = chain_now_past(client, btc_published.timestamp).await;
    let updated =
        submit_and_read_back(&chain, btc, *btc_feed_account, next_round, BTC_SECOND_VALUE).await;

    assert_eq!(
        updated,
        OraclePriceAccount {
            price: expected_price(BTC_SECOND_VALUE),
            timestamp: next_round,
            ..*btc_published
        },
        "an update moved something other than the price and the timestamp"
    );
    assert_ne!(
        updated.price, btc_published.price,
        "the second round carried a different value, so a price that did not move means the \
         update was not applied"
    );
}
