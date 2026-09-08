//! What crosses the boundary into a transaction, and what it costs (M3-10).
//!
//! Every other figure in `COSTS.md` is a program *body*. LEZ reads a
//! transaction's inputs before any body is entered — `read_lee_inputs` takes the
//! program ids, then every pre-state account with its data, then the instruction
//! words — and those cycles are spent whatever the program then does. `cost.rs`
//! and `read_cost.rs` both exclude them by construction: their guests build
//! accounts in setup, so the reading cancels out of every difference.
//!
//! This is the term that turns "the body fits" into "the transaction fits" (P1),
//! and the per-mode byte half of P2's cost measurement.
//!
//! # The rate is measured, not carried over
//!
//! ADR 26 put the read at "about 113 cycles" from one end-to-end observation: a
//! 127,814-byte payload whose transaction overran the budget, with 14.5M of the
//! overrun attributed to the read by subtraction. `input_cost.rs` measures it
//! directly instead: it varies only the length of what it is given, so
//! differencing two runs cancels everything that does not scale with that length —
//! the guest's own startup and journal commit included.
//!
//! It comes out at exactly 113 cycles a word and dead linear, which settles two
//! things: the estimate was right, and the unit is a **word**. Multiplying a
//! serialised size in bytes would count each word four times, and an earlier draft
//! of the M3-09 report did exactly that.

use aggregator_program::feed_account::FeedAccount;
use aggregator_program::instruction::Instruction as AggregatorInstruction;
use borsh::BorshSerialize;
use kanon_methods::{
    AGGREGATOR_ELF, AGGREGATOR_READ_CONSUMER_ELF, INPUT_COST_ELF, PULL_CONSUMER_ELF, PULL_COST_ELF,
    READ_COST_ELF,
};
use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::ProgramId;
use reference_consumer_aggregator_read::kanon_idl::{
    AccountId as IdlAccountId, OraclePriceAccount,
};
use reference_consumer_aggregator_read::order::OrderAccount as PushOrder;
use reference_consumer_aggregator_read::read::{price_account_address, REDSTONE_SOURCE_ID};
use reference_consumer_aggregator_read::source::{source_address, PriceSource};
use reference_consumer_pull::instruction::Instruction as PullInstruction;
use reference_consumer_pull::trust::trust_address;
use reference_consumer_pull::{FeedTrust, OrderAccount as PullOrder};
use risc0_zkvm::{default_executor, ExecutorEnv};

use kanon_clock::CLOCK_ACCOUNT_ID;

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

#[path = "support/lez.rs"]
mod lez;

/// Published in `COSTS.md`, and asserted here so the two cannot drift.
mod expected {
    /// What one word of input costs once the reading is under way.
    ///
    /// Exact rather than "about": the measurement is linear to the cycle across
    /// two orders of magnitude, which is what makes it a rate rather than an
    /// average. ADR 26 estimated it at "about 113" by subtraction from one
    /// end-to-end run; this is the same number, measured.
    pub const CYCLES_PER_WORD: u64 = 113;

    /// A run of the read guest over four empty inputs.
    ///
    /// Not four framed reads on their own: it includes what any run of any guest
    /// costs — zkVM startup and the journal commit — because nothing here
    /// subtracts a floor from it. The same floor sits inside every per-mode read
    /// figure below, so it cancels when two of them are compared and does not when
    /// one is quoted alone. What it bounds is a transaction that carries nothing,
    /// which is the useful reading of it.
    ///
    /// The marginal rates are unaffected: differencing two input sizes cancels
    /// this whole term.
    pub const FIXED: u64 = 4_426;

    /// What each recurring transaction carries: pre-state accounts, then the
    /// instruction. Both cross the same boundary, because `read_lee_inputs` reads
    /// the accounts before the instruction.
    pub const PULL_SETTLE: (usize, usize) = (496, 790);
    pub const PUSH_UPDATE: (usize, usize) = (454, 726);
    pub const PUSH_READ: (usize, usize) = (562, 33);

    /// And what each pays to be read, measured end to end over those inputs
    /// rather than multiplied out of them. An account costs far more than its
    /// words, so a word count times a rate understates every one of these -- the
    /// push read by nearly half.
    pub const PULL_SETTLE_READ: u64 = 188_072;
    pub const PUSH_UPDATE_READ: u64 = 175_826;
    pub const PUSH_READ_READ: u64 = 122_783;

    /// The bodies these sit in front of.
    ///
    /// Executed here rather than copied from `read_cost.rs`. A constant carried
    /// across test binaries is one nothing would notice going stale, and the
    /// figure this file exists to add is only interesting beside a body that is
    /// current. The runs are the same guests and stages `read_cost.rs` measures,
    /// so a disagreement between the two files is a real disagreement.
    pub const PULL_SETTLE_BODY: u64 = 3_048_613;

