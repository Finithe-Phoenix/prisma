// prisma/syscall_handler.hpp — x86_64 → host syscall dispatch (F2-SY-001/002).
//
// When the guest executes `SYSCALL` (0F 05), the generated ARM64 code calls
// `prisma_syscall_handler` via `blr`. The handler reads the guest's register
// state from the CpuStateFrame (RAX = syscall number, RDI/RSI/RDX/R10/R8/R9
// = args), translates to a host POSIX call, and writes the result back to
// guest RAX (and CF for error indication via the carry flag slot).
//
// Platform: each syscall is implemented via the host C library (POSIX). The
// same source works on Linux ARM64 and macOS ARM64 for the common subset
// (read, write, open, close, exit, brk, etc.). Unimplemented syscall
// numbers return -ENOSYS.

#pragma once

#include <cstdint>
#include <mutex>
#include <array>

namespace prisma::runtime {

// x86_64 struct sigaction for kernel ABI
struct GuestSigaction {
    std::uint64_t handler;
    std::uint64_t sa_flags;
    std::uint64_t sa_restorer;
    std::uint64_t sa_mask;
};

// x86_64 ucontext and sigcontext for signal delivery
struct GuestSigcontext {
    std::uint64_t r8, r9, r10, r11, r12, r13, r14, r15;
    std::uint64_t rdi, rsi, rbp, rbx, rdx, rax, rcx, rsp, rip, eflags;
    std::uint16_t cs, gs, fs, pad0;
    std::uint64_t err, trapno, oldmask, cr2;
    std::uint64_t fpstate;
    std::uint64_t reserved[8];
};

struct GuestUcontext {
    std::uint64_t uc_flags;
    std::uint64_t uc_link;
    std::uint64_t uc_stack_sp;
    std::int32_t uc_stack_flags;
    std::uint32_t padding;
    std::uint64_t uc_stack_size;
    GuestSigcontext uc_mcontext;
    std::uint64_t uc_sigmask;
    std::uint64_t reserved[15]; // padding to match sizeof(ucontext_t) on Linux
};

// Returns a copy of the guest sigaction for a given signal (1-64),
// thread-safe. Returns empty struct if sig is out of bounds.
GuestSigaction get_guest_sigaction(int sig);

struct CpuStateFrame;

extern "C" void prisma_syscall_handler(CpuStateFrame* state);

}  // namespace prisma::runtime
