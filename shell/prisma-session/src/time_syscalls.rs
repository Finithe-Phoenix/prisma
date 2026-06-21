//! Pointer-checked time syscalls — the first end-to-end syscall handlers.
//!
//! `clock_gettime` / `gettimeofday` take a guest pointer to write the time into.
//! Each samples the host clock, marshals it to the guest struct, and writes it
//! through a [`GuestRegion`] so the out-pointer is bounds- and permission-checked
//! before the host touches guest memory — a bad pointer becomes a guest `EFAULT`
//! rather than an out-of-bounds host write. This is where the memory-safety
//! primitive, the clock sampler, and the struct marshalling compose.

use std::time::{Duration, Instant};

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_clock::{monotonic_timespec, realtime_timespec, realtime_timeval};
use prisma_runtime::guest_structs::Timespec;

/// `CLOCK_REALTIME` clock id (Linux): wall-clock time.
pub const CLOCK_REALTIME: u64 = 0;

/// `CLOCK_MONOTONIC` clock id (Linux): steady time since a fixed reference.
pub const CLOCK_MONOTONIC: u64 = 1;

/// Why a time syscall failed (each maps to a guest errno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// The clock id is not one we model — guest `EINVAL`.
    UnknownClock(u64),
    /// A `timespec` field is out of range (negative / overflowing) — `EINVAL`.
    InvalidValue,
    /// The out-pointer is not writable guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `clock_gettime(clk_id, tp)`: sample the clock and write its `timespec` to the
/// guest `tp` pointer, range- and permission-checked. `monotonic_start` is the
/// reference point for `CLOCK_MONOTONIC` (the session/process start the caller
/// holds); it is ignored for `CLOCK_REALTIME`.
///
/// # Errors
/// [`TimeError::UnknownClock`] for an unmodelled clock, [`TimeError::Fault`] if
/// `tp` is not writable guest memory.
pub fn clock_gettime(
    mem: &mut GuestRegion,
    clk_id: u64,
    tp: u64,
    monotonic_start: Instant,
) -> Result<(), TimeError> {
    let ts = match clk_id {
        CLOCK_REALTIME => realtime_timespec(),
        CLOCK_MONOTONIC => monotonic_timespec(monotonic_start),
        other => return Err(TimeError::UnknownClock(other)),
    };
    mem.write(tp, &ts.to_guest_bytes())
        .map_err(TimeError::Fault)
}

/// `gettimeofday(tv, tz)`: write the current wall-clock time as a `timeval` to
/// the guest `tv` pointer. The obsolete timezone argument is ignored.
///
/// # Errors
/// [`TimeError::Fault`] if `tv` is not writable guest memory.
pub fn gettimeofday(mem: &mut GuestRegion, tv: u64) -> Result<(), TimeError> {
    mem.write(tv, &realtime_timeval().to_guest_bytes())
        .map_err(TimeError::Fault)
}

/// `nanosleep(req, rem)`: read the requested sleep interval from the guest `req`
/// `timespec` and return it as a host [`Duration`] for the caller to sleep on.
/// The `rem` out-pointer (time remaining on an interrupted sleep) is the
/// caller's to fill; this parses and validates the request.
///
/// # Errors
/// [`TimeError::Fault`] if `req` is not readable guest memory,
/// [`TimeError::InvalidValue`] if the `timespec` is negative or out of range
/// (the kernel's `EINVAL` for `nanosleep`).
pub fn nanosleep_request(mem: &GuestRegion, req: u64) -> Result<Duration, TimeError> {
    let bytes = mem.read(req, Timespec::SIZE).map_err(TimeError::Fault)?;
    let ts = Timespec::from_guest_bytes(bytes).ok_or(TimeError::Fault(RangeError::Unmapped))?;
    ts.to_duration().ok_or(TimeError::InvalidValue)
}

