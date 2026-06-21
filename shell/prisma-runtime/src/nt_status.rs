//! NTSTATUS code decoding.
//!
//! Every Windows status/exception code is a packed 32-bit NTSTATUS: a 2-bit
//! severity, a customer/reserved bit, a 12-bit facility and a 16-bit code. The
//! dispatcher classifies a guest exception by these fields — severity decides
//! whether it is fatal, facility/code identify it. This is the pure decoder
//! over the codes [`crate::guest_exception`] produces.

/// The 2-bit severity in the top of an NTSTATUS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Success,
    Informational,
    Warning,
    Error,
}

/// A decoded NTSTATUS value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtStatus(pub u32);

impl NtStatus {
    /// Severity from bits 30-31.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self.0 >> 30 {
            0 => Severity::Success,
            1 => Severity::Informational,
            2 => Severity::Warning,
            _ => Severity::Error,
        }
    }

    /// Customer-defined bit (29): set by third-party status codes.
    #[must_use]
    pub const fn is_customer(self) -> bool {
        self.0 & (1 << 29) != 0
    }

    /// Facility from bits 16-27 (12 bits).
    #[must_use]
    pub const fn facility(self) -> u16 {
        ((self.0 >> 16) & 0x0FFF) as u16
    }

    /// Status code from bits 0-15.
    #[must_use]
    pub const fn code(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// Whether this is an error-severity status (`0xC...` codes). Mirrors
    /// `HostFault::is_fatal` but works on any raw NTSTATUS.
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(self.severity(), Severity::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_exception::codes;

    #[test]
    fn access_violation_decodes_to_error_facility0_code5() {
        let s = NtStatus(codes::ACCESS_VIOLATION); // 0xC0000005
        assert_eq!(s.severity(), Severity::Error);
        assert!(s.is_error());
        assert!(!s.is_customer());
        assert_eq!(s.facility(), 0);
        assert_eq!(s.code(), 5);
    }

    #[test]
    fn breakpoint_is_warning_severity_not_error() {
        let s = NtStatus(codes::BREAKPOINT); // 0x80000003
        assert_eq!(s.severity(), Severity::Warning);
        assert!(!s.is_error());
        assert_eq!(s.code(), 3);
    }

    #[test]
    fn severity_covers_all_four_quadrants() {
        assert_eq!(NtStatus(0x0000_0000).severity(), Severity::Success);
        assert_eq!(NtStatus(0x4000_0000).severity(), Severity::Informational);
        assert_eq!(NtStatus(0x8000_0000).severity(), Severity::Warning);
        assert_eq!(NtStatus(0xC000_0000).severity(), Severity::Error);
    }

    #[test]
    fn customer_and_facility_bits_decode() {
        // Customer bit set, facility 0x123, code 0x4567.
        let s = NtStatus((1 << 29) | (0x123 << 16) | 0x4567);
        assert!(s.is_customer());
        assert_eq!(s.facility(), 0x123);
        assert_eq!(s.code(), 0x4567);
    }

    #[test]
    fn all_guest_exception_codes_classify_by_severity() {
        // The 0xC... codes are errors; the 0x8... codes are warnings.
        for c in [
            codes::ACCESS_VIOLATION,
            codes::ILLEGAL_INSTRUCTION,
            codes::INT_DIVIDE_BY_ZERO,
        ] {
            assert!(NtStatus(c).is_error());
        }
        for c in [codes::DATATYPE_MISALIGNMENT, codes::BREAKPOINT] {
            assert!(!NtStatus(c).is_error());
        }
    }
}
