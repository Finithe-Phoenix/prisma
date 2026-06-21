//! The guest syscall entry point.
//!
//! Routes an x86-64 Linux syscall number + its six register arguments to the
//! right pointer-checked handler, and encodes the result the way the guest
//! reads `rax`: the success value, or a negative errno on failure. The handlers
//! own the memory safety (every guest pointer goes through a [`GuestRegion`]);
//! this layer owns the routing and the kernel return convention.
//!
//! Coverage grows as handlers land; an unrouted number returns `-ENOSYS`.

use std::num::NonZeroUsize;
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
    pub const READV: u64 = 19;
    pub const WRITEV: u64 = 20;
    pub const PREAD64: u64 = 17;
    pub const PWRITE64: u64 = 18;
    pub const PREADV: u64 = 295;
    pub const PWRITEV: u64 = 296;
    pub const CLOSE: u64 = 3;
    pub const FSTAT: u64 = 5;
    pub const LSEEK: u64 = 8;
    pub const RT_SIGACTION: u64 = 13;
    pub const RT_SIGPROCMASK: u64 = 14;
    pub const TKILL: u64 = 200;
    pub const TGKILL: u64 = 234;
    pub const DUP: u64 = 32;
    pub const DUP2: u64 = 33;
    pub const DUP3: u64 = 292;
    pub const NANOSLEEP: u64 = 35;
    pub const SCHED_YIELD: u64 = 24;
    pub const FSYNC: u64 = 74;
    pub const FDATASYNC: u64 = 75;
    pub const FTRUNCATE: u64 = 77;
    pub const GETPID: u64 = 39;
    pub const GETUID: u64 = 102;
    pub const GETGID: u64 = 104;
    pub const GETEUID: u64 = 107;
    pub const GETEGID: u64 = 108;
    pub const UMASK: u64 = 95;
    pub const GETPGRP: u64 = 111;
    pub const GETPGID: u64 = 121;
    pub const GETSID: u64 = 124;
    pub const GETTIMEOFDAY: u64 = 96;
    pub const RT_SIGPENDING: u64 = 127;
    pub const GETTID: u64 = 186;
    pub const SET_TID_ADDRESS: u64 = 218;
    pub const SCHED_SETAFFINITY: u64 = 203;
    pub const SCHED_GETAFFINITY: u64 = 204;
    pub const TIME: u64 = 201;
    pub const CLOCK_GETTIME: u64 = 228;
    pub const CLOCK_GETRES: u64 = 229;
    pub const CLOCK_NANOSLEEP: u64 = 230;
}

/// Negative Linux errno values returned in `rax` on failure.
mod errno {
    pub const ESRCH: i64 = -3;
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

/// The only flag `dup3` accepts (`O_CLOEXEC`, octal 02000000 on x86-64 Linux).
/// We do not model close-on-exec yet, but accepting the flag — and rejecting any
/// other bit — keeps the ABI faithful.
const O_CLOEXEC: i32 = 0o2_000_000;

/// The CPU-affinity mask the guest sees: the low `available_parallelism` bits
/// set, capped at 64 CPUs (a single `u64` word). Shared by `sched_getaffinity`
/// (reported) and `sched_setaffinity` (the set of CPUs a request may select).
fn online_cpu_mask() -> u64 {
    let ncpus = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    if ncpus >= 64 {
        u64::MAX
    } else {
        (1u64 << ncpus) - 1
    }
}

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
    /// File-mode creation mask (`umask`): the bits cleared from the mode of files
    /// the guest creates. The conventional default is `0o022`.
    pub umask: u32,
}

