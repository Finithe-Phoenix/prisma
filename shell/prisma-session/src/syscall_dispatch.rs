//! The guest syscall entry point.
//!
//! Routes an x86-64 Linux syscall number + its six register arguments to the
//! right pointer-checked handler, and encodes the result the way the guest
//! reads `rax`: the success value, or a negative errno on failure. The handlers
//! own the memory safety (every guest pointer goes through a [`GuestRegion`]);
//! this layer owns the routing and the kernel return convention.
//!
//! Coverage grows as handlers land; an unrouted number returns `-ENOSYS`.

use std::time::Instant;

use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::fd_table::FdTable;
use prisma_runtime::guest_signal::SignalState;

use crate::io_syscalls::{self, IoError};
use crate::sig_syscalls::{self, SigError};
use crate::time_syscalls::{self, TimeError};

/// x86-64 Linux syscall numbers we route (`arch/x86/entry/syscalls/syscall_64.tbl`).
mod nr {
    pub const READ: u64 = 0;
    pub const WRITE: u64 = 1;
    pub const CLOSE: u64 = 3;
    pub const LSEEK: u64 = 8;
    pub const RT_SIGACTION: u64 = 13;
    pub const RT_SIGPROCMASK: u64 = 14;
    pub const DUP: u64 = 32;
    pub const DUP2: u64 = 33;
    pub const NANOSLEEP: u64 = 35;
    pub const SCHED_YIELD: u64 = 24;
    pub const FSYNC: u64 = 74;
    pub const FDATASYNC: u64 = 75;
    pub const GETPID: u64 = 39;
    pub const GETUID: u64 = 102;
    pub const GETGID: u64 = 104;
    pub const GETEUID: u64 = 107;
    pub const GETEGID: u64 = 108;
    pub const GETTIMEOFDAY: u64 = 96;
    pub const RT_SIGPENDING: u64 = 127;
    pub const GETTID: u64 = 186;
    pub const SET_TID_ADDRESS: u64 = 218;
    pub const TIME: u64 = 201;
    pub const CLOCK_GETTIME: u64 = 228;
    pub const CLOCK_GETRES: u64 = 229;
    pub const CLOCK_NANOSLEEP: u64 = 230;
}

/// Negative Linux errno values returned in `rax` on failure.
mod errno {
    pub const EIO: i64 = -5;
    pub const EBADF: i64 = -9;
    pub const EFAULT: i64 = -14;
    pub const EINVAL: i64 = -22;
    pub const ENOSYS: i64 = -38;
}

/// The uid/gid the guest sees for `getuid`/`geteuid`/`getgid`/`getegid`. The
/// host is Windows (no POSIX credentials), so the guest runs as a fixed
/// unprivileged user — the conventional non-root id a Linux desktop assigns.
const GUEST_UID: i64 = 1000;

/// Per-thread state the syscall layer carries across calls: the fd table and the
/// signal mask, plus the monotonic-clock reference. The guest memory region is
/// passed per call (it is borrowed from the live address space).
#[derive(Debug)]
pub struct SyscallContext {
    /// Open file descriptors.
    pub fds: FdTable,
    /// Blocked-signal mask + pending queue.
    pub signals: SignalState,
    /// `CLOCK_MONOTONIC` reference point (session start).
    pub monotonic_start: Instant,
}

impl SyscallContext {
    /// A fresh context: standard streams open, nothing blocked, monotonic clock
    /// anchored now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fds: FdTable::new(),
            signals: SignalState::new(),
            monotonic_start: Instant::now(),
        }
    }
}

