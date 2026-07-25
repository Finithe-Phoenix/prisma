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

namespace prisma::runtime {

struct CpuStateFrame;

// Thread-local guest bookkeeping used by the Linux startup syscall surface.
// This is intentionally independent from CpuStateFrame: clone() will later
// own one instance per guest thread without changing the JIT-visible frame
// layout or offsets.
struct GuestThreadStartupState {
    std::uint64_t tid{0};
    std::uint64_t clear_child_tid{0};
    std::uint64_t robust_list_head{0};
    std::uint64_t robust_list_len{0};
};

[[nodiscard]] GuestThreadStartupState current_guest_thread_startup_state() noexcept;

extern "C" void prisma_syscall_handler(CpuStateFrame* state);

}  // namespace prisma::runtime
