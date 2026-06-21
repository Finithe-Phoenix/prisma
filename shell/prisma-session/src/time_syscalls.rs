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
use prisma_runtime::guest_structs::{ITimerval, Timespec, Tms};

/// Clock ticks per second (`sysconf(_SC_CLK_TCK)`) — the unit `times` reports in.
const CLK_TCK: i64 = 100;

/// `CLOCK_REALTIME` clock id (Linux): wall-clock time.
pub const CLOCK_REALTIME: u64 = 0;

/// `CLOCK_MONOTONIC` clock id (Linux): steady time since a fixed reference.
pub const CLOCK_MONOTONIC: u64 = 1;

/// `CLOCK_PROCESS_CPUTIME_ID` (2): per-process CPU time. Not tracked, so it is
/// approximated by the monotonic elapsed run time (exact for a CPU-bound
/// single-threaded program).
pub const CLOCK_PROCESS_CPUTIME_ID: u64 = 2;
/// `CLOCK_THREAD_CPUTIME_ID` (3): per-thread CPU time; same approximation.
pub const CLOCK_THREAD_CPUTIME_ID: u64 = 3;
/// `CLOCK_MONOTONIC_RAW` (4): monotonic, NTP-unadjusted — same source here.
pub const CLOCK_MONOTONIC_RAW: u64 = 4;
/// `CLOCK_REALTIME_COARSE` (5): low-resolution wall clock.
pub const CLOCK_REALTIME_COARSE: u64 = 5;
/// `CLOCK_MONOTONIC_COARSE` (6): low-resolution monotonic clock.
pub const CLOCK_MONOTONIC_COARSE: u64 = 6;
/// `CLOCK_BOOTTIME` (7): monotonic including suspend — same as monotonic here
/// (the emulator never suspends).
pub const CLOCK_BOOTTIME: u64 = 7;

/// The resolution (in nanoseconds) the two `*_COARSE` clocks report — the
/// conventional 4 ms tick (`CONFIG_HZ=250`). The precise clocks report 1 ns.
const COARSE_RES_NS: i64 = 4_000_000;

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
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE => realtime_timespec(),
        CLOCK_MONOTONIC
        | CLOCK_MONOTONIC_RAW
        | CLOCK_MONOTONIC_COARSE
        | CLOCK_BOOTTIME
        // CPU-time clocks are not tracked; approximate by elapsed run time.
        | CLOCK_PROCESS_CPUTIME_ID
        | CLOCK_THREAD_CPUTIME_ID => monotonic_timespec(monotonic_start),
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
    // Only the sleepable clocks are valid for clock_nanosleep; the COARSE / RAW
    // and CPU-time clocks are EINVAL here, as in the kernel.
    match clk_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME => {}
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
    let nsec = match clk_id {
        CLOCK_REALTIME_COARSE | CLOCK_MONOTONIC_COARSE => COARSE_RES_NS,
        CLOCK_REALTIME
        | CLOCK_MONOTONIC
        | CLOCK_MONOTONIC_RAW
        | CLOCK_BOOTTIME
        | CLOCK_PROCESS_CPUTIME_ID
        | CLOCK_THREAD_CPUTIME_ID => 1,
        other => return Err(TimeError::UnknownClock(other)),
    };
    if res == 0 {
        return Ok(()); // null res: the clock id is validated, nothing to write
    }
    let resolution = Timespec { sec: 0, nsec };
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