impl Default for SyscallContext {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn arg_i32(v: u64) -> i32 {
    // The guest passes the value in a 64-bit register; the C ABI reads its low
    // 32 bits as the `int` argument. Truncation is the intended semantics.
    v as i32
}

#[allow(clippy::cast_possible_truncation)]
fn arg_usize(v: u64) -> usize {
    // `size_t`/`count` argument — the register value as a host index/length.
    v as usize
}

#[allow(clippy::cast_possible_wrap)]
fn arg_i64(v: u64) -> i64 {
    // A signed 64-bit argument (e.g. an lseek offset): the register bits
    // reinterpreted as `i64`, which is exactly the C ABI's `off_t`.
    v as i64
}

#[allow(clippy::cast_possible_truncation)]
fn arg_u32(v: u64) -> u32 {
    // An unsigned 32-bit argument (e.g. a signal number) — the low 32 bits.
    v as u32
}

fn io_errno(e: &IoError) -> i64 {
    match e {
        IoError::BadFd => errno::EBADF,
        IoError::Invalid => errno::EINVAL,
        IoError::Fault(_) => errno::EFAULT,
        IoError::Host(_) => errno::EIO,
    }
}

const fn time_errno(e: TimeError) -> i64 {
    match e {
        TimeError::UnknownClock(_) | TimeError::InvalidValue => errno::EINVAL,
        TimeError::Fault(_) => errno::EFAULT,
    }
}

const fn sig_errno(e: SigError) -> i64 {
    match e {
        SigError::BadHow(_) | SigError::BadSignal(_) => errno::EINVAL,
        SigError::Fault(_) => errno::EFAULT,
    }
}

/// Dispatch one guest syscall, returning the value the guest reads in `rax`
/// (the success result, or a negative errno). `mem` is the guest memory the
/// pointer arguments resolve against.
#[allow(clippy::cast_possible_wrap)]
pub fn dispatch(
    ctx: &mut SyscallContext,
    mem: &mut GuestRegion,
    number: u64,
    args: [u64; 6],
) -> i64 {
    match number {
        nr::READ => {
            match io_syscalls::read(&ctx.fds, mem, arg_i32(args[0]), args[1], arg_usize(args[2])) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::WRITE => {
            match io_syscalls::write(&ctx.fds, mem, arg_i32(args[0]), args[1], arg_usize(args[2])) {
                // A short/full write returns the byte count.
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::CLOSE => match io_syscalls::close(&mut ctx.fds, arg_i32(args[0])) {
            Ok(()) => 0,
            Err(e) => io_errno(&e),
        },
        // lseek(fd, offset, whence): offset is signed; whence in args[2].
        nr::LSEEK => {
            match io_syscalls::lseek(
                &ctx.fds,
                arg_i32(args[0]),
                arg_i64(args[1]),
                arg_i32(args[2]),
            ) {
                Ok(pos) => i64::try_from(pos).unwrap_or(i64::MAX),
                Err(e) => io_errno(&e),
            }
        }
        nr::DUP => match io_syscalls::dup(&mut ctx.fds, arg_i32(args[0])) {
            Ok(fd) => i64::from(fd),
            Err(e) => io_errno(&e),
        },
        nr::DUP2 => match io_syscalls::dup2(&mut ctx.fds, arg_i32(args[0]), arg_i32(args[1])) {
            Ok(fd) => i64::from(fd),
            Err(e) => io_errno(&e),
        },
        nr::TIME => match time_syscalls::time(mem, args[0]) {
            Ok(secs) => secs,
            Err(e) => time_errno(e),
        },
        nr::FSYNC => match io_syscalls::fsync(&ctx.fds, arg_i32(args[0])) {
            Ok(()) => 0,
            Err(e) => io_errno(&e),
        },
        nr::FDATASYNC => match io_syscalls::fdatasync(&ctx.fds, arg_i32(args[0])) {
            Ok(()) => 0,
            Err(e) => io_errno(&e),
        },
        nr::RT_SIGPENDING => match sig_syscalls::rt_sigpending(&ctx.signals, mem, args[0]) {
            Ok(()) => 0,
            Err(e) => sig_errno(e),
        },
        // rt_sigaction(sig, act, oldact).
        nr::RT_SIGACTION => {
            match sig_syscalls::rt_sigaction(
                &mut ctx.signals,
                mem,
                arg_u32(args[0]),
                args[1],
                args[2],
            ) {
                Ok(()) => 0,
                Err(e) => sig_errno(e),
            }
        }
        nr::NANOSLEEP => match time_syscalls::nanosleep_request(mem, args[0]) {
            Ok(duration) => {
                std::thread::sleep(duration);
                0
            }
            Err(e) => time_errno(e),
        },
        // clock_nanosleep(clk_id, flags, req, rem): the request is args[2].
        nr::CLOCK_NANOSLEEP => {
            match time_syscalls::clock_nanosleep_request(mem, args[0], args[2]) {
                Ok(duration) => {
                    std::thread::sleep(duration);
                    0
                }
                Err(e) => time_errno(e),
            }
        }
        nr::CLOCK_GETRES => match time_syscalls::clock_getres(mem, args[0], args[1]) {
            Ok(()) => 0,
            Err(e) => time_errno(e),
        },
        nr::RT_SIGPROCMASK => {
            match sig_syscalls::rt_sigprocmask(
                &mut ctx.signals,
                mem,
                arg_i32(args[0]),
                args[1],
                args[2],
            ) {
                Ok(()) => 0,
                Err(e) => sig_errno(e),
            }
        }
        // getpid / gettid: the guest runs inside the host process, so the host
        // pid is the correct answer; a single-threaded guest has tid == pid.
        nr::GETPID | nr::GETTID => i64::from(std::process::id()),
        // set_tid_address(tidptr): the kernel records `tidptr` as the thread's
        // clear_child_tid (cleared + futex-woken on thread exit) and returns the
        // caller's tid. With no thread teardown yet, honouring clear_child_tid is
        // a no-op; the contract that matters to glibc startup is the tid return.
        nr::SET_TID_ADDRESS => i64::from(std::process::id()),
        // sched_yield: relinquish the CPU to the host scheduler, then succeed.
        // Always returns 0 (it cannot fail in the Linux ABI).
        nr::SCHED_YIELD => {
            std::thread::yield_now();
            0
        }
        // getuid / geteuid / getgid / getegid: the host (Windows) has no POSIX
        // uid, so the guest is presented as a single unprivileged user. Real
        // and effective ids coincide — there is no setuid transition to model.
        nr::GETUID | nr::GETEUID | nr::GETGID | nr::GETEGID => GUEST_UID,
        nr::GETTIMEOFDAY => match time_syscalls::gettimeofday(mem, args[0]) {
            Ok(()) => 0,
            Err(e) => time_errno(e),
        },
        nr::CLOCK_GETTIME => {
            match time_syscalls::clock_gettime(mem, args[0], args[1], ctx.monotonic_start) {
                Ok(()) => 0,
                Err(e) => time_errno(e),
            }
        }
        _ => errno::ENOSYS,
    }
}

#[cfg(test)]
mod tests {
    use super::{dispatch, SyscallContext};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_signal::Sigset;
    use prisma_runtime::guest_structs::Timespec;

    const SYS_READ: u64 = 0;
    const SYS_WRITE: u64 = 1;
    const SYS_NANOSLEEP: u64 = 35;
    const SYS_GETTIMEOFDAY: u64 = 96;
    const SYS_CLOCK_GETTIME: u64 = 228;
    const SYS_CLOCK_GETRES: u64 = 229;
    const CLOCK_REALTIME: u64 = 0;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn rt_sigaction_routes_and_rejects_sigkill() {
        const SYS_RT_SIGACTION: u64 = 13;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 32];
        let mut mem = region(&mut buf);
        // rt_sigaction(SIGKILL=9, act=0x1000, oldact=0) -> -EINVAL (-22).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_RT_SIGACTION,
                [9, 0x1000, 0, 0, 0, 0]
            ),
            -22
        );
        // A valid signal with null act/oldact is a no-op success (0).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_RT_SIGACTION, [11, 0, 0, 0, 0, 0]),
            0
        );
    }

