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
use prisma_runtime::guest_structs::{ITimerval, SigAltStack, Timeval};

use crate::info_syscalls::{self, RusageError};
use crate::io_syscalls::{self, GetcwdError, IoError};
use crate::resource_syscalls::{self, RlimitError};
use crate::sched_syscalls::{self, SchedError};
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
    pub const POLL: u64 = 7;
    pub const PPOLL: u64 = 271;
    pub const LSEEK: u64 = 8;
    pub const RT_SIGACTION: u64 = 13;
    pub const RT_SIGPROCMASK: u64 = 14;
    pub const SIGALTSTACK: u64 = 131;
    pub const TKILL: u64 = 200;
    pub const TGKILL: u64 = 234;
    pub const DUP: u64 = 32;
    pub const DUP2: u64 = 33;
    pub const DUP3: u64 = 292;
    pub const FCNTL: u64 = 72;
    pub const GETCWD: u64 = 79;
    pub const GETCPU: u64 = 309;
    pub const SYSINFO: u64 = 99;
    pub const GETRLIMIT: u64 = 97;
    pub const GETRUSAGE: u64 = 98;
    pub const SETRLIMIT: u64 = 160;
    pub const PRLIMIT64: u64 = 302;
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
    pub const SET_ROBUST_LIST: u64 = 273;
    pub const MEMBARRIER: u64 = 324;
    pub const SCHED_SETAFFINITY: u64 = 203;
    pub const SCHED_GETAFFINITY: u64 = 204;
    pub const SCHED_GET_PRIORITY_MAX: u64 = 146;
    pub const SCHED_GET_PRIORITY_MIN: u64 = 147;
    pub const GETPRIORITY: u64 = 140;
    pub const SETPRIORITY: u64 = 141;
    pub const SCHED_SETSCHEDULER: u64 = 144;
    pub const SCHED_GETSCHEDULER: u64 = 145;
    pub const SCHED_SETPARAM: u64 = 142;
    pub const SCHED_GETPARAM: u64 = 143;
    pub const TIME: u64 = 201;
    pub const TIMES: u64 = 100;
    pub const SETITIMER: u64 = 38;
    pub const GETITIMER: u64 = 36;
    pub const UNAME: u64 = 63;
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
    pub const ERANGE: i64 = -34;
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
    /// The alternate signal stack (`sigaltstack`); disabled by default.
    pub altstack: SigAltStack,
    /// The three interval timers (`setitimer`/`getitimer`), indexed by `which`:
    /// `ITIMER_REAL` (0), `ITIMER_VIRTUAL` (1), `ITIMER_PROF` (2). All disarmed
    /// (zero) by default. Expiry delivery is not modelled yet; this preserves the
    /// armed value across `setitimer`/`getitimer` round-trips so a guest reads
    /// back what it set.
    pub itimers: [ITimerval; 3],
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
            // SS_DISABLE (flags = 2): no alternate stack installed yet.
            altstack: SigAltStack {
                sp: 0,
                flags: 2,
                size: 0,
            },
            itimers: [ZERO_ITIMERVAL; 3],
        }
    }
}

/// A disarmed interval timer: both the reload period and the time-to-next-expiry
/// are zero.
const ZERO_ITIMERVAL: ITimerval = ITimerval {
    interval: Timeval { sec: 0, usec: 0 },
    value: Timeval { sec: 0, usec: 0 },
};

/// The number of interval timers (`ITIMER_REAL`/`VIRTUAL`/`PROF`); a `which`
/// outside `0..ITIMER_COUNT` is `EINVAL`.
const ITIMER_COUNT: u64 = 3;

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

const fn rusage_errno(e: &RusageError) -> i64 {
    match e {
        RusageError::InvalidWho => errno::EINVAL,
        RusageError::Fault(_) => errno::EFAULT,
    }
}

const fn rlimit_errno(e: &RlimitError) -> i64 {
    match e {
        RlimitError::InvalidResource => errno::EINVAL,
        RlimitError::Fault(_) => errno::EFAULT,
    }
}

