//! Property-fuzz coverage for the time syscalls.
//!
//! A guest drives `clock_gettime` / `gettimeofday` / `nanosleep_request` /
//! `clock_nanosleep_request` / `clock_getres` / `time` with untrusted clock ids
//! and pointers. The invariants under test:
//!
//! * no input ever panics — a bad pointer is a guest `Fault`, never an
//!   out-of-bounds host access;
//! * only CLOCK_REALTIME (0) and CLOCK_MONOTONIC (1) are accepted; every other
//!   id is `UnknownClock`, checked *before* any guest memory is touched;
//! * a negative / overflowing `timespec` request is `InvalidValue`, never a
//!   host panic or a silent wrap.

use std::time::Instant;

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_structs::Timespec;
use prisma_session::time_syscalls::{
    clock_getres, clock_gettime, clock_nanosleep_request, gettimeofday, nanosleep_request, time,
    TimeError, CLOCK_MONOTONIC, CLOCK_REALTIME,
};
use proptest::prelude::*;

const BASE: u64 = 0x1000;
const LEN: usize = 64;

proptest! {
    /// Arbitrary `(clk_id, tp)` never panics. A modelled clock with an in-region
    /// pointer writes a parseable `timespec`; anything else is a clean error.
    #[test]
    fn clock_gettime_never_panics(clk_id in any::<u64>(), tp in any::<u64>()) {
        let start = Instant::now();
        let mut buf = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match clock_gettime(&mut mem, clk_id, tp, start) {
            Ok(()) => {
                // Success implies a modelled clock and an in-region 16-byte write.
                prop_assert!(clk_id == CLOCK_REALTIME || clk_id == CLOCK_MONOTONIC);
                prop_assert!(Timespec::from_guest_bytes(&buf[0..16]).is_some());
            }
            Err(TimeError::UnknownClock(c)) => prop_assert_eq!(c, clk_id),
            Err(TimeError::Fault(_)) => {}
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// An unmodelled clock is rejected before any write: the out region stays
    /// zero, so the error path never touched guest memory.
    #[test]
    fn unknown_clock_writes_nothing(clk_id in 2u64..u64::MAX) {
        let start = Instant::now();
        let mut buf = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        prop_assert_eq!(
            clock_gettime(&mut mem, clk_id, BASE, start),
            Err(TimeError::UnknownClock(clk_id))
        );
        prop_assert_eq!(&buf[0..16], &[0u8; 16]);
    }

    /// `gettimeofday` never panics; an in-region pointer yields a parseable
    /// `timeval`, a wild one yields `Fault`.
    #[test]
    fn gettimeofday_never_panics(tv in any::<u64>()) {
        let mut buf = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match gettimeofday(&mut mem, tv) {
            Ok(()) | Err(TimeError::Fault(_)) => {}
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `nanosleep_request` never panics, and a negative seconds field is always
    /// `InvalidValue` — never a wrap into a huge sleep.
    #[test]
    fn nanosleep_rejects_negative_and_never_panics(sec in any::<i64>(), nsec in any::<i64>()) {
        let mut buf = [0u8; LEN];
        buf[0..8].copy_from_slice(&sec.to_le_bytes());
        buf[8..16].copy_from_slice(&nsec.to_le_bytes());
        let mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match nanosleep_request(&mem, BASE) {
            Ok(_dur) => {
                // A duration only comes back from a non-negative, in-range request.
                prop_assert!(sec >= 0 && (0..1_000_000_000).contains(&nsec));
            }
            Err(TimeError::InvalidValue) => {}
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `clock_nanosleep_request` validates the clock id the same way before it
    /// ever reads the request pointer.
    #[test]
    fn clock_nanosleep_validates_clock_first(clk_id in any::<u64>()) {
        let mut buf = [0u8; LEN]; // a zeroed (instant) request at BASE
        let mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        let r = clock_nanosleep_request(&mem, clk_id, BASE);
        if clk_id != CLOCK_REALTIME && clk_id != CLOCK_MONOTONIC {
            prop_assert_eq!(r, Err(TimeError::UnknownClock(clk_id)));
        }
    }

    /// `clock_getres` never panics; a null `res` just validates the clock id.
    #[test]
    fn clock_getres_never_panics(clk_id in any::<u64>(), res in any::<u64>()) {
        let mut buf = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match clock_getres(&mut mem, clk_id, res) {
            Ok(()) => prop_assert!(clk_id == CLOCK_REALTIME || clk_id == CLOCK_MONOTONIC),
            Err(TimeError::UnknownClock(c)) => prop_assert_eq!(c, clk_id),
            Err(TimeError::Fault(_)) => {}
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `time(tloc)` never panics; a null `tloc` returns the seconds without a
    /// write, an in-region one mirrors the returned value into guest memory.
    #[test]
    fn time_never_panics_and_mirrors(tloc in any::<u64>()) {
        let mut buf = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match time(&mut mem, tloc) {
            Ok(secs) => {
                prop_assert!(secs > 0); // wall clock is past the epoch
                if tloc == BASE {
                    let mirrored = i64::from_le_bytes(buf[0..8].try_into().unwrap());
                    prop_assert_eq!(mirrored, secs);
                }
            }
            Err(TimeError::Fault(_)) => prop_assert_ne!(tloc, 0),
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }
}

#[test]
fn null_tloc_returns_seconds_without_writing() {
    let mut buf = [0u8; LEN];
    let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
    let secs = time(&mut mem, 0).expect("null tloc is fine");
    assert!(secs > 0);
    assert_eq!(&buf[0..8], &[0u8; 8]); // nothing written
}