    /// The push consumer's whole `settle` body, not the read inside it.
    ///
    /// The transactions below both execute a settlement, so the bodies compared
    /// against them have to be settlements too. `read_cost.rs` publishes a
    /// read-against-read ratio of 448, which is the right figure for comparing
    /// what the two modes' *reads* cost; putting it beside a transaction ratio
    /// would compare a settlement with a read.
    pub const PUSH_SETTLE_BODY: u64 = 13_404;
    /// What is left of each transaction once its body and its input read are taken
    /// out: the dispatcher, the generated validator, the instruction's decode and
    /// the `SpelOutput` wrapping.
    ///
    /// A difference across harnesses rather than a measurement of its own — the
    /// transaction comes from the product ELF, the body from a cost guest and the
    /// read from `input_cost` — so it carries whatever those three disagree by.
    pub const PULL_SETTLE_REST: u64 = 289_124;
    pub const PUSH_UPDATE_REST: u64 = 259_469;
    pub const PUSH_READ_REST: u64 = 188_480;

    /// The push update's body, which `cost.rs` owns and asserts. `submit_cost`
    /// brackets it differently and this file does not re-execute it, so this one
    /// row is carried between files rather than checked across them.
    pub const PUSH_UPDATE_BODY_FROM_COST_RS: u64 = 3_048_414;

    pub use super::lez::CYCLE_BUDGET as BUDGET;

    /// What each transaction is of it, as `COSTS.md` prints it.
    pub const PULL_SETTLE_SHARE: &str = "10.51%";
    pub const PUSH_UPDATE_SHARE: &str = "10.38%";
    pub const PUSH_READ_SHARE: &str = "0.97%";

    pub use super::lez::REMOVABLE;

    /// Ten crossings for five packages, on the interface assumption `[M3-09:01]`
    /// records: a primitive that returns an address rather than a public key.
    pub const CALLS: u64 = 10;

    /// A free syscall, then `m0`'s band.
    pub const SYSCALL: [u64; 3] = [0, 1_000, 10_000];

    /// What a whole pull settlement would cost with one, and the reduction in
    /// tenths: 6.8x, 6.7x, 5.7x. Rounded rather than floored, as `read_cost.rs`
    /// rounds the body-level equivalents.
    pub const SETTLEMENT_WITH_PRECOMPILE: [u64; 3] = [515_539, 525_539, 615_539];
    pub const SETTLEMENT_REDUCTION_TENTHS: [u64; 3] = [68, 67, 57];

    /// What a byte of an account's `data` costs, measured by varying that data
    /// rather than the instruction.
    ///
    /// Its own measurement because it is its own claim: the instruction rate says
    /// nothing about the account path, which arrives through a different
    /// `env::read()` and could be priced differently. It is not.
    pub const CYCLES_PER_DATA_BYTE: u64 = 113;

    /// The stages those guests answer to, as `read_cost.rs` numbers them. Only the
    /// floor and the whole settlement are used here: a transaction executes a
    /// settlement, so the body beside it is one.
    pub const FLOOR: u8 = 0;
    pub const SETTLE: u8 = 2;

    /// A whole transaction, executed: the product ELF over the same inputs, so
    /// the dispatcher, the generated validator, the instruction's decode into its
    /// real enum and the `SpelOutput` wrapping are all inside the figure.
    ///
    /// The difference between these and body-plus-read is that wrapping, and it is
    /// not small: 289,124 on a pull settlement, 188,480 on a push read.
    pub const PULL_SETTLE_TRANSACTION: u64 = 3_525_809;
    pub const PUSH_UPDATE_TRANSACTION: u64 = 3_483_709;
    pub const PUSH_READ_TRANSACTION: u64 = 324_667;

    /// What counting the whole transaction does to the per-mode headline: a pull
    /// settlement is 227 times a push settlement as bodies, and 10 times as
    /// executed transactions.
    pub const BODIES_APART: u64 = 227;
    pub const TRANSACTIONS_APART: u64 = 10;

    /// The lengths differenced. Far enough apart that the fixed part is a rounding
    /// error, and both well inside `MAX_PAYLOAD_BYTES`.
    pub const SHORT: usize = 1_024;
    pub const LONG: usize = 16_384;
}

