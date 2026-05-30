// prisma/guest_signal.hpp — F1-RT-011 guest signal delivery.
//
// When the host signal handler catches a SIGSEGV / SIGILL / SIGFPE /
// SIGBUS that is NOT explained by SMC (the SmcGuard from F1-RT-010
// gets first crack and consumes the SMC writes), the runtime needs
// to translate the host fault into a guest exception:
//
//   #PF (page fault)        — guest accessed unmapped memory
//   #UD (illegal opcode)    — guest hit an undefined instruction
//   #DE (divide error)      — guest divided by zero or overflowed quot
//   #GP (general protection)— catch-all for the long tail
//
// The runtime registers a `GuestSignalHandler` callback. On a
// non-SMC fault the host SIGSEGV trampoline calls the registered
// handler with a populated `GuestSignal`; the handler decides whether
// to (a) write the guest's IDT-reflection state and resume execution
// at the guest's #PF/#UD/#DE/#GP handler, or (b) abort the JIT
// region by longjmp'ing through the existing ScopedProtected.
//
// This header just defines the surface; the host trampoline
// integration lives in signal_handler.cpp and is wired to the
// runtime's pluggable callback.

#pragma once

#include <cstdint>
#include <functional>

namespace prisma::runtime {

enum class GuestException : std::uint8_t {
    PageFault          = 0x0E,  // #PF — vector 14, x86 IDT slot
    InvalidOpcode      = 0x06,  // #UD — vector 6
    DivideError        = 0x00,  // #DE — vector 0
    GeneralProtection  = 0x0D,  // #GP — vector 13
    AlignmentCheck     = 0x11,  // #AC — vector 17
    Other              = 0xFF,
};

struct GuestSignal {
    GuestException kind;

    // Faulting host address that triggered the signal. For PageFault
    // this is the unmapped guest page; for InvalidOpcode it's the
    // address of the bad instruction stream byte (within the JIT
    // buffer) — callers translate back to guest_pc via the
    // translation cache.
    std::uintptr_t fault_addr;

    // Guest PC where the offending instruction lives. Zero if the
    // runtime can't recover it from the host trap context.
    std::uint64_t guest_pc;

    // For #PF: 1 = write fault, 0 = read fault, 2 = exec fault. For
    // other kinds: zero.
    std::uint8_t  pf_kind;
};

// Decision the user-supplied callback returns to the trampoline.
enum class GuestSignalDisposition : std::uint8_t {
    Resume         = 0,  // handled — resume guest execution
    LongjmpAbort   = 1,  // unhandled — longjmp through ScopedProtected
};

// Callback signature. Must be async-signal-safe (no malloc, no
// std::cout, etc. — only signal-safe primitives).
using GuestSignalHandler =
    std::function<GuestSignalDisposition(const GuestSignal&)>;

// Install / clear the global guest-signal handler. Thread-safe via
// std::atomic. Calling install with `nullptr` is the same as clear.
void install_guest_signal_handler(GuestSignalHandler handler);
void clear_guest_signal_handler();

// Read the currently-installed handler (may be empty). Cheap; uses
// an atomic load. Used by the SIGSEGV trampoline.
[[nodiscard]] bool has_guest_signal_handler() noexcept;

// For tests: synthesise a GuestSignal without going through the
// kernel and dispatch it to the registered handler. Returns the
// callback's disposition, or LongjmpAbort if no handler is set.
[[nodiscard]] GuestSignalDisposition
deliver_guest_signal(const GuestSignal& sig);

}  // namespace prisma::runtime
