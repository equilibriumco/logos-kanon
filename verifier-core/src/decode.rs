//! The RedStone data-package wire format.
//!
//! # The shape of a payload
//!
//! A RedStone payload is parsed **from the end**. That is not a quirk: on EVM
//! the payload is appended to a transaction's calldata, so the front of the
//! buffer belongs to the call and only the tail is RedStone's. Everything here
//! reads backwards from the last byte, and every field is a fixed width so the
//! walk needs no separators.
//!
//! ```text
//!  ┌──────────────── one data package, repeated ────────────────┐
//!  │ data points │ timestamp │ value size │ point count │ sig   │  … │ pkg count │ unsigned metadata │ md size │ marker │
//!  │   variable  │    6      │     4      │      3      │  65   │    │     2     │      variable     │    3    │   9    │
//!  └────────────────────────────────────────────────────────────┘
//! ```
//!
//! and each data point is a 32-byte feed id followed by a value of `value size`
//! bytes. Multi-byte integers are big-endian.
//!
//! # What is signed
//!
//! The signature covers the data points **and** the timestamp, value size and
//! point count that follow them — but not itself. Getting that span wrong is the
//! single most consequential mistake available in this file: a decoder that
//! signs too little would let an attacker rewrite the excluded field, and one
//! that signs too much would reject every genuine package. [`DataPackage::signable`]
//! is that span, and it is what [`crate::VerifierBackend::keccak256`] is fed.
//!
//! # Provenance
//!
//! Field widths and ordering follow RedStone's own Rust SDK, which is Boost
//! licensed. The BUSL-1.1 EVM connector was deliberately not read; see `NOTICE`.
//!
//! # What this module does not do
//!
//! It decodes and it bounds-checks. It does not recover signers, count them
//! against a threshold, or look at the clock — those are separate steps, and
//! keeping them out means a malformed payload and an unauthorised one fail in
//! different places for different reasons.

use crate::backend::Signature;

/// The 9-byte marker every RedStone payload ends with, `0x000002ed57011e0000`.
///
/// A version tag as much as a magic number: it is how a consumer tells a
/// RedStone payload from arbitrary trailing calldata.
pub const REDSTONE_MARKER: [u8; 9] = [0, 0, 2, 237, 87, 1, 30, 0, 0];

/// Width of a data feed identifier, right-padded ASCII (`"BTC"` and 29 zeros).
pub const FEED_ID_BYTES: usize = 32;

const MARKER_BYTES: usize = 9;
const UNSIGNED_METADATA_SIZE_BYTES: usize = 3;
const PACKAGE_COUNT_BYTES: usize = 2;
const POINT_COUNT_BYTES: usize = 3;
const VALUE_SIZE_BYTES: usize = 4;
const TIMESTAMP_BYTES: usize = 6;
const SIGNATURE_BYTES: usize = Signature::LEN;

/// Bytes after the last package, when there is no unsigned metadata: the
/// package count, the metadata section's own length, and the marker.
///
/// Public because assembling or trimming a payload needs it, and every place
/// that restated the sum was a place it could drift from the decoder.
pub const EMPTY_ENVELOPE_BYTES: usize =
    PACKAGE_COUNT_BYTES + UNSIGNED_METADATA_SIZE_BYTES + MARKER_BYTES;

/// The longest payload this decoder will read.
///
/// Not a framing rule: a longer payload can be perfectly well formed. It is a
/// cost rule, and the only one this crate is in a position to state.
///
/// Getting a payload into guest memory costs about 113 cycles per byte, before
/// any verification and before this function is called. LEZ allows 33,554,432
/// cycles in a public transaction and `MAX_RECOVERIES` bounds verification at
/// about 19.4M of them, which leaves roughly 125 KB of payload before the two
/// together exhaust the budget -- with nothing left for the program doing the
/// verifying. 32 KiB is a third of that: the read costs about 3.7M cycles, the
/// pair comes to 69% of the budget, and the remaining 31% belongs to the caller.
///
/// It is not a tight fit for anything real. RedStone's own payloads run to
/// hundreds of bytes per feed, so this admits a bundle of ten feeds at twenty
/// signers each and then some.
///
/// **The read is not refundable.** LEZ reads a program's whole instruction data
/// before the program's first instruction (`lee_core`'s `read_lee_inputs`), so
/// by the time this check runs the cycles are already spent. What the limit is
/// for is the caller: a payload above it is one Kanon will not verify, which is
/// what lets a relayer, an aggregator or a sequencer refuse it earlier, where
/// refusing is still free. See ADR 27.
pub const MAX_PAYLOAD_BYTES: usize = 32 * 1024;