impl SyscallContext {
    /// A fresh context: standard streams open, nothing blocked, monotonic clock
    /// anchored now, the conventional `0o022` umask.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fds: FdTable::new(),
            signals: SignalState::new(),
            monotonic_start: Instant::now(),
            umask: 0o022,
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
        // readv/writev(fd, iov, iovcnt): scatter/gather across the iovec array.
        nr::READV => {
            match io_syscalls::readv(&ctx.fds, mem, arg_i32(args[0]), args[1], arg_i32(args[2])) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::WRITEV => {
            match io_syscalls::writev(&ctx.fds, mem, arg_i32(args[0]), args[1], arg_i32(args[2])) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        // pread64/pwrite64(fd, buf, count, offset): positional I/O at args[3].
        nr::PREAD64 => {
            match io_syscalls::pread(
                &ctx.fds,
                mem,
                arg_i32(args[0]),
                args[1],
                arg_usize(args[2]),
                args[3],
            ) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::PWRITE64 => {
            match io_syscalls::pwrite(
                &ctx.fds,
                mem,
                arg_i32(args[0]),
                args[1],
                arg_usize(args[2]),
                args[3],
            ) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        // preadv/pwritev(fd, iov, iovcnt, offset): vectored positional I/O. On
        // x86-64 the full offset is in args[3] (the pos_h split is unused).
        nr::PREADV => {
            match io_syscalls::preadv(
                &ctx.fds,
                mem,
                arg_i32(args[0]),
                args[1],
                arg_i32(args[2]),
                args[3],
            ) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::PWRITEV => {
            match io_syscalls::pwritev(
                &ctx.fds,
                mem,
                arg_i32(args[0]),
                args[1],
                arg_i32(args[2]),
                args[3],
            ) {
                Ok(n) => n as i64,
                Err(e) => io_errno(&e),
            }
        }
        nr::CLOSE => match io_syscalls::close(&mut ctx.fds, arg_i32(args[0])) {
            Ok(()) => 0,
            Err(e) => io_errno(&e),
        },
        // fstat(fd, statbuf): write the file metadata to the guest stat pointer.
        nr::FSTAT => match io_syscalls::fstat(&ctx.fds, mem, arg_i32(args[0]), args[1]) {
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
        // dup3(oldfd, newfd, flags): like dup2, but the kernel rejects
        // oldfd == newfd (dup2 returns newfd unchanged there) and any flag bit
        // other than O_CLOEXEC -> EINVAL.
        nr::DUP3 => {
            let oldfd = arg_i32(args[0]);
            let newfd = arg_i32(args[1]);
            let flags = arg_i32(args[2]);
            if oldfd == newfd || (flags & !O_CLOEXEC) != 0 {
                errno::EINVAL
            } else {
                match io_syscalls::dup2(&mut ctx.fds, oldfd, newfd) {
                    Ok(fd) => i64::from(fd),
                    Err(e) => io_errno(&e),
                }
            }
        }
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
        // ftruncate(fd, length): resize the file (length is a signed off_t).
        nr::FTRUNCATE => match io_syscalls::ftruncate(&ctx.fds, arg_i32(args[0]), arg_i64(args[1]))
        {
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
        // tkill(tid, sig) / tgkill(tgid, tid, sig): send a signal to a thread.
        // The guest is single-threaded, so the only valid target is itself
        // (tid == pid, and for tgkill tgid == pid too); a non-positive id is
        // EINVAL and any other id is ESRCH. The signal itself is validated and
        // raised by send_signal.
        nr::TKILL => {
            let tid = arg_i32(args[0]);
            let me = std::process::id();
            if tid <= 0 {
                errno::EINVAL
            } else if tid as u32 != me {
                errno::ESRCH
            } else {
                match sig_syscalls::send_signal(&mut ctx.signals, arg_u32(args[1])) {
                    Ok(()) => 0,
                    Err(e) => sig_errno(e),
                }
            }
        }
        nr::TGKILL => {
            let tgid = arg_i32(args[0]);
            let tid = arg_i32(args[1]);
            let me = std::process::id();
            if tgid <= 0 || tid <= 0 {
                errno::EINVAL
            } else if tgid as u32 != me || tid as u32 != me {
                errno::ESRCH
            } else {
                match sig_syscalls::send_signal(&mut ctx.signals, arg_u32(args[2])) {
                    Ok(()) => 0,
                    Err(e) => sig_errno(e),
                }
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
        // umask(mask): install `mask & 0o777` as the file-mode creation mask and
        // return the previous one. Always succeeds.
        nr::UMASK => {
            let old = ctx.umask;
            ctx.umask = arg_u32(args[0]) & 0o777;
            i64::from(old)
        }
        // getpgrp: the calling process's group id. Our lone guest is its own
        // process-group leader, so its pgid is its pid.
        nr::GETPGRP => i64::from(std::process::id()),
        // getsid(pid) / getpgid(pid): the session / process-group id of `pid`
        // (0 = the caller). The lone guest leads both its session and its group,
        // so each equals its pid; any other pid does not exist in our
        // single-process model -> ESRCH.
        nr::GETSID | nr::GETPGID => {
            let me = std::process::id();
            let want = arg_i32(args[0]);
            if want == 0 || (want > 0 && want as u32 == me) {
                i64::from(me)
            } else {
                errno::ESRCH
            }
        }
        // sched_getaffinity(pid, cpusetsize, mask): write the guest's CPU
        // affinity to the `mask` pointer. We model up to 64 host CPUs (a single
        // u64 word), so `cpusetsize` must be at least 8 bytes (else EINVAL, as
        // the kernel does when the buffer is smaller than its cpumask). pid 0 or
        // self is the caller; any other pid is ESRCH. Returns the bytes written.
        nr::SCHED_GETAFFINITY => {
            let me = std::process::id();
            let want = arg_i32(args[0]);
            if want != 0 && !(want > 0 && want as u32 == me) {
                errno::ESRCH
            } else if arg_usize(args[1]) < 8 {
                errno::EINVAL
            } else {
                match mem.write(args[2], &online_cpu_mask().to_le_bytes()) {
                    Ok(()) => 8,
                    Err(_) => errno::EFAULT,
                }
            }
        }
        // sched_setaffinity(pid, cpusetsize, mask): read the requested CPU mask
        // and accept it if it selects at least one online CPU (else EINVAL, as
        // the kernel does). Affinity is advisory in our model — the guest runs
        // on every available CPU regardless — so a valid request is a no-op
        // success. pid 0/self is the caller; any other pid is ESRCH.
        nr::SCHED_SETAFFINITY => {
            let me = std::process::id();
            let want = arg_i32(args[0]);
            let size = arg_usize(args[1]);
            if want != 0 && !(want > 0 && want as u32 == me) {
                errno::ESRCH
            } else if size == 0 {
                errno::EINVAL
            } else {
                let n = size.min(8);
                match mem.read(args[2], n) {
                    Ok(bytes) => {
                        let mut raw = [0u8; 8];
                        raw[..n].copy_from_slice(bytes);
                        if u64::from_le_bytes(raw) & online_cpu_mask() == 0 {
                            errno::EINVAL // the mask selects no online CPU
                        } else {
                            0
                        }
                    }
                    Err(_) => errno::EFAULT,
                }
            }
        }
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
    use prisma_runtime::guest_structs::{Iovec, Timespec};

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
    fn getpgrp_returns_the_pid_as_the_group_leader() {
        const SYS_GETPGRP: u64 = 111;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // The lone guest is its own process-group leader: pgid == pid.
        let pid = i64::from(std::process::id());
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETPGRP, [0; 6]), pid);
        // It takes no arguments, so garbage registers change nothing.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETPGRP, [u64::MAX; 6]),
            pid
        );
    }

    #[test]
    fn getsid_returns_the_sid_for_self_and_esrch_for_others() {
        const SYS_GETSID: u64 = 124;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        let pid = std::process::id();
        // getsid(0) -> own session id (== pid, the guest is its own leader).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETSID, [0, 0, 0, 0, 0, 0]),
            i64::from(pid)
        );
        // getsid(self_pid) -> same.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETSID,
                [u64::from(pid), 0, 0, 0, 0, 0]
            ),
            i64::from(pid)
        );
        // getsid of any other pid -> -ESRCH (-3): no such process here.
        let other = u64::from(pid).wrapping_add(1);
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETSID, [other, 0, 0, 0, 0, 0]),
            -3
        );
    }

    #[test]
    fn getpgid_mirrors_getsid_for_self_and_others() {
        const SYS_GETPGID: u64 = 121;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        let pid = std::process::id();
        // getpgid(0) and getpgid(self) -> own group id (== pid, the leader).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETPGID, [0, 0, 0, 0, 0, 0]),
            i64::from(pid)
        );
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETPGID,
                [u64::from(pid), 0, 0, 0, 0, 0]
            ),
            i64::from(pid)
        );
        // Any other pid -> -ESRCH (-3): no such process here.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETPGID,
                [u64::from(pid).wrapping_add(1), 0, 0, 0, 0, 0]
            ),
            -3
        );
    }

    #[test]
    fn dup3_requires_distinct_fds_and_valid_flags() {
        const SYS_DUP3: u64 = 292;
        const O_CLOEXEC: u64 = 0o2_000_000;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // dup3(1, 1, 0): oldfd == newfd -> -EINVAL (dup2 would allow it).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 1, 0, 0, 0, 0]),
            -22
        );
        // dup3(1, 9, <bad flag>) -> -EINVAL before any fd work.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 9, 0x1, 0, 0, 0]),
            -22
        );
        // dup3(1, 9, O_CLOEXEC) -> 9 (stdout duplicated onto fd 9).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 9, O_CLOEXEC, 0, 0, 0]),
            9
        );
        assert!(ctx.fds.is_open(9));
    }

    #[test]
    fn umask_returns_the_previous_mask_and_installs_the_new() {
        const SYS_UMASK: u64 = 95;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // The default mask is 0o022; the first call returns it.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_UMASK, [0o077, 0, 0, 0, 0, 0]),
            0o022
        );
        // The next call returns the just-installed 0o077; only the low 9 mode
        // bits of the argument are kept (0o7777 -> 0o777).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_UMASK, [0o7777, 0, 0, 0, 0, 0]),
            0o077
        );
        assert_eq!(ctx.umask, 0o777);
    }

    #[test]
    fn sched_getaffinity_writes_a_nonempty_cpu_mask() {
        const SYS_SCHED_GETAFFINITY: u64 = 204;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // sched_getaffinity(0, 8, mask=0x1000) writes 8 bytes and returns 8.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETAFFINITY,
                [0, 8, 0x1000, 0, 0, 0]
            ),
            8
        );
        let mask = u64::from_le_bytes(mem.read(0x1000, 8).unwrap().try_into().unwrap());
        // At least one CPU is online, so the mask has at least bit 0 set.
        assert!(mask & 1 == 1);
        assert!(mask != 0);
        // A cpusetsize below the 8-byte model mask is -EINVAL.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETAFFINITY,
                [0, 4, 0x1000, 0, 0, 0]
            ),
            -22
        );
        // Another process's affinity is -ESRCH.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETAFFINITY,
                [999_999, 8, 0x1000, 0, 0, 0]
            ),
            -3
        );
        // A bad mask pointer is -EFAULT (buf is only 16 bytes from 0x1000).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETAFFINITY,
                [0, 8, 0x100A, 0, 0, 0]
            ),
            -14
        );
    }

    #[test]
    fn sched_setaffinity_accepts_a_valid_mask_and_rejects_an_empty_one() {
        const SYS_SCHED_SETAFFINITY: u64 = 203;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        // A mask selecting CPU 0 (bit 0) at guest 0x1000.
        buf[0] = 1;
        let mut mem = region(&mut buf);
        // sched_setaffinity(0, 8, 0x1000) accepts (CPU 0 is online) -> 0.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETAFFINITY,
                [0, 8, 0x1000, 0, 0, 0]
            ),
            0
        );
        // A zero cpusetsize is -EINVAL.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETAFFINITY,
                [0, 0, 0x1000, 0, 0, 0]
            ),
            -22
        );
        // Another process is -ESRCH.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETAFFINITY,
                [999_999, 8, 0x1000, 0, 0, 0]
            ),
            -3
        );
        // A bad mask pointer is -EFAULT.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETAFFINITY,
                [0, 8, 0x100A, 0, 0, 0]
            ),
            -14
        );
    }

    #[test]
    fn sched_setaffinity_rejects_an_empty_mask() {
        const SYS_SCHED_SETAFFINITY: u64 = 203;
        let mut ctx = SyscallContext::new();
        // An all-zero mask selects no CPU at all -> -EINVAL, independent of how
        // many CPUs the host has.
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETAFFINITY,
                [0, 8, 0x1000, 0, 0, 0]
            ),
            -22
        );
    }

    #[test]
    fn readv_and_writev_route_through_the_dispatcher() {
        const SYS_READV: u64 = 19;
        const SYS_WRITEV: u64 = 20;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 64];
        buf[0..3].copy_from_slice(b"foo");
        buf[16..19].copy_from_slice(b"bar");
        // iovec array at guest 0x1020 -> {0x1000,3}, {0x1010,3}.
        buf[0x20..0x30].copy_from_slice(
            &Iovec {
                base: 0x1000,
                len: 3,
            }
            .to_guest_bytes(),
        );
        buf[0x30..0x40].copy_from_slice(
            &Iovec {
                base: 0x1010,
                len: 3,
            }
            .to_guest_bytes(),
        );
        let mut mem = region(&mut buf);
        // writev(1, 0x1020, 2) gathers "foo"+"bar" -> 6 bytes to stdout.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_WRITEV, [1, 0x1020, 2, 0, 0, 0]),
            6
        );
        // readv from an unopen fd validates the (writable) dest buffers, then
        // faults on the fd lookup -> -EBADF, never blocking.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_READV, [7, 0x1020, 2, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn tkill_and_tgkill_signal_the_guest_itself() {
        const SYS_TKILL: u64 = 200;
        const SYS_TGKILL: u64 = 234;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        let pid = u64::from(std::process::id());

        // tkill(self, 11) raises signal 11 pending; returns 0.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_TKILL, [pid, 11, 0, 0, 0, 0]),
            0
        );
        assert!(ctx.signals.pending_set().contains(11));
        // tgkill(self, self, 17) raises 17.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_TGKILL, [pid, pid, 17, 0, 0, 0]),
            0
        );
        assert!(ctx.signals.pending_set().contains(17));

        // Another thread/process id is -ESRCH.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_TKILL, [pid + 1, 11, 0, 0, 0, 0]),
            -3
        );
        // A non-positive id is -EINVAL.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_TKILL, [0, 11, 0, 0, 0, 0]),
            -22
        );
        // An out-of-range signal is -EINVAL (from send_signal).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_TKILL, [pid, 99, 0, 0, 0, 0]),
            -22
        );
    }

    #[test]
    fn pread64_and_pwrite64_route_with_their_offset() {
        use prisma_runtime::fd_table::FdEntry;
        use std::io::Write as _;
        const SYS_PREAD64: u64 = 17;
        const SYS_PWRITE64: u64 = 18;
        let path = std::env::temp_dir().join(format!("prisma_disp_pio_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed");
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open");
        let mut ctx = SyscallContext::new();
        let fd = u64::try_from(ctx.fds.allocate(FdEntry::File(file)).expect("alloc")).unwrap();

        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // pread64(fd, 0x1000, 5, 6) reads "world" -> 5.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PREAD64, [fd, 0x1000, 5, 6, 0, 0]),
            5
        );
        assert_eq!(mem.read(0x1000, 5).unwrap(), b"world");
        // pwrite64(fd, 0x1008, 2, 0) writes "XY" at offset 0 -> 2.
        buf[8..10].copy_from_slice(b"XY");
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PWRITE64, [fd, 0x1008, 2, 0, 0, 0]),
            2
        );
        // pread64 on a non-seekable stream (stdin) -> -EINVAL.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PREAD64, [0, 0x1000, 1, 0, 0, 0]),
            -22
        );

        // Resource discipline: drop the table (closes the fd) then unlink.
        ctx.fds = prisma_runtime::fd_table::FdTable::new();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ftruncate_routes_to_a_file_resize() {
        use prisma_runtime::fd_table::FdEntry;
        use std::io::Write as _;
        const SYS_FTRUNCATE: u64 = 77;
        let path =
            std::env::temp_dir().join(format!("prisma_disp_trunc_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed");
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open");
        let mut ctx = SyscallContext::new();
        let fd = u64::try_from(ctx.fds.allocate(FdEntry::File(file)).expect("alloc")).unwrap();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);

        // ftruncate(fd, 4) -> 0, file shrinks to 4 bytes.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FTRUNCATE, [fd, 4, 0, 0, 0, 0]),
            0
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 4);
        // A negative length is -EINVAL; stdout (1) is not a regular file -> -EINVAL.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_FTRUNCATE,
                [fd, u64::MAX, 0, 0, 0, 0]
            ),
            -22
        );
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FTRUNCATE, [1, 0, 0, 0, 0, 0]),
            -22
        );

        ctx.fds = prisma_runtime::fd_table::FdTable::new();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn preadv_and_pwritev_route_with_their_offset() {
        use prisma_runtime::fd_table::FdEntry;
        use prisma_runtime::guest_structs::Iovec;
        const SYS_PREADV: u64 = 295;
        const SYS_PWRITEV: u64 = 296;
        let path = std::env::temp_dir().join(format!("prisma_disp_pv_{}.tmp", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("temp file");
        let mut ctx = SyscallContext::new();
        let fd = u64::try_from(ctx.fds.allocate(FdEntry::File(file)).expect("alloc")).unwrap();

        let mut buf = [0u8; 128];
        buf[0..3].copy_from_slice(b"foo");
        buf[16..19].copy_from_slice(b"bar");
        for (at, iov) in [
            (
                0x20,
                Iovec {
                    base: 0x1000,
                    len: 3,
                },
            ),
            (
                0x30,
                Iovec {
                    base: 0x1010,
                    len: 3,
                },
            ),
            (
                0x60,
                Iovec {
                    base: 0x1040,
                    len: 3,
                },
            ),
            (
                0x70,
                Iovec {
                    base: 0x1050,
                    len: 3,
                },
            ),
        ] {
            buf[at..at + 16].copy_from_slice(&iov.to_guest_bytes());
        }
        let mut mem = region(&mut buf);
        // pwritev at offset 8 gathers "foobar" -> 6.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PWRITEV, [fd, 0x1020, 2, 8, 0, 0]),
            6
        );
        // preadv at offset 8 scatters it back into dst1/dst2.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PREADV, [fd, 0x1060, 2, 8, 0, 0]),
            6
        );
        assert_eq!(mem.read(0x1040, 3).unwrap(), b"foo");
        assert_eq!(mem.read(0x1050, 3).unwrap(), b"bar");

        ctx.fds = prisma_runtime::fd_table::FdTable::new();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fstat_routes_and_writes_the_guest_stat() {
        use prisma_runtime::fd_table::FdEntry;
        use prisma_runtime::guest_structs::Stat;
        use std::io::Write as _;
        const SYS_FSTAT: u64 = 5;
        let path =
            std::env::temp_dir().join(format!("prisma_disp_fstat_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed"); // 11 bytes
        }
        let file = std::fs::File::open(&path).expect("open");
        let mut ctx = SyscallContext::new();
        let fd = u64::try_from(ctx.fds.allocate(FdEntry::File(file)).expect("alloc")).unwrap();

        let mut buf = [0u8; 160];
        let mut mem = region(&mut buf);
        // fstat(fd, 0x1000) -> 0, writes the stat with size 11.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FSTAT, [fd, 0x1000, 0, 0, 0, 0]),
            0
        );
        let st = Stat::from_guest_bytes(mem.read(0x1000, Stat::SIZE).unwrap()).unwrap();
        assert_eq!(st.size, 11);
        // A bad statbuf pointer is -EFAULT; an unopen fd is -EBADF.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FSTAT, [fd, 0x1090, 0, 0, 0, 0]),
            -14
        );
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FSTAT, [88, 0x1000, 0, 0, 0, 0]),
            -9
        );

        ctx.fds = prisma_runtime::fd_table::FdTable::new();
        let _ = std::fs::remove_file(&path);
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
