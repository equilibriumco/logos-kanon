//! What a settlement costs before the program is entered.
//!
//! LEZ reads a program's whole instruction data into guest memory before its first
//! instruction, and `COSTS.md` publishes what that comes to for this consumer — so
//! the size it multiplies is measured here rather than reasoned about, because
//! there are two ways to get it wrong and this file has been caught by both.
//!
//! **The codec is `risc0_zkvm::serde`, not Borsh.** A reader assuming Borsh puts
//! the instruction at 761 bytes: a variant tag, the feed id, a length prefix and
//! the payload. Serde writes `u32` words and packs neither the `[u8; 32]` nor the
//! `Vec<u8>` into them, so it is 758 words.
//!
//! **And the per-unit cost is per word, not per physical byte.** ADR 26 derives
//! its ~113 from one measurement: a 127,814-byte payload whose read cost 14.5M
//! cycles. That payload is about 127,814 *words* once serde has finished with it,
//! so 113 is what a word costs, and multiplying the 3,032 physical bytes by it
//! would count each word four times — 57.8M against a measured 14.5M, on ADR 26's
//! own experiment.

#[path = "../../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use reference_consumer_pull::instruction::Instruction;

/// The captured payload, and the feed id it is verified against.
fn settle() -> Instruction {
    let vector = vectors::named("BTC");
    let mut feed_id = [0u8; 32];
    let bytes = vector.feed_id.as_bytes();
    feed_id[..bytes.len()].copy_from_slice(bytes);
    Instruction::Settle {
        feed_id,
        payload: vector.payload.clone(),
    }
}

/// What `COSTS.md` publishes, pinned so the two cannot drift.
///
/// Exact equality for the reason every cycle figure is: the encoding is a
/// deterministic function of the capture and the codec, so a range would only
/// hide a change. A failure means one of those moved.
#[test]
fn a_settlement_carries_what_is_published() {
    let words = risc0_zkvm::serde::to_vec(&settle()).expect("the instruction encodes");
    assert_eq!(
        words.len(),
        758,
        "the encoded `Settle` moved, and with it what LEZ reads before `settle` begins"
    );
    assert_eq!(
        words.len() * 4,
        3_032,
        "a word is four bytes; this is what the instruction occupies, and is *not* what \
         the ~113 multiplies -- see the module comment"
    );
}

/// Printed by the code that asserts it, like every figure `COSTS.md` carries.
///
/// ```sh
/// cargo test -p reference-consumer-pull --test wire -- --nocapture the_wire
/// ```
#[test]
fn the_wire_size_is_reproducible() {
    let vector = vectors::named("BTC");
    let words = risc0_zkvm::serde::to_vec(&settle()).expect("the instruction encodes");
    println!("\n| a `Settle` on the wire | |");
    println!("| --- | ---: |");
    println!("| the captured payload | {} bytes |", vector.payload.len());
    println!("| encoded, risc0 serde | {} u32 words |", words.len());
    println!("| which occupies | {} bytes |", words.len() * 4);
    println!(
        "| at about 113 cycles a word (ADR 26) | ~{} cycles |\n",
        words.len() * 113
    );
}