/// Cycles for one shape of input, and what the guest reported reading.
fn read_cost(accounts: &[AccountWithMetadata], instruction: usize) -> u64 {
    let words: Vec<u32> = vec![0x5A5A_5A5A; instruction];
    let env = ExecutorEnv::builder()
        .write(&ProgramId::from([7u32; 8]))
        .expect("self id")
        .write(&Option::<ProgramId>::None)
        .expect("caller id")
        .write(&accounts.to_vec())
        .expect("pre-states")
        .write(&words)
        .expect("instruction")
        .build()
        .expect("env");
    let session = default_executor()
        .execute(env, INPUT_COST_ELF)
        .expect("execution");
    let (pre, ins): (u32, u32) = session.journal.decode().expect("journal");
    assert_eq!(
        (pre as usize, ins as usize),
        (accounts.len(), instruction),
        "the guest reported reading {pre} accounts and {ins} words, so it did not read \
         what it was given and this is not a measurement of reading"
    );
    session.cycles()
}

/// A transaction carrying nothing still costs something to be read.
#[test]
fn an_empty_transaction_still_pays_to_be_read() {
    assert_eq!(
        read_cost(&[], 0),
        expected::FIXED,
        "the fixed part of reading a transaction's inputs moved"
    );
}

/// What a word of input costs, measured rather than carried over from ADR 26.
#[test]
fn an_input_word_costs_what_is_published() {
    let (short, long) = (
        read_cost(&[], expected::SHORT),
        read_cost(&[], expected::LONG),
    );
    let word_delta = (expected::LONG - expected::SHORT) as u64;
    assert_eq!(
        long - short,
        expected::CYCLES_PER_WORD * word_delta,
        "reading {word_delta} more words cost {} rather than {}",
        long - short,
        expected::CYCLES_PER_WORD * word_delta
    );
}

/// The rate is a rate, not an average over the range it was taken across.
///
/// Asserted as an exact total rather than a quotient: dividing first would let a
/// deviation of almost a cycle a word pass, and every figure in this file
/// multiplies by this rate.
#[test]
fn the_rate_does_not_bend_with_the_input_size() {
    let mut previous: Option<(usize, u64)> = None;
    for words in [expected::SHORT, 4_096, expected::LONG] {
        let cycles = read_cost(&[], words);
        if let Some((last_words, last_cycles)) = previous {
            let delta = (words - last_words) as u64;
            assert_eq!(
                cycles - last_cycles,
                expected::CYCLES_PER_WORD * delta,
                "between {last_words} and {words} words the read cost {} rather than {}, \
                 so it is not linear and one rate cannot describe it",
                cycles - last_cycles,
                expected::CYCLES_PER_WORD * delta
            );
        }
        previous = Some((words, cycles));
    }
}

/// Accounts cost the same as instruction words, which is why both are counted.
///
/// They arrive through a different `env::read()` and could in principle be priced
/// differently. Measuring an account-carrying input against a bare one of the same
/// total width is what says they are not.
#[test]
fn an_account_costs_more_than_its_width() {
    let f = fixtures();
    let width = words(&f.pull_accounts);
    let as_accounts = read_cost(&f.pull_accounts, 0);
    let as_instruction = read_cost(&[], width);
    assert!(
        as_accounts > as_instruction,
        "{width} words of accounts cost {as_accounts} and the same width of instruction \
         {as_instruction}: if these ever agree, an account has stopped costing more than \
         its bytes and the per-mode figures below could be counted rather than measured"
    );
    assert_eq!(
        (as_accounts - expected::FIXED) / (as_instruction - expected::FIXED),
        1,
        "an account is now more than twice its width, which is a change in what \
         deserialising one does rather than in how much of it there is"
    );
}

/// A byte of an account's data costs what an instruction word costs.
///
/// Measured by varying the `data` of one account and differencing, because the
/// instruction rate says nothing about this path: accounts arrive through their
/// own `env::read()` and are deserialised into `Data` rather than copied into a
/// vector, so the two could have differed. The table publishes 113 for both, and
/// this is the half of that claim the instruction measurement does not make.
#[test]
fn a_byte_of_account_data_costs_what_is_published() {
    let mut previous: Option<(usize, u64)> = None;
    for bytes in [64usize, 256, 1_024] {
        let account = account(ProgramId::from([7u32; 8]), vec![0xAB; bytes], [0x31; 32]);
        let cycles = read_cost(std::slice::from_ref(&account), 0);
        if let Some((last_bytes, last_cycles)) = previous {
            let delta = (bytes - last_bytes) as u64;
            assert_eq!(
                cycles - last_cycles,
                expected::CYCLES_PER_DATA_BYTE * delta,
                "between {last_bytes} and {bytes} bytes of account data the read cost {} \
                 rather than {}, so account data is not on the instruction's rate",
                cycles - last_cycles,
                expected::CYCLES_PER_DATA_BYTE * delta
            );
        }
        previous = Some((bytes, cycles));
    }
}