/// `clock_nanosleep(clk_id, flags, req, rem)`: like [`nanosleep_request`] but for
/// a specific clock — validate `clk_id` (only CLOCK_REALTIME / CLOCK_MONOTONIC
/// are modelled), then read and validate the requested interval, returning it as
/// a host [`Duration`] for the caller to sleep on. The relative-vs-absolute
/// `flags` and the `rem` out-pointer are the caller's to honour.
///
/// # Errors
/// [`TimeError::UnknownClock`] for an unmodelled clock, [`TimeError::Fault`] if
/// `req` is unreadable, [`TimeError::InvalidValue`] for a negative/overflowing
/// interval.
pub fn clock_nanosleep_request(
    mem: &GuestRegion,
    clk_id: u64,
    req: u64,
) -> Result<Duration, TimeError> {
    match clk_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC => {}
        other => return Err(TimeError::UnknownClock(other)),
    }
    nanosleep_request(mem, req)
}

/// `clock_getres(clk_id, res)`: write the clock's resolution to the guest `res`
/// `timespec`. A null `res` (address 0) just validates `clk_id` per the kernel.
/// Both modelled clocks report a 1-nanosecond resolution (the high-res-timer
/// granularity Linux advertises for CLOCK_REALTIME / CLOCK_MONOTONIC).
///
/// # Errors
/// [`TimeError::UnknownClock`] for an unmodelled clock, [`TimeError::Fault`] if
/// `res` is non-null and not writable guest memory.
pub fn clock_getres(mem: &mut GuestRegion, clk_id: u64, res: u64) -> Result<(), TimeError> {
    match clk_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC => {}
        other => return Err(TimeError::UnknownClock(other)),
    }
    if res == 0 {
        return Ok(()); // null res: the clock id is validated, nothing to write
    }
    let resolution = Timespec { sec: 0, nsec: 1 };
    mem.write(res, &resolution.to_guest_bytes())
        .map_err(TimeError::Fault)
}

