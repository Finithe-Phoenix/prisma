//! Blocking-call policy for the opt-in threaded session.
//!
//! Guest timespecs are copied while mappings are stable. The driver then waits
//! outside its JIT/arena critical section, where group exit can interrupt it.
//! Host stream/file transfers, positioned I/O, file sync, truncate, seek and
//! descriptor stat return ENOSYS until the driver owns cancellable host I/O.
//! Closing or replacing a file-backed descriptor can also invoke a blocking host
//! close. The dispatcher handles standard-stream markers and guest-only operations.

use std::time::Duration;

use prisma_orchestrator::backed_address_space::BackedAddressSpace;

use prisma_runtime::fd_table::{FdEntry, FdTable};

use crate::syscall_dispatch::{arg_i32, errno, nr, time_errno, O_CLOEXEC};
use crate::time_syscalls;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BlockingCall {
    Immediate,
    Sleep(Duration),
    Return(i64),
}

pub(crate) fn classify(
    mem: &BackedAddressSpace,
    fds: &FdTable,
    sysno: u64,
    args: [u64; 6],
) -> BlockingCall {
    let duration = match sysno {
        nr::NANOSLEEP => time_syscalls::nanosleep_request(mem, args[0]),
        nr::CLOCK_NANOSLEEP => {
            // A relative duration is the only kind the driver can park for.
            // Treating an absolute timestamp as a duration can oversleep by years.
            if args[1] != 0 {
                const TIMER_ABSTIME: u64 = 1;
                return BlockingCall::Return(if args[1] == TIMER_ABSTIME {
                    errno::ENOSYS
                } else {
                    errno::EINVAL
                });
            }
            time_syscalls::clock_nanosleep_request(mem, args[0], args[2])
        }
        nr::SCHED_YIELD => return BlockingCall::Sleep(Duration::ZERO),
        nr::CLOSE if matches!(fds.get(arg_i32(args[0])), Some(FdEntry::File(_))) => {
            return BlockingCall::Return(errno::ENOSYS);
        }
        nr::DUP2 | nr::DUP3 => {
            let source = arg_i32(args[0]);
            let target = arg_i32(args[1]);
            let valid_flags = sysno == nr::DUP2 || (arg_i32(args[2]) & !O_CLOEXEC) == 0;
            if source != target
                && valid_flags
                && fds.is_open(source)
                && matches!(fds.get(target), Some(FdEntry::File(_)))
            {
                return BlockingCall::Return(errno::ENOSYS);
            }
            return BlockingCall::Immediate;
        }
        nr::READ
        | nr::WRITE
        | nr::READV
        | nr::WRITEV
        | nr::PREAD64
        | nr::PWRITE64
        | nr::PREADV
        | nr::PWRITEV
        | nr::FSYNC
        | nr::FDATASYNC
        | nr::FTRUNCATE
        | nr::LSEEK
        | nr::FSTAT => return BlockingCall::Return(errno::ENOSYS),
        _ => return BlockingCall::Immediate,
    };
    match duration {
        Ok(interval) => BlockingCall::Sleep(interval),
        Err(error) => BlockingCall::Return(time_errno(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_orchestrator::address_space::Protection;
    use prisma_runtime::guest_structs::Timespec;

    #[test]
    fn validates_and_copies_relative_sleep_before_parking() {
        let mut mem = BackedAddressSpace::new();
        let fds = FdTable::new();
        let req = 0x4000;
        mem.map_with_bytes(
            req,
            &Timespec { sec: 2, nsec: 17 }.to_guest_bytes(),
            Protection::ReadWrite,
        )
        .unwrap();
        let expected = BlockingCall::Sleep(Duration::new(2, 17));
        assert_eq!(
            classify(&mem, &fds, nr::NANOSLEEP, [req, 0, 0, 0, 0, 0]),
            expected
        );
        assert_eq!(
            classify(&mem, &fds, nr::CLOCK_NANOSLEEP, [1, 0, req, 0, 0, 0]),
            expected
        );
        assert_eq!(
            classify(&mem, &fds, nr::NANOSLEEP, [0; 6]),
            BlockingCall::Return(errno::EFAULT)
        );
        assert_eq!(
            classify(&mem, &fds, nr::CLOCK_NANOSLEEP, [42, 0, req, 0, 0, 0]),
            BlockingCall::Return(errno::EINVAL)
        );
        assert_eq!(
            classify(&mem, &fds, nr::CLOCK_NANOSLEEP, [1, 1, req, 0, 0, 0]),
            BlockingCall::Return(errno::ENOSYS)
        );
        mem.write(req, &Timespec { sec: -1, nsec: 0 }.to_guest_bytes())
            .unwrap();
        assert_eq!(
            classify(&mem, &fds, nr::NANOSLEEP, [req, 0, 0, 0, 0, 0]),
            BlockingCall::Return(errno::EINVAL)
        );
    }

    #[test]
    fn host_io_is_deferred_but_guest_memory_and_metadata_are_dispatched() {
        let mem = BackedAddressSpace::new();
        let fds = FdTable::new();
        for nr in [nr::READ, nr::WRITEV, nr::PREADV, nr::FDATASYNC, nr::FSTAT] {
            assert_eq!(
                classify(&mem, &fds, nr, [0; 6]),
                BlockingCall::Return(errno::ENOSYS)
            );
        }
        for nr in [
            nr::BRK,
            nr::POLL,
            nr::GETPID,
            nr::IOCTL,
            nr::FSTATFS,
            nr::FCNTL,
        ] {
            assert_eq!(classify(&mem, &fds, nr, [0; 6]), BlockingCall::Immediate);
        }
        assert_eq!(
            classify(&mem, &fds, nr::SCHED_YIELD, [0; 6]),
            BlockingCall::Sleep(Duration::ZERO)
        );
    }

    #[test]
    fn close_and_dup_only_restrict_calls_that_would_drop_a_file() {
        let mem = BackedAddressSpace::new();
        let mut fds = FdTable::new();
        // Replace stdin to exercise a host File installed at a standard fd number.
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        assert!(fds.close(0));
        assert_eq!(fds.allocate(FdEntry::File(file)), Some(0));
        // An aliased standard-stream marker at a non-standard fd also stays usable.
        assert_eq!(fds.dup(1), Some(3));

        let route =
            |nr, source, target, flags| classify(&mem, &fds, nr, [source, target, flags, 0, 0, 0]);
        assert_eq!(
            route(nr::CLOSE, 0, 0, 0),
            BlockingCall::Return(errno::ENOSYS)
        );
        assert_eq!(route(nr::CLOSE, 3, 0, 0), BlockingCall::Immediate);
        assert_eq!(route(nr::CLOSE, 4, 0, 0), BlockingCall::Immediate);
        for nr in [nr::DUP2, nr::DUP3] {
            assert_eq!(route(nr, 1, 0, 0), BlockingCall::Return(errno::ENOSYS));
            assert_eq!(route(nr, 0, 3, 0), BlockingCall::Immediate);
            assert_eq!(route(nr, 0, 0, 0), BlockingCall::Immediate);
            assert_eq!(route(nr, 4, 0, 0), BlockingCall::Immediate);
        }
        assert_eq!(route(nr::DUP3, 3, 0, 1), BlockingCall::Immediate);
        assert_eq!(
            route(nr::DUP3, 3, 0, 0o2_000_000),
            BlockingCall::Return(errno::ENOSYS)
        );
    }
}