/// What each mode carries across the boundary, asserted.
///
/// The accounts are the half every other figure in `COSTS.md` leaves out, and on
/// the push read they are the larger half by nine times.
#[test]
fn what_crosses_the_boundary_is_what_is_published() {
    let f = fixtures();
    assert_eq!(
        (words(&f.pull_accounts), instruction_words(&f.settle)),
        expected::PULL_SETTLE,
        "what a pull settlement carries moved"
    );
    assert_eq!(
        (words(&f.submit_accounts), instruction_words(&f.submit)),
        expected::PUSH_UPDATE,
        "what a push update carries moved"
    );
    assert_eq!(
        (words(&f.read_accounts), push_read_instruction_words(&f)),
        expected::PUSH_READ,
        "what a push consumer's read carries moved"
    );
}

/// The push consumer's instruction, whose type this test cannot name.
///
/// Its enum is generated inside its guest and there is no host-side declaration to
/// encode — `[M3-06:01]` records why the pull consumer has one and this does not.
/// So it is counted from the rule rather than from the type: risc0's serde writes
/// one word for a variant tag and one per byte of a `[u8; 32]`, and `settle` takes
/// a single `feed_id: [u8; 32]`.
///
/// The rule is not assumed. The pull consumer's `Settle` is a real type carrying
/// the same tag and feed id, an order id, and a `Vec<u8>`, and it encodes to
/// exactly `1 + 32 + 32 + 1 + 724` for a 724-byte payload — which is what
/// `what_crosses_the_boundary_is_what_is_published` pins at 790. The push
/// consumer's own `settle` takes neither an order id nor a payload, so only the
/// tag and the feed id are returned.
fn push_read_instruction_words(fixtures: &Fixtures) -> usize {
    const TAG: usize = 1;
    const ID: usize = 32;
    let pull = instruction_words(&fixtures.settle);
    let payload = match &fixtures.settle {
        PullInstruction::Settle { payload, .. } => payload.len(),
        _ => unreachable!("the fixture is a settlement"),
    };
    assert_eq!(
        pull,
        TAG + ID + ID + 1 + payload,
        "risc0's serde no longer writes a tag, thirty-two words for each `[u8; 32]` \
         and a length-prefixed byte per word, so the push consumer's instruction cannot \
         be counted from that rule either"
    );
    TAG + ID
}

/// What each mode actually pays to be read, measured rather than modelled.
///
/// Modelling it was wrong, and instructively so: an account costs far more than
/// its words, because deserialising an `AccountWithMetadata` builds a `Data`, an
/// `AccountId` and a `ProgramId` rather than copying a span. So each figure here
/// is one run of the real prefix over that mode's real accounts and its real
/// instruction width, and nothing is multiplied out.
#[test]
fn what_each_mode_pays_to_be_read_is_what_is_published() {
    let f = fixtures();
    for (name, cycles, expected) in [
        (
            "pull settlement",
            read_cost(&f.pull_accounts, instruction_words(&f.settle)),
            expected::PULL_SETTLE_READ,
        ),
        (
            "push update",
            read_cost(&f.submit_accounts, instruction_words(&f.submit)),
            expected::PUSH_UPDATE_READ,
        ),
        (
            "push read",
            read_cost(&f.read_accounts, push_read_instruction_words(&f)),
            expected::PUSH_READ_READ,
        ),
    ] {
        assert_eq!(cycles, expected, "what a {name} pays to be read moved");
    }
}

/// A whole transaction: the product ELF, over the inputs LEZ would hand it.
///
/// This is the figure `read_lee_inputs` exists in front of. Nothing is modelled and
/// nothing is left out but the proving itself: the program deserialises the
/// instruction into its own enum, the generated validator checks the accounts, the
/// body runs, and `SpelOutput` wraps the result.
fn transaction(elf: &[u8], accounts: &[AccountWithMetadata], instruction: &[u32]) -> u64 {
    let env = ExecutorEnv::builder()
        .write(&ProgramId::from([7u32; 8]))
        .expect("self id")
        .write(&Option::<ProgramId>::None)
        .expect("caller id")
        .write(&accounts.to_vec())
        .expect("pre-states")
        .write(&instruction.to_vec())
        .expect("instruction")
        .build()
        .expect("env");
    default_executor()
        .execute(env, elf)
        .expect("the transaction executes; a refusal panics in the guest")
        .cycles()
}

/// The same, for a program whose id is the aggregator's rather than ours.
fn aggregator_transaction(accounts: &[AccountWithMetadata], instruction: &[u32]) -> u64 {
    let env = ExecutorEnv::builder()
        .write(&ProgramId::from([9u32; 8]))
        .expect("self id")
        .write(&Option::<ProgramId>::None)
        .expect("caller id")
        .write(&accounts.to_vec())
        .expect("pre-states")
        .write(&instruction.to_vec())
        .expect("instruction")
        .build()
        .expect("env");
    default_executor()
        .execute(env, AGGREGATOR_ELF)
        .expect("the transaction executes; a refusal panics in the guest")
        .cycles()
}

