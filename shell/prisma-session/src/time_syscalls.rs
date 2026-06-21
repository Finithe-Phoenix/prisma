//! Pointer-checked time syscalls — the first end-to-end syscall handlers.
//!
//! `clock_gettime` / `gettimeofday` take a guest pointer to write the time into.
//! Each samples the host clock, marshals it to the guest struct, and writes it
//! through a [`GuestRegion`] so the out-pointer is bounds- and permission-checked
//! before the host touches guest memory — a bad pointer becomes a guest `EFAULT`
//! rather than an out-of-bounds host write. This is where the memory-safety
//! primitive, the clock sampler, and the struct marshalling compose.

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_clock::{realtime_timespec, realtime_timeval};

/// `CLOCK_REALTIME` clock id (Linux). `CLOCK_MONOTONIC` (id 1) is added once its
/// sampler lands.
pub const CLOCK_REALTIME: u64 = 0;

/// Why a time syscall failed (each maps to a guest errno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// The clock id is not one we model — guest `EINVAL`.
    UnknownClock(u64),
    /// The out-pointer is not writable guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `clock_gettime(clk_id, tp)`: sample the clock and write its `timespec` to the
/// guest `tp` pointer, range- and permission-checked.
///
/// # Errors
/// [`TimeError::UnknownClock`] for an unmodelled clock, [`TimeError::Fault`] if
/// `tp` is not writable guest memory.
pub fn clock_gettime(mem: &mut GuestRegion, clk_id: u64, tp: u64) -> Result<(), TimeError> {
    let ts = match clk_id {
        CLOCK_REALTIME => realtime_timespec(),
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

#[cfg(test)]
mod tests {
    use super::{clock_gettime, gettimeofday, TimeError, CLOCK_REALTIME};
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::{Timespec, Timeval};

    const AFTER_2020: i64 = 1_600_000_000;

    #[test]
    fn clock_gettime_writes_a_plausible_timespec() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        clock_gettime(&mut mem, CLOCK_REALTIME, 0x1000).expect("write ok");
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert!(ts.sec > AFTER_2020);
        assert!((0..1_000_000_000).contains(&ts.nsec));
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
            clock_gettime(&mut mem, 99, 0x1000),
            Err(TimeError::UnknownClock(99))
        );
    }

    #[test]
    fn an_out_of_range_pointer_is_efault() {
        let mut buf = [0u8; 16];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // The timespec needs 16 bytes; writing at offset 8 runs off the end.
        assert_eq!(
            clock_gettime(&mut mem, CLOCK_REALTIME, 0x1008),
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
}
