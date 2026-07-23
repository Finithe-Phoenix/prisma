// core/src/runtime/syscall_handler.cpp — F2-SY-001/002.
//
// x86_64 Linux syscall dispatch via host POSIX C library. The handler runs
// in C++ land, called from JIT-generated ARM64 code via `blr`. It reads
// guest register state from CpuStateFrame, performs the requested operation,
// and writes results back to the frame.
//
// x86_64 syscall ABI:  RAX = number, RDI/RSI/RDX/R10/R8/R9 = args[0..5].
// Return: RAX = result (negative errno on error, no CF flag set yet).
//
// NOTE: This file is POSIX-only (target = Linux ARM64). MSVC builds get a
// stub that returns -ENOSYS for everything; the real handler only compiles
// on GCC/Clang with POSIX headers available.

#include "prisma/syscall_handler.hpp"

#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <array>
#include <thread>
#include <atomic>
#include <condition_variable>
#include <unordered_map>
#include <vector>

#include "prisma/dispatcher.hpp"
#include <cstdio>
#include <cstdlib>

#include "prisma/cpu_state.hpp"
#include "prisma/ir.hpp"

#ifndef _MSC_VER

#include <cstring>
#include <dirent.h>
#include <pthread.h>

#include <fcntl.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <sys/epoll.h>
#include <sys/mman.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/resource.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/uio.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <unistd.h>

namespace prisma::runtime {

namespace {

// -- Threading Support --------------------------------------------------------

constexpr std::uint64_t kCloneVm = 0x00000100;
constexpr std::uint64_t kCloneFs = 0x00000200;
constexpr std::uint64_t kCloneFiles = 0x00000400;
constexpr std::uint64_t kCloneSighand = 0x00000800;
constexpr std::uint64_t kCloneParentSettid = 0x00100000;
constexpr std::uint64_t kCloneChildCleartid = 0x00200000;
constexpr std::uint64_t kCloneThread = 0x00010000;
constexpr std::uint64_t kCloneSettls = 0x00080000;
constexpr std::uint64_t kCloneChildSettid = 0x01000000;

constexpr int kFutexWait = 0;
constexpr int kFutexWake = 1;
constexpr int kFutexPrivateFlag = 128;

thread_local std::uint64_t tls_clear_child_tid = 0;
thread_local std::atomic<bool> tls_thread_exit_flag{false};
thread_local std::uint64_t tls_robust_list_head = 0;
thread_local std::size_t tls_robust_list_len = 0;

void handle_thread_exit() {
    if (tls_clear_child_tid != 0) {
        auto* addr = reinterpret_cast<std::atomic_ref<std::uint32_t>::value_type*>(tls_clear_child_tid);
        std::atomic_ref<std::uint32_t> atom(*addr);
        atom.store(0, std::memory_order_seq_cst);
        atom.notify_all();
    }
}

// Global process-wide exit flag for exit_group
std::atomic<bool> g_process_exit{false};

} // namespace

// x86_64 Linux syscall numbers used in this file.
enum X64Sysno : std::uint64_t {
    kX64Read        = 0,
    kX64Write       = 1,
    kX64Open        = 2,
    kX64Close       = 3,
    kX64Stat        = 4,
    kX64Fstat       = 5,
    kX64Lstat       = 6,
    kX64Poll        = 7,
    kX64Mmap        = 9,
    kX64Mprotect    = 10,
    kX64Munmap      = 11,
    kX64Brk         = 12,
    kX64RtSigaction = 13,
    kX64RtSigprocmask = 14,
    kX64Select      = 23,
    kX64Ioctl       = 16,
    kX64Readv       = 19,
    kX64Writev      = 20,
    kX64Dup         = 32,
    kX64Dup2        = 33,
    kX64Nanosleep   = 35,
    kX64Getcwd      = 79,
    kX64Fcntl       = 72,
    kX64Chdir       = 80,
    kX64Fchdir       = 81,
    kX64Rename       = 82,
    kX64Mkdir        = 83,
    kX64Rmdir        = 84,
    kX64Unlink       = 87,
    kX64Readlink     = 89,
    kX64Lseek        = 8,
    kX64Getpid       = 39,
    kX64Getuid       = 102,
    kX64Geteuid      = 107,
    kX64Gettid       = 186,
    kX64Getppid      = 110,
    kX64Getgid       = 104,
    kX64Getegid      = 108,
    kX64SetRobustList = 273,
    kX64GetRobustList = 274,
    kX64Exit         = 60,
    kX64Execve       = 59,
    kX64ExitGroup    = 231,
    kX64Wait4        = 61,
    kX64Uname        = 63,
    kX64Getdents     = 78,
    kX64Getdents64   = 217,
    kX64SetTidAddress = 218,
    kX64ClockGettime = 228,
    kX64Gettimeofday = 96,
    kX64Time         = 201,
    kX64RtSigreturn  = 15,
    kX64RtSigsuspend = 130,
    kX64ArchPrctl    = 158,
    kX64Prctl        = 157,
    kX64Prlimit64    = 302,
    kX64EpollCreate1 = 291,
    kX64EpollCtl     = 233,
    kX64EpollWait    = 232,
    kX64Socket       = 41,
    kX64Connect      = 42,
    kX64Accept       = 43,
    kX64Sendto       = 44,
    kX64Recvfrom     = 45,
    kX64Sendmsg      = 46,
    kX64Recvmsg      = 47,
    kX64Shutdown     = 48,
    kX64Bind         = 49,
    kX64Listen       = 50,
    kX64Getsockname  = 51,
    kX64Getpeername  = 52,
    kX64Socketpair   = 53,
    kX64Setsockopt   = 54,
    kX64Getsockopt   = 55,
    kX64Clone        = 56,
    kX64Futex        = 202,
    kX64Accept4      = 288,
};

}  // namespace prisma::runtime