/// Every mode's whole transaction, executed and asserted.
///
/// The figure P1 is actually about. Body-plus-read is not it: the dispatcher, the
/// validator, the instruction's decode and the `SpelOutput` wrapping are between
/// 188,480 and 289,124 cycles depending on the mode, and none of them is reachable
/// by calling a function — only by running the program.
#[test]
fn what_each_mode_costs_as_a_transaction_is_what_is_published() {
    let f = fixtures();
    assert_eq!(
        transaction(
            PULL_CONSUMER_ELF,
            &f.pull_accounts,
            &risc0_zkvm::serde::to_vec(&f.settle).expect("encodes"),
        ),
        expected::PULL_SETTLE_TRANSACTION,
        "a pull settlement moved"
    );
    assert_eq!(
        aggregator_transaction(
            &f.submit_accounts,
            &risc0_zkvm::serde::to_vec(&f.submit).expect("encodes"),
        ),
        expected::PUSH_UPDATE_TRANSACTION,
        "a push update moved"
    );
    assert_eq!(
        transaction(
            AGGREGATOR_READ_CONSUMER_ELF,
            &f.read_accounts,
            &risc0_zkvm::serde::to_vec(&(7u32, f.feed_id)).expect("encodes"),
        ),
        expected::PUSH_READ_TRANSACTION,
        "a push consumer's read moved"
    );
}

/// A transaction's share of the per-transaction budget, formatted the way
/// `COSTS.md` prints it.
fn budget_share(cycles: u64) -> String {
    format!("{:.2}%", cycles as f64 * 100.0 / expected::BUDGET as f64)
}

/// Every transaction fits, and by how much.
///
/// P1 is a question about the budget, and the totals above only answer it beside
/// the limit. Asserted rather than left to the reader's division, so a re-pin that
/// pushed a transaction over the budget could not pass as an updated figure.
#[test]
fn every_transaction_fits_the_budget() {
    for (name, cycles, share) in [
        (
            "pull settlement",
            expected::PULL_SETTLE_TRANSACTION,
            expected::PULL_SETTLE_SHARE,
        ),
        (
            "push update",
            expected::PUSH_UPDATE_TRANSACTION,
            expected::PUSH_UPDATE_SHARE,
        ),
        (
            "push read",
            expected::PUSH_READ_TRANSACTION,
            expected::PUSH_READ_SHARE,
        ),
    ] {
        assert!(
            cycles < expected::BUDGET,
            "a {name} is {cycles} cycles against a {} budget, so it does not fit",
            expected::BUDGET
        );
        assert_eq!(
            budget_share(cycles),
            share,
            "a {name}'s share of the budget moved"
        );
    }
}

/// What each transaction spends outside its body and its input read.
///
/// Published as a column, so asserted as one. It is a difference across three
/// harnesses — the product ELF, a cost guest and `input_cost` — and carries
/// whatever they disagree by, which this repository has measured at tens to
/// hundreds of cycles against residuals in the hundreds of thousands.
#[test]
fn what_a_transaction_spends_outside_its_body_is_what_is_published() {
    let f = fixtures();
    let pull_body = pull_body(expected::SETTLE) - pull_body(expected::FLOOR);
    let push_body = push_body(expected::SETTLE) - push_body(expected::FLOOR);

    for (name, whole, body, read, rest) in [
        (
            "pull settlement",
            expected::PULL_SETTLE_TRANSACTION,
            pull_body,
            read_cost(&f.pull_accounts, instruction_words(&f.settle)),
            expected::PULL_SETTLE_REST,
        ),
        (
            "push update",
            expected::PUSH_UPDATE_TRANSACTION,
            expected::PUSH_UPDATE_BODY_FROM_COST_RS,
            read_cost(&f.submit_accounts, instruction_words(&f.submit)),
            expected::PUSH_UPDATE_REST,
        ),
        (
            "push read",
            expected::PUSH_READ_TRANSACTION,
            push_body,
            read_cost(&f.read_accounts, push_read_instruction_words(&f)),
            expected::PUSH_READ_REST,
        ),
    ] {
        assert_eq!(
            whole - body - read,
            rest,
            "what a {name} spends outside its body and its read moved"
        );
    }
}

/// A body, from the guest that owns it, floor subtracted the way `read_cost.rs`
/// subtracts it.
fn pull_body(stage: u8) -> u64 {
    let f = fixtures();
    let vector = vectors::named("BTC");
    let input = (
        stage,
        vector.payload.clone(),
        f.pull_order.clone(),
        f.trust.clone(),
        f.clock.clone(),
        CLOCK_ACCOUNT_ID.to_vec(),
        f.feed_id.to_vec(),
        [7u32; 8],
    );
    execute(PULL_COST_ELF, &input, stage)
}

