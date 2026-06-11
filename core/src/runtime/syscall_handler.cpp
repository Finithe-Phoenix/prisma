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
#include <cstdlib>

#include "prisma/cpu_state.hpp"
#include "prisma/ir.hpp"

#ifndef _MSC_VER

#include <cstring>
#include <pthread.h>

#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/utsname.h>
#include <unistd.h>

namespace prisma::runtime {

// x86_64 Linux syscall numbers used in this file.
enum X64Sysno : std::uint64_t {
    kX64Read        = 0,
    kX64Write       = 1,
    kX64Open        = 2,
    kX64Close       = 3,
    kX64Stat        = 4,
    kX64Fstat       = 5,
    kX64Lstat       = 6,
    kX64Mmap        = 9,
    kX64Mprotect    = 10,
    kX64Munmap      = 11,
    kX64Brk         = 12,
    kX64RtSigaction = 13,
    kX64RtSigprocmask = 14,
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
    kX64Exit         = 60,
    kX64ExitGroup    = 231,
    kX64Uname        = 63,
    kX64Getdents     = 78,
    kX64Getdents64   = 217,
    kX64ClockGettime = 228,
    kX64Gettimeofday = 96,
    kX64Time         = 201,
};

}  // namespace prisma::runtime

extern "C" void prisma_syscall_handler(prisma::runtime::CpuStateFrame* state) {
    using namespace prisma::runtime;
    using ir::Gpr;

    const std::uint64_t sysno = state->gpr[static_cast<std::size_t>(Gpr::Rax)];
    const std::uint64_t a1 = state->gpr[static_cast<std::size_t>(Gpr::Rdi)];
    const std::uint64_t a2 = state->gpr[static_cast<std::size_t>(Gpr::Rsi)];
    const std::uint64_t a3 = state->gpr[static_cast<std::size_t>(Gpr::Rdx)];
    const std::uint64_t a4 = state->gpr[static_cast<std::size_t>(Gpr::R10)];
    const std::uint64_t a5 = state->gpr[static_cast<std::size_t>(Gpr::R8)];
    const std::uint64_t a6 = state->gpr[static_cast<std::size_t>(Gpr::R9)];

    std::int64_t result = 0;

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
                    static_cast<std::intptr_t>(addr) -
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
            ::exit(static_cast<int>(a1));
            break;
        }
        case kX64ExitGroup: {
            ::exit(static_cast<int>(a1));
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
            if (result == static_cast<std::int64_t>(MAP_FAILED)) {
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

        // -- Default: unimplemented -> ENOSYS ----------------------------------
        default:
            result = -ENOSYS;
            break;
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
