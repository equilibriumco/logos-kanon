//! Prices, and the median across signers.
//!
//! A price arrives as a big-endian field whose width is declared per package, so
//! two signers reporting the same number can encode it in different numbers of
//! bytes. Everything here works on one fixed width instead, which is what makes
//! the derived [`Ord`] numeric rather than lexicographic-over-a-variable-length
//! slice.
//!
//! The median follows RedStone's own definition, including the overflow-safe
//! average on even counts.
//!
//! RedStone and LEZ disagree about how a fractional price is written. RedStone
//! scales by a power of ten; the LEZ price account is `Q64.64`, so the real
//! price is the stored integer over `2^64`. Nothing on chain carries the
//! exponent to reconcile them, so [`Value::to_q64_64`] converts and the
//! convention is documented rather than negotiated.

/// The largest decimal exponent a feed may declare.
///
/// `10^19 < 2^64 < 10^20`. Stopping here keeps `10^decimals` inside a `u64`,
/// which is what lets the conversion divide by a single-limb divisor instead of
/// implementing full 256-bit division.
pub const MAX_DECIMALS: u8 = 19;

/// `10^i` for every exponent a feed may declare.
const POW10: [u64; MAX_DECIMALS as usize + 1] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
    100_000_000_000_000_000,
    1_000_000_000_000_000_000,
    10_000_000_000_000_000_000,
];

/// A price: 32 bytes, big-endian, left-padded from whatever width the wire used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Value(pub [u8; Value::LEN]);

impl Value {
    /// Width in bytes. Matches RedStone's own value width.
    pub const LEN: usize = 32;

    /// One, for the midpoint's carry.
    const fn one() -> Self {
        let mut bytes = [0u8; Self::LEN];
        bytes[Self::LEN - 1] = 1;
        Self(bytes)
    }

    /// Left-pads a big-endian slice into a [`Value`].
    ///
    /// Leading zero bytes are padding rather than magnitude, so they are dropped
    /// before the width is checked: a 32-byte field holding a small number is the
    /// common case on the wire. Returns `None` when more than [`Self::LEN`]
    /// significant bytes remain, which no `Value` can hold.
    #[must_use]
    pub fn from_be_slice(bytes: &[u8]) -> Option<Self> {
        let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
        let digits = bytes.get(first..)?;
        if digits.len() > Self::LEN {
            return None;
        }
        let mut out = [0u8; Self::LEN];
        let start = Self::LEN - digits.len();
        out.get_mut(start..)?.copy_from_slice(digits);
        Some(Self(out))
    }