fn push_body(stage: u8) -> u64 {
    let f = fixtures();
    let input = (
        stage,
        f.push_order.clone(),
        f.source.clone(),
        f.price.clone(),
        f.clock.clone(),
        CLOCK_ACCOUNT_ID.to_vec(),
        f.price_id.to_vec(),
        f.feed_id.to_vec(),
        [9u32; 8],
        [7u32; 8],
    );
    execute(READ_COST_ELF, &input, stage)
}

/// Runs a cost guest and insists it did the work, because a refusal is cheaper
/// than the thing being measured and would read as a small figure.
///
/// The stage decides whether no work is allowed. Stage 0 does none by definition;
/// every other stage reporting none took an error path. An earlier version guessed
/// at this from the cycle count, which accepted any refusal cheap enough to look
/// like a floor.
fn execute<T: serde_crate::Serialize>(elf: &[u8], input: &T, stage: u8) -> u64 {
    let env = ExecutorEnv::builder()
        .write(input)
        .expect("input")
        .build()
        .expect("env");
    let session = default_executor().execute(env, elf).expect("execution");
    let (did_work, _value, _post_states): (u32, u64, u32) =
        session.journal.decode().expect("journal");
    assert!(
        did_work > 0 || stage == expected::FLOOR,
        "stage {stage} reported no work, so it took an error path and this is not a \
         measurement of the work"
    );
    session.cycles()
}

/// The boundary does not qualify the per-mode gap; it changes its order.
///
/// A pull settlement's body is 227 times a push settlement's. As executed
/// transactions the two are ten times apart, because a push settlement is 13,404
/// cycles of body inside a 324,667-cycle transaction — everything else is being
/// read and being dispatched, and neither scales with what the body does. That is
/// the difference between "the body fits" and "the transaction fits", and it is
/// what P1 asks for.
///
/// Both sides are settlements. `read_cost.rs`'s 448 is a read against a read,
/// which answers a different question and does not belong beside a ratio of
/// transactions.
#[test]
fn counting_the_whole_transaction_closes_the_gap_by_an_order_of_magnitude() {
    // Both stage 2: the transactions below execute settlements, so the bodies put
    // beside them are settlements. Comparing a pull settlement with a push *read*
    // is what `read_cost.rs` does, and it is the right comparison there and the
    // wrong one here.
    let pull_body = pull_body(expected::SETTLE) - pull_body(expected::FLOOR);
    let push_body = push_body(expected::SETTLE) - push_body(expected::FLOOR);

    // Executed rather than copied: these are the same guests and stages
    // `read_cost.rs` measures, so a disagreement between the two files is a real
    // one rather than a stale constant nothing would notice.
    assert_eq!(
        (pull_body, push_body),
        (expected::PULL_SETTLE_BODY, expected::PUSH_SETTLE_BODY),
        "a body moved, and it has moved in `read_cost.rs` too"
    );
    assert_eq!(
        pull_body / push_body,
        expected::BODIES_APART,
        "the gap between the bodies moved"
    );
    assert_eq!(
        expected::PULL_SETTLE_TRANSACTION / expected::PUSH_READ_TRANSACTION,
        expected::TRANSACTIONS_APART,
        "the gap between the transactions moved"
    );
}

/// What a precompile is worth against a whole transaction, not against a body.
///
/// M3-09 prices it against the `settle` body, which is the right figure for
/// comparing modes and the wrong one for scoping the work: a precompile touches
/// neither the input read nor the dispatch and wrapping around the body, and those
/// are 465,105 cycles of a pull settlement. So the reduction it buys on a
/// transaction is a fraction of what the body figures suggest.
///
/// Like every difference between a transaction and a body here, this subtracts a
/// figure from `verify_cost` out of one from the product ELF, so it carries their
/// disagreement too — 42 to 722 cycles where this file has measured it, against a
/// residual of half a million.
#[test]
fn a_precompile_is_worth_less_against_a_transaction_than_against_a_body() {
    let f = fixtures();
    let whole = transaction(
        PULL_CONSUMER_ELF,
        &f.pull_accounts,
        &risc0_zkvm::serde::to_vec(&f.settle).expect("encodes"),
    );
    assert_eq!(
        whole,
        expected::PULL_SETTLE_TRANSACTION,
        "the settlement moved"
    );

    for (i, syscall) in expected::SYSCALL.into_iter().enumerate() {
        let remains = whole
            .checked_sub(expected::REMOVABLE)
            .expect("a settlement contains the verification whose rows are removed")
            + expected::CALLS * syscall;
        assert_eq!(
            remains,
            expected::SETTLEMENT_WITH_PRECOMPILE[i],
            "a settlement with a precompile at {syscall} cycles a call moved"
        );
        assert_eq!(
            (whole * 10 + remains / 2) / remains,
            expected::SETTLEMENT_REDUCTION_TENTHS[i],
            "the reduction at {syscall} cycles a call moved"
        );
    }
}

