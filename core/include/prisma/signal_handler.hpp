// prisma/signal_handler.hpp — install SIGSEGV / SIGILL handlers for JIT.
//
// In a full DBT these fire for three reasons that we care about:
//
//   1. Guest code faulted (guest accessed unmapped memory, executed an
//      illegal instruction). We need to reflect the fault into guest
//      state — deliver a SIGSEGV / SIGILL *inside* the guest.
//
//   2. Self-modifying code: the write-protection we place on translated
//      code pages trips when the guest writes a page we've translated.
//      The handler invalidates the translation cache entry and returns.
//
//   3. Host-side bugs in Prisma itself. The handler prints diagnostics
//      and terminates — no "silent wrong answer" allowed.
//
// Status: Fase 0 skeleton. Installs a thread-safe handler that routes
// to a per-thread recovery callback; tests exercise the "recover from
// SIGSEGV" path using a tiny trampoline. Guest-side fault delivery and
// W^X-driven SMC invalidation are the Fase 1+ extensions.

#pragma once

#include <csetjmp>
#include <csignal>
#include <cstdint>

namespace prisma::runtime {

// Reason the recovery point was entered. Passed as the argument to
// setjmp/longjmp via `sig_longjmp_val`.
enum class FaultKind : int {
    None    = 0,
    Segv    = 1,
    Ill     = 2,
    Bus     = 3,  // includes misaligned-access faults on some platforms
    Unknown = 99,
};

// Install SIGSEGV / SIGILL / SIGBUS handlers. Safe to call more than
// once; each call reasserts Prisma's handlers in case a test framework
// or embedding host changed them. Not thread-safe with respect to other
// code that installs handlers for the same signals.
void install_handlers();

// Optional SMC invalidation hook used by the SIGSEGV path after
// SmcGuard consumes a write fault on a protected guest-code page.
// The callback receives each cache key registered for the faulting
// page. Passing nullptr restores the no-op behaviour.
using SmcInvalidateCallback = void (*)(std::uint64_t cache_key,
                                       void* ctx) noexcept;
void set_smc_invalidate_callback(SmcInvalidateCallback callback,
                                 void* ctx) noexcept;
void clear_smc_invalidate_callback() noexcept;

// Drains SMC fault bookkeeping queued by the SIGSEGV handler and runs
// the registered invalidation callback for each affected cache key —
// in normal context, where allocation and locking are legal. The
// dispatcher calls this between blocks; tests call it after provoking
// a fault. Returns the number of pages drained (0 = nothing pending).
std::size_t drain_smc_invalidations();

// Thread-local "protected scope" state. A call site does:
//
//   jmp_buf jb;
//   if (setjmp(jb) == 0) {
//       ScopedProtected guard(jb);
//       invoke_jit_code();   // may trap
//   } else {
//       // trap happened; guard is destroyed via stack unwind and state
//       // is restored.
//   }
//
// This is the minimal interface the tests use to verify trap recovery.
class ScopedProtected {
public:
    explicit ScopedProtected(std::jmp_buf& jb) noexcept;
    ~ScopedProtected() noexcept;
    ScopedProtected(const ScopedProtected&) = delete;
    ScopedProtected& operator=(const ScopedProtected&) = delete;
    ScopedProtected(ScopedProtected&&) = delete;
    ScopedProtected& operator=(ScopedProtected&&) = delete;

private:
    std::jmp_buf* prev_;
};

// For inspection in tests: returns the most recent FaultKind that the
// current thread recovered from, or None if it has not recovered from a
// fault. Reset to None by ScopedProtected construction.
[[nodiscard]] FaultKind last_fault_kind() noexcept;

}  // namespace prisma::runtime