/// `setitimer(which, new, old)`: report the current interval timer to `old`
/// (when non-null) and install the one at `new` (when non-null), returning the
/// timer in effect afterwards. Both pointers go through the range-checked
/// [`GuestRegion`].
///
/// The handler is pure in the timer state — the caller owns the three timers
/// (REAL/VIRTUAL/PROF), passing the selected one as `current` and storing the
/// returned value — so it stays decoupled from where that state lives. The timer
/// is *stored* but not yet armed for delivery (that needs the signal-delivery
/// loop); `getitimer` reports back exactly what was stored.
///
/// # Errors
/// [`TimeError::Fault`] if a non-null `new`/`old` is not accessible guest memory.
pub fn setitimer(
    mem: &mut GuestRegion,
    new_ptr: u64,
    old_ptr: u64,
    current: ITimerval,
) -> Result<ITimerval, TimeError> {
    if old_ptr != 0 {
        mem.write(old_ptr, &current.to_guest_bytes())
            .map_err(TimeError::Fault)?;
    }
    if new_ptr != 0 {
        let bytes = mem
            .read(new_ptr, ITimerval::SIZE)
            .map_err(TimeError::Fault)?;
        let next =
            ITimerval::from_guest_bytes(bytes).ok_or(TimeError::Fault(RangeError::Unmapped))?;
        return Ok(next);
    }
    Ok(current)
}

/// `times(buf)`: write the process CPU times to the guest `tms` at `buf` (when
/// non-null) and return the elapsed real time in clock ticks since the session
/// started (`monotonic_start`).
///
/// Per-process user/system CPU time is not tracked in this model, so the `tms`
/// fields are reported as zero; the return value is the meaningful part a shell
/// uses for wall-clock timing.
///
/// # Errors
/// [`TimeError::Fault`] if a non-null `buf` is not writable guest memory.
pub fn times(mem: &mut GuestRegion, buf: u64, monotonic_start: Instant) -> Result<i64, TimeError> {
    if buf != 0 {
        let tms = Tms {
            utime: 0,
            stime: 0,
            cutime: 0,
            cstime: 0,
        };
        mem.write(buf, &tms.to_guest_bytes())
            .map_err(TimeError::Fault)?;
    }
    let elapsed = monotonic_timespec(monotonic_start);
    // sec * CLK_TCK + nsec / (1e9 / CLK_TCK); for CLK_TCK = 100 that is nsec/1e7.
    Ok(elapsed.sec * CLK_TCK + elapsed.nsec / 10_000_000)
}

/// `getitimer(which, curr)`: write the selected interval timer to the guest
/// `curr` pointer.
///
/// # Errors
/// [`TimeError::Fault`] if `curr` is not writable guest memory.
pub fn getitimer(
    mem: &mut GuestRegion,
    curr_ptr: u64,
    current: ITimerval,
) -> Result<(), TimeError> {
    mem.write(curr_ptr, &current.to_guest_bytes())
        .map_err(TimeError::Fault)
}

#[cfg(test)]
mod tests {
    use super::{
        clock_getres, clock_gettime, clock_nanosleep_request, getitimer, gettimeofday, setitimer,
        times, TimeError, CLOCK_BOOTTIME, CLOCK_MONOTONIC, CLOCK_MONOTONIC_COARSE,
        CLOCK_PROCESS_CPUTIME_ID, CLOCK_REALTIME, CLOCK_REALTIME_COARSE,
    };
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::{ITimerval, Timespec, Timeval, Tms};
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

