#include <catch2/catch_test_macros.hpp>
#include <catch2/generators.hpp>

#include <cerrno>
#include <cstdint>

#include "prisma/cpu_state.hpp"
#include "prisma/ir.hpp"
#include "prisma/syscall_handler.hpp"

using namespace prisma::ir;
using namespace prisma::runtime;

namespace {

[[maybe_unused]] constexpr std::uint64_t kX64Getpid = 39;
[[maybe_unused]] constexpr std::uint64_t kX64Getuid = 102;
[[maybe_unused]] constexpr std::uint64_t kX64Getgid = 104;
[[maybe_unused]] constexpr std::uint64_t kX64Gettid = 186;
[[maybe_unused]] constexpr std::uint64_t kX64ArchPrctl = 158;
[[maybe_unused]] constexpr std::uint64_t kX64Brk = 12;
[[maybe_unused]] constexpr std::uint64_t kX64Getcwd = 79;

[[maybe_unused]] constexpr std::uint64_t ARCH_SET_FS = 0x1001;
[[maybe_unused]] constexpr std::uint64_t ARCH_SET_GS = 0x1002;
[[maybe_unused]] constexpr std::uint64_t ARCH_GET_FS = 0x1003;
[[maybe_unused]] constexpr std::uint64_t ARCH_GET_GS = 0x1004;

void set_syscall(CpuStateFrame& frame, std::uint64_t sysno,
                 std::uint64_t a1 = 0, std::uint64_t a2 = 0,
                 std::uint64_t a3 = 0, std::uint64_t a4 = 0,
                 std::uint64_t a5 = 0, std::uint64_t a6 = 0) {
    frame[Gpr::Rax] = sysno;
    frame[Gpr::Rdi] = a1;
    frame[Gpr::Rsi] = a2;
    frame[Gpr::Rdx] = a3;
    frame[Gpr::R10] = a4;
    frame[Gpr::R8]  = a5;
    frame[Gpr::R9]  = a6;
}

}  // namespace

TEST_CASE("syscall_handler: unknown syscall number returns -ENOSYS") {
    CpuStateFrame frame{};
    set_syscall(frame, 9999);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == static_cast<std::uint64_t>(-ENOSYS));
}

TEST_CASE("syscall_handler: does not clobber unrelated registers") {
    CpuStateFrame frame{};
    set_syscall(frame, 9999);
    frame[Gpr::Rbx] = 0xDEAD;
    frame[Gpr::Rcx] = 0xBEEF;
    frame[Gpr::Rsp] = 0xCAFE;
    frame[Gpr::Rbp] = 0xCODE;

    prisma_syscall_handler(&frame);

    REQUIRE(frame[Gpr::Rbx] == 0xDEAD);
    REQUIRE(frame[Gpr::Rcx] == 0xBEEF);
    REQUIRE(frame[Gpr::Rsp] == 0xCAFE);
    REQUIRE(frame[Gpr::Rbp] == 0xCODE);
}

#ifndef _MSC_VER

TEST_CASE("syscall_handler: getpid returns a positive process id") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Getpid);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] > 0);
}

TEST_CASE("syscall_handler: getuid returns a non-negative user id") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Getuid);
    prisma_syscall_handler(&frame);
    REQUIRE(static_cast<std::int64_t>(frame[Gpr::Rax]) >= 0);
}

TEST_CASE("syscall_handler: getgid returns a non-negative group id") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Getgid);
    prisma_syscall_handler(&frame);
    REQUIRE(static_cast<std::int64_t>(frame[Gpr::Rax]) >= 0);
}

TEST_CASE("syscall_handler: gettid returns a positive thread id") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Gettid);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] > 0);
}

TEST_CASE("syscall_handler: getcwd returns current directory") {
    CpuStateFrame frame{};
    char buf[4096];
    set_syscall(frame, kX64Getcwd, reinterpret_cast<std::uint64_t>(buf), sizeof(buf));
    prisma_syscall_handler(&frame);
    REQUIRE(static_cast<std::int64_t>(frame[Gpr::Rax]) > 0);
    REQUIRE(buf[0] == '/');
}

TEST_CASE("syscall_handler: brk with zero returns current break") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Brk, 0);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] > 0);
}

TEST_CASE("syscall_handler: arch_prctl ARCH_SET_FS stores fs_base") {
    CpuStateFrame frame{};
    constexpr std::uint64_t kFsVal = 0x7F0000000000;
    set_syscall(frame, kX64ArchPrctl, ARCH_SET_FS, kFsVal);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == 0);
    REQUIRE(frame.fs_base == kFsVal);
}

TEST_CASE("syscall_handler: arch_prctl ARCH_SET_GS stores gs_base") {
    CpuStateFrame frame{};
    constexpr std::uint64_t kGsVal = 0x7F0000001000;
    set_syscall(frame, kX64ArchPrctl, ARCH_SET_GS, kGsVal);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == 0);
    REQUIRE(frame.gs_base == kGsVal);
}

TEST_CASE("syscall_handler: arch_prctl ARCH_GET_FS reads back fs_base") {
    CpuStateFrame frame{};
    frame.fs_base = 0x7F0000002000;
    std::uint64_t result = 0;
    set_syscall(frame, kX64ArchPrctl, ARCH_GET_FS, reinterpret_cast<std::uint64_t>(&result));
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == 0);
    REQUIRE(result == 0x7F0000002000);
}

TEST_CASE("syscall_handler: arch_prctl ARCH_GET_GS reads back gs_base") {
    CpuStateFrame frame{};
    frame.gs_base = 0x7F0000003000;
    std::uint64_t result = 0;
    set_syscall(frame, kX64ArchPrctl, ARCH_GET_GS, reinterpret_cast<std::uint64_t>(&result));
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == 0);
    REQUIRE(result == 0x7F0000003000);
}

TEST_CASE("syscall_handler: arch_prctl with unknown code returns -EINVAL") {
    CpuStateFrame frame{};
    constexpr std::uint64_t kUnknownArchPrctlCode = 0xDEAD;
    set_syscall(frame, kX64ArchPrctl, kUnknownArchPrctlCode);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == static_cast<std::uint64_t>(-EINVAL));
}

TEST_CASE("syscall_handler: repeated idempotent syscalls return stable results") {
    auto sysno = GENERATE(kX64Getpid, kX64Getuid, kX64Getgid);
    CpuStateFrame a{}, b{};
    set_syscall(a, sysno);
    set_syscall(b, sysno);
    prisma_syscall_handler(&a);
    prisma_syscall_handler(&b);
    REQUIRE(a[Gpr::Rax] == b[Gpr::Rax]);
}

#else  // _MSC_VER

TEST_CASE("syscall_handler: MSVC stub returns -ENOSYS for getpid") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Getpid);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == static_cast<std::uint64_t>(-ENOSYS));
}

TEST_CASE("syscall_handler: MSVC stub returns -ENOSYS for getuid") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Getuid);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == static_cast<std::uint64_t>(-ENOSYS));
}

TEST_CASE("syscall_handler: MSVC stub returns -ENOSYS for brk") {
    CpuStateFrame frame{};
    set_syscall(frame, kX64Brk);
    prisma_syscall_handler(&frame);
    REQUIRE(frame[Gpr::Rax] == static_cast<std::uint64_t>(-ENOSYS));
}

#endif  // _MSC_VER