const fn sched_errno(e: &SchedError) -> i64 {
    match e {
        SchedError::InvalidParam => errno::EINVAL,
        SchedError::Fault(_) => errno::EFAULT,
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
        // poll(fds, nfds, timeout): readiness is immediate in this model, so the
        // timeout (args[2]) is unused and the call never blocks.
        nr::POLL => match io_syscalls::poll(&ctx.fds, mem, args[0], arg_u32(args[1])) {
            Ok(n) => n as i64,
            Err(e) => io_errno(&e),
        },
        // ppoll(fds, nfds, tmo, sigmask, sigsetsize): the tmo timespec (args[2])
        // is range-checked but unused; sigmask/sigsetsize are ignored.
        nr::PPOLL => match io_syscalls::ppoll(&ctx.fds, mem, args[0], arg_u32(args[1]), args[2]) {
            Ok(n) => n as i64,
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
        // fcntl(fd, cmd, arg): F_DUPFD/-CLOEXEC duplicate onto a floor, the
        // descriptor/status-flag and advisory-lock commands round-trip in the
        // single-process model. The handler validates the fd and cmd.
        nr::FCNTL => match io_syscalls::fcntl(
            &mut ctx.fds,
            mem,
            arg_i32(args[0]),
            arg_i32(args[1]),
            args[2],
        ) {
            Ok(v) => v,
            Err(e) => io_errno(&e),
        },
        nr::TIME => match time_syscalls::time(mem, args[0]) {
            Ok(secs) => secs,
            Err(e) => time_errno(e),
        },
        // uname(buf): write the system identification; a bad pointer is EFAULT.
        nr::UNAME => match info_syscalls::uname(mem, args[0]) {
            Ok(()) => 0,
            Err(_) => errno::EFAULT,
        },
        // getcwd(buf, size): write the cwd path; ERANGE if the buffer is too
        // small, EFAULT if it is unwritable. Returns the path length incl. NUL.
        nr::GETCWD => match io_syscalls::getcwd(mem, args[0], args[1] as usize) {
            Ok(n) => i64::try_from(n).unwrap_or(errno::EINVAL),
            Err(GetcwdError::Range) => errno::ERANGE,
            Err(GetcwdError::Fault(_)) => errno::EFAULT,
        },
        // getrusage(who, usage): report the (all-zero) resource counters.
        nr::GETRUSAGE => match info_syscalls::getrusage(mem, arg_i32(args[0]), args[1]) {
            Ok(()) => 0,
            Err(e) => rusage_errno(&e),
        },
        // getcpu(cpu, node, tcache): report CPU 0 / node 0; tcache is ignored.
        nr::GETCPU => match info_syscalls::getcpu(mem, args[0], args[1]) {
            Ok(()) => 0,
            Err(_) => errno::EFAULT,
        },
        // sysinfo(buf): write system stats (real uptime, synthetic memory).
        nr::SYSINFO => match info_syscalls::sysinfo(mem, args[0], ctx.monotonic_start) {
            Ok(()) => 0,
            Err(_) => errno::EFAULT,
        },
        // getrlimit(resource, rlim): write the session's fixed limit.
        nr::GETRLIMIT => match resource_syscalls::getrlimit(mem, arg_u32(args[0]), args[1]) {
            Ok(()) => 0,
            Err(e) => rlimit_errno(&e),
        },
        // setrlimit(resource, rlim): the limits are fixed, so the new value is
        // accepted (and range-checked) but not stored — prlimit64 with no old.
        nr::SETRLIMIT => match resource_syscalls::prlimit64(mem, arg_u32(args[0]), args[1], 0) {
            Ok(()) => 0,
            Err(e) => rlimit_errno(&e),
        },
        // prlimit64(pid, resource, new, old): report old, accept-without-storing
        // new. pid is ignored (single-process model).
        nr::PRLIMIT64 => {
            match resource_syscalls::prlimit64(mem, arg_u32(args[1]), args[2], args[3]) {
                Ok(()) => 0,
                Err(e) => rlimit_errno(&e),
            }
        }
        // times(buf): write the process tms (zeroed CPU fields) and return the
        // elapsed wall-clock ticks since the session started.
        nr::TIMES => match time_syscalls::times(mem, args[0], ctx.monotonic_start) {
            Ok(ticks) => ticks,
            Err(e) => time_errno(e),
        },
        // setitimer(which, new, old): write the prior value to `old` (when
        // non-null), then arm `which` with the value at `new`. The armed value is
        // preserved in the context; expiry delivery is not modelled yet.
        nr::SETITIMER => {
            let which = args[0];
            if which >= ITIMER_COUNT {
                errno::EINVAL
            } else {
                let slot = which as usize;
                match time_syscalls::setitimer(mem, args[1], args[2], ctx.itimers[slot]) {
                    Ok(next) => {
                        ctx.itimers[slot] = next;
                        0
                    }
                    Err(e) => time_errno(e),
                }
            }
        }
        // getitimer(which, curr): write the armed value of `which` to `curr`.
        nr::GETITIMER => {
            let which = args[0];
            if which >= ITIMER_COUNT {
                errno::EINVAL
            } else {
                match time_syscalls::getitimer(mem, args[1], ctx.itimers[which as usize]) {
                    Ok(()) => 0,
                    Err(e) => time_errno(e),
                }
            }
        }
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
        // sigaltstack(ss, old_ss): report/install the alternate signal stack,
        // threading the per-session `altstack` through the pure handler.
        nr::SIGALTSTACK => match sig_syscalls::sigaltstack(mem, args[0], args[1], ctx.altstack) {
            Ok(next) => {
                ctx.altstack = next;
                0
            }
            Err(e) => sig_errno(e),
        },
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
        // set_robust_list(head, len): the kernel records the per-thread robust
        // futex list head for cleanup at thread exit. glibc calls this at startup
        // even single-threaded; with no thread teardown the list is never walked,
        // so honouring it is a no-op that must still succeed. `len` must be the
        // size of a `robust_list_head` (24 bytes on x86-64) or it is EINVAL.
        nr::SET_ROBUST_LIST => {
            if args[1] == 24 {
                0
            } else {
                errno::EINVAL
            }
        }
        // membarrier(cmd, flags): MEMBARRIER_CMD_QUERY (0) reports the supported
        // command set as a bitmask. The single-thread model has no cross-thread
        // ordering to enforce, so no commands are advertised (mask 0), and any
        // specific command request is EINVAL (consistent with the empty set).
        nr::MEMBARRIER => {
            if args[0] == 0 {
                0
            } else {
                errno::EINVAL
            }
        }
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
        // sched_get_priority_max/min(policy): the real-time policies
        // (SCHED_FIFO=1, SCHED_RR=2) span 1..=99; the normal policies
        // (SCHED_OTHER=0, SCHED_BATCH=3, SCHED_IDLE=5) have a fixed priority 0.
        // An unknown policy is EINVAL.
        nr::SCHED_GET_PRIORITY_MAX => match arg_i32(args[0]) {
            0 | 3 | 5 => 0,
            1 | 2 => 99,
            _ => errno::EINVAL,
        },
        nr::SCHED_GET_PRIORITY_MIN => match arg_i32(args[0]) {
            0 | 3 | 5 => 0,
            1 | 2 => 1,
            _ => errno::EINVAL,
        },
        // getpriority(which, who): report the default nice value (0). The session
        // does not model per-process nice levels. `which` must be PRIO_PROCESS
        // (0) / PRIO_PGRP (1) / PRIO_USER (2). NB: the raw syscall returns the
        // nice value directly (glibc maps it to 20-nice).
        nr::GETPRIORITY => {
            if args[0] > 2 {
                errno::EINVAL
            } else {
                0
            }
        }
        // setpriority(which, who, prio): accepted (not stored — nice is not
        // modelled) for a valid `which`.
        nr::SETPRIORITY => {
            if args[0] > 2 {
                errno::EINVAL
            } else {
                0
            }
        }
        // sched_getscheduler(pid): the only policy the session models is
        // SCHED_OTHER (0).
        nr::SCHED_GETSCHEDULER => 0,
        // sched_setscheduler(pid, policy, param): only SCHED_OTHER is accepted;
        // any other policy is EINVAL (the param is not consulted for OTHER).
        nr::SCHED_SETSCHEDULER => {
            if arg_i32(args[1]) == 0 {
                0
            } else {
                errno::EINVAL
            }
        }
        // sched_getparam(pid, param): write the static priority (0 for the only
        // policy modelled, SCHED_OTHER); a bad pointer is EFAULT.
        nr::SCHED_GETPARAM => match sched_syscalls::sched_getparam(mem, args[1]) {
            Ok(()) => 0,
            Err(_) => errno::EFAULT,
        },
        // sched_setparam(pid, param): accept only priority 0 (SCHED_OTHER).
        nr::SCHED_SETPARAM => match sched_syscalls::sched_setparam(mem, args[1]) {
            Ok(()) => 0,
            Err(e) => sched_errno(&e),
        },
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
    fn fcntl_routes_dupfd_and_reports_ebadf_for_unopen() {
        const SYS_FCNTL: u64 = 72;
        const F_DUPFD: u64 = 0;
        const F_GETFD: u64 = 1;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // fcntl(1, F_DUPFD, 10): duplicate stdout onto the lowest fd >= 10.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FCNTL, [1, F_DUPFD, 10, 0, 0, 0]),
            10
        );
        assert!(ctx.fds.is_open(10));
        // fcntl on an unopen fd is -EBADF (-9), before any cmd dispatch.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_FCNTL, [9, F_GETFD, 0, 0, 0, 0]),
            -9
        );
    }

    #[test]
    fn uname_routes_and_writes_the_identity() {
        use prisma_runtime::guest_structs::Utsname;
        const SYS_UNAME: u64 = 63;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; Utsname::SIZE];
        let mut mem = region(&mut buf);
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_UNAME, [0x1000, 0, 0, 0, 0, 0]),
            0
        );
        // sysname field reads back as "Linux".
        assert_eq!(&mem.read(0x1000, 5).unwrap(), b"Linux");
        // A bad pointer faults: -EFAULT (-14).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_UNAME, [0x9000, 0, 0, 0, 0, 0]),
            -14
        );
    }

    #[test]
    fn getcwd_routes_writes_root_and_reports_erange() {
        const SYS_GETCWD: u64 = 79;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // getcwd(buf, 16) -> 2 ("/" + NUL); the bytes land in guest memory.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETCWD, [0x1000, 16, 0, 0, 0, 0]),
            2
        );
        assert_eq!(mem.read(0x1000, 2).unwrap(), b"/\0");
        // A 1-byte buffer cannot hold "/" + NUL -> -ERANGE (-34).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETCWD, [0x1000, 1, 0, 0, 0, 0]),
            -34
        );
    }

    #[test]
    fn getrusage_routes_and_writes_zeroed_counters() {
        use prisma_runtime::guest_structs::Rusage;
        const SYS_GETRUSAGE: u64 = 98;
        let mut ctx = SyscallContext::new();
        let mut buf = [0xffu8; Rusage::SIZE];
        let mut mem = region(&mut buf);
        // getrusage(RUSAGE_SELF=0, 0x1000) -> 0; the struct is zeroed.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETRUSAGE, [0, 0x1000, 0, 0, 0, 0]),
            0
        );
        assert!(mem
            .read(0x1000, Rusage::SIZE)
            .unwrap()
            .iter()
            .all(|&b| b == 0));
        // An unknown who -> -EINVAL (-22).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETRUSAGE, [5, 0x1000, 0, 0, 0, 0]),
            -22
        );
    }

    #[test]
    fn rlimit_syscalls_route_and_report_fixed_limits() {
        use prisma_runtime::guest_structs::Rlimit;
        const SYS_GETRLIMIT: u64 = 97;
        const SYS_PRLIMIT64: u64 = 302;
        const RLIMIT_NOFILE: u64 = 7;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 2 * Rlimit::SIZE];
        let mut mem = region(&mut buf);
        // getrlimit(RLIMIT_NOFILE, 0x1000) -> 0; soft 1024 lands at the front.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETRLIMIT,
                [RLIMIT_NOFILE, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        let r = Rlimit::from_guest_bytes(mem.read(0x1000, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((r.cur, r.max), (1024, 4096));
        // prlimit64(pid, RLIMIT_NOFILE, new=0, old=0x1010) -> 0; old written.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_PRLIMIT64,
                [0, RLIMIT_NOFILE, 0, 0x1010, 0, 0]
            ),
            0
        );
        let o = Rlimit::from_guest_bytes(mem.read(0x1010, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((o.cur, o.max), (1024, 4096));
        // An out-of-range resource -> -EINVAL (-22).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETRLIMIT, [16, 0x1000, 0, 0, 0, 0]),
            -22
        );
    }

    #[test]
    fn getcpu_routes_and_reports_zero_zero() {
        const SYS_GETCPU: u64 = 309;
        let mut ctx = SyscallContext::new();
        let mut buf = [0xffu8; 8];
        let mut mem = region(&mut buf);
        // getcpu(cpu=0x1000, node=0x1004, tcache=0) -> 0; both report 0.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETCPU, [0x1000, 0x1004, 0, 0, 0, 0]),
            0
        );
        assert_eq!(mem.read(0x1000, 4).unwrap(), &0u32.to_le_bytes());
        assert_eq!(mem.read(0x1004, 4).unwrap(), &0u32.to_le_bytes());
        // A bad pointer faults: -EFAULT (-14).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETCPU, [0x9000, 0, 0, 0, 0, 0]),
            -14
        );
    }

    #[test]
    fn sysinfo_routes_and_writes_synthetic_stats() {
        use prisma_runtime::guest_structs::Sysinfo;
        const SYS_SYSINFO: u64 = 99;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; Sysinfo::SIZE];
        let mut mem = region(&mut buf);
        // sysinfo(0x1000) -> 0; totalram (offset 32) is the synthetic 2 GiB.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_SYSINFO, [0x1000, 0, 0, 0, 0, 0]),
            0
        );
        assert_eq!(
            &mem.read(0x1000, Sysinfo::SIZE).unwrap()[32..40],
            &(2u64 * 1024 * 1024 * 1024).to_le_bytes()
        );
        // A bad pointer faults: -EFAULT (-14).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_SYSINFO, [0x9000, 0, 0, 0, 0, 0]),
            -14
        );
    }

    #[test]
    fn priority_and_scheduler_policy_syscalls_route() {
        const SYS_GETPRIORITY: u64 = 140;
        const SYS_SETPRIORITY: u64 = 141;
        const SYS_SCHED_SETSCHEDULER: u64 = 144;
        const SYS_SCHED_GETSCHEDULER: u64 = 145;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // getpriority(PRIO_PROCESS=0, 0) -> 0 (default nice).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETPRIORITY, [0, 0, 0, 0, 0, 0]),
            0
        );
        // An out-of-range `which` (3) -> -EINVAL (-22).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETPRIORITY, [3, 0, 0, 0, 0, 0]),
            -22
        );
        // setpriority(PRIO_USER=2, who, prio) -> 0 (accepted, not stored).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_SETPRIORITY, [2, 100, 5, 0, 0, 0]),
            0
        );
        // sched_getscheduler(pid) -> SCHED_OTHER (0).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETSCHEDULER,
                [0, 0, 0, 0, 0, 0]
            ),
            0
        );
        // sched_setscheduler(pid, SCHED_OTHER=0, param) -> 0.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETSCHEDULER,
                [0, 0, 0, 0, 0, 0]
            ),
            0
        );
        // sched_setscheduler with SCHED_FIFO=1 -> -EINVAL (-22).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETSCHEDULER,
                [0, 1, 0, 0, 0, 0]
            ),
            -22
        );
    }

    #[test]
    fn sched_param_syscalls_route() {
        use prisma_runtime::guest_structs::SchedParam;
        const SYS_SCHED_GETPARAM: u64 = 143;
        const SYS_SCHED_SETPARAM: u64 = 142;
        let mut ctx = SyscallContext::new();
        let mut buf = [0xffu8; SchedParam::SIZE];
        let mut mem = region(&mut buf);
        // sched_getparam(pid, 0x1000) -> 0; priority reads back as 0.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_GETPARAM,
                [0, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        let p = SchedParam::from_guest_bytes(mem.read(0x1000, SchedParam::SIZE).unwrap()).unwrap();
        assert_eq!(p.priority, 0);
        // sched_setparam with priority 0 -> 0.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETPARAM,
                [0, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        // sched_setparam with a non-zero priority -> -EINVAL (-22).
        mem.write(0x1000, &SchedParam { priority: 9 }.to_guest_bytes())
            .unwrap();
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SCHED_SETPARAM,
                [0, 0x1000, 0, 0, 0, 0]
            ),
            -22
        );
    }

    #[test]
    fn thread_init_compat_syscalls_route() {
        const SYS_SET_ROBUST_LIST: u64 = 273;
        const SYS_MEMBARRIER: u64 = 324;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // set_robust_list(head, len=24) -> 0 (accepted no-op).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SET_ROBUST_LIST,
                [0x1000, 24, 0, 0, 0, 0]
            ),
            0
        );
        // A wrong len -> -EINVAL (-22).
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SET_ROBUST_LIST,
                [0x1000, 16, 0, 0, 0, 0]
            ),
            -22
        );
        // membarrier(MEMBARRIER_CMD_QUERY=0, 0) -> 0 (empty supported set).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_MEMBARRIER, [0, 0, 0, 0, 0, 0]),
            0
        );
        // membarrier of an actual command -> -EINVAL (not advertised).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_MEMBARRIER, [1, 0, 0, 0, 0, 0]),
            -22
        );
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
    fn sigaltstack_routes_and_persists_the_alt_stack() {
        use prisma_runtime::guest_structs::SigAltStack;
        const SYS_SIGALTSTACK: u64 = 131;
        let mut ctx = SyscallContext::new();
        // A new alt-stack at guest 0x1000.
        let new = SigAltStack {
            sp: 0x1_4000_6000,
            flags: 0,
            size: 0x4000,
        };
        let mut buf = [0u8; 64];
        buf[0..24].copy_from_slice(&new.to_guest_bytes());
        let mut mem = region(&mut buf);
        // sigaltstack(ss=0x1000, old_ss=0x1020): install new, report old.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SIGALTSTACK,
                [0x1000, 0x1020, 0, 0, 0, 0]
            ),
            0
        );
        // old_ss holds the previous (disabled, flags=2) alt-stack.
        let old = SigAltStack::from_guest_bytes(&buf[0x20..0x20 + 24]).unwrap();
        assert_eq!(old.flags, 2);
        // The new alt-stack persisted in the context.
        assert_eq!(ctx.altstack, new);
        // A query-only call (ss=0) reports the now-installed stack.
        let mut buf2 = [0u8; 32];
        let mut mem2 = region(&mut buf2);
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem2,
                SYS_SIGALTSTACK,
                [0, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        assert_eq!(SigAltStack::from_guest_bytes(&buf2[0..24]).unwrap(), new);
    }

    #[test]
    fn poll_routes_and_reports_ready_fds() {
        use prisma_runtime::guest_structs::PollFd;
        const SYS_POLL: u64 = 7;
        const POLLIN: i16 = 0x1;
        const POLLOUT: i16 = 0x4;
        let mut ctx = SyscallContext::new();
        // Two pollfds at guest 0x1000: stdin(POLLIN), stdout(POLLOUT).
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(
            &PollFd {
                fd: 0,
                events: POLLIN,
                revents: 0,
            }
            .to_guest_bytes(),
        );
        buf[8..16].copy_from_slice(
            &PollFd {
                fd: 1,
                events: POLLOUT,
                revents: 0,
            }
            .to_guest_bytes(),
        );
        let mut mem = region(&mut buf);
        // poll(0x1000, 2, timeout=0) -> 2 ready; timeout ignored.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_POLL, [0x1000, 2, 0, 0, 0, 0]),
            2
        );
        let p0 = PollFd::from_guest_bytes(mem.read(0x1000, 8).unwrap()).unwrap();
        assert_eq!(p0.revents, POLLIN);
        // nfds == 0 reports nothing ready.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_POLL, [0x1000, 0, 0, 0, 0, 0]),
            0
        );
    }

    #[test]
    fn ppoll_routes_and_honours_the_timeout_pointer() {
        use prisma_runtime::guest_structs::{PollFd, Timespec};
        const SYS_PPOLL: u64 = 271;
        const POLLOUT: i16 = 0x4;
        let mut ctx = SyscallContext::new();
        // A pollfd for stdout at 0x1000; a {0,0} timeout at 0x1010.
        let mut buf = [0u8; 64];
        buf[0..8].copy_from_slice(
            &PollFd {
                fd: 1,
                events: POLLOUT,
                revents: 0,
            }
            .to_guest_bytes(),
        );
        buf[16..32].copy_from_slice(&Timespec { sec: 0, nsec: 0 }.to_guest_bytes());
        let mut mem = region(&mut buf);
        // ppoll(0x1000, 1, tmo=0x1010, sigmask=0, sigsetsize=0) -> 1 ready.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PPOLL, [0x1000, 1, 0x1010, 0, 0, 0]),
            1
        );
        // A bad timeout pointer is -EFAULT (the timespec is range-checked).
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_PPOLL, [0x1000, 1, 0x103A, 0, 0, 0]),
            -14
        );
    }

    #[test]
    fn sched_priority_bounds_match_the_policy() {
        const SYS_MAX: u64 = 146;
        const SYS_MIN: u64 = 147;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // SCHED_OTHER (0): a normal process has a fixed priority 0.
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_MAX, [0, 0, 0, 0, 0, 0]), 0);
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_MIN, [0, 0, 0, 0, 0, 0]), 0);
        // SCHED_FIFO (1) / SCHED_RR (2): real-time priorities span 1..=99.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_MAX, [1, 0, 0, 0, 0, 0]),
            99
        );
        assert_eq!(dispatch(&mut ctx, &mut mem, SYS_MIN, [2, 0, 0, 0, 0, 0]), 1);
        // An unknown policy is -EINVAL.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_MAX, [42, 0, 0, 0, 0, 0]),
            -22
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

    #[test]
    fn times_routes_and_writes_a_zeroed_tms() {
        use prisma_runtime::guest_structs::Tms;
        const SYS_TIMES: u64 = 100;
        let mut ctx = SyscallContext::new();
        let mut buf = [0xffu8; Tms::SIZE];
        let mut mem = region(&mut buf);
        // times(buf) -> non-negative tick count; the CPU fields are reported zero.
        let ticks = dispatch(&mut ctx, &mut mem, SYS_TIMES, [0x1000, 0, 0, 0, 0, 0]);
        assert!(ticks >= 0);
        let tms = Tms::from_guest_bytes(mem.read(0x1000, Tms::SIZE).unwrap()).unwrap();
        assert_eq!((tms.utime, tms.stime, tms.cutime, tms.cstime), (0, 0, 0, 0));
        // times(0): a null buffer is allowed (just returns the tick count).
        assert!(dispatch(&mut ctx, &mut mem, SYS_TIMES, [0, 0, 0, 0, 0, 0]) >= 0);
    }

    #[test]
    fn setitimer_arms_a_timer_that_getitimer_reads_back() {
        use prisma_runtime::guest_structs::{ITimerval, Timeval};
        const SYS_SETITIMER: u64 = 38;
        const SYS_GETITIMER: u64 = 36;
        const ITIMER_REAL: u64 = 0;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 2 * ITimerval::SIZE];
        let mut mem = region(&mut buf);
        let armed = ITimerval {
            interval: Timeval { sec: 1, usec: 0 },
            value: Timeval {
                sec: 2,
                usec: 500_000,
            },
        };
        mem.write(0x1000, &armed.to_guest_bytes()).unwrap();
        // setitimer(ITIMER_REAL, new=0x1000, old=0x1020): old (disarmed) is written
        // back, the timer is armed from `new`.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_SETITIMER,
                [ITIMER_REAL, 0x1000, 0x1020, 0, 0, 0]
            ),
            0
        );
        let old = ITimerval::from_guest_bytes(mem.read(0x1020, ITimerval::SIZE).unwrap()).unwrap();
        assert_eq!(old.value, Timeval { sec: 0, usec: 0 }); // was disarmed
                                                            // getitimer(ITIMER_REAL, curr=0x1020) reads back exactly what we armed.
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_GETITIMER,
                [ITIMER_REAL, 0x1020, 0, 0, 0, 0]
            ),
            0
        );
        let cur = ITimerval::from_guest_bytes(mem.read(0x1020, ITimerval::SIZE).unwrap()).unwrap();
        assert_eq!(cur, armed);
    }

    #[test]
    fn itimer_syscalls_reject_an_out_of_range_which() {
        const SYS_SETITIMER: u64 = 38;
        const SYS_GETITIMER: u64 = 36;
        let mut ctx = SyscallContext::new();
        let mut buf = [0u8; 32];
        let mut mem = region(&mut buf);
        // which = 3 is past ITIMER_PROF -> -EINVAL (-22), no memory touched.
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_SETITIMER, [3, 0x1000, 0, 0, 0, 0]),
            -22
        );
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_GETITIMER, [3, 0x1000, 0, 0, 0, 0]),
            -22
        );
    }
}