namespace {
    std::mutex g_sigaction_mutex;
    std::array<prisma::runtime::GuestSigaction, 65> g_guest_sigactions{};
}

namespace prisma::runtime {
    GuestSigaction get_guest_sigaction(int sig) {
        if (sig < 1 || sig >= 65) return {};
        std::lock_guard<std::mutex> lk(g_sigaction_mutex);
        return g_guest_sigactions[static_cast<std::size_t>(sig)];
    }
}

// F2-SY-038: strace-like syscall logger. Check PRISMA_STRACE env var once.
static bool strace_enabled() noexcept {
    static const bool enabled = []() noexcept {
        const char* v = std::getenv("PRISMA_STRACE");
        return v != nullptr && v[0] != '\0';
    }();
    return enabled;
}

static const char* syscall_name(std::uint64_t n) noexcept {
    using namespace prisma::runtime;
    switch (n) {
        case kX64Read:        return "read";
        case kX64Write:       return "write";
        case kX64Open:        return "open";
        case kX64Close:       return "close";
        case kX64Stat:        return "stat";
        case kX64Fstat:       return "fstat";
        case kX64Lstat:       return "lstat";
        case kX64Poll:        return "poll";
        case kX64Lseek:       return "lseek";
        case kX64Mmap:        return "mmap";
        case kX64Mprotect:    return "mprotect";
        case kX64Munmap:      return "munmap";
        case kX64Brk:         return "brk";
        case kX64RtSigaction: return "rt_sigaction";
        case kX64RtSigprocmask: return "rt_sigprocmask";
        case kX64Select:      return "select";
        case kX64Ioctl:       return "ioctl";
        case kX64Readv:       return "readv";
        case kX64Writev:      return "writev";
        case kX64Dup:         return "dup";
        case kX64Dup2:        return "dup2";
        case kX64Nanosleep:   return "nanosleep";
        case kX64Fcntl:       return "fcntl";
        case kX64Getcwd:      return "getcwd";
        case kX64Chdir:       return "chdir";
        case kX64Fchdir:      return "fchdir";
        case kX64Rename:      return "rename";
        case kX64Mkdir:       return "mkdir";
        case kX64Rmdir:       return "rmdir";
        case kX64Unlink:      return "unlink";
        case kX64Readlink:    return "readlink";
        case kX64Getpid:      return "getpid";
        case kX64Getuid:      return "getuid";
        case kX64Geteuid:     return "geteuid";
        case kX64Gettid:      return "gettid";
        case kX64Getppid:     return "getppid";
        case kX64Getgid:      return "getgid";
        case kX64Getegid:     return "getegid";
        case kX64SetRobustList: return "set_robust_list";
        case kX64GetRobustList: return "get_robust_list";
        case kX64Exit:        return "exit";
        case kX64Execve:      return "execve";
        case kX64ExitGroup:   return "exit_group";
        case kX64Wait4:       return "wait4";
        case kX64Uname:       return "uname";
        case kX64Futex:       return "futex";
        case kX64Getdents:    return "getdents";
        case kX64Getdents64:  return "getdents64";
        case kX64SetTidAddress: return "set_tid_address";
        case kX64ClockGettime: return "clock_gettime";
        case kX64Gettimeofday: return "gettimeofday";
        case kX64Time:        return "time";
        case kX64ArchPrctl:   return "arch_prctl";
        case kX64Prctl:       return "prctl";
        case kX64Prlimit64:   return "prlimit64";
        case kX64RtSigreturn: return "rt_sigreturn";
        case kX64RtSigsuspend: return "rt_sigsuspend";
        case kX64EpollCreate1: return "epoll_create1";
        case kX64EpollCtl:    return "epoll_ctl";
        case kX64EpollWait:   return "epoll_wait";
        case kX64Socket:      return "socket";
        case kX64Connect:     return "connect";
        case kX64Accept:      return "accept";
        case kX64Sendto:      return "sendto";
        case kX64Recvfrom:    return "recvfrom";
        case kX64Sendmsg:     return "sendmsg";
        case kX64Recvmsg:     return "recvmsg";
        case kX64Shutdown:    return "shutdown";
        case kX64Bind:        return "bind";
        case kX64Listen:      return "listen";
        case kX64Getsockname: return "getsockname";
        case kX64Getpeername: return "getpeername";
        case kX64Socketpair:  return "socketpair";
        case kX64Setsockopt:  return "setsockopt";
        case kX64Getsockopt:  return "getsockopt";
        case kX64Clone:       return "clone";
        case kX64Accept4:     return "accept4";
        default:              return "???";
    }
}