    #[test]
    fn lseek_routes_to_the_fd_table() {
        const SYS_LSEEK: u64 = 8;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // lseek on stdout (fd 1) is not seekable -> -EBADF (-9).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_LSEEK, [1, 0, 0, 0, 0, 0]),
            -9
        );
        // An unknown whence -> -EINVAL (-22), checked before the fd.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_LSEEK, [1, 0, 9, 0, 0, 0]),
            -22
        );
    }

    #[test]
    fn write_routes_and_returns_the_byte_count() {
        let mut ctx = SyscallContext::new();
        let mut buf = *b"hello!!!";
        let mut mem = region(&mut buf);
        // write(1, 0x1000, 5) -> 5 (stdout, captured by the harness).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_WRITE, [1, 0x1000, 5, 0, 0, 0]),
            5
        );
    }

    #[test]
    fn write_to_a_bad_fd_returns_negative_ebadf() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // fd 7 is not open -> -EBADF (-9).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_WRITE, [7, 0x1000, 1, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn clock_gettime_routes_and_writes_the_guest_timespec() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_CLOCK_GETTIME,
                [CLOCK_REALTIME, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert!(ts.sec > 1_600_000_000);
    }

    #[test]
    fn an_out_of_range_pointer_returns_negative_efault() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8]; // too small for a 16-byte timeval
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETTIMEOFDAY,
                [0x1000, 0, 0, 0, 0, 0]
            ),
            -14 // -EFAULT
        );
    }

    #[test]
    fn getpid_and_gettid_return_the_host_pid() {
        const SYS_GETPID: u64 = 39;
        const SYS_GETTID: u64 = 186;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        let pid = i64::from(std::process::id());
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]), pid);
        // A single-threaded guest's tid equals its pid.
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETTID, [0; 6]), pid);
        assert!(pid > 0);
    }

    #[test]
    fn credential_syscalls_return_the_unprivileged_id() {
        const SYS_GETUID: u64 = 102;
        const SYS_GETGID: u64 = 104;
        const SYS_GETEUID: u64 = 107;
        const SYS_GETEGID: u64 = 108;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        for nr in [SYS_GETUID, SYS_GETGID, SYS_GETEUID, SYS_GETEGID] {
            // Real and effective ids coincide: the guest is one fixed user.
            assert_eq!(dispatch(&mut ctx, &mut mem, nr, [0; 6]), 1000);
        }
    }

    #[test]
    fn sched_yield_succeeds() {
        const SYS_SCHED_YIELD: u64 = 24;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // sched_yield ignores its arguments and always returns 0.
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_SCHED_YIELD, [0; 6]), 0);
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_SCHED_YIELD, [u64::MAX; 6]),
            0
        );
    }

    #[test]
    fn set_tid_address_returns_the_tid() {
        const SYS_SET_TID_ADDRESS: u64 = 218;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        let pid = i64::from(std::process::id());
        // Returns the caller's tid (== pid here); the tidptr arg is recorded as
        // clear_child_tid (a no-op until thread teardown) and never faults.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SET_TID_ADDRESS,
                [0x1000, 0, 0, 0, 0, 0]
            ),
            pid
        );
        // A wild tidptr is still fine — we do not dereference it.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SET_TID_ADDRESS,
                [u64::MAX, 0, 0, 0, 0, 0]
            ),
            pid
        );
    }

    #[test]
    fn an_unrouted_syscall_is_negative_enosys() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        assert_eq!(dispatch(&mut ctx, &mut mem, 9999, [0; 6]), -38);
    }

    #[test]
    fn close_dup_dup2_route_to_the_fd_table() {
        const SYS_CLOSE: u64 = 3;
        const SYS_DUP: u64 = 32;
        const SYS_DUP2: u64 = 33;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // dup(1) -> 3.
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]), 3);
        // dup2(1, 9) -> 9.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_DUP2, [1, 9, 0, 0, 0, 0]),
            9
        );
        // close(9) -> 0; closing it again -> -EBADF.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_CLOSE, [9, 0, 0, 0, 0, 0]),
            0
        );
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_CLOSE, [9, 0, 0, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn time_routes_and_returns_epoch_seconds() {
        const SYS_TIME: u64 = 201;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // time(NULL) returns the seconds without writing.
        let secs = dispatch(&mut ctx, &mut mem, SYS_TIME, [0, 0, 0, 0, 0, 0]);
        assert!(secs > 1_600_000_000);
    }

    #[test]
    fn fsync_routes_to_the_fd_table() {
        const SYS_FSYNC: u64 = 74;
        const SYS_FDATASYNC: u64 = 75;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // fsync/fdatasync on stdout (1) are a successful no-op (0).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FSYNC, [1, 0, 0, 0, 0, 0]),
            0
        );
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FDATASYNC, [1, 0, 0, 0, 0, 0]),
            0
        );
        // An unopen fd -> -EBADF.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FSYNC, [9, 0, 0, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn clock_nanosleep_routes_a_zero_request_immediately() {
        const SYS_CLOCK_NANOSLEEP: u64 = 230;
        const CLOCK_MONOTONIC: u64 = 1;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&Timespec { sec: 0, nsec: 0 }.to_guest_bytes());
        let mut mem = region(&mut buf);
        // clock_nanosleep(MONOTONIC, flags=0, req=0x1000, rem=0) -> 0 instantly.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_CLOCK_NANOSLEEP,
                [CLOCK_MONOTONIC, 0, 0x1000, 0, 0, 0]
            ),
            0
        );
        // An unknown clock -> -EINVAL.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_CLOCK_NANOSLEEP,
                [42, 0, 0x1000, 0, 0, 0]
            ),
            -22
        );
    }

    #[test]
    fn rt_sigpending_routes_and_writes_the_set() {
        const SYS_RT_SIGPENDING: u64 = 127;
        let mut ctx = SyscallContext::new();
        ctx.signals.raise(11);
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_RT_SIGPENDING,
                [0x1000, 0, 0, 0, 0, 0]
            ),
            0
        );
        let set = Sigset::from_guest_bytes(&buf).unwrap();
        assert!(set.contains(11));
    }

    #[test]
    fn read_from_a_write_only_stream_routes_to_negative_ebadf() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // read(1, ...) — stdout is write-only for the guest -> -EBADF (-9).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_READ, [1, 0x1000, 1, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn nanosleep_routes_and_a_zero_request_returns_immediately() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        buf[..16].copy_from_slice(&Timespec { sec: 0, nsec: 0 }.to_guest_bytes());
        let mut mem = region(&mut buf);
        // A zero-duration nanosleep returns 0 without a meaningful wait.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_NANOSLEEP, [0x1000, 0, 0, 0, 0, 0]),
            0
        );
    }

    #[test]
    fn clock_getres_routes_and_writes_the_resolution() {
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_CLOCK_GETRES,
                [CLOCK_REALTIME, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        let ts = Timespec::from_guest_bytes(mem.read(0x1000, 16).unwrap()).unwrap();
        assert_eq!((ts.sec, ts.nsec), (0, 1)); // 1ns resolution
    }
}
