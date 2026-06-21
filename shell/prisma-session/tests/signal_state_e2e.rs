//! End-to-end checks for the signal syscalls' shared state.
//!
//! These drive the public `dispatch` entry point to validate that the signal
//! mask, pending set, and per-signal dispositions persist correctly across
//! *other* syscalls — the composed behaviour a real guest depends on, which the
//! single-call unit tests cannot observe. The kernel rule that SIGKILL/SIGSTOP
//! can never be blocked is also checked through the public boundary.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_signal::{SigAction, Sigset};
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_RT_SIGACTION: u64 = 13;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_GETPID: u64 = 39;
const SYS_RT_SIGPENDING: u64 = 127;

const SIG_SETMASK: u64 = 2;

fn region(buf: &mut [u8]) -> GuestRegion<'_> {
    GuestRegion::new(0x1000, Protection::ReadWrite, buf)
}

#[test]
fn blocked_mask_persists_across_an_unrelated_syscall() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    // A mask blocking signal 10 at guest 0x1000.
    let mut want = Sigset::empty();
    want.insert(10);
    buf[0..8].copy_from_slice(&want.to_guest_bytes());
    let mut mem = region(&mut buf);

    // rt_sigprocmask(SETMASK, set=0x1000, oldset=0).
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPROCMASK,
            [SIG_SETMASK, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    assert!(ctx.signals.blocked().contains(10));
    // An unrelated syscall must not disturb the mask.
    let _ = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert!(ctx.signals.blocked().contains(10));
}

#[test]
fn pending_set_survives_and_reports_through_dispatch() {
    let mut ctx = SyscallContext::new();
    ctx.signals.raise(12);
    ctx.signals.raise(7);
    let mut buf = [0u8; 8];
    let mut mem = region(&mut buf);

    // An unrelated syscall first — the pending set must survive it.
    let _ = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    // rt_sigpending(set=0x1000) writes the still-pending set.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPENDING,
            [0x1000, 0, 0, 0, 0, 0]
        ),
        0
    );
    let reported = Sigset::from_guest_bytes(&buf).unwrap();
    assert!(reported.contains(12) && reported.contains(7));
    assert!(!reported.contains(11));
}

#[test]
fn sigkill_and_sigstop_can_never_be_blocked_through_dispatch() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 8];
    // Request a mask blocking SIGKILL (9) and SIGSTOP (19).
    let mut want = Sigset::empty();
    want.insert(9);
    want.insert(19);
    buf[0..8].copy_from_slice(&want.to_guest_bytes());
    let mut mem = region(&mut buf);

    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPROCMASK,
            [SIG_SETMASK, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    // The kernel rule holds end to end: neither is actually blocked.
    assert!(!ctx.signals.blocked().contains(9));
    assert!(!ctx.signals.blocked().contains(19));
}

#[test]
fn installed_disposition_persists_and_oldact_reports_it() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 64];
    let act = SigAction {
        handler: 0x1_4000_9000,
        flags: 4,
        restorer: 0,
        mask: Sigset::empty(),
    };
    buf[0..32].copy_from_slice(&act.to_guest_bytes());
    let mut mem = region(&mut buf);

    // rt_sigaction(sig=11, act=0x1000, oldact=0) installs the handler.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGACTION,
            [11, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    assert_eq!(ctx.signals.action(11), Some(act));
    // It survives an unrelated syscall.
    let _ = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert_eq!(ctx.signals.action(11), Some(act));

    // Re-installing the default reports the previously-installed action in oldact.
    let def = SigAction::default();
    let mut buf2 = [0u8; 64];
    buf2[0..32].copy_from_slice(&def.to_guest_bytes());
    let mut mem2 = region(&mut buf2);
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem2,
            SYS_RT_SIGACTION,
            [11, 0x1000, 0x1020, 0, 0, 0]
        ),
        0
    );
    let old = SigAction::from_guest_bytes(&buf2[32..64]).unwrap();
    assert_eq!(old, act);
}