extern "C" void prisma_syscall_handler(prisma::runtime::CpuStateFrame* state) {
    using namespace prisma::runtime;
    using prisma::ir::Gpr;

    const std::uint64_t sysno = state->gpr[static_cast<std::size_t>(Gpr::Rax)];
    const std::uint64_t a1 = state->gpr[static_cast<std::size_t>(Gpr::Rdi)];
    const std::uint64_t a2 = state->gpr[static_cast<std::size_t>(Gpr::Rsi)];
    const std::uint64_t a3 = state->gpr[static_cast<std::size_t>(Gpr::Rdx)];
    const std::uint64_t a4 = state->gpr[static_cast<std::size_t>(Gpr::R10)];
    const std::uint64_t a5 = state->gpr[static_cast<std::size_t>(Gpr::R8)];
    const std::uint64_t a6 = state->gpr[static_cast<std::size_t>(Gpr::R9)];

    std::int64_t result = 0;

    if (strace_enabled()) {
        std::fprintf(stderr, "[prisma-strace] %s(%llu, %llu, %llu, %llu, %llu, %llu) = ",
                     syscall_name(sysno),
                     static_cast<unsigned long long>(a1),
                     static_cast<unsigned long long>(a2),
                     static_cast<unsigned long long>(a3),
                     static_cast<unsigned long long>(a4),
                     static_cast<unsigned long long>(a5),
                     static_cast<unsigned long long>(a6));
    }

    switch (sysno) {
        // -- F2-SY-002: minimal stdio ------------------------------------------
        case kX64Read: {
            const int fd = static_cast<int>(a1);
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            std::size_t count = static_cast<std::size_t>(a3);
            result = static_cast<std::int64_t>(::read(fd, buf, count));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Write: {
            const int fd = static_cast<int>(a1);
            const void* buf =
                reinterpret_cast<const void*>(static_cast<std::uintptr_t>(a2));
            std::size_t count = static_cast<std::size_t>(a3);
            result = static_cast<std::int64_t>(::write(fd, buf, count));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Open: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            int flags = static_cast<int>(a2);
            int mode = static_cast<int>(a3);
            result = static_cast<std::int64_t>(::open(path, flags, mode));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Close: {
            const int fd = static_cast<int>(a1);
            result = ::close(fd);
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-003: stat family --------------------------------------------
        case kX64Stat: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            result = ::stat(path, static_cast<struct ::stat*>(buf));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Fstat: {
            const int fd = static_cast<int>(a1);
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            result = ::fstat(fd, static_cast<struct ::stat*>(buf));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Lstat: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            result = ::lstat(path, static_cast<struct ::stat*>(buf));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-005: brk ----------------------------------------------------
        case kX64Brk: {
            void* addr = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1));
            // If addr is 0 (or below current break), return current break.
            if (addr == nullptr || addr < ::sbrk(0)) {
                result = reinterpret_cast<std::int64_t>(::sbrk(0));
            } else {
                result = reinterpret_cast<std::int64_t>(::sbrk(
                    reinterpret_cast<std::intptr_t>(addr) -
                    reinterpret_cast<std::intptr_t>(::sbrk(0))));
                if (reinterpret_cast<void*>(static_cast<std::uintptr_t>(result))
                    == reinterpret_cast<void*>(-1)) {
                    result = -errno;
                }
            }
            break;
        }

        // -- F2-SY-010: exit ---------------------------------------------------
        case kX64Exit: {
            handle_thread_exit();
            tls_thread_exit_flag.store(true, std::memory_order_relaxed);
            // Instead of ::exit, we just tell this thread to stop.
            if (!tls_current_dispatcher) ::exit(static_cast<int>(a1));
            // Let the dispatcher naturally return out of its loop.
            // We set the halt sentinel so that it stops immediately.
            state->guest_pc = CpuStateFrame::kHaltSentinel;
            return;
        }
        case kX64ExitGroup: {
            g_process_exit.store(true, std::memory_order_relaxed);
            ::exit(static_cast<int>(a1));
            break;
        }

        // -- F2-SY-011: execve ------------------------------------------------
        case kX64Execve: {
            const char* path = reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            int fd = ::open(path, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) {
                unsigned char header[20];
                ssize_t bytes_read = ::read(fd, header, sizeof(header));
                ::close(fd);
                if (bytes_read >= 20 && header[0] == 0x7F && header[1] == 'E' && header[2] == 'L' && header[3] == 'F') {
                    const std::uint16_t e_machine = static_cast<std::uint16_t>(
                        static_cast<unsigned int>(header[18])
                        | (static_cast<unsigned int>(header[19]) << 8U));
                    if (e_machine == 62) {
                        std::fprintf(stderr, "execve: cross-ISA re-entry not yet implemented\n");
                        result = -ENOSYS;
                    } else {
                        result = -ENOEXEC;
                    }
                } else {
                    result = -ENOEXEC;
                }
            } else {
                result = -errno;
            }
            break;
        }

        // -- F2-SY-026: wait4 --------------------------------------------------
        case kX64Wait4: {
            // wait4(pid, wstatus, options, rusage) — delegate to host wait4.
            // a5 (rusage) is passed as nullptr when guest doesn't need it.
            result = static_cast<std::int64_t>(::wait4(
                static_cast<int>(a1),
                reinterpret_cast<int*>(static_cast<std::uintptr_t>(a2)),
                static_cast<int>(a3),
                reinterpret_cast<struct ::rusage*>(
                    static_cast<std::uintptr_t>(a4))));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-009: getpid / getuid / gettid ------------------------------
        case kX64Getpid:
            result = static_cast<std::int64_t>(::getpid());
            break;
        case kX64Getuid:
            result = static_cast<std::int64_t>(::getuid());
            break;
        case kX64Geteuid:
            result = static_cast<std::int64_t>(::geteuid());
            break;
        case kX64Gettid: {
            // Use pthread_self() as a lightweight tid proxy. On Linux
            // this matches the kernel tid for the main thread; on macOS
            // it returns a pthread_t that is unique per-thread.
            result = static_cast<std::int64_t>(
                reinterpret_cast<std::uintptr_t>(::pthread_self()));
            break;
        }

        // -- F2-SY-008: time ---------------------------------------------------
        case kX64Gettimeofday: {
            void* tv = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1));
            void* tz = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            result = ::gettimeofday(static_cast<struct ::timeval*>(tv),
                                    static_cast<struct ::timezone*>(tz));
            if (result < 0) result = -errno;
            break;
        }
        case kX64ClockGettime: {
            clockid_t clk = static_cast<clockid_t>(a1);
            void* tp = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            result = ::clock_gettime(clk, static_cast<struct ::timespec*>(tp));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-012: dup / dup2 --------------------------------------------
        case kX64Dup: {
            result = ::dup(static_cast<int>(a1));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Dup2: {
            result = ::dup2(static_cast<int>(a1), static_cast<int>(a2));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-004: mmap / munmap / mprotect (simple stubs) ---------------
        case kX64Mmap:
            // Simplified mmap: a1=addr, a2=length, a3=prot, a4=flags,
            // a5=fd, a6=offset. Use ::mmap directly.
            result = reinterpret_cast<std::int64_t>(::mmap(
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1)),
                static_cast<std::size_t>(a2),
                static_cast<int>(a3),
                static_cast<int>(a4),
                static_cast<int>(a5),
                static_cast<::off_t>(a6)));
            if (result == reinterpret_cast<std::int64_t>(MAP_FAILED)) {
                result = -errno;
            }
            break;
        case kX64Munmap:
            result = ::munmap(
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1)),
                static_cast<std::size_t>(a2));
            if (result < 0) result = -errno;
            break;
        case kX64Mprotect:
            result = ::mprotect(
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1)),
                static_cast<std::size_t>(a2),
                static_cast<int>(a3));
            if (result < 0) result = -errno;
            break;

        // -- F2-SY-017: uname --------------------------------------------------
        case kX64Uname: {
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a1));
            result = ::uname(static_cast<struct ::utsname*>(buf));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-019: getcwd / chdir / fchdir ------------------------------
        case kX64Getcwd: {
            char* buf = reinterpret_cast<char*>(static_cast<std::uintptr_t>(a1));
            result = static_cast<std::int64_t>(
                reinterpret_cast<std::uintptr_t>(::getcwd(buf, a2)));
            if (buf == nullptr) result = -errno;
            break;
        }
        case kX64Chdir: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            result = ::chdir(path);
            if (result < 0) result = -errno;
            break;
        }

        case kX64Fchdir: {
            const int fd = static_cast<int>(a1);
            result = ::fchdir(fd);
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-020: unlink / rename / mkdir / rmdir -----------------------
        case kX64Unlink: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            result = ::unlink(path);
            if (result < 0) result = -errno;
            break;
        }
        case kX64Rename: {
            const char* oldpath =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            const char* newpath =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a2));
            result = ::rename(oldpath, newpath);
            if (result < 0) result = -errno;
            break;
        }
        case kX64Mkdir: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            result = ::mkdir(path, static_cast<::mode_t>(a2));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Rmdir: {
            const char* path =
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1));
            result = ::rmdir(path);
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-021: lseek --------------------------------------------------
        case kX64Lseek: {
            result = ::lseek(static_cast<int>(a1), static_cast<::off_t>(a2),
                             static_cast<int>(a3));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-008: nanosleep ---------------------------------------------
        case kX64Nanosleep: {
            const struct ::timespec* req =
                reinterpret_cast<const struct ::timespec*>(
                    static_cast<std::uintptr_t>(a1));
            struct ::timespec* rem =
                reinterpret_cast<struct ::timespec*>(
                    static_cast<std::uintptr_t>(a2));
            result = ::nanosleep(req, rem);
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-023: poll ----------------------------------------------------
        case kX64Poll: {
            // poll(fds, nfds, timeout) — POSIX, works on any host.
            result = static_cast<std::int64_t>(::poll(
                reinterpret_cast<struct ::pollfd*>(
                    static_cast<std::uintptr_t>(a1)),
                static_cast<unsigned int>(a2),
                static_cast<int>(a3)));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-023: select --------------------------------------------------
        case kX64Select: {
            // select(nfds, readfds, writefds, exceptfds, timeout) — POSIX.
            result = static_cast<std::int64_t>(::select(
                static_cast<int>(a1),
                reinterpret_cast<::fd_set*>(
                    static_cast<std::uintptr_t>(a2)),
                reinterpret_cast<::fd_set*>(
                    static_cast<std::uintptr_t>(a3)),
                reinterpret_cast<::fd_set*>(
                    static_cast<std::uintptr_t>(a4)),
                reinterpret_cast<struct ::timeval*>(
                    static_cast<std::uintptr_t>(a5))));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-013: fcntl -------------------------------------------------
        case kX64Fcntl: {
            result = ::fcntl(static_cast<int>(a1), static_cast<int>(a2), a3);
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-022: readv / writev ---------------------------------------
        case kX64Readv: {
            result = static_cast<std::int64_t>(::readv(
                static_cast<int>(a1),
                reinterpret_cast<const struct ::iovec*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<int>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        // -- F2-SY-006: clone --------------------------------------------------
        case kX64Clone: {
            const std::uint64_t flags = a1;
            const std::uint64_t child_stack = a2;
            const std::uint64_t parent_tidptr = a3;
            const std::uint64_t child_tidptr = a4;
            const std::uint64_t tls = a5;

            // In Prisma, we only support threads sharing memory (kCloneVm).
            // A fork() emulation without kCloneVm would require process forking.
            if ((flags & kCloneVm) == 0) {
                result = -ENOSYS;
                break;
            }

            Dispatcher* parent_dispatcher = tls_current_dispatcher;
            if (!parent_dispatcher) {
                result = -ENOSYS;
                break;
            }

            // Capture the parent state before making modifications for the child.
            CpuStateFrame child_state = *state;
            
            // The child returns 0.
            child_state.gpr[static_cast<std::size_t>(Gpr::Rax)] = 0;
            
            // Set the child stack if provided.
            if (child_stack != 0) {
                child_state.gpr[static_cast<std::size_t>(Gpr::Rsp)] = child_stack;
            }

            // Set TLS if requested.
            if ((flags & kCloneSettls) != 0) {
                child_state.fs_base = tls;
            }

            // We must simulate the host TID. We use the child's host gettid().
            // But we don't have the child TID until it starts.
            // We use a promise/future to retrieve the child's TID before returning to the parent guest.
            // (Alternatively, we can use a condvar to wait for the child to publish its TID).
            
            struct ChildStartup {
                std::mutex mu;
                std::condition_variable cv;
                pid_t tid = 0;
            };
            auto startup = std::make_shared<ChildStartup>();

            std::thread child_thread([parent_dispatcher, child_state, flags, child_tidptr, startup]() {
                // Initialize thread-local dispatcher for the child
                Dispatcher child_dispatcher(parent_dispatcher->translator(),
                                            parent_dispatcher->reader());
                child_dispatcher.set_state(child_state);
                child_dispatcher.set_exit_flag(&tls_thread_exit_flag);

                // Publish the TID to the parent
                pid_t host_tid = ::gettid();
                {
                    std::lock_guard<std::mutex> lock(startup->mu);
                    startup->tid = host_tid;
                }
                startup->cv.notify_one();

                // Handle child tid pointers
                if ((flags & kCloneChildSettid) != 0 && child_tidptr != 0) {
                    *reinterpret_cast<std::uint32_t*>(child_tidptr) =
                        static_cast<std::uint32_t>(host_tid);
                }
                if ((flags & kCloneChildCleartid) != 0) {
                    tls_clear_child_tid = child_tidptr;
                } else {
                    tls_clear_child_tid = 0;
                }

                // Run the child
                while (!g_process_exit.load(std::memory_order_relaxed)) {
                    // child_state.guest_pc is pointing to the next instruction after syscall
                    auto res = child_dispatcher.run(child_dispatcher.state().guest_pc, 10000);
                    if (res.exit == DispatchExit::Halted || 
                        res.exit == DispatchExit::FetchFailed || 
                        res.exit == DispatchExit::TranslationFailed) {
                        break;
                    }
                }

                handle_thread_exit();
            });

            child_thread.detach();

            // Wait for child to initialize and give us its TID
            std::unique_lock<std::mutex> lock(startup->mu);
            startup->cv.wait(lock, [&]() { return startup->tid != 0; });
            
            pid_t child_tid = startup->tid;

            if ((flags & kCloneParentSettid) != 0 && parent_tidptr != 0) {
                *reinterpret_cast<std::uint32_t*>(parent_tidptr) =
                    static_cast<std::uint32_t>(child_tid);
            }

            result = static_cast<std::int64_t>(child_tid);
            break;
        }

        // -- F2-SY-007: futex --------------------------------------------------
        case kX64Futex: {
            const std::uint64_t uaddr = a1;
            const int futex_op = static_cast<int>(a2);
            const std::uint32_t val = static_cast<std::uint32_t>(a3);
            // const std::uint64_t timeout = a4; // Optional timeout struct ptr

            const int cmd = futex_op & ~kFutexPrivateFlag;
            auto* addr = reinterpret_cast<std::atomic_ref<std::uint32_t>::value_type*>(uaddr);
            std::atomic_ref<std::uint32_t> atom(*addr);

            if (cmd == kFutexWait) {
                // Wait blockingly.
                // In C++20, atomic_wait returns void, but if the value doesn't match 'val' it returns immediately.
                if (atom.load(std::memory_order_seq_cst) != val) {
                    result = -EAGAIN;
                } else {
                    // Note: proper timeout handling requires waiting with timeout,
                    // but C++20 atomic_wait doesn't support timeout!
                    // For Prisma Fase 2 MVP, we'll do an infinite wait.
                    // If timeout was passed, we'd theoretically need a custom condvar fallback or host syscall.
                    atom.wait(val, std::memory_order_seq_cst);
                    result = 0;
                }
            } else if (cmd == kFutexWake) {
                // Wake up to 'val' threads.
                // In C++20, we can only notify_one() or notify_all().
                if (val == 1) {
                    atom.notify_one();
                } else {
                    atom.notify_all();
                }
                result = val; // return number of awakened (heuristic)
            } else {
                result = -ENOSYS;
            }
            break;
        }

        // -- F2-SY-032: robust_futex -------------------------------------------
        case kX64SetRobustList: {
            tls_robust_list_head = a1;
            tls_robust_list_len = static_cast<std::size_t>(a2);
            result = 0;
            break;
        }
        case kX64GetRobustList: {
            const int pid = static_cast<int>(a1);
            if (pid != 0) {
                result = -EPERM;
            } else {
                if (a2 != 0) {
                    *reinterpret_cast<std::uint64_t*>(static_cast<std::uintptr_t>(a2)) = tls_robust_list_head;
                }
                if (a3 != 0) {
                    *reinterpret_cast<std::uint64_t*>(static_cast<std::uintptr_t>(a3)) = static_cast<std::uint64_t>(tls_robust_list_len);
                }
                result = 0;
            }
            break;
        }

        // -- F2-SY-020: readlink ----------------------------------------------
        case kX64Readlink: {
            result = static_cast<std::int64_t>(::readlink(
                reinterpret_cast<const char*>(static_cast<std::uintptr_t>(a1)),
                reinterpret_cast<char*>(static_cast<std::uintptr_t>(a2)),
                static_cast<std::size_t>(a3)));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-009: getppid / getgid / getegid ----------------------------
        case kX64Getppid:
            result = static_cast<std::int64_t>(::getppid());
            break;
        case kX64Getgid:
            result = static_cast<std::int64_t>(::getgid());
            break;
        case kX64Getegid:
            result = static_cast<std::int64_t>(::getegid());
            break;

        // -- F2-SY-008: time --------------------------------------------------
        case kX64Time: {
            result = static_cast<std::int64_t>(::time(
                reinterpret_cast<::time_t*>(static_cast<std::uintptr_t>(a1))));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-014 / F2-SY-036: ioctl (passthrough) ------------------------
        case kX64Ioctl: {
            const int fd = static_cast<int>(a1);
            const unsigned long cmd = static_cast<unsigned long>(a2);
            
            if (cmd == 0x5401) { // TCGETS
                if (!::isatty(fd)) {
                    result = -ENOTTY;
                } else {
                    result = ::ioctl(fd, cmd, a3);
                    if (result < 0) result = -errno;
                }
            } else if (cmd == 0x5413) { // TIOCGWINSZ
                struct GuestWinsize {
                    unsigned short ws_row;
                    unsigned short ws_col;
                    unsigned short ws_xpixel;
                    unsigned short ws_ypixel;
                };
                if (a3 != 0) {
                    auto* ws = reinterpret_cast<GuestWinsize*>(static_cast<std::uintptr_t>(a3));
                    ws->ws_row = 24;
                    ws->ws_col = 80;
                    ws->ws_xpixel = 0;
                    ws->ws_ypixel = 0;
                    result = 0;
                } else {
                    result = -EFAULT;
                }
            } else {
                result = ::ioctl(fd, cmd, a3);
                if (result < 0) result = -errno;
            }
            break;
        }

        // -- F2-SY-028: prctl (process control) --------------------------------
        case kX64Prctl: {
            // Pass through to host prctl. The option codes are
            // architecture-independent (defined in <sys/prctl.h>).
            result = static_cast<std::int64_t>(::prctl(
                static_cast<int>(a1),
                static_cast<unsigned long>(a2),
                static_cast<unsigned long>(a3),
                static_cast<unsigned long>(a4),
                static_cast<unsigned long>(a5)));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-027: prlimit64 ----------------------------------------------
        case kX64Prlimit64: {
            // prlimit64(pid, resource, new_rlim, old_rlim). On ARM64 Linux
            // the struct rlimit64 layout matches x86_64 (both 64-bit LE),
            // so we pass the user pointers directly to the raw syscall.
            result = static_cast<std::int64_t>(::syscall(
                SYS_prlimit64,
                static_cast<int>(a1),
                static_cast<int>(a2),
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a3)),
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a4))));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-029: arch_prctl (TLS segment base management) ---------------
        case kX64ArchPrctl: {
            // x86_64 Linux arch_prctl codes (from <asm/prctl.h> / <sys/prctl.h>).
            constexpr std::uint64_t ARCH_SET_GS = 0x1001;
            constexpr std::uint64_t ARCH_SET_FS = 0x1002;
            constexpr std::uint64_t ARCH_GET_FS = 0x1003;
            constexpr std::uint64_t ARCH_GET_GS = 0x1004;

            const std::uint64_t code = a1;
            const std::uint64_t addr = a2;

            switch (code) {
                case ARCH_SET_FS:
                    state->fs_base = addr;
                    result = 0;
                    break;
                case ARCH_SET_GS:
                    state->gs_base = addr;
                    result = 0;
                    break;
                case ARCH_GET_FS: {
                    std::uint64_t* user_ptr =
                        reinterpret_cast<std::uint64_t*>(static_cast<std::uintptr_t>(addr));
                    *user_ptr = state->fs_base;
                    result = 0;
                    break;
                }
                case ARCH_GET_GS: {
                    std::uint64_t* user_ptr =
                        reinterpret_cast<std::uint64_t*>(static_cast<std::uintptr_t>(addr));
                    *user_ptr = state->gs_base;
                    result = 0;
                    break;
                }
                default:
                    result = -EINVAL;
                    break;
            }
            break;
        }

        // -- F2-SY-025: getdents64 (directory listing) -------------------------
        case kX64Getdents64: {
            // x86_64 linux_dirent64 structure layout on the guest side:
            //   offset 0: d_ino    (u64)
            //   offset 8: d_off    (i64)
            //   offset 16: d_reclen (u16)
            //   offset 18: d_type   (u8)
            //   offset 19: d_name[] (null-terminated, padded to 8 bytes)
            //
            // We delegate to the host kernel's getdents64 via raw syscall,
            // which produces the identical binary layout on ARM64 Linux.
            const int fd = static_cast<int>(a1);
            void* buf = reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2));
            const std::uint32_t count = static_cast<std::uint32_t>(a3);
            result = static_cast<std::int64_t>(
                ::syscall(SYS_getdents64, fd, buf, count));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-030: set_tid_address ----------------------------------------
        case kX64SetTidAddress:
            tls_clear_child_tid = a1;
            result = static_cast<std::int64_t>(::gettid());
            break;

        // -- F2-SY-024: epoll --------------------------------------------------
        case kX64EpollCreate1: {
            // epoll_create1(flags) — returns an epoll fd.
            result = static_cast<std::int64_t>(::epoll_create1(
                static_cast<int>(a1)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64EpollCtl: {
            // epoll_ctl(epfd, op, fd, event) — control interface.
            result = static_cast<std::int64_t>(::epoll_ctl(
                static_cast<int>(a1),
                static_cast<int>(a2),
                static_cast<int>(a3),
                reinterpret_cast<struct ::epoll_event*>(
                    static_cast<std::uintptr_t>(a4))));
            if (result < 0) result = -errno;
            break;
        }
        case kX64EpollWait: {
            // epoll_wait(epfd, events, maxevents, timeout).
            result = static_cast<std::int64_t>(::epoll_wait(
                static_cast<int>(a1),
                reinterpret_cast<struct ::epoll_event*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<int>(a3),
                static_cast<int>(a4)));
            if (result < 0) result = -errno;
            break;
        }

        // -- F2-SY-015/016: socket families -----------------------------------
        case kX64Socket: {
            result = ::socket(static_cast<int>(a1), static_cast<int>(a2),
                              static_cast<int>(a3));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Connect: {
            result = ::connect(
                static_cast<int>(a1),
                reinterpret_cast<const struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<::socklen_t>(a3));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Accept: {
            result = ::accept(
                static_cast<int>(a1),
                reinterpret_cast<struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Sendto: {
            result = static_cast<std::int64_t>(::sendto(
                static_cast<int>(a1),
                reinterpret_cast<const void*>(static_cast<std::uintptr_t>(a2)),
                static_cast<std::size_t>(a3),
                static_cast<int>(a4),
                reinterpret_cast<const struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a5)),
                static_cast<::socklen_t>(a6)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Recvfrom: {
            result = static_cast<std::int64_t>(::recvfrom(
                static_cast<int>(a1),
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a2)),
                static_cast<std::size_t>(a3),
                static_cast<int>(a4),
                reinterpret_cast<struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a5)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a6))));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Sendmsg: {
            result = static_cast<std::int64_t>(::sendmsg(
                static_cast<int>(a1),
                reinterpret_cast<const struct ::msghdr*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<int>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Recvmsg: {
            result = static_cast<std::int64_t>(::recvmsg(
                static_cast<int>(a1),
                reinterpret_cast<struct ::msghdr*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<int>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Shutdown: {
            result = ::shutdown(static_cast<int>(a1), static_cast<int>(a2));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Bind: {
            result = ::bind(
                static_cast<int>(a1),
                reinterpret_cast<const struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                static_cast<::socklen_t>(a3));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Listen: {
            result = ::listen(static_cast<int>(a1), static_cast<int>(a2));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Getsockname: {
            result = ::getsockname(
                static_cast<int>(a1),
                reinterpret_cast<struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Getpeername: {
            result = ::getpeername(
                static_cast<int>(a1),
                reinterpret_cast<struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a3)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Socketpair: {
            result = ::socketpair(static_cast<int>(a1), static_cast<int>(a2),
                                  static_cast<int>(a3),
                                  reinterpret_cast<int*>(
                                      static_cast<std::uintptr_t>(a4)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Setsockopt: {
            result = ::setsockopt(
                static_cast<int>(a1), static_cast<int>(a2),
                static_cast<int>(a3),
                reinterpret_cast<const void*>(static_cast<std::uintptr_t>(a4)),
                static_cast<::socklen_t>(a5));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Getsockopt: {
            result = ::getsockopt(
                static_cast<int>(a1), static_cast<int>(a2),
                static_cast<int>(a3),
                reinterpret_cast<void*>(static_cast<std::uintptr_t>(a4)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a5)));
            if (result < 0) result = -errno;
            break;
        }
        case kX64Accept4: {
#if defined(__APPLE__)
            result = -ENOSYS; // macOS doesn't have accept4
#else
            result = ::accept4(
                static_cast<int>(a1),
                reinterpret_cast<struct ::sockaddr*>(
                    static_cast<std::uintptr_t>(a2)),
                reinterpret_cast<::socklen_t*>(
                    static_cast<std::uintptr_t>(a3)),
                static_cast<int>(a4));
            if (result < 0) result = -errno;
#endif
            break;
        }

        // -- F2-SY-017/018: Señales al guest -----------------------------------
        case kX64RtSigaction: {
            // rt_sigaction(sig, act, oact, sigsetsize)
            int sig = static_cast<int>(a1);
            const auto* act = reinterpret_cast<const prisma::runtime::GuestSigaction*>(
                static_cast<std::uintptr_t>(a2));
            auto* oact = reinterpret_cast<prisma::runtime::GuestSigaction*>(
                static_cast<std::uintptr_t>(a3));
            std::size_t sigsetsize = static_cast<std::size_t>(a4);

            if (sigsetsize != 8) {
                result = -EINVAL;
                break;
            }
            if (sig < 1 || sig >= 65 || sig == SIGKILL || sig == SIGSTOP) {
                result = -EINVAL;
                break;
            }

            std::lock_guard<std::mutex> lk(g_sigaction_mutex);
            if (oact != nullptr) {
                *oact = g_guest_sigactions[static_cast<std::size_t>(sig)];
            }
            if (act != nullptr) {
                g_guest_sigactions[static_cast<std::size_t>(sig)] = *act;
                // If it's not a synchronous signal we already handle, maybe
                // we should install a host handler so the host kernel delivers it to us.
                // For this implementation, we just rely on whatever host signals 
                // we've set up, or the fact that this is mainly for Segv/Ill etc.
            }
            result = 0;
            break;
        }
        case kX64RtSigprocmask: {
            // rt_sigprocmask(how, set, oset, sigsetsize)
            int how = static_cast<int>(a1);
            const auto* set = reinterpret_cast<const std::uint64_t*>(
                static_cast<std::uintptr_t>(a2));
            auto* oset = reinterpret_cast<std::uint64_t*>(
                static_cast<std::uintptr_t>(a3));
            std::size_t sigsetsize = static_cast<std::size_t>(a4);

            if (sigsetsize != 8) {
                result = -EINVAL;
                break;
            }

            if (oset != nullptr) {
                *oset = state->blocked_signals;
            }
            if (set != nullptr) {
                std::uint64_t unblockable = (1ULL << (SIGKILL - 1)) | (1ULL << (SIGSTOP - 1));
                std::uint64_t new_mask = *set & ~unblockable;
                
                if (how == SIG_BLOCK) {
                    state->blocked_signals |= new_mask;
                } else if (how == SIG_UNBLOCK) {
                    state->blocked_signals &= ~new_mask;
                } else if (how == SIG_SETMASK) {
                    state->blocked_signals = new_mask;
                } else {
                    result = -EINVAL;
                    break;
                }
            }
            result = 0;
            break;
        }
        case kX64RtSigreturn: {
            // The guest handler did `ret`, which popped the restorer address.
            // Now RSP points directly to the `ucontext_t` (or siginfo + ucontext_t).
            // Actually, Linux ABI says `ucontext_t` is at RSP.
            std::uint64_t rsp = state->gpr[static_cast<std::size_t>(Gpr::Rsp)];
            
            // For safety, we should really use a GuestMemoryReader, but syscall_handler
            // assumes direct memory access.
            const auto* uc = reinterpret_cast<const GuestUcontext*>(static_cast<std::uintptr_t>(rsp));
            
            // Restore GPRs
            state->gpr[static_cast<std::size_t>(Gpr::R8)] = uc->uc_mcontext.r8;
            state->gpr[static_cast<std::size_t>(Gpr::R9)] = uc->uc_mcontext.r9;
            state->gpr[static_cast<std::size_t>(Gpr::R10)] = uc->uc_mcontext.r10;
            state->gpr[static_cast<std::size_t>(Gpr::R11)] = uc->uc_mcontext.r11;
            state->gpr[static_cast<std::size_t>(Gpr::R12)] = uc->uc_mcontext.r12;
            state->gpr[static_cast<std::size_t>(Gpr::R13)] = uc->uc_mcontext.r13;
            state->gpr[static_cast<std::size_t>(Gpr::R14)] = uc->uc_mcontext.r14;
            state->gpr[static_cast<std::size_t>(Gpr::R15)] = uc->uc_mcontext.r15;
            state->gpr[static_cast<std::size_t>(Gpr::Rdi)] = uc->uc_mcontext.rdi;
            state->gpr[static_cast<std::size_t>(Gpr::Rsi)] = uc->uc_mcontext.rsi;
            state->gpr[static_cast<std::size_t>(Gpr::Rbp)] = uc->uc_mcontext.rbp;
            state->gpr[static_cast<std::size_t>(Gpr::Rbx)] = uc->uc_mcontext.rbx;
            state->gpr[static_cast<std::size_t>(Gpr::Rdx)] = uc->uc_mcontext.rdx;
            state->gpr[static_cast<std::size_t>(Gpr::Rax)] = uc->uc_mcontext.rax;
            state->gpr[static_cast<std::size_t>(Gpr::Rcx)] = uc->uc_mcontext.rcx;
            state->gpr[static_cast<std::size_t>(Gpr::Rsp)] = uc->uc_mcontext.rsp;
            
            state->guest_pc = uc->uc_mcontext.rip;
            state->rflags = uc->uc_mcontext.eflags;
            
            // Restore signal mask
            state->blocked_signals = uc->uc_sigmask;
            
            // We do NOT set `result` to anything that gets written to RAX,
            // because `rt_sigreturn` restores RAX from the context!
            // To prevent the common syscall epilogue from overwriting RAX and CF,
            // we could either add a flag or just let it return normally and 
            // modify the syscall epilogue in this file.
            // Wait, the syscall epilogue in this file writes `result` to RAX!
            // We must bypass it or set a special flag.
            // Actually, we can just return directly from this switch case!
            if (strace_enabled()) {
                std::fprintf(stderr, " -> <restored rip=0x%llx>\n", static_cast<unsigned long long>(state->guest_pc));
            }
            return; // EXIT EARLY to avoid overwriting RAX!
        }
        case kX64RtSigsuspend: {
            // rt_sigsuspend(mask, sigsetsize)
            result = -ENOSYS; // To be implemented with dispatcher coordination
            break;
        }

        default:
            result = -ENOSYS;
            break;
    }

    if (strace_enabled()) {
        std::fprintf(stderr, "%lld\n", static_cast<long long>(result));
    }

    state->gpr[static_cast<std::size_t>(Gpr::Rax)] = static_cast<std::uint64_t>(result);
}

#else   // _MSC_VER — stub returning -ENOSYS for all syscalls

extern "C" void prisma_syscall_handler(prisma::runtime::CpuStateFrame* state) {
    // All syscalls return -ENOSYS on MSVC builds.
    state->gpr[static_cast<std::size_t>(prisma::ir::Gpr::Rax)] =
        static_cast<std::uint64_t>(-ENOSYS);
}

#endif  // _MSC_VER
