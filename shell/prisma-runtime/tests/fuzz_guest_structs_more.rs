//! Property-fuzz the round-trip of the larger guest structs.
//!
//! The original `fuzz_guest_structs` covers the first four (iovec/timespec/
//! timeval/winsize). This sweeps the rest — the records that decode untrusted
//! guest bytes on the stat/poll/epoll/signal/fcntl paths — over arbitrary field
//! values, pinning the same two invariants: `from_guest_bytes(to_guest_bytes(x))
//! == Some(x)` for every field assignment, and a buffer shorter than `SIZE`
//! decodes to `None` (never reads past the end).

use prisma_runtime::guest_structs::{
    EpollEvent, Flock, ITimerval, PollFd, Rlimit, Rusage, SchedParam, SigAltStack, Stat, Statfs,
    Sysinfo, Termios, Timespec, Timeval, Tms,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn termios_round_trips(
        iflag in any::<u32>(), oflag in any::<u32>(), cflag in any::<u32>(),
        lflag in any::<u32>(), line in any::<u8>(), cc in any::<[u8; 19]>(),
    ) {
        let t = Termios { iflag, oflag, cflag, lflag, line, cc };
        prop_assert_eq!(Termios::from_guest_bytes(&t.to_guest_bytes()), Some(t));
    }

    #[test]
    fn stat_round_trips(
        dev in any::<u64>(), ino in any::<u64>(), nlink in any::<u64>(),
        mode in any::<u32>(), uid in any::<u32>(), gid in any::<u32>(),
        rdev in any::<u64>(), size in any::<i64>(), blksize in any::<i64>(),
        blocks in any::<i64>(),
        atime in (any::<i64>(), any::<i64>()), mtime in (any::<i64>(), any::<i64>()),
        ctime in (any::<i64>(), any::<i64>()),
    ) {
        let st = Stat {
            dev, ino, nlink, mode, uid, gid, rdev, size, blksize, blocks,
            atime: Timespec { sec: atime.0, nsec: atime.1 },
            mtime: Timespec { sec: mtime.0, nsec: mtime.1 },
            ctime: Timespec { sec: ctime.0, nsec: ctime.1 },
        };
        prop_assert_eq!(Stat::from_guest_bytes(&st.to_guest_bytes()), Some(st));
    }

    #[test]
    fn flock_round_trips(
        typ in any::<i16>(), whence in any::<i16>(), start in any::<i64>(),
        len in any::<i64>(), pid in any::<i32>(),
    ) {
        let fl = Flock { typ, whence, start, len, pid };
        prop_assert_eq!(Flock::from_guest_bytes(&fl.to_guest_bytes()), Some(fl));
    }

    #[test]
    fn pollfd_round_trips(fd in any::<i32>(), events in any::<i16>(), revents in any::<i16>()) {
        let p = PollFd { fd, events, revents };
        prop_assert_eq!(PollFd::from_guest_bytes(&p.to_guest_bytes()), Some(p));
    }

    #[test]
    fn epoll_event_round_trips(events in any::<u32>(), data in any::<u64>()) {
        let e = EpollEvent { events, data };
        prop_assert_eq!(EpollEvent::from_guest_bytes(&e.to_guest_bytes()), Some(e));
    }

    #[test]
    fn sigaltstack_round_trips(sp in any::<u64>(), flags in any::<i32>(), size in any::<u64>()) {
        let ss = SigAltStack { sp, flags, size };
        prop_assert_eq!(SigAltStack::from_guest_bytes(&ss.to_guest_bytes()), Some(ss));
    }

    #[test]
    fn rlimit_round_trips(cur in any::<u64>(), max in any::<u64>()) {
        let rl = Rlimit { cur, max };
        prop_assert_eq!(Rlimit::from_guest_bytes(&rl.to_guest_bytes()), Some(rl));
    }

    #[test]
    fn itimerval_round_trips(
        interval in (any::<i64>(), any::<i64>()), value in (any::<i64>(), any::<i64>()),
    ) {
        let it = ITimerval {
            interval: Timeval { sec: interval.0, usec: interval.1 },
            value: Timeval { sec: value.0, usec: value.1 },
        };
        prop_assert_eq!(ITimerval::from_guest_bytes(&it.to_guest_bytes()), Some(it));
    }

    #[test]
    fn tms_round_trips(
        utime in any::<i64>(), stime in any::<i64>(),
        cutime in any::<i64>(), cstime in any::<i64>(),
    ) {
        let t = Tms { utime, stime, cutime, cstime };
        prop_assert_eq!(Tms::from_guest_bytes(&t.to_guest_bytes()), Some(t));
    }

    /// `Rusage` is encode-only (the kernel writes it, the guest reads it). For
    /// arbitrary fields, encoding never panics, produces exactly `SIZE` bytes,
    /// and lands a field at its known offset (`ru_utime.sec` @0, `ru_maxrss` @32).
    #[test]
    fn rusage_encodes_at_known_offsets(
        utime_sec in any::<i64>(), maxrss in any::<i64>(),
    ) {
        let r = Rusage { utime: Timeval { sec: utime_sec, usec: 0 }, maxrss, ..Rusage::ZERO };
        let b = r.to_guest_bytes();
        prop_assert_eq!(b.len(), Rusage::SIZE);
        prop_assert_eq!(&b[0..8], &utime_sec.to_le_bytes());
        prop_assert_eq!(&b[32..40], &maxrss.to_le_bytes());
    }

    /// `Sysinfo` is encode-only; `uptime` lands @0 and `totalram` @32 for
    /// arbitrary values, sized exactly at `SIZE`, with the alignment gaps zero.
    #[test]
    fn sysinfo_encodes_at_known_offsets(uptime in any::<i64>(), totalram in any::<u64>()) {
        let si = Sysinfo {
            uptime,
            loads: [0; 3],
            totalram,
            freeram: 0,
            sharedram: 0,
            bufferram: 0,
            totalswap: 0,
            freeswap: 0,
            procs: 0,
            totalhigh: 0,
            freehigh: 0,
            mem_unit: 1,
        };
        let b = si.to_guest_bytes();
        prop_assert_eq!(b.len(), Sysinfo::SIZE);
        prop_assert_eq!(&b[0..8], &uptime.to_le_bytes());
        prop_assert_eq!(&b[32..40], &totalram.to_le_bytes());
        prop_assert!(b[82..88].iter().all(|&x| x == 0));
        prop_assert!(b[108..112].iter().all(|&x| x == 0));
    }

    /// `Statfs` is encode-only; `f_type` lands @0 and `f_bavail` @32 for
    /// arbitrary values, sized exactly at `SIZE`, with the four spare words zero.
    #[test]
    fn statfs_encodes_at_known_offsets(f_type in any::<u64>(), bavail in any::<u64>()) {
        let s = Statfs {
            f_type,
            bsize: 4096,
            blocks: 0,
            bfree: 0,
            bavail,
            files: 0,
            ffree: 0,
            fsid: 0,
            namelen: 255,
            frsize: 4096,
            flags: 0,
        };
        let b = s.to_guest_bytes();
        prop_assert_eq!(b.len(), Statfs::SIZE);
        prop_assert_eq!(&b[0..8], &f_type.to_le_bytes());
        prop_assert_eq!(&b[32..40], &bavail.to_le_bytes());
        prop_assert!(b[88..120].iter().all(|&x| x == 0));
    }

    #[test]
    fn sched_param_round_trips(priority in any::<i32>()) {
        let p = SchedParam { priority };
        prop_assert_eq!(SchedParam::from_guest_bytes(&p.to_guest_bytes()), Some(p));
    }

    /// A buffer one byte shorter than the wire size always decodes to `None`.
    #[test]
    fn one_byte_short_is_rejected(pad in any::<u8>()) {
        let short = |n: usize| vec![pad; n - 1];
        prop_assert!(Termios::from_guest_bytes(&short(Termios::SIZE)).is_none());
        prop_assert!(Stat::from_guest_bytes(&short(Stat::SIZE)).is_none());
        prop_assert!(Flock::from_guest_bytes(&short(Flock::SIZE)).is_none());
        prop_assert!(PollFd::from_guest_bytes(&short(PollFd::SIZE)).is_none());
        prop_assert!(EpollEvent::from_guest_bytes(&short(EpollEvent::SIZE)).is_none());
        prop_assert!(SigAltStack::from_guest_bytes(&short(SigAltStack::SIZE)).is_none());
        prop_assert!(Rlimit::from_guest_bytes(&short(Rlimit::SIZE)).is_none());
        prop_assert!(ITimerval::from_guest_bytes(&short(ITimerval::SIZE)).is_none());
        prop_assert!(Tms::from_guest_bytes(&short(Tms::SIZE)).is_none());
        prop_assert!(SchedParam::from_guest_bytes(&short(SchedParam::SIZE)).is_none());
    }
}
