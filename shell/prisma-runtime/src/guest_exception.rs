//! Host-fault to Windows guest-exception mapping.
//!
//! When JITed guest code faults, the host delivers a hardware signal
//! (SIGSEGV/SIGILL/...). A Windows guest expects that to surface through SEH as
//! an NTSTATUS exception code (`EXCEPTION_ACCESS_VIOLATION`, ...). This module
//! is the pure, platform-neutral translation between the two; the signal
//! handler classifies the host fault and this maps it to the code the guest's
//! `__try`/`__except` machinery dispatches on.

/// A hardware fault delivered by the host while running guest code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFault {
    /// Invalid memory access (SIGSEGV).
    Segv,
    /// Illegal/undefined instruction (SIGILL).
    Ill,
    /// Arithmetic fault such as integer divide-by-zero (SIGFPE).
    Fpe,
    /// Misaligned access / bus error (SIGBUS).
    Bus,
    /// Breakpoint/trap (SIGTRAP).
    Trap,
}

/// Windows NTSTATUS exception codes the guest's SEH dispatches on (ntstatus.h).
pub mod codes {
    pub const ACCESS_VIOLATION: u32 = 0xC000_0005;
    pub const ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
    pub const INT_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
    pub const DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
    pub const BREAKPOINT: u32 = 0x8000_0003;
}

impl HostFault {
    /// Classify a POSIX signal number into a [`HostFault`]. `None` for signals
    /// that are not guest hardware faults (the runtime handles those itself).
    #[must_use]
    pub const fn from_signal(signo: i32) -> Option<Self> {
        match signo {
            11 => Some(Self::Segv), // SIGSEGV
            4 => Some(Self::Ill),   // SIGILL
            8 => Some(Self::Fpe),   // SIGFPE
            7 => Some(Self::Bus),   // SIGBUS
            5 => Some(Self::Trap),  // SIGTRAP
            _ => None,
        }
    }

    /// The Windows NTSTATUS exception code this fault surfaces as in the guest.
    #[must_use]
    pub const fn guest_exception_code(self) -> u32 {
        match self {
            Self::Segv => codes::ACCESS_VIOLATION,
            Self::Ill => codes::ILLEGAL_INSTRUCTION,
            // x86 #DE (divide error) is the dominant SIGFPE source from DIV/IDIV.
            Self::Fpe => codes::INT_DIVIDE_BY_ZERO,
            Self::Bus => codes::DATATYPE_MISALIGNMENT,
            Self::Trap => codes::BREAKPOINT,
        }
    }

    /// Whether this exception is, by Windows convention, non-continuable
    /// (the high "severity error" nibble `0xC` rather than `0x8`).
    #[must_use]
    pub const fn is_fatal(self) -> bool {
        self.guest_exception_code() & 0xF000_0000 == 0xC000_0000
    }
}

/// Map a POSIX signal straight to a guest exception code, or `None` if the
/// signal is not a guest hardware fault.
#[must_use]
pub const fn signal_to_guest_code(signo: i32) -> Option<u32> {
    match HostFault::from_signal(signo) {
        Some(f) => Some(f.guest_exception_code()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segv_maps_to_access_violation() {
        assert_eq!(
            HostFault::Segv.guest_exception_code(),
            0xC000_0005,
            "EXCEPTION_ACCESS_VIOLATION"
        );
    }

    #[test]
    fn each_fault_has_its_canonical_code() {
        assert_eq!(HostFault::Ill.guest_exception_code(), 0xC000_001D);
        assert_eq!(HostFault::Fpe.guest_exception_code(), 0xC000_0094);
        assert_eq!(HostFault::Bus.guest_exception_code(), 0x8000_0002);
        assert_eq!(HostFault::Trap.guest_exception_code(), 0x8000_0003);
    }

    #[test]
    fn signal_classification_round_trips() {
        assert_eq!(HostFault::from_signal(11), Some(HostFault::Segv));
        assert_eq!(HostFault::from_signal(4), Some(HostFault::Ill));
        assert_eq!(HostFault::from_signal(8), Some(HostFault::Fpe));
        assert_eq!(HostFault::from_signal(7), Some(HostFault::Bus));
        assert_eq!(HostFault::from_signal(5), Some(HostFault::Trap));
        // SIGINT (2) / SIGTERM (15) are not guest hardware faults.
        assert_eq!(HostFault::from_signal(2), None);
        assert_eq!(HostFault::from_signal(15), None);
    }

    #[test]
    fn severity_nibble_marks_fatal() {
        // 0xC... = non-continuable error; 0x8... = warning-severity (continuable).
        assert!(HostFault::Segv.is_fatal());
        assert!(HostFault::Fpe.is_fatal());
        assert!(!HostFault::Bus.is_fatal());
        assert!(!HostFault::Trap.is_fatal());
    }

    #[test]
    fn signal_to_guest_code_composes() {
        assert_eq!(signal_to_guest_code(11), Some(0xC000_0005));
        assert_eq!(signal_to_guest_code(99), None);
    }
}