/// Prints the table `COSTS.md` publishes.
///
/// ```sh
/// cargo test --release -p kanon-methods --test bytes -- --nocapture the_boundary_table
/// ```
#[test]
fn the_boundary_table_is_reproducible() {
    let f = fixtures();
    let pull_body = pull_body(expected::SETTLE) - pull_body(expected::FLOOR);
    // Stage 2, so the row is a settlement beside a settlement's transaction.
    let push_body = push_body(expected::SETTLE) - push_body(expected::FLOOR);
    let rows = [
        (
            "pull settlement",
            expected::PULL_SETTLE,
            read_cost(&f.pull_accounts, instruction_words(&f.settle)),
            pull_body,
            expected::PULL_SETTLE_TRANSACTION,
        ),
        (
            "push update",
            expected::PUSH_UPDATE,
            read_cost(&f.submit_accounts, instruction_words(&f.submit)),
            // `cost.rs` owns and asserts this body; `submit_cost` brackets it
            // differently and this file does not re-execute it, so the column is
            // left to the file that measures it.
            0,
            expected::PUSH_UPDATE_TRANSACTION,
        ),
        (
            "push read",
            expected::PUSH_READ,
            read_cost(&f.read_accounts, push_read_instruction_words(&f)),
            push_body,
            expected::PUSH_READ_TRANSACTION,
        ),
    ];
    println!("\n| | accounts | instruction | to be read | body | transaction |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    for (name, (pre, ins), read, body, whole) in rows {
        let body = if body == 0 {
            "see `cost.rs`".to_owned()
        } else {
            body.to_string()
        };
        println!("| {name} | {pre} | {ins} | {read} | {body} | {whole} |");
    }
    println!("\n| the read, decomposed | cycles |");
    println!("| --- | ---: |");
    println!(
        "| a read of four empty inputs, guest floor included | {} |",
        read_cost(&[], 0)
    );
    println!("| an instruction word | {} |", expected::CYCLES_PER_WORD);
    println!(
        "| a byte of an account's data | {} |\n",
        expected::CYCLES_PER_DATA_BYTE
    );

    let whole = expected::PULL_SETTLE_TRANSACTION;
    println!("| a pull settlement, whole | cycles | reduction |");
    println!("| --- | ---: | ---: |");
    println!("| today | {whole} | |");
    for (i, syscall) in expected::SYSCALL.into_iter().enumerate() {
        let remains = expected::SETTLEMENT_WITH_PRECOMPILE[i];
        let tenths = expected::SETTLEMENT_REDUCTION_TENTHS[i];
        println!(
            "| with a precompile at {syscall} cycles a call | {remains} | {}.{}x |",
            tenths / 10,
            tenths % 10
        );
    }
    println!();
}

/// The words a pre-state vector costs to read.
fn words(accounts: &[AccountWithMetadata]) -> usize {
    risc0_zkvm::serde::to_vec(&accounts.to_vec())
        .expect("pre-states encode")
        .len()
}

/// The words an instruction costs, encoded the way the dispatcher decodes it.
fn instruction_words<T: serde_crate::Serialize>(instruction: &T) -> usize {
    risc0_zkvm::serde::to_vec(instruction)
        .expect("the instruction encodes")
        .len()
}

fn account(owner: ProgramId, data: Vec<u8>, id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance: 0,
            data: Data::try_from(data).expect("fits"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: AccountId::new(id),
    }
}

/// Borsh, because that is what an account's `data` holds.
fn encoded<T: BorshSerialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.serialize(&mut out).expect("it encodes");
    out
}

const BASE: [u8; 32] = [0xB7; 32];
const QUOTE: [u8; 32] = [0x05; 32];
const OWNER: [u8; 32] = [0x41; 32];
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 300_000;
const THRESHOLD: u8 = 3;

/// Everything the accounting is taken over, built once.
struct Fixtures {
    pull_accounts: [AccountWithMetadata; 3],
    submit_accounts: [AccountWithMetadata; 3],
    read_accounts: [AccountWithMetadata; 4],
    settle: PullInstruction,
    submit: AggregatorInstruction,
    // The same bytes, unwrapped, because the cost guests take account data rather
    // than accounts.
    pull_order: Vec<u8>,
    trust: Vec<u8>,
    push_order: Vec<u8>,
    source: Vec<u8>,
    price: Vec<u8>,
    clock: Vec<u8>,
    feed_id: [u8; 32],
    price_id: [u8; 32],
}

