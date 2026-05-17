// core/src/runtime/signal_handler.cpp — POSIX signal handlers for the JIT.
//
// Implementation notes:
//
//   * setjmp/longjmp is the simplest portable mechanism to resume normal
//     control flow from a signal handler on POSIX. siglongjmp is the
//     signal-safe variant and is what we use.
//
//   * Per-thread state lives in a `thread_local` jmp_buf pointer and
//     FaultKind slot. Signal handlers must only touch async-signal-safe
//     state; thread_local POD access is safe in practice on glibc and
//     Darwin, which is what we target.
//
//   * We install with SA_SIGINFO so future extensions can read the
//     faulting address (for SMC detection) and CPU context.

#include "prisma/signal_handler.hpp"

#include <csetjmp>
#include <csignal>
#include <cstdlib>
#include <cstddef>
#include <cstring>

namespace prisma::runtime {

namespace {

// Per-thread recovery state.
thread_local std::jmp_buf* tls_current_jb = nullptr;
thread_local FaultKind     tls_last_fault = FaultKind::None;

// Map POSIX signal numbers to our FaultKind enum.
FaultKind fault_for_signal(int sig) noexcept {
    switch (sig) {
        case SIGSEGV: return FaultKind::Segv;
        case SIGILL:  return FaultKind::Ill;
        case SIGBUS:  return FaultKind::Bus;
        default:      return FaultKind::Unknown;
    }
}

// The handler itself. Must be async-signal-safe — we only touch
// thread_local POD, raise the signal by re-default, or longjmp.
[[noreturn]] void prisma_sig_handler(int sig, siginfo_t* /*info*/, void* /*ctx*/) {
    if (tls_current_jb != nullptr) {
        tls_last_fault = fault_for_signal(sig);
        // siglongjmp back to the ScopedProtected setup. The non-zero
        // value lets the caller tell "setjmp returned from longjmp"
        // from "setjmp returned fresh".
        std::longjmp(*tls_current_jb, static_cast<int>(tls_last_fault));
    }

    // No protection scope: restore the default handler and re-raise.
    // This terminates the process with the usual core dump.
    struct sigaction dfl{};
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);  // may be a macro on Darwin
    ::sigaction(sig, &dfl, nullptr);
    ::raise(sig);
    // raise() returns; the re-raised signal fires before this next line.
    std::_Exit(128 + sig);
}

void install_one(int sig) {
    struct sigaction act{};
    act.sa_sigaction = &prisma_sig_handler;
    act.sa_flags = SA_SIGINFO | SA_NODEFER;
    sigemptyset(&act.sa_mask);  // may be a macro on Darwin
    ::sigaction(sig, &act, nullptr);
}

}  // namespace

void install_handlers() {
    install_one(SIGSEGV);
    install_one(SIGILL);
    install_one(SIGBUS);
}

ScopedProtected::ScopedProtected(std::jmp_buf& jb) noexcept
    : prev_(tls_current_jb) {
    tls_current_jb = &jb;
    tls_last_fault = FaultKind::None;
}

ScopedProtected::~ScopedProtected() noexcept {
    tls_current_jb = prev_;
}

FaultKind last_fault_kind() noexcept {
    return tls_last_fault;
}

}  // namespace prisma::runtime