    /// Whether the value is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; Self::LEN]
    }

    /// `self >> 1`.
    fn halved(&self) -> Self {
        let mut out = [0u8; Self::LEN];
        let mut carry = 0u8;
        for (slot, &byte) in out.iter_mut().zip(self.0.iter()) {
            *slot = (byte >> 1) | carry;
            carry = (byte & 1) << 7;
        }
        Self(out)
    }

    fn is_odd(&self) -> bool {
        self.0[Self::LEN - 1] & 1 == 1
    }

    /// Big-endian addition, wrapping on overflow.
    ///
    /// Only ever called on halved operands and a carry of one, where the sum
    /// cannot exceed [`Self::LEN`] bytes, so the wrap is unreachable rather than
    /// tolerated.
    fn wrapping_add(&self, other: &Self) -> Self {
        let mut out = [0u8; Self::LEN];
        let mut carry = 0u16;
        for index in (0..Self::LEN).rev() {
            let sum = u16::from(self.0[index]) + u16::from(other.0[index]) + carry;
            out[index] = sum.to_le_bytes()[0];
            carry = sum >> 8;
        }
        Self(out)
    }

    /// Whether the top bit is set, which is what a negative price looks like.
    ///
    /// The wire field is unsigned, so nothing distinguishes a genuinely enormous
    /// price from a small negative one read as `int256`. No real feed reports a
    /// value this large, so treating the whole upper half as unusable costs
    /// nothing and is the only reading under which "negative price" means
    /// anything here.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.0[0] & 0x80 != 0
    }

    /// `self << 64`, or `None` when that does not fit.
    fn shifted_left_64(&self) -> Option<Self> {
        const LIMB: usize = 8;
        if self.0.iter().take(LIMB).any(|&byte| byte != 0) {
            return None;
        }
        let mut out = [0u8; Self::LEN];
        out.get_mut(..Self::LEN - LIMB)?
            .copy_from_slice(self.0.get(LIMB..)?);
        Some(Self(out))
    }

    /// `self / divisor`, truncating, for a single-limb divisor.
    ///
    /// Schoolbook long division, one output byte per step. The running remainder
    /// stays below `divisor`, so `remainder << 8 | byte` needs a `u128` to hold
    /// it and the quotient digit it yields is always below 256.
    fn divided_by(&self, divisor: u64) -> Self {
        let mut out = [0u8; Self::LEN];
        let mut remainder = 0u128;
        for (slot, &byte) in out.iter_mut().zip(self.0.iter()) {
            let accumulated = (remainder << 8) | u128::from(byte);
            *slot = (accumulated / u128::from(divisor)).to_le_bytes()[0];
            remainder = accumulated % u128::from(divisor);
        }
        Self(out)
    }

    /// The low 16 bytes as a `u128`, or `None` when the value is wider.
    fn to_u128(self) -> Option<u128> {
        const WIDTH: usize = 16;
        if self.0.iter().take(Self::LEN - WIDTH).any(|&byte| byte != 0) {
            return None;
        }
        let low: [u8; WIDTH] = self.0.get(Self::LEN - WIDTH..)?.try_into().ok()?;
        Some(u128::from_be_bytes(low))
    }

    /// This value, read as `10^-decimals` units, converted to LEZ `Q64.64`.
    ///
    /// `(self << 64) / 10^decimals`, truncating toward zero, so the relative
    /// error is below `2^-64`.
    ///
    /// Returns `None` when the result needs more than a `u128`, and for a
    /// `decimals` above [`MAX_DECIMALS`], which a checked feed configuration has
    /// already excluded. The shift and the narrowing fail together in practice:
    /// a result within `u128` implies an input below `2^128`, well under the
    /// `2^192` the shift needs.
    ///
    /// Never returns `Some(0)` for a non-zero value. Zero is the price account's
    /// "no valid price" sentinel, and `2^64` exceeding `10^MAX_DECIMALS` is what
    /// puts the smallest non-zero input at `1` rather than at the sentinel.
    #[must_use]
    pub fn to_q64_64(&self, decimals: u8) -> Option<u128> {
        let divisor = *POW10.get(usize::from(decimals))?;
        self.shifted_left_64()?.divided_by(divisor).to_u128()
    }

    /// The midpoint of two values, without overflowing.
    ///
    /// `(a + b) / 2` would overflow 32 bytes, so this is
    /// `(a >> 1) + (b >> 1) + 1` when both operands are odd. RedStone's own form,
    /// and the reason its median can average the two middle values of a full-width
    /// set at all.
    #[must_use]
    pub fn midpoint(&self, other: &Self) -> Self {
        let halves = self.halved().wrapping_add(&other.halved());
        if self.is_odd() && other.is_odd() {
            halves.wrapping_add(&Self::one())
        } else {
            halves
        }
    }
}

