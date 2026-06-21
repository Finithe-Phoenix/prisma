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
    fn nanosleep_with_an_unreadable_request_is_efault() {
        use super::nanosleep_request;
        let mut buf = [0u8; 8]; // too short for a 16-byte timespec
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            nanosleep_request(&mem, 0x1000),
            Err(TimeError::Fault(RangeError::Unmapped))
        );
    }
}