/// Why a payload could not be decoded.
///
/// Every variant means "this payload is not well formed". None of them means
/// "this payload is not authorised" — that distinction is the reason decoding
/// and verification are separate steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The payload does not end with [`REDSTONE_MARKER`].
    MissingMarker,
    /// A field ran past the start of the buffer.
    Truncated,
    /// A length field described more bytes than the payload contains.
    LengthOutOfRange,
    /// An integer field was wider than 8 bytes of significant digits.
    ///
    /// Unreachable with the current wire format, and kept deliberately: every
    /// caller passes a fixed width of 6 bytes or fewer, so the check cannot
    /// fire today. It exists so that a future field wider than `u64` fails
    /// closed here rather than wrapping silently.
    NumberOverflow,
    /// The payload is longer than [`MAX_PAYLOAD_BYTES`].
    ///
    /// The only variant here that is not about framing. A payload this long can
    /// be well formed; it is refused because reading it and verifying it will
    /// not both fit in a transaction.
    TooLong { len: usize, max: usize },
    /// The payload declares no data packages.
    ///
    /// Rejected rather than returned as an empty set, because an empty payload
    /// satisfies "every signer in it is authorised" vacuously, and a threshold
    /// check downstream is only as good as its input being non-empty.
    NoDataPackages,
    /// A data package declares no data points.
    NoDataPoints,
    /// A data point value has zero width.
    ZeroWidthValue,
    /// Bytes remained after the declared packages were consumed.
    ///
    /// Trailing data is a decode failure rather than something to ignore: a
    /// payload that carries unaccounted bytes is not the payload that was
    /// signed, whatever else is true of it.
    TrailingBytes(usize),
}

/// Reads fixed-width big-endian fields backwards from the end of a slice.
struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// How many bytes remain unread.
    const fn remaining(&self) -> usize {
        self.bytes.len()
    }

    /// Takes `n` bytes from the end.
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let split = self
            .bytes
            .len()
            .checked_sub(n)
            .ok_or(DecodeError::Truncated)?;
        let (head, tail) = self.bytes.split_at(split);
        self.bytes = head;
        Ok(tail)
    }

    /// Takes an `n`-byte big-endian unsigned integer from the end.
    fn take_uint(&mut self, n: usize) -> Result<u64, DecodeError> {
        let bytes = self.take(n)?;
        // Leading zeros are padding, not magnitude: RedStone writes a 3-byte
        // count and a 6-byte timestamp, neither of which fills a u64.
        let significant = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
        let digits = &bytes[significant..];
        if digits.len() > 8 {
            return Err(DecodeError::NumberOverflow);
        }
        let mut value = 0u64;
        for &byte in digits {
            value = (value << 8) | u64::from(byte);
        }
        Ok(value)
    }
}

/// One `(feed id, value)` pair, borrowed from the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataPoint<'a> {
    /// The 32-byte feed identifier, right-padded ASCII.
    pub feed_id: &'a [u8; FEED_ID_BYTES],
    /// The value, big-endian, `value_size` bytes wide.
    ///
    /// Left as bytes on purpose. Width is a per-package property and the
    /// scaling convention is a per-feed one, so interpreting it here would bake
    /// in an assumption that belongs to the value-sanity step instead.
    pub value: &'a [u8],
}

impl DataPoint<'_> {
    /// The feed id with its right padding removed, for comparison against a
    /// configured feed name.
    #[must_use]
    pub fn feed_id_trimmed(&self) -> &[u8] {
        let end = self
            .feed_id
            .iter()
            .rposition(|&b| b != 0)
            .map_or(0, |last| last + 1);
        &self.feed_id[..end]
    }
}

