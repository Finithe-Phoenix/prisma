//! Property-fuzz the guest-struct marshalling (`Iovec`, `Timespec`, `Timeval`,
//! `Winsize`).
//!
//! These structs decode untrusted guest bytes on the syscall hot path, so a
//! marshalling bug (off-by-one slice, wrong endianness, sign mishandling) is a
//! correctness hole. The unit tests check fixed values; this sweeps arbitrary
//! field values and buffer lengths to pin the three wire-form invariants:
//!
//! * **round-trip** — `from_guest_bytes(to_guest_bytes(x)) == Some(x)` for every
//!   field assignment;
//! * **exact width** — encoding always produces exactly `SIZE` bytes, and
//!   decoding consumes only the first `SIZE` (trailing bytes are ignored);
//! * **short-buffer safety** — decoding a buffer shorter than `SIZE` returns
//!   `None` rather than reading past the end.

use prisma_runtime::guest_structs::{Iovec, Timespec, Timeval, Winsize};
use proptest::prelude::*;

proptest! {
    #[test]
    fn iovec_round_trips(base in any::<u64>(), len in any::<u64>()) {
        let v = Iovec { base, len };
        let bytes = v.to_guest_bytes();
        prop_assert_eq!(bytes.len(), Iovec::SIZE);
        prop_assert_eq!(Iovec::from_guest_bytes(&bytes), Some(v));
    }

    #[test]
    fn timespec_round_trips(sec in any::<i64>(), nsec in any::<i64>()) {
        let t = Timespec { sec, nsec };
        prop_assert_eq!(Timespec::from_guest_bytes(&t.to_guest_bytes()), Some(t));
    }

    #[test]
    fn timeval_round_trips(sec in any::<i64>(), usec in any::<i64>()) {
        let t = Timeval { sec, usec };
        prop_assert_eq!(Timeval::from_guest_bytes(&t.to_guest_bytes()), Some(t));
    }

    #[test]
    fn winsize_round_trips(row in any::<u16>(), col in any::<u16>(),
                           xpixel in any::<u16>(), ypixel in any::<u16>()) {
        let w = Winsize { row, col, xpixel, ypixel };
        let bytes = w.to_guest_bytes();
        prop_assert_eq!(bytes.len(), Winsize::SIZE);
        prop_assert_eq!(Winsize::from_guest_bytes(&bytes), Some(w));
    }

    /// Trailing bytes past `SIZE` are ignored: decoding a long buffer matches
    /// decoding just its first `SIZE` bytes.
    #[test]
    fn decode_consumes_only_the_fixed_width(
        sec in any::<i64>(), nsec in any::<i64>(),
        tail in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let mut buf = Timespec { sec, nsec }.to_guest_bytes().to_vec();
        buf.extend_from_slice(&tail);
        prop_assert_eq!(
            Timespec::from_guest_bytes(&buf),
            Timespec::from_guest_bytes(&buf[..Timespec::SIZE])
        );
    }

    /// A buffer shorter than the wire size never decodes — it must not read past
    /// the end.
    #[test]
    fn short_buffers_are_rejected(short in prop::collection::vec(any::<u8>(), 0..16)) {
        prop_assert!(short.len() < 16);
        // 16-byte structs reject anything under 16 bytes...
        prop_assert!(Iovec::from_guest_bytes(&short).is_none());
        prop_assert!(Timespec::from_guest_bytes(&short).is_none());
        prop_assert!(Timeval::from_guest_bytes(&short).is_none());
        // ...and the 8-byte Winsize rejects anything under 8.
        if short.len() < 8 {
            prop_assert!(Winsize::from_guest_bytes(&short).is_none());
        }
    }

    /// `Timespec::to_duration` matches its documented contract: a non-negative
    /// `sec` with `nsec` in `0..2^32` yields the matching `Duration`; anything
    /// else is `None`. Never panics on arbitrary input.
    #[test]
    fn timespec_to_duration_matches_contract(sec in any::<i64>(), nsec in any::<i64>()) {
        let t = Timespec { sec, nsec };
        match t.to_duration() {
            Some(d) => {
                prop_assert!(sec >= 0 && nsec >= 0);
                prop_assert_eq!(d.as_secs(), u64::try_from(sec).unwrap());
                prop_assert_eq!(u64::from(d.subsec_nanos()), u64::try_from(nsec).unwrap());
            }
            None => prop_assert!(sec < 0 || nsec < 0 || u32::try_from(nsec).is_err()),
        }
    }
}