fn fixtures() -> Fixtures {
    let vector = vectors::named("BTC");
    let mut feed_id = [0u8; 32];
    let bytes = vector.feed_id.as_bytes();
    feed_id[..bytes.len()].copy_from_slice(bytes);
    let ours = ProgramId::from([7u32; 8]);
    let aggregator = ProgramId::from([9u32; 8]);
    let signers: Vec<[u8; 20]> = vector.signers.iter().map(|s| *s.as_bytes()).collect();

    let trust = encoded(&FeedTrust {
        data_service_id: "redstone-primary-prod".to_owned(),
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers: signers.clone(),
        threshold: THRESHOLD,
    });
    let pull_order = encoded(&PullOrder {
        owner: OWNER,
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        limit_price_q64: 1,
        filled: false,
    });
    let feed = encoded(&FeedAccount {
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        decimals: DECIMALS,
        max_age_ms: MAX_AGE_MS,
        signers,
        threshold: THRESHOLD,
        paused: false,
    });
    let price = encoded(&OraclePriceAccount {
        base_asset: IdlAccountId::new(BASE),
        quote_asset: IdlAccountId::new(QUOTE),
        price: (65_000u128 << 64) + 1,
        timestamp: vector.timestamp_ms,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    });
    let source = encoded(&PriceSource {
        feed_id,
        aggregator,
        base_asset: BASE,
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS,
    });
    let push_order = encoded(&PushOrder {
        owner: OWNER,
        feed_id,
        base_asset: BASE,
        quote_asset: QUOTE,
        max_age_ms: MAX_AGE_MS,
        limit_price_q64: 1,
        filled: false,
    });
    let clock = {
        let mut out = 1u64.to_le_bytes().to_vec();
        out.extend_from_slice(&vector.timestamp_ms.to_le_bytes());
        out
    };

    // The address the registration publishes to, derived the way the consumer
    // derives it: the push guest's read checks it first, so a stand-in id would
    // make that stage measure a refusal.
    let price_id = *price_account_address(&aggregator, &feed_id).value();
    let trust_id = *trust_address(&ours, &feed_id).value();
    // The order account is at its derived address like the rest, which `settle`'s
    // `pda` constraint now requires: the transaction runs the product ELF and the
    // generated validator refuses an account anywhere else.
    let order_id = [0x31u8; 32];
    let order_acct_id = *spel_framework::pda::compute_pda(
        &ours,
        &[
            &order_id,
            &spel_framework::pda::seed_from_str(reference_consumer_pull::ORDER_ACCOUNT_SEED),
        ],
    )
    .value();
    let source_id = *source_address(&ours, &feed_id).value();
    // The feed lives at its own PDA and the price account derives from *that*
    // address rather than from the feed id (ADR 32).
    let feed_acct_id = *spel_framework::pda::compute_pda(
        &aggregator,
        &[
            &feed_id,
            &spel_framework::pda::seed_from_str("KANON_FEED_ACCOUNT"),
        ],
    )
    .value();
    // A minute older, so a submission against it is an update rather than a
    // replay: the recurring case, and the one M2-18 measures.
    let stored = encoded(&OraclePriceAccount {
        base_asset: IdlAccountId::new(BASE),
        quote_asset: IdlAccountId::new(QUOTE),
        price: (64_000u128 << 64) + 1,
        timestamp: vector.timestamp_ms - 60_000,
        source_id: REDSTONE_SOURCE_ID,
        confidence_interval: 0,
    });

    Fixtures {
        pull_accounts: [
            account(ours, pull_order.clone(), order_acct_id),
            account(ours, trust.clone(), trust_id),
            account(ProgramId::default(), clock.clone(), CLOCK_ACCOUNT_ID),
        ],
        submit_accounts: [
            account(aggregator, feed, feed_acct_id),
            account(aggregator, stored, price_id),
            account(ProgramId::default(), clock.clone(), CLOCK_ACCOUNT_ID),
        ],
        read_accounts: [
            account(ours, push_order.clone(), [0x51; 32]),
            account(ours, source.clone(), source_id),
            account(aggregator, price.clone(), price_id),
            account(ProgramId::default(), clock.clone(), CLOCK_ACCOUNT_ID),
        ],
        settle: PullInstruction::Settle {
            order_id,
            feed_id,
            payload: vector.payload.clone(),
        },
        submit: AggregatorInstruction::SubmitPrice {
            payload: vector.payload,
        },
        pull_order,
        trust,
        push_order,
        source,
        price,
        clock,
        feed_id,
        price_id,
    }
}