/// One signed data package, borrowed from the payload.
#[derive(Debug, Clone, Copy)]
pub struct DataPackage<'a> {
    /// Exactly the bytes this package's signature covers.
    signable: &'a [u8],
    /// The data points region, `point_count` entries of `32 + value_size`.
    points: &'a [u8],
    point_count: usize,
    value_size: usize,
    /// Milliseconds since the Unix epoch, as the signer stamped it.
    ///
    /// Milliseconds matches the LEZ clock program, so the staleness comparison
    /// needs no unit conversion.
    pub timestamp_ms: u64,
    /// The 65-byte recoverable signature over [`Self::signable`].
    pub signature: Signature,
}

impl<'a> DataPackage<'a> {
    /// The bytes the signature covers: the data points, then the timestamp,
    /// value size and point count. Not the signature itself.
    #[must_use]
    pub const fn signable(&self) -> &'a [u8] {
        self.signable
    }

    /// How many data points this package carries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.point_count
    }

    /// Whether the package carries no data points. Never true for a package
    /// that decoded successfully; present because clippy asks for it next to
    /// `len`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.point_count == 0
    }

    /// The data points, in payload order. Within a package they *are* in
    /// order, unlike the packages themselves.
    pub fn data_points(&self) -> impl Iterator<Item = DataPoint<'a>> + '_ {
        // Yields exactly `point_count` items. That is an invariant of decoding
        // rather than of this function: `points` was taken as exactly
        // `point_count * (FEED_ID_BYTES + value_size)` bytes, with checked
        // arithmetic, and a short buffer failed as `Truncated` before this
        // package existed. The `filter_map` below can therefore never skip an
        // index -- it is how the slicing is expressed without indexing that
        // could panic, not a tolerance for missing points. A caller may rely on
        // the count matching [`Self::point_count`].
        let stride = FEED_ID_BYTES + self.value_size;
        (0..self.point_count).filter_map(move |index| {
            let start = index.checked_mul(stride)?;
            let feed = self.points.get(start..start + FEED_ID_BYTES)?;
            let value = self.points.get(start + FEED_ID_BYTES..start + stride)?;
            Some(DataPoint {
                feed_id: feed.try_into().ok()?,
                value,
            })
        })
    }
}

/// A decoded RedStone payload: a marker, and the packages before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload<'a> {
    body: &'a [u8],
    package_count: usize,
}