    #[test]
    fn setitimer_reports_old_and_installs_new_and_getitimer_reads_back() {
        let mut buf = [0u8; 96];
        // A new timer at guest 0x1000: reload 1.0s, first expiry 0.5s.
        let new = ITimerval {
            interval: Timeval { sec: 1, usec: 0 },
            value: Timeval {
                sec: 0,
                usec: 500_000,
            },
        };
        buf[0..32].copy_from_slice(&new.to_guest_bytes());
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // The currently-disarmed timer.
        let current = ITimerval {
            interval: Timeval { sec: 0, usec: 0 },
            value: Timeval { sec: 0, usec: 0 },
        };
        // setitimer(new=0x1000, old=0x1020): install new, report old (disarmed).
        let installed = setitimer(&mut mem, 0x1000, 0x1020, current).expect("ok");
        assert_eq!(installed, new);
        let old = ITimerval::from_guest_bytes(&buf[0x20..0x20 + 32]).unwrap();
        assert_eq!(old, current);

        // getitimer reads the installed timer back into a fresh region.
        let mut buf2 = [0u8; 32];
        let mut mem2 = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf2);
        getitimer(&mut mem2, 0x1000, installed).expect("read");
        assert_eq!(ITimerval::from_guest_bytes(&buf2).unwrap(), new);
    }

    #[test]
    fn setitimer_null_new_keeps_current_and_bad_pointer_faults() {
        let current = ITimerval {
            interval: Timeval { sec: 2, usec: 0 },
            value: Timeval { sec: 1, usec: 0 },
        };
        let mut buf = [0u8; 32];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // Both null: nothing reported/installed — current is unchanged.
        assert_eq!(setitimer(&mut mem, 0, 0, current).unwrap(), current);
        // A too-short old region (only 8 bytes from 0x1018) is EFAULT.
        assert_eq!(
            setitimer(&mut mem, 0, 0x1018, current),
            Err(TimeError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn times_zeroes_cpu_and_returns_elapsed_ticks() {
        let start = Instant::now();
        let mut buf = [0u8; 32];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // times(buf) -> a non-negative tick count; the tms CPU fields are zeroed.
        let ticks = times(&mut mem, 0x1000, start).expect("ok");
        assert!(ticks >= 0);
        let tms = Tms::from_guest_bytes(mem.read(0x1000, 32).unwrap()).unwrap();
        assert_eq!((tms.utime, tms.stime, tms.cutime, tms.cstime), (0, 0, 0, 0));
        // A null buf still returns the tick count without writing.
        assert!(times(&mut mem, 0, start).unwrap() >= 0);
        // A bad buf pointer is EFAULT (only 8 bytes from 0x1018, need 32).
        assert_eq!(
            times(&mut mem, 0x1018, start),
            Err(TimeError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn clock_gettime_accepts_the_extended_clock_ids() {
        let start = Instant::now();
        // COARSE / BOOTTIME / CPUTIME all sample successfully (no UnknownClock).
        for clk in [
            CLOCK_REALTIME_COARSE,
            CLOCK_MONOTONIC_COARSE,
            CLOCK_BOOTTIME,
            CLOCK_PROCESS_CPUTIME_ID,
        ] {
            let mut buf = [0u8; 16];
            let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
            clock_gettime(&mut mem, clk, 0x1000, start).expect("sampled");
            let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
            assert!(ts.sec >= 0 && (0..1_000_000_000).contains(&ts.nsec));
        }
        // An id past the modelled set is still rejected.
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            clock_gettime(&mut mem, 99, 0x1000, start),
            Err(TimeError::UnknownClock(99))
        );
    }

    #[test]
    fn clock_getres_reports_coarse_resolution_for_coarse_clocks() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // A COARSE clock reports the 4 ms tick.
        clock_getres(&mut mem, CLOCK_MONOTONIC_COARSE, 0x1000).expect("ok");
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert_eq!((ts.sec, ts.nsec), (0, 4_000_000));
        // A precise clock reports 1 ns.
        clock_getres(&mut mem, CLOCK_REALTIME, 0x1000).expect("ok");
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert_eq!((ts.sec, ts.nsec), (0, 1));
    }

    #[test]
    fn clock_nanosleep_accepts_boottime_but_not_coarse() {
        let mut buf = [0u8; 16];
        let mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // a {0,0} request on BOOTTIME parses to an instant sleep.
        assert_eq!(
            clock_nanosleep_request(&mem, CLOCK_BOOTTIME, 0x1000)
                .unwrap()
                .as_nanos(),
            0
        );
        // a COARSE clock is not valid for clock_nanosleep.
        assert_eq!(
            clock_nanosleep_request(&mem, CLOCK_MONOTONIC_COARSE, 0x1000),
            Err(TimeError::UnknownClock(CLOCK_MONOTONIC_COARSE))
        );
    }
}
