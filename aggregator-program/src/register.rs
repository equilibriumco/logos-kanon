//! Feed registration: the parameters every later submission is checked against.
//!
//! A registration is the whole of a feed's configuration, and it is the only
//! instruction that creates a feed account. What it writes is what
//! [`crate::submit::submit_price`] reads on every update, so the validation here
//! is not a convenience: an unchecked registration is a signer set, a threshold
//! or a staleness window that no later check can recover from.
//!
//! None of that validation is written here. [`FeedConfig::try_new`] already
//! decides what a usable feed is -- M1 built it for the verifier -- and a second
//! set of rules in this module would be a second answer to the same question.
//! So registration constructs the configuration it is about to store and reports
//! what that construction refuses.
//!
//! # The feed's address
//!
//! `for_public_pda(program, sha256(feed_id || zero_pad_32("KANON_FEED_ACCOUNT")))`,
//! declared as `#[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]`
//! on the guest handler rather than checked here, for the reason ADR 32 gives
//! about the price account: the derivation is what a client has to reproduce, so
//! the constraint is what publishes it in the IDL. `mut` and not `init` because
//! of the three pre-states below -- see [`register_feed`].
//!
//! Two things follow from the address being the feed id's. A client can find a
//! feed without being told where it is, which is what the SDK, the CLI, the
//! relayer and every consumer need. And one feed id has one account, so a second
//! registration of the same feed arrives at an account that already holds one and
//! is refused -- which is the whole of R2's atomicity, and a property of the
//! address space rather than a check anyone has to remember.
//!
//! # Three states, not two
//!
//! Because the address is fixed, re-registering after
//! [`deregister_feed`](crate::manage::deregister_feed) has to work, or a feed id
//! would be spent the first time it was retired. LEZ makes that a question about
//! data rather than about the account: rule 4 forbids a program giving up
//! ownership and rule 3 forbids resetting the nonce, so a deregistered account
//! stays this program's forever and cannot be handed back as
//! `Account::default()`.
//!
//! So a feed account arrives in one of three states, which is the same shape
//! `submit_price` faces at the price account:
//!
//! - **fully default** -- never registered, and the claim makes it ours;
//! - **ours, with no data** -- deregistered, and re-registering rewrites it
//!   without claiming, because it is already claimed;
//! - **anything else** -- a feed is there, or the account is not one this program
//!   can write, and either way the registration is refused.
//!
//! The third state is also why the guest declares the feed account `mut` rather
//! than `init`. `init` emits `accounts[0] != Account::default()` in the
//! dispatcher, which is the two-state question, and it would refuse every
//! re-registration as `AccountAlreadyInitialized` before this function ran. The
//! check below is therefore the only gate, on chain as well as in a test.

use lee_core::account::{Account, AccountWithMetadata, Data};
use lee_core::program::{AccountPostState, ProgramId};
use spel_framework::pda::seed_from_str;
use spel_framework::spel_output::AutoClaim;
use verifier_core::SignerAddress;
use verifier_core::{AssetPair, ConfigError, FeedConfig};

use kanon_idl::OraclePriceAccount;

use crate::FeedAccount;

/// The name seed the feed account's address is derived from.
pub const FEED_ACCOUNT_SEED: &str = "KANON_FEED_ACCOUNT";

/// Why a registration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    /// The feed account is not empty, so this feed id is already registered.
    ///
    /// The whole account is compared and not its owner alone: an account
    /// carrying a balance or a nonce is one this program can never claim, and
    /// letting it through here would move the refusal to
    /// `validate_execution`, which reports it against the post-state rather
    /// than against the input. ADR 32 records the same trap at the price
    /// account.
    ///
    /// Nothing upstream repeats this check: the declaration is `mut` rather than
    /// `init`, so a squatted address reaches here rather than being turned away
    /// by the dispatcher.
    AlreadyRegistered,
    /// The parameters do not describe a usable feed.
    ///
    /// Reported through [`ConfigError`] rather than restated, so a registration
    /// and a submission refuse the same input for the same reason with the same
    /// code.
    Config(ConfigError),
    /// The pair does not match the one this feed id has already published.
    ///
    /// A feed id's pair is fixed from its first publication, and the price
    /// account is what remembers it: a retirement empties the *feed* account and
    /// leaves the *price* account where it was, so a re-registration under a new
    /// pair would produce a feed that can never publish. `submit_price` reads the
    /// expected pair off the published account on every write but the first, so
    /// every payload for the new pair would answer `AssetMismatch` for ever.
    ///
    /// Refused here rather than allowed and diagnosed later, because a RedStone
    /// feed id *is* its asset — `BTC` means BTC/USD — so re-pointing an id at
    /// another pair is a mistake in every case rather than a use case with a
    /// cost. Registering a different pair means a different feed id.
    PairChanged,
    /// The serialised feed does not fit an account's data.
    ///
    /// Unreachable while [`ConfigError::TooManySigners`] bounds the signer list:
    /// a feed at the maximum is under a kilobyte against a 100 KiB limit. Kept
    /// so that raising the bound fails closed here rather than panicking in a
    /// guest, where a panic aborts the transaction instead of refusing the
    /// instruction.
    FeedTooLarge,
}

