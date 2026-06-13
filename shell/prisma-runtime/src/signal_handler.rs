//! Signal handling for the JIT.
//!
//! Mirrors C++ `core/src/runtime/signal_handler.cpp`. This module provides the
//! safe, testable foundation: the [`FaultKind`] taxonomy and the
//! signal-number-to-fault classification, plus the [`SignalHandler`] arming
//! state.
//!
//! The full fault-recovery mechanism (catch SIGSEGV/SIGILL/SIGBUS during JIT
//! execution and resume control flow) is **intentionally deferred to an RFC**.
//! The C++ implementation uses `sigaction` + `siglongjmp` out of the signal
//! handler. A faithful Rust port needs `sigsetjmp`/`siglongjmp` via FFI, which
//! is `unsafe` and subtle: longjmp out of a handler skips Rust destructors and
//! breaks unwinding, so the protected region must be destructor-free (or the
//! design must use an alternative such as catch_unwind with a handler that
//! rewrites the trap context). Per the project's "correctness > performance,
//! prove it" rule, that belongs in a reviewed RFC, not a rushed commit.

/// Why a protected region trapped. Mirrors C++ `FaultKind`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FaultKind {
    /// No fault occurred.
    #[default]
    None,
    /// Segmentation fault (SIGSEGV) — bad memory access / W^X / SMC page.
    Segv,
    /// Illegal instruction (SIGILL).
    Ill,
    /// Bus error (SIGBUS) — misalignment / mapping error.
    Bus,
    /// A signal outside the handled set.
    Unknown,
}

/// POSIX signal numbers handled by the JIT runtime (Linux/aarch64 + generic).
pub mod signo {
    pub const SIGILL: i32 = 4;
    pub const SIGBUS: i32 = 7; // Linux value (Darwin differs; classified by libc at install time)
    pub const SIGSEGV: i32 = 11;
}

/// Map a POSIX signal number to a [`FaultKind`]. Mirrors C++
/// `fault_for_signal`. Uses the runtime's `libc` signal constants where
/// available so the mapping is correct per platform.
#[must_use]
pub fn fault_for_signal(sig: i32) -> FaultKind {
    #[cfg(unix)]
    {
        if sig == libc::SIGSEGV {
            return FaultKind::Segv;
        }
        if sig == libc::SIGILL {
            return FaultKind::Ill;
        }
        if sig == libc::SIGBUS {
            return FaultKind::Bus;
        }
        FaultKind::Unknown
    }
    #[cfg(not(unix))]
    {
        match sig {
            signo::SIGSEGV => FaultKind::Segv,
            signo::SIGILL => FaultKind::Ill,
            signo::SIGBUS => FaultKind::Bus,
            _ => FaultKind::Unknown,
        }
    }
}

/// Minimal signal handler API used by migration code. Installation/recovery is
/// deferred to an RFC (see module docs); `arm`/`disarm` track intent.
#[derive(Debug, Default, Clone)]
pub struct SignalHandler {
    armed: bool,
}

impl SignalHandler {
    /// Creates a new handler.
    #[must_use]
    pub const fn new() -> Self {
        Self { armed: false }
    }

    /// Arms the handler.
    pub const fn arm(&mut self) {
        self.armed = true;
    }

    /// Disarms the handler.
    pub const fn disarm(&mut self) {
        self.armed = false;
    }

    /// Whether handler is currently armed.
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed
    }
}

#[cfg(test)]
mod tests {
    use super::{fault_for_signal, FaultKind, SignalHandler};

    #[test]
    fn default_fault_is_none() {
        assert_eq!(FaultKind::default(), FaultKind::None);
    }

    #[test]
    fn classifies_known_signals() {
        // Uses libc constants on unix; falls back to the generic table.
        #[cfg(unix)]
        {
            assert_eq!(fault_for_signal(libc::SIGSEGV), FaultKind::Segv);
            assert_eq!(fault_for_signal(libc::SIGILL), FaultKind::Ill);
            assert_eq!(fault_for_signal(libc::SIGBUS), FaultKind::Bus);
        }
        #[cfg(not(unix))]
        {
            assert_eq!(fault_for_signal(11), FaultKind::Segv);
            assert_eq!(fault_for_signal(4), FaultKind::Ill);
        }
    }

    #[test]
    fn unknown_signal_maps_to_unknown() {
        assert_eq!(fault_for_signal(9 /* SIGKILL, not handled */), FaultKind::Unknown);
        assert_eq!(fault_for_signal(0), FaultKind::Unknown);
    }

    #[test]
    fn arming_roundtrips() {
        let mut h = SignalHandler::new();
        assert!(!h.is_armed());
        h.arm();
        assert!(h.is_armed());
        h.disarm();
        assert!(!h.is_armed());
    }
}
