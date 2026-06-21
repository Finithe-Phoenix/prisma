//! Robustness coverage for the syscall dispatcher — the guest syscall entry
//! point, fed arbitrary numbers and register arguments.
//!
//! A guest issues syscalls with fully untrusted `(number, args)`, so `dispatch`
//! must never panic and must answer an unrouted number with `-ENOSYS`. The
//! memory region is zeroed so the time syscalls read a `{0,0}` request (an
//! instant sleep) rather than blocking the test.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};
use proptest::prelude::*;

const ENOSYS: i64 = -38;

#[test]
fn dispatch_never_panics_over_the_low_syscall_range() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 64];
    let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };
    // A pointer into the zeroed region for pointer args; fd-shaped args (4096)
    // are harmlessly unopen, so no syscall blocks or grows unbounded.
    for number in 0u64..512 {
        let _ = dispatch(&mut ctx, &mut mem, number, [0x1000, 0x1000, 8, 0, 0, 0]);
    }
}

#[test]
fn dispatch_never_panics_on_extreme_args() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 16];
    let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };
    // Wild pointers / counts: every handler must fault (EFAULT/EBADF), not panic.
    for number in [0u64, 1, 3, 14, 96, 228, 229] {
        let _ = dispatch(
            &mut ctx,
            &mut mem,
            number,
            [u64::MAX, u64::MAX, u64::MAX, 0, 0, 0],
        );
        let _ = dispatch(
            &mut ctx,
            &mut mem,
            number,
            [0, 0xDEAD_BEEF, 0xFFFF, 0, 0, 0],
        );
    }
}

#[test]
fn unrouted_numbers_return_negative_enosys() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 16];
    let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };
    for number in [400u64, 1000, 65_535, u64::MAX, 9999] {
        assert_eq!(dispatch(&mut ctx, &mut mem, number, [0; 6]), ENOSYS);
    }
}

proptest! {
    // Fuzz every routed number with arbitrary register arguments: a fresh
    // context and a zeroed region per case, so no handler panics and the run
    // never blocks. The region stays zeroed precisely so the sleeping time
    // syscalls (nanosleep / clock_nanosleep) read a `{0,0}` (instant) request
    // even when a fuzzed pointer lands inside it; a pointer outside it faults.
    #[test]
    fn dispatch_never_panics_for_arbitrary_number_and_args(
        number in 0u64..400,
        args in any::<[u64; 6]>(),
    ) {
        let mut ctx = SyscallContext::new();
        let buf = [0u8; 256];
        let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };
        // The return is the guest's `rax`: a success value or a negative errno,
        // never a panic. We only assert the call returns (no hang, no abort).
        let _ = dispatch(&mut ctx, &mut mem, number, args);
    }
}