/// `time(tloc)`: return the current wall-clock time in whole seconds since the
/// Unix epoch, also writing it (as an 8-byte `time_t`) to the guest `tloc`
/// pointer when `tloc` is non-null.
///
/// # Errors
/// [`TimeError::Fault`] if `tloc` is non-null and not writable guest memory.
pub fn time(mem: &mut GuestRegion, tloc: u64) -> Result<i64, TimeError> {
    let secs = realtime_timespec().sec;
    if tloc != 0 {
        mem.write(tloc, &secs.to_le_bytes())
            .map_err(TimeError::Fault)?;
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::{clock_gettime, gettimeofday, TimeError, CLOCK_MONOTONIC, CLOCK_REALTIME};
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::{Timespec, Timeval};
    use std::time::Instant;

    const AFTER_2020: i64 = 1_600_000_000;

    #[test]
    fn clock_gettime_writes_a_plausible_timespec() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        clock_gettime(&mut mem, CLOCK_REALTIME, 0x1000, Instant::now()).expect("write ok");
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert!(ts.sec > AFTER_2020);
        assert!((0..1_000_000_000).contains(&ts.nsec));
    }

    #[test]
    fn clock_gettime_monotonic_writes_a_nonnegative_timespec() {
        let start = Instant::now();
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        clock_gettime(&mut mem, CLOCK_MONOTONIC, 0x1000, start).expect("write ok");
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        // A monotonic sample since `start` is small and never negative.
        assert!(ts.sec >= 0 && ts.nsec >= 0);
        assert!(ts.sec < AFTER_2020); // not a wall-clock value
    }

    #[test]
    fn gettimeofday_writes_a_plausible_timeval() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x2000, Protection::ReadWrite, &mut buf);
        gettimeofday(&mut mem, 0x2000).expect("write ok");
        let tv = Timeval::from_guest_bytes(mem.read(0x2000, 16).unwrap()).unwrap();
        assert!(tv.sec > AFTER_2020);
        assert!((0..1_000_000).contains(&tv.usec));
    }

    #[test]
    fn an_unmodelled_clock_is_einval() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            clock_gettime(&mut mem, 99, 0x1000, Instant::now()),
            Err(TimeError::UnknownClock(99))
        );
    }

    #[test]
    fn an_out_of_range_pointer_is_efault() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // The timespec needs 16 bytes; writing at offset 8 runs off the end.
        assert_eq!(
            clock_gettime(&mut mem, CLOCK_REALTIME, 0x1008, Instant::now()),
            Err(TimeError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn a_read_only_target_is_efault() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadOnly, &mut buf);
        assert_eq!(
            gettimeofday(&mut mem, 0x1000),
            Err(TimeError::Fault(RangeError::NotWritable))
        );
    }

    #[test]
    fn nanosleep_parses_a_valid_request_into_a_duration() {
        use super::nanosleep_request;
        use std::time::Duration;
        // 1.5 seconds as a guest timespec at 0x1000.
        let mut buf = [0u8; 16];
        buf.copy_from_slice(
            &Timespec {
                sec: 1,
                nsec: 500_000_000,
            }
            .to_guest_bytes(),
        );
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            nanosleep_request(&mem, 0x1000).unwrap(),
            Duration::new(1, 500_000_000)
        );
    }

    #[test]
    fn nanosleep_rejects_a_negative_request_as_einval() {
        use super::nanosleep_request;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&Timespec { sec: -1, nsec: 0 }.to_guest_bytes());
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            nanosleep_request(&mem, 0x1000),
            Err(TimeError::InvalidValue)
        );
    }

    #[test]
    fn clock_getres_reports_one_nanosecond_for_both_clocks() {
        use super::clock_getres;
        for clk in [CLOCK_REALTIME, CLOCK_MONOTONIC] {
            let mut buf = [0xFFu8; 16];
            let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
            clock_getres(&mut mem, clk, 0x1000).expect("res written");
            let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
            assert_eq!((ts.sec, ts.nsec), (0, 1));
        }
    }

    #[test]
    fn clock_getres_null_res_validates_the_clock_without_writing() {
        use super::clock_getres;
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // Null res (0) with a known clock succeeds and writes nothing.
        clock_getres(&mut mem, CLOCK_MONOTONIC, 0).expect("ok");
        // An unknown clock is still EINVAL even with a null res.
        assert_eq!(
            clock_getres(&mut mem, 42, 0),
            Err(TimeError::UnknownClock(42))
        );
    }

    #[test]
    fn nanosleep_with_an_unreadable_request_is_efault() {
        use super::nanosleep_request;
        let mut buf = [0u8; 8]; // too short for a 16-byte timespec
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            nanosleep_request(&mem, 0x1000),
            Err(TimeError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn time_returns_and_optionally_writes_the_epoch_seconds() {
        use super::time;
        let mut buf = [0u8; 8];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        let secs = time(&mut mem, 0x1000).expect("time ok");
        assert!(secs > AFTER_2020);
        // The same value was written to tloc as an 8-byte time_t.
        assert_eq!(i64::from_le_bytes(buf), secs);
    }

    #[test]
    fn time_with_null_tloc_just_returns_the_seconds() {
        use super::time;
        let mut buf = [0u8; 8];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        let secs = time(&mut mem, 0).expect("null tloc ok");
        assert!(secs > AFTER_2020);
        assert_eq!(buf, [0u8; 8]); // nothing written
    }

    #[test]
    fn clock_nanosleep_validates_the_clock_then_the_request() {
        use super::clock_nanosleep_request;
        use std::time::Duration;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&Timespec { sec: 2, nsec: 0 }.to_guest_bytes());
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // A known clock + valid request -> the duration.
        assert_eq!(
            clock_nanosleep_request(&mem, CLOCK_MONOTONIC, 0x1000).unwrap(),
            Duration::new(2, 0)
        );
        // An unknown clock is EINVAL (checked before the request is read).
        assert_eq!(
            clock_nanosleep_request(&mem, 42, 0x1000),
            Err(TimeError::UnknownClock(42))
        );
    }

    #[test]
    fn clock_nanosleep_rejects_a_negative_request() {
        use super::clock_nanosleep_request;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&Timespec { sec: -1, nsec: 0 }.to_guest_bytes());
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            clock_nanosleep_request(&mem, CLOCK_REALTIME, 0x1000),
            Err(TimeError::InvalidValue)
        );
    }

    #[test]
    fn time_with_an_unwritable_tloc_is_efault() {
        use super::time;
        let mut buf = [0u8; 8];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadOnly, &mut buf);
        assert_eq!(
            time(&mut mem, 0x1000),
            Err(TimeError::Fault(RangeError::NotWritable))
        );
    }
}