impl<'a> Payload<'a> {
    /// Decodes the framing of `bytes`.
    ///
    /// Validates the marker, the declared counts and every length field against
    /// the buffer, and locates each package's signed span. It does no
    /// cryptography: a payload that decodes is well formed, not trusted.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] for any malformed input, and [`DecodeError::TooLong`]
    /// for a payload past [`MAX_PAYLOAD_BYTES`]. Never panics, which matters
    /// because this runs in a guest where a panic aborts the transaction rather
    /// than rejecting the package.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        // First, and before the marker: everything below is proportional to the
        // payload, and this is the one check that is not.
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(DecodeError::TooLong {
                len: bytes.len(),
                max: MAX_PAYLOAD_BYTES,
            });
        }

        let mut cursor = Cursor::new(bytes);

        let marker = cursor.take(MARKER_BYTES)?;
        if marker != REDSTONE_MARKER {
            return Err(DecodeError::MissingMarker);
        }

        // Unsigned metadata is exactly that: not covered by any signature, so
        // nothing may be read out of it. It is skipped, and its declared size is
        // range-checked so a hostile size cannot wrap the cursor.
        let metadata_size = usize::try_from(cursor.take_uint(UNSIGNED_METADATA_SIZE_BYTES)?)
            .map_err(|_| DecodeError::LengthOutOfRange)?;
        if metadata_size > cursor.remaining() {
            return Err(DecodeError::LengthOutOfRange);
        }
        let _ = cursor.take(metadata_size)?;

        let package_count = usize::try_from(cursor.take_uint(PACKAGE_COUNT_BYTES)?)
            .map_err(|_| DecodeError::LengthOutOfRange)?;
        if package_count == 0 {
            return Err(DecodeError::NoDataPackages);
        }

        Ok(Self {
            body: cursor.bytes,
            package_count,
        })
    }

    /// How many data packages the payload declares.
    #[must_use]
    pub const fn package_count(&self) -> usize {
        self.package_count
    }

    /// Decodes every package, calling `visit` on each.
    ///
    /// **Order is last-first.** The payload is walked backwards, so the package
    /// written last is visited first. RedStone's own SDK collects them the same
    /// way. Nothing downstream should depend on it: a threshold counts distinct
    /// signers, and a rule that depended on package order would be one an
    /// attacker could influence by reordering.
    ///
    /// A visitor rather than an iterator because the packages are laid out
    /// back-to-front and each one's extent depends on fields inside it, so they
    /// cannot be indexed without walking. Returning a fallible visitor keeps
    /// the caller's rejection reason distinct from a decode failure.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if any package is malformed, or if bytes remain once the
    /// declared count is consumed.
    pub fn for_each_package<E, F>(&self, mut visit: F) -> Result<Result<(), E>, DecodeError>
    where
        F: FnMut(DataPackage<'a>) -> Result<(), E>,
    {
        let mut cursor = Cursor::new(self.body);

        for _ in 0..self.package_count {
            // The signable span is measured from here, before the signature is
            // removed, because it ends where the signature begins.
            let signable_end = cursor.remaining();
            let signature = cursor.take(SIGNATURE_BYTES)?;

            let point_count = usize::try_from(cursor.take_uint(POINT_COUNT_BYTES)?)
                .map_err(|_| DecodeError::LengthOutOfRange)?;
            if point_count == 0 {
                return Err(DecodeError::NoDataPoints);
            }

            let value_size = usize::try_from(cursor.take_uint(VALUE_SIZE_BYTES)?)
                .map_err(|_| DecodeError::LengthOutOfRange)?;
            if value_size == 0 {
                return Err(DecodeError::ZeroWidthValue);
            }

            let timestamp_ms = cursor.take_uint(TIMESTAMP_BYTES)?;

            let points_len = point_count
                .checked_mul(
                    FEED_ID_BYTES
                        .checked_add(value_size)
                        .ok_or(DecodeError::LengthOutOfRange)?,
                )
                .ok_or(DecodeError::LengthOutOfRange)?;
            let points = cursor.take(points_len)?;

            // Everything from the start of the data points up to where the
            // signature began. `signable_end` is an offset into `self.body`
            // because the cursor only ever walks backwards over it.
            let signable = self
                .body
                .get(cursor.remaining()..signable_end - SIGNATURE_BYTES)
                .ok_or(DecodeError::Truncated)?;

            let package = DataPackage {
                signable,
                points,
                point_count,
                value_size,
                timestamp_ms,
                signature: Signature(signature.try_into().map_err(|_| DecodeError::Truncated)?),
            };

            if let Err(err) = visit(package) {
                return Ok(Err(err));
            }
        }

        if cursor.remaining() != 0 {
            return Err(DecodeError::TrailingBytes(cursor.remaining()));
        }

        Ok(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;
    use crate::test_support::PayloadBuilder;

    /// `(timestamp, [(feed id, value)])` per package, in visit order.
    type Decoded = Vec<(u64, Vec<(Vec<u8>, Vec<u8>)>)>;

    fn collect(payload: &Payload<'_>) -> Decoded {
        let mut seen = Vec::new();
        payload
            .for_each_package(|package| {
                let points = package
                    .data_points()
                    .map(|point| (point.feed_id_trimmed().to_vec(), point.value.to_vec()))
                    .collect();
                seen.push((package.timestamp_ms, points));
                Ok::<(), ()>(())
            })
            .expect("decodes")
            .expect("visitor does not reject");
        seen
    }

    #[test]
    fn a_single_package_round_trips() {
        let bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[0, 0, 0, 1])], 1_770_000_000_000, 0xAA)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        assert_eq!(payload.package_count(), 1);

        let seen = collect(&payload);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 1_770_000_000_000, "timestamp is milliseconds");
        assert_eq!(seen[0].1[0].0, b"BTC".to_vec(), "feed id loses its padding");
        assert_eq!(seen[0].1[0].1, std::vec![0, 0, 0, 1]);
    }

    #[test]
    fn several_packages_and_several_points_each() {
        let bytes = PayloadBuilder::default()
            .opaque_package(
                &[(b"BTC", &[1, 2, 3, 4]), (b"ETH", &[5, 6, 7, 8])],
                111,
                0x01,
            )
            .opaque_package(&[(b"SOL", &[9, 9, 9, 9])], 222, 0x02)
            .metadata(b"ignored entirely")
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        assert_eq!(payload.package_count(), 2);

        // Last-first: the payload is walked backwards, so the SOL package
        // written second is visited first.
        let seen = collect(&payload);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, 222);
        assert_eq!(seen[0].1.len(), 1);
        assert_eq!(seen[0].1[0].0, b"SOL".to_vec());
        assert_eq!(seen[1].0, 111);
        assert_eq!(seen[1].1.len(), 2);
        assert_eq!(seen[1].1[0].0, b"BTC".to_vec());
        assert_eq!(seen[1].1[1].0, b"ETH".to_vec());
    }

    #[test]
    fn the_signed_span_covers_the_points_and_the_three_trailing_fields() {
        // The assertion this whole module turns on. The span must be the data
        // points plus timestamp, value size and point count -- and must exclude
        // the signature.
        let value = [7u8; 4];
        let bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &value)], 1_234_567, 0xAB)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let mut spans = Vec::new();
        payload
            .for_each_package(|package| {
                spans.push(package.signable().to_vec());
                Ok::<(), ()>(())
            })
            .expect("decodes")
            .expect("no rejection");

        let expected =
            FEED_ID_BYTES + value.len() + TIMESTAMP_BYTES + VALUE_SIZE_BYTES + POINT_COUNT_BYTES;
        assert_eq!(spans[0].len(), expected);
        assert!(
            !spans[0].ends_with(&[0xAB; SIGNATURE_BYTES]),
            "the signature must not be inside its own signed span"
        );
        assert_eq!(
            &spans[0][..3],
            b"BTC",
            "the span starts at the first feed id"
        );
    }

    #[test]
    fn unsigned_metadata_is_skipped_and_changes_nothing() {
        let without = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 500, 0x11)
            .build();
        let with = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 500, 0x11)
            .metadata(b"a much longer unsigned metadata block")
            .build();

        assert_eq!(
            collect(&Payload::decode(&without).unwrap()),
            collect(&Payload::decode(&with).unwrap()),
            "metadata is outside every signature, so it cannot affect a decode"
        );
    }

    #[test]
    fn a_payload_without_the_marker_is_rejected() {
        let mut bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 1, 0x01)
            .build();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;

        assert_eq!(Payload::decode(&bytes), Err(DecodeError::MissingMarker));
    }

    #[test]
    fn an_empty_input_is_rejected_rather_than_panicking() {
        assert_eq!(Payload::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(
            Payload::decode(&REDSTONE_MARKER),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn every_package_yields_exactly_the_point_count_it_declares() {
        // `data_points` is a `filter_map`, so a short buffer would show up as
        // silently fewer points rather than an error. Decoding guarantees it
        // cannot happen; this is the assertion that keeps the guarantee true if
        // the slicing is ever refactored, because a dropped point would let a
        // threshold check downstream pass on less data than the package claims.
        let bytes = PayloadBuilder::default()
            .opaque_package(
                &[(b"BTC", &[1, 2, 3, 4]), (b"ETH", &[5, 6, 7, 8])],
                111,
                0x01,
            )
            .opaque_package(&[(b"SOL", &[9; 32])], 222, 0x02)
            .opaque_package(
                &[(b"XMR", &[7, 7]), (b"ZEC", &[8, 8]), (b"DOT", &[9, 9])],
                333,
                0x03,
            )
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");

        let mut packages = 0;
        payload
            .for_each_package(|package| {
                assert_eq!(
                    package.data_points().count(),
                    package.point_count,
                    "data_points must yield exactly point_count items"
                );
                packages += 1;
                Ok::<(), ()>(())
            })
            .expect("decodes")
            .expect("visitor does not reject");

        assert_eq!(packages, 3, "the fixture declares three packages");
    }

    #[test]
    fn truncation_at_every_length_is_rejected_rather_than_panicking() {
        // The property that matters most: no prefix of a valid payload may
        // panic. A panic in a guest aborts the transaction instead of rejecting
        // the package.
        let bytes = PayloadBuilder::default()
            .opaque_package(
                &[(b"BTC", &[1, 2, 3, 4]), (b"ETH", &[5, 6, 7, 8])],
                999,
                0x22,
            )
            .metadata(b"meta")
            .build();

        for cut in 0..bytes.len() {
            let truncated = &bytes[cut..];
            if let Ok(payload) = Payload::decode(truncated) {
                let _ = payload.for_each_package(|_| Ok::<(), ()>(()));
            }
        }

        // What this does not cover: it shortens the buffer but never perturbs a
        // *length* field, so it exercises "the bytes ran out" and not "a length
        // claims more than exists". `a_declared_package_count_larger_than_the_body_is_rejected`
        // covers one such field by hand; the general case is an arbitrary-bytes
        // property test, which is a separate planned task rather than something
        // this loop quietly already does.
    }

    #[test]
    fn a_payload_past_the_size_limit_is_refused_before_it_is_parsed() {
        // The limit is a cost rule, not a framing one, so the payload it
        // refuses is otherwise perfectly good: the same bytes under the limit
        // decode. Checked before the marker, because every other check is
        // proportional to the payload and this one is not.
        let mut bytes = std::vec![0u8; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(
            Payload::decode(&bytes),
            Err(DecodeError::TooLong {
                len: MAX_PAYLOAD_BYTES + 1,
                max: MAX_PAYLOAD_BYTES,
            }),
            "and not MissingMarker, which these bytes also are"
        );

        bytes.truncate(MAX_PAYLOAD_BYTES);
        assert_eq!(
            Payload::decode(&bytes),
            Err(DecodeError::MissingMarker),
            "one byte shorter and the size is no longer what is wrong with it"
        );
    }

    #[test]
    fn a_declared_package_count_larger_than_the_body_is_rejected() {
        let mut bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 1, 0x01)
            .build();

        // The package count sits just before the metadata size and marker.
        let count_at = bytes.len() - EMPTY_ENVELOPE_BYTES;
        bytes[count_at] = 0xFF;
        bytes[count_at + 1] = 0xFF;

        let payload = Payload::decode(&bytes).expect("the framing still decodes");
        assert!(
            payload.for_each_package(|_| Ok::<(), ()>(())).is_err(),
            "a count the body cannot satisfy must fail rather than read past it"
        );
    }

    #[test]
    fn zero_packages_is_rejected() {
        let mut bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 1, 0x01)
            .build();
        let count_at = bytes.len() - EMPTY_ENVELOPE_BYTES;
        bytes[count_at] = 0;
        bytes[count_at + 1] = 0;

        assert_eq!(Payload::decode(&bytes), Err(DecodeError::NoDataPackages));
    }

    #[test]
    fn trailing_bytes_before_the_packages_are_rejected() {
        let valid = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 1, 0x01)
            .build();
        let mut bytes = std::vec![0xDEu8; 16];
        bytes.extend_from_slice(&valid);

        let payload = Payload::decode(&bytes).expect("framing decodes");
        assert_eq!(
            payload.for_each_package(|_| Ok::<(), ()>(())),
            Err(DecodeError::TrailingBytes(16)),
            "unaccounted bytes mean this is not the payload that was signed"
        );
    }

    #[test]
    fn a_visitor_rejection_is_not_a_decode_error() {
        let bytes = PayloadBuilder::default()
            .opaque_package(&[(b"BTC", &[1, 1, 1, 1])], 1, 0x01)
            .build();
        let payload = Payload::decode(&bytes).unwrap();

        let outcome = payload.for_each_package(|_| Err("not an authorised signer"));
        assert_eq!(
            outcome,
            Ok(Err("not an authorised signer")),
            "malformed and unauthorised must stay distinguishable"
        );
    }
}
