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
}