/// The median of `values`, as RedStone defines it.
///
/// An even count averages the two middle values rather than taking the lower of
/// them, which would bias every even-signer feed downwards.
///
/// Sorts `values` in place: the caller owns the buffer, and `verifier-core` has
/// no allocator to copy it into. `core`'s `sort_unstable` needs none either.
///
/// `None` only for an empty slice.
#[must_use]
pub fn median(values: &mut [Value]) -> Option<Value> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();

    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        let lower = values.get(middle - 1)?;
        let upper = values.get(middle)?;
        Some(lower.midpoint(upper))
    } else {
        values.get(middle).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_and_a_wide_encoding_of_the_same_number_are_equal() {
        // Two packages may declare different `value_size` for the same feed.
        // Comparing their raw slices would order a 4-byte value above a 32-byte
        // one, so both are left-padded to a fixed width first.
        let narrow = Value::from_be_slice(&[1, 0]).expect("2 bytes fit");

        let mut wide_bytes = [0u8; 32];
        wide_bytes[30] = 1;
        let wide = Value::from_be_slice(&wide_bytes).expect("32 bytes fit");

        assert_eq!(narrow, wide, "256 is 256 at either wire width");
        assert_eq!(narrow.0, wide_bytes);
    }

    #[test]
    fn ordering_is_numeric_not_lexicographic_on_the_wire_width() {
        let small = Value::from_be_slice(&[0xFF]).expect("fits");
        let large = Value::from_be_slice(&[0x01, 0x00]).expect("fits");
        assert!(small < large, "255 must order below 256");
    }

    #[test]
    fn leading_zeros_are_padding_and_do_not_count_against_the_width() {
        // RedStone commonly writes a 32-byte field holding a small number, so a
        // 40-byte field of mostly zeros is representable.
        let mut wide = [0u8; 40];
        wide[39] = 7;
        assert_eq!(
            Value::from_be_slice(&wide).map(|v| v.0[31]),
            Some(7),
            "only significant bytes count"
        );
    }

    #[test]
    fn a_value_too_wide_to_represent_is_rejected_rather_than_truncated() {
        let mut wide = [0u8; 33];
        wide[0] = 1;
        assert_eq!(Value::from_be_slice(&wide), None);
    }

    #[test]
    fn zero_is_recognised_at_any_wire_width() {
        assert!(Value::from_be_slice(&[]).expect("empty is zero").is_zero());
        assert!(Value::from_be_slice(&[0; 32]).expect("fits").is_zero());
        assert!(!Value::from_be_slice(&[1]).expect("fits").is_zero());
    }

    #[test]
    fn the_midpoint_of_the_two_largest_values_does_not_overflow() {
        // The reason RedStone uses (a >> 1) + (b >> 1) + carry rather than
        // (a + b) / 2: the sum does not fit in 32 bytes.
        let max = Value([0xFF; 32]);
        assert_eq!(max.midpoint(&max), max, "midpoint of x and x is x");
    }

    #[test]
    fn the_midpoint_rounds_the_way_redstone_rounds() {
        let three = Value::from_be_slice(&[3]).expect("fits");
        let four = Value::from_be_slice(&[4]).expect("fits");
        let five = Value::from_be_slice(&[5]).expect("fits");

        // 3 and 4 -> 3: one odd operand contributes no carry.
        assert_eq!(three.midpoint(&four), three);
        // 3 and 5 -> 4: both odd, so the carry lands.
        assert_eq!(three.midpoint(&five), four);
        // 4 and 4 -> 4.
        assert_eq!(four.midpoint(&four), four);
    }

    #[test]
    fn the_midpoint_carries_across_byte_boundaries() {
        let a = Value::from_be_slice(&[0x01, 0x00]).expect("fits"); // 256
        let b = Value::from_be_slice(&[0x02, 0x00]).expect("fits"); // 512
        assert_eq!(
            a.midpoint(&b),
            Value::from_be_slice(&[0x01, 0x80]).expect("fits"),
            "384"
        );
    }

    fn v(n: u8) -> Value {
        Value::from_be_slice(&[n]).expect("one byte fits")
    }

    fn from_u128(n: u128) -> Value {
        Value::from_be_slice(&n.to_be_bytes()).expect("16 bytes fit")
    }

    #[test]
    fn one_whole_unit_converts_to_the_q64_64_representation_of_one() {
        // The definition, from the account's own constructor: 1.0 is 1 << 64.
        let one = from_u128(100_000_000);
        assert_eq!(one.to_q64_64(8), Some(1u128 << 64));
    }

    #[test]
    fn a_realistic_price_survives_the_conversion() {
        // $3000.12345678 as RedStone writes it at eight decimals.
        let price = from_u128(300_012_345_678);
        assert_eq!(price.to_q64_64(8), Some(55_342_509_596_753_479_111_897));
    }

    #[test]
    fn a_zero_exponent_is_a_plain_left_shift() {
        assert_eq!(v(5).to_q64_64(0), Some(5u128 << 64));
    }

    #[test]
    fn the_conversion_truncates_toward_zero() {
        // 1/10 in Q64.64 is not representable, and rounding up would let a price
        // land above what the signers actually agreed on.
        assert_eq!(v(1).to_q64_64(1), Some(1_844_674_407_370_955_161));
    }

    #[test]
    fn no_non_zero_price_can_reach_the_sentinel() {
        // Zero means "no valid price" to every consumer of the account, so the
        // smallest input at the largest exponent is the case that matters:
        // 2^64 > 10^19 is what keeps it at one rather than at zero.
        assert_eq!(v(1).to_q64_64(MAX_DECIMALS), Some(1));
        for decimals in 0..=MAX_DECIMALS {
            assert_ne!(v(1).to_q64_64(decimals), Some(0), "at 10^-{decimals}");
        }
    }

    #[test]
    fn a_price_too_large_for_the_account_is_rejected_rather_than_wrapped() {
        // The boundary: 2^64 - 1 is the largest whole number of units a Q64.64
        // u128 can hold, and one more does not fit.
        assert!(from_u128(u128::from(u64::MAX)).to_q64_64(0).is_some());
        assert_eq!(from_u128(1u128 << 64).to_q64_64(0), None);
    }

    #[test]
    fn a_value_too_wide_to_shift_is_rejected_rather_than_truncated() {
        let mut wide = [0u8; 32];
        wide[7] = 1; // 2^192, the first value the shift cannot hold
        assert_eq!(Value(wide).to_q64_64(0), None);
    }

    #[test]
    fn an_exponent_beyond_the_supported_range_yields_nothing() {
        // A checked configuration excludes this, so the guard is what keeps the
        // table lookup from being the one panicking index in the crate.
        assert_eq!(v(1).to_q64_64(MAX_DECIMALS + 1), None);
        assert_eq!(v(1).to_q64_64(u8::MAX), None);
    }

    #[test]
    fn the_top_bit_marks_a_value_no_price_should_reach() {
        assert!(!v(1).is_negative());
        assert!(!from_u128(u128::MAX).is_negative());

        let mut top = [0u8; 32];
        top[0] = 0x80;
        assert!(Value(top).is_negative(), "-1 as int256 is 2^255");
        assert!(Value([0xFF; 32]).is_negative());
    }

    #[test]
    fn the_median_of_an_empty_set_is_nothing() {
        assert_eq!(median(&mut []), None);
    }

    #[test]
    fn an_odd_count_takes_the_middle_value() {
        assert_eq!(median(&mut [v(9)]), Some(v(9)));
        assert_eq!(median(&mut [v(1), v(5), v(9)]), Some(v(5)));
        assert_eq!(median(&mut [v(1), v(2), v(3), v(4), v(5)]), Some(v(3)));
    }

    #[test]
    fn an_even_count_averages_the_two_middle_values() {
        // RedStone's definition. Not "the lower of the two", which is the easy
        // mistake and silently biases every even-signer feed downwards.
        assert_eq!(median(&mut [v(2), v(4)]), Some(v(3)));
        assert_eq!(median(&mut [v(1), v(2), v(4), v(9)]), Some(v(3)));
    }

    #[test]
    fn the_input_order_does_not_matter() {
        // Packages are walked last-first and nothing downstream may depend on
        // payload order, so the median must not either.
        let ascending = median(&mut [v(1), v(3), v(7), v(9)]);
        let descending = median(&mut [v(9), v(7), v(3), v(1)]);
        let shuffled = median(&mut [v(7), v(1), v(9), v(3)]);

        assert_eq!(ascending, descending);
        assert_eq!(ascending, shuffled);
        assert_eq!(ascending, Some(v(5)), "midpoint of 3 and 7");
    }

    #[test]
    fn duplicate_values_do_not_disturb_the_median() {
        assert_eq!(median(&mut [v(4), v(4), v(4)]), Some(v(4)));
        assert_eq!(median(&mut [v(4), v(4), v(4), v(4)]), Some(v(4)));
    }
}
