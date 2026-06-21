//! Host clock sampled into the guest's time structs.
//!
//! `clock_gettime(CLOCK_REALTIME)` and `gettimeofday` report wall-clock time;
//! the value the guest reads is the host's current time marshalled into its
//! `timespec` / `timeval`. This module is that sampling — converting a host
//! `SystemTime` into the guest structs. Writing the result into guest memory is
//! the caller's job (it owns the out-pointer and its range check).

use std::time::{SystemTime, UNIX_EPOCH};

use crate::guest_structs::{Timespec, Timeval};

/// Seconds + nanoseconds since the Unix epoch for the host's current wall clock.
/// A host clock set before the epoch (which should not happen) clamps to zero
/// rather than reporting a negative time to the guest.
fn since_epoch() -> (i64, u32) {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or((0, 0), |d| {
            (
                i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
                d.subsec_nanos(),
            )
        })
}

/// Current `CLOCK_REALTIME` as a guest `timespec` (whole seconds + nanoseconds).
#[must_use]
pub fn realtime_timespec() -> Timespec {
    let (sec, nsec) = since_epoch();
    Timespec {
        sec,
        nsec: i64::from(nsec),
    }
}

/// Current wall-clock time as a guest `timeval` (whole seconds + microseconds),
/// the form `gettimeofday` reports.
#[must_use]
pub fn realtime_timeval() -> Timeval {
    let (sec, nsec) = since_epoch();
    Timeval {
        sec,
        // Nanoseconds within the second are < 1e9, so /1000 fits microseconds.
        usec: i64::from(nsec / 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::{realtime_timespec, realtime_timeval};

    /// A timestamp captured after 2020 is well past this epoch second.
    const AFTER_2020: i64 = 1_600_000_000;

    #[test]
    fn realtime_timespec_is_a_plausible_recent_time() {
        let ts = realtime_timespec();
        assert!(ts.sec > AFTER_2020, "wall clock should be well after 2020");
        // Nanoseconds are within one second.
        assert!((0..1_000_000_000).contains(&ts.nsec));
    }

    #[test]
    fn realtime_timeval_microseconds_are_in_range() {
        let tv = realtime_timeval();
        assert!(tv.sec > AFTER_2020);
        // Microseconds are within one second.
        assert!((0..1_000_000).contains(&tv.usec));
    }

    #[test]
    fn timespec_and_timeval_agree_on_the_second() {
        // Sampled microseconds apart, the whole-second fields must be within 1.
        let ts = realtime_timespec();
        let tv = realtime_timeval();
        assert!((tv.sec - ts.sec).abs() <= 1);
    }
}
