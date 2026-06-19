//! Property-based robustness fuzzing for the x86 decoder.
//!
//! The decoder is fed untrusted guest code, so the hard invariant is that
//! `decode_one`/`decode_one_at` must terminate with `Ok`/`Err` on *any* byte
//! sequence — never panic (slice OOB, unwrap, arithmetic overflow) — and never
//! claim to have consumed bytes it did not see. Complements the C-side AFL++
//! harness (CLAUDE.md: "AFL++ contra el decoder desde el momento que existe").

use proptest::prelude::*;

use prisma_decoder::decode::{decode_one, decode_one_at};

proptest! {
    /// Arbitrary bytes at offset 0 must decode to Ok/Err without panicking.
    #[test]
    fn decode_one_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let _ = decode_one(&bytes, 0);
    }

    /// The guest-PC-aware entry point is equally panic-free for any PC,
    /// including the wrapping edges near u64::MAX that RIP-relative math hits.
    #[test]
    fn decode_one_at_never_panics(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        guest_pc in any::<u64>(),
    ) {
        let _ = decode_one_at(&bytes, 0, guest_pc);
    }

    /// Decoding from any in-bounds offset (including offset == len, which must
    /// report truncation rather than index out of bounds) never panics.
    #[test]
    fn decode_one_at_any_offset_never_panics(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        frac in 0u8..=255,
    ) {
        let offset = (frac as usize * (bytes.len() + 1)) / 256;
        let _ = decode_one(&bytes, offset);
    }

    /// On success the decoder must consume at least one byte and never claim
    /// more than the buffer holds past the start offset.
    #[test]
    fn bytes_consumed_stays_in_bounds(bytes in prop::collection::vec(any::<u8>(), 1..64)) {
        if let Ok(decoded) = decode_one(&bytes, 0) {
            prop_assert!(decoded.bytes_consumed >= 1);
            prop_assert!(decoded.bytes_consumed <= bytes.len());
        }
    }

    /// Decoding is a pure function of (bytes, offset, guest_pc): identical input
    /// yields identical output, so cache keys and differential runs are stable.
    #[test]
    fn decode_is_deterministic(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        guest_pc in any::<u64>(),
    ) {
        let first = decode_one_at(&bytes, 0, guest_pc);
        let second = decode_one_at(&bytes, 0, guest_pc);
        prop_assert_eq!(first, second);
    }
}
