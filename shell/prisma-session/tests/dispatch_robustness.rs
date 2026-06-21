//! Robustness coverage for the syscall dispatcher — the guest syscall entry
//! point, fed arbitrary numbers and register arguments.
//!
//! A guest issues syscalls with fully untrusted `(number, args)`, so `dispatch`
//! must never panic and must answer an unrouted number with `-ENOSYS`. The
//! memory region is zeroed so the time syscalls read a `{0,0}` request (an
//! instant sleep) rather than blocking the test.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const ENOSYS: i64 = -38;

#[test]
fn dispatch_never_panics_over_the_low_syscall_range() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 64];
    let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
    // A pointer into the zeroed region for pointer args; fd-shaped args (4096)
    // are harmlessly unopen, so no syscall blocks or grows unbounded.
    for number in 0u64..512 {
        let _ = dispatch(&mut ctx, &mut mem, number, [0x1000, 0x1000, 8, 0, 0, 0]);
    }
}

#[test]
fn dispatch_never_panics_on_extreme_args() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
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
    let mut buf = [0u8; 16];
    let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
    for number in [400u64, 1000, 65_535, u64::MAX, 9999] {
        assert_eq!(dispatch(&mut ctx, &mut mem, number, [0; 6]), ENOSYS);
    }
}
