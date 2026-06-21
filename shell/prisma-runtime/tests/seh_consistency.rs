//! Cross-module SEH-layer consistency + robustness.
//!
//! The three SEH pieces are built separately — guest_exception (host fault ->
//! NTSTATUS code), exception_record (the 152-byte delivery payload), nt_status
//! (severity/facility decode). This asserts they agree on the same fault and
//! that record construction never panics on arbitrary inputs, the contract the
//! dispatcher relies on when it turns a host fault into a guest exception.

use proptest::prelude::*;

use prisma_runtime::exception_record::{ExceptionRecord, EXCEPTION_NONCONTINUABLE};
use prisma_runtime::guest_exception::HostFault;
use prisma_runtime::nt_status::NtStatus;

const ALL_FAULTS: [HostFault; 5] = [
    HostFault::Segv,
    HostFault::Ill,
    HostFault::Fpe,
    HostFault::Bus,
    HostFault::Trap,
];

#[test]
fn fault_code_severity_and_record_flag_all_agree() {
    for fault in ALL_FAULTS {
        let code = fault.guest_exception_code();

        // guest_exception's is_fatal and nt_status's severity must agree on
        // whether the code is an error (0xC...) — two independent decodings.
        assert_eq!(
            fault.is_fatal(),
            NtStatus(code).is_error(),
            "is_fatal vs NtStatus severity disagree for {fault:?}"
        );

        // The EXCEPTION_RECORD's code field is exactly the fault's code, and its
        // NONCONTINUABLE flag is set iff the fault is fatal.
        let rec = ExceptionRecord::from_fault(fault, 0x1000, &[]);
        let bytes = rec.to_bytes();
        let rec_code = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let rec_flags = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(rec_code, code);
        assert_eq!(rec_flags & EXCEPTION_NONCONTINUABLE != 0, fault.is_fatal());
    }
}

#[test]
fn every_posix_fault_signal_classifies_consistently() {
    // SIGSEGV/SIGILL/SIGFPE/SIGBUS/SIGTRAP all map to a fault whose code decodes
    // back to a non-zero status; non-fault signals classify to None.
    for signo in [11, 4, 8, 7, 5] {
        let fault = HostFault::from_signal(signo).expect("a guest hardware fault");
        assert_ne!(fault.guest_exception_code(), 0);
    }
    for signo in [2, 9, 15, 0, 99] {
        assert!(HostFault::from_signal(signo).is_none());
    }
}

proptest! {
    /// EXCEPTION_RECORD construction never panics on an arbitrary faulting
    /// address and information-word set, and the serialized record reports the
    /// faulting address and a clamped parameter count.
    #[test]
    fn exception_record_construction_never_panics(
        fault_idx in 0usize..ALL_FAULTS.len(),
        address in any::<u64>(),
        info in prop::collection::vec(any::<u64>(), 0..40),
    ) {
        let fault = ALL_FAULTS[fault_idx];
        let rec = ExceptionRecord::from_fault(fault, address, &info);
        let bytes = rec.to_bytes();
        // ExceptionAddress (offset 16) round-trips.
        prop_assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), address);
        // NumberParameters (offset 24) never exceeds the ABI's 15.
        let n = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        prop_assert!(n <= 15);
    }
}