impl From<ConfigError> for RegisterError {
    fn from(err: ConfigError) -> Self {
        Self::Config(err)
    }
}

impl core::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlreadyRegistered => {
                f.write_str("this feed id is already registered at its derived address")
            }
            Self::FeedTooLarge => f.write_str("the serialised feed does not fit an account's data"),
            Self::PairChanged => {
                f.write_str("this feed id has already published a different asset pair")
            }
            Self::Config(err) => write!(f, "{err:?}"),
        }
    }
}

/// What the guest returns, so its handler is a `?`.
impl From<RegisterError> for spel_framework::error::SpelError {
    fn from(err: RegisterError) -> Self {
        Self::custom(err.code(), err.to_string())
    }
}

impl RegisterError {
    /// A stable number per leaf cause, in the 900 block: this instruction's own,
    /// the way `SubmitError` numbers the layers below it. `Config` keeps the 300
    /// block it already has, so one cause has one number wherever it surfaces.
    #[must_use]
    pub const fn code(&self) -> u32 {
        match self {
            Self::AlreadyRegistered => 901,
            Self::FeedTooLarge => 902,
            Self::PairChanged => 903,
            Self::Config(err) => crate::submit::config_code(*err),
        }
    }
}

/// Registers a feed, and returns the post-states in the instruction's account
/// order.
///
/// The admin gate is the caller's: the guest handler calls
/// [`crate::admin::authorise`] before this, so reaching here means the signer is
/// the authority.
///
/// # Errors
///
/// [`RegisterError`] when the feed id is already registered or the parameters do
/// not describe a usable feed.
#[expect(
    clippy::too_many_arguments,
    reason = "a registration is the feed's whole configuration, and naming each field is what makes it checkable from the IDL"
)]
pub fn register_feed(
    feed: AccountWithMetadata,
    admin: AccountWithMetadata,
    config: AccountWithMetadata,
    price_account: AccountWithMetadata,
    feed_id: [u8; 32],
    base_asset: [u8; 32],
    quote_asset: [u8; 32],
    decimals: u8,
    max_age_ms: u64,
    signers: Vec<[u8; SignerAddress::LEN]>,
    threshold: u8,
    self_program_id: ProgramId,
) -> Result<Vec<AccountPostState>, RegisterError> {
    // Decided once, on one discriminator, for the reason ADR 32 records about the
    // price account: an account with data and a default owner has a claimable
    // owner and an unwritable body, and deciding the claim separately is what
    // lets that state through.
    let first = feed.account == Account::default();
    let deregistered = !first
        && feed.account.program_owner == self_program_id
        && feed.account.data.as_ref().is_empty();
    if !first && !deregistered {
        return Err(RegisterError::AlreadyRegistered);
    }

    // Constructed and dropped: what is wanted is the refusal, not the value.
    // Storing the configuration would mean storing borrowed signers, and the
    // account is the storage.
    // The pair the feed id already published, if it ever did. A retirement does
    // not touch the price account, so this outlives the registration that wrote
    // it -- which is exactly why it is the right place to read the pair from.
    // Undecodable or empty means nothing was ever published under this id, and
    // any pair is still available.
    if let Ok(published) = OraclePriceAccount::try_from(&price_account.account.data) {
        let publishing = AssetPair::new(
            published.base_asset.into_value(),
            published.quote_asset.into_value(),
        );
        if publishing != AssetPair::new(base_asset, quote_asset) {
            return Err(RegisterError::PairChanged);
        }
    }

    let addresses: Vec<SignerAddress> = signers.iter().copied().map(SignerAddress).collect();
    FeedConfig::try_new(
        &feed_id,
        AssetPair::new(base_asset, quote_asset),
        decimals,
        max_age_ms,
        &addresses,
        threshold,
    )?;

    let stored = FeedAccount {
        feed_id,
        base_asset,
        quote_asset,
        decimals,
        max_age_ms,
        signers,
        threshold,
        // A feed accepts submissions from the moment it exists. Registering it
        // paused would need an unpause before the first price, and nothing asks
        // for that.
        paused: false,
    };

    let mut account = feed.account;
    account.data = Data::try_from(borsh::to_vec(&stored).map_err(|_| RegisterError::FeedTooLarge)?)
        .map_err(|_| RegisterError::FeedTooLarge)?;

    // From the same discriminator the check used, and not from the account's
    // owner. A re-registration must not claim: LEZ refuses a claim on an account
    // whose owner is not the default one, so claiming again would fail the
    // transaction rather than be ignored.
    let feed_post = if first {
        AutoClaim::pda_from_seeds(&[&feed_id, &seed_from_str(FEED_ACCOUNT_SEED)])
            .to_post_state(account)
    } else {
        AccountPostState::new(account)
    };

    Ok(vec![
        feed_post,
        AccountPostState::new(admin.account),
        AccountPostState::new(config.account),
        // Read for its pair and otherwise untouched. A registration never writes
        // a price; `submit_price` does.
        AccountPostState::new(price_account.account),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{AccountId, Nonce};
    use lee_core::program::{Claim, ProgramId};
    use spel_framework::pda::compute_pda;
    use verifier_core::feed::MAX_SIGNERS;

    const OURS: ProgramId = [7u32; 8];
    const BASE: [u8; 32] = [0xB7; 32];
    const QUOTE: [u8; 32] = [0x05; 32];
    const DECIMALS: u8 = 8;
    const MAX_AGE_MS: u64 = 60_000;

    fn feed_id(name: &[u8]) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[..name.len()].copy_from_slice(name);
        id
    }

    /// The address the constraint derives, computed the way a client would.
    fn feed_address(id: &[u8; 32]) -> AccountId {
        compute_pda(&OURS, &[id, &seed_from_str(FEED_ACCOUNT_SEED)])
    }

    /// An unregistered feed account: default, at the address its feed id derives.
    fn unregistered(id: &[u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: feed_address(id),
        }
    }

    /// A price account for a feed id that has never published.
    fn unpublished() -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            is_authorized: false,
            account_id: AccountId::new([0x9E; 32]),
        }
    }

    /// Any account this instruction only passes through.
    fn passthrough(tag: u8) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: [42u32; 8],
                balance: 3,
                data: Data::default(),
                nonce: Nonce(1),
            },
            is_authorized: true,
            account_id: AccountId::new([tag; 32]),
        }
    }

    fn signers(n: u8) -> Vec<[u8; SignerAddress::LEN]> {
        (1..=n).map(|i| [i; SignerAddress::LEN]).collect()
    }

    /// A registration with the RFP's default shape: three of five.
    fn register(
        feed: AccountWithMetadata,
        id: [u8; 32],
        set: Vec<[u8; SignerAddress::LEN]>,
        threshold: u8,
    ) -> Result<Vec<AccountPostState>, RegisterError> {
        register_feed(
            feed,
            passthrough(0xAD),
            passthrough(0xC0),
            // Nothing published, so any pair is available.
            unpublished(),
            id,
            BASE,
            QUOTE,
            DECIMALS,
            MAX_AGE_MS,
            set,
            threshold,
            OURS,
        )
    }

    fn stored_in(post: &AccountPostState) -> FeedAccount {
        use borsh::BorshDeserialize;
        FeedAccount::try_from_slice(post.account().data.as_ref()).expect("decodes")
    }

    #[test]
    fn a_registration_stores_the_configuration_it_was_given() {
        let id = feed_id(b"BTC");

        let posts = register(unregistered(&id), id, signers(5), 3).expect("a usable feed");

        let stored = stored_in(&posts[0]);
        assert_eq!(stored.feed_id, id, "the padded id is stored as given");
        assert_eq!(stored.base_asset, BASE);
        assert_eq!(stored.quote_asset, QUOTE);
        assert_eq!(stored.decimals, DECIMALS);
        assert_eq!(stored.max_age_ms, MAX_AGE_MS);
        assert_eq!(stored.signers, signers(5));
        assert_eq!(stored.threshold, 3);
    }

    #[test]
    fn a_feed_accepts_submissions_from_the_moment_it_exists() {
        // Registering it paused would need an unpause before the first price,
        // and nothing asks for that.
        let id = feed_id(b"ETH");

        let posts = register(unregistered(&id), id, signers(3), 2).expect("a usable feed");

        assert!(!stored_in(&posts[0]).paused);
    }

    #[test]
    fn the_stored_feed_is_one_the_verifier_accepts() {
        // The registration and the submission path have to agree about what a
        // usable feed is. They do by construction -- `FeedConfig::try_new` is
        // what refused above -- and this is that stated as an assertion.
        let id = feed_id(b"SOL");

        let posts = register(unregistered(&id), id, signers(5), 3).expect("a usable feed");

        let stored = stored_in(&posts[0]);
        let addresses = stored.signer_addresses();
        let config = stored
            .config(&addresses)
            .expect("the stored feed configures");
        assert_eq!(config.threshold(), 3);
        assert_eq!(config.feed_id(), &id);
    }

    #[test]
    fn the_post_states_are_the_feed_then_the_three_it_passes_through() {
        let id = feed_id(b"XMR");
        let admin = passthrough(0xAD);
        let config = passthrough(0xC0);
        let price = unpublished();

        let posts = register_feed(
            unregistered(&id),
            admin.clone(),
            config.clone(),
            price.clone(),
            id,
            BASE,
            QUOTE,
            DECIMALS,
            MAX_AGE_MS,
            signers(3),
            2,
            OURS,
        )
        .expect("a usable feed");

        assert_eq!(posts.len(), 4);
        assert_eq!(*posts[1].account(), admin.account, "the admin is untouched");
        assert_eq!(
            *posts[2].account(),
            config.account,
            "the config is untouched"
        );
        assert_eq!(
            *posts[3].account(),
            price.account,
            "a registration reads the price account and never writes it"
        );
    }

    #[test]
    fn the_feed_is_claimed_at_the_address_its_id_derives() {
        // The claim and the declared constraint have to name one address. This
        // derives it from the seeds; the guest suite derives it from the
        // generated validator, which is the other side.
        let id = feed_id(b"ZEC");

        let posts = register(unregistered(&id), id, signers(3), 2).expect("a usable feed");

        let seed = match posts[0].required_claim() {
            Some(Claim::Pda(seed)) => seed,
            other => panic!("expected a PDA claim, got {other:?}"),
        };
        assert_eq!(
            AccountId::for_public_pda(&OURS, &seed),
            feed_address(&id),
            "the claim names the address the constraint publishes"
        );
    }

    #[test]
    fn a_feed_id_that_is_already_registered_is_refused() {
        // One feed id has one account, so a second registration arrives at an
        // account that is no longer default. That is R2's atomicity: nothing
        // partial can be written, because the write never begins.
        let id = feed_id(b"BTC");
        let mut existing = unregistered(&id);
        existing.account.program_owner = OURS;
        existing.account.data = Data::try_from(
            borsh::to_vec(&stored_in(
                &register(unregistered(&id), id, signers(3), 2).unwrap()[0],
            ))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            register(existing, id, signers(3), 2),
            Err(RegisterError::AlreadyRegistered)
        );
    }

    #[test]
    fn an_account_carrying_anything_at_all_is_refused() {
        // The whole account and not its owner: a balance or a nonce on an
        // otherwise empty account is a pre-state this program can never claim,
        // and refusing it here reports it against the input rather than leaving
        // `validate_execution` to refuse the post-state.
        let id = feed_id(b"BTC");
        for mutate in [
            |a: &mut Account| a.balance = 1,
            |a: &mut Account| a.nonce = Nonce(1),
            |a: &mut Account| a.program_owner = [9u32; 8],
        ] {
            let mut feed = unregistered(&id);
            mutate(&mut feed.account);
            assert_eq!(
                register(feed, id, signers(3), 2),
                Err(RegisterError::AlreadyRegistered)
            );
        }
    }

    /// One refusal: what it should be, and the parameters that produce it.
    type Case = (RegisterError, Vec<[u8; SignerAddress::LEN]>, u8, [u8; 32]);

    #[test]
    fn the_parameters_are_refused_by_the_rule_the_verifier_already_had() {
        // Each of these is a `ConfigError`, reported rather than restated, so a
        // registration and a submission refuse the same input with the same
        // code. Not an exhaustive list of `ConfigError` -- `verifier-core` owns
        // that -- but one of each shape reachable from a registration.
        let id = feed_id(b"BTC");
        let cases: Vec<Case> = vec![
            (
                RegisterError::Config(ConfigError::ThresholdZero),
                signers(3),
                0,
                id,
            ),
            (
                RegisterError::Config(ConfigError::NoSigners),
                Vec::new(),
                1,
                id,
            ),
            (
                RegisterError::Config(ConfigError::DuplicateSigner),
                vec![[1u8; 20], [1u8; 20]],
                1,
                id,
            ),
            (
                RegisterError::Config(ConfigError::ZeroSignerAddress),
                vec![[0u8; 20]],
                1,
                id,
            ),
            (
                RegisterError::Config(ConfigError::ZeroFeedId),
                signers(3),
                2,
                [0u8; 32],
            ),
            (
                RegisterError::Config(ConfigError::ThresholdExceedsSigners {
                    threshold: 4,
                    signers: 3,
                }),
                signers(3),
                4,
                id,
            ),
        ];

        for (expected, set, threshold, wanted_id) in cases {
            assert_eq!(
                register(unregistered(&wanted_id), wanted_id, set, threshold),
                Err(expected),
                "the registration must refuse what the verifier refuses"
            );
        }
    }

    #[test]
    fn no_two_causes_share_an_error_code() {
        // `a_config_error_keeps_the_number_it_has_everywhere_else` already pins
        // every code this type answers with. What it does not do is fail when a
        // fourth variant arrives unnumbered, or assert the codes differ from one
        // another -- the two halves of the convention `SubmitError` and
        // `AdminError` carry. No collision is reachable at two variants plus a
        // delegated block, so this is maintenance rather than a defect.
        let causes = [
            RegisterError::AlreadyRegistered,
            RegisterError::FeedTooLarge,
            RegisterError::PairChanged,
        ];

        // No wildcard arm, so a new variant fails to compile here. It does not
        // prove the list is complete -- an author can add the arm and forget the
        // entry -- so the list is kept by hand; what this buys is that the
        // omission is loud.
        for cause in &causes {
            match cause {
                RegisterError::AlreadyRegistered
                | RegisterError::FeedTooLarge
                | RegisterError::PairChanged
                | RegisterError::Config(_) => {}
            }
        }

        let mut codes: Vec<u32> = causes.iter().map(RegisterError::code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before, "two causes answer with one code");
    }

    #[test]
    fn a_config_error_keeps_the_number_it_has_everywhere_else() {
        // The 900 block is this instruction's own; a cause that already has a
        // number keeps it, so an operator reading a code off a failed
        // transaction does not have to know which instruction produced it.
        assert_eq!(RegisterError::AlreadyRegistered.code(), 901);
        assert_eq!(RegisterError::FeedTooLarge.code(), 902);
        assert_eq!(RegisterError::PairChanged.code(), 903);
        assert_eq!(
            RegisterError::Config(ConfigError::ThresholdZero).code(),
            crate::submit::config_code(ConfigError::ThresholdZero)
        );
    }

    #[test]
    fn the_maximum_signer_set_still_fits_an_account() {
        // `FeedTooLarge` is documented as unreachable while `TooManySigners`
        // bounds the list. This is what makes that true rather than asserted.
        let id = feed_id(b"BTC");
        let max = u8::try_from(MAX_SIGNERS).expect("MAX_SIGNERS fits a u8");

        let posts = register(unregistered(&id), id, signers(max), max).expect("the maximum fits");

        assert_eq!(stored_in(&posts[0]).signers.len(), MAX_SIGNERS);
    }
}
