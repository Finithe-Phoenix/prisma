//! End-to-end checks for the process-hierarchy syscalls (`getpgrp`, `getpgid`,
//! `getsid`).
//!
//! These landed after the process-identity e2e and model one fact: the lone
//! guest leads its own session and its own process group. Driven through the
//! public `dispatch` entry point, that means every identity query for the
//! caller — pid, tid, pgrp, pgid(0/self), sid(0/self) — returns the same value,
//! and any *other* pid is `ESRCH`. None of these pure reads disturbs process
//! state.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_GETPID: u64 = 39;
const SYS_GETPGRP: u64 = 111;
const SYS_GETPGID: u64 = 121;
const SYS_GETSID: u64 = 124;
const SYS_GETTID: u64 = 186;

const ESRCH: i64 = -3;

fn region(buf: &mut [u8]) -> GuestRegion<'_> {
    GuestRegion::new(0x1000, Protection::ReadWrite, buf)
}

#[test]
fn the_guest_leads_its_own_session_and_group() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 8];
    let mut mem = region(&mut buf);

    let pid = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert!(pid > 0);
    // The caller is the leader of everything it belongs to, so all of these
    // coincide with the pid.
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETTID, [0; 6]), pid);
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETPGRP, [0; 6]), pid);
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETPGID, [0, 0, 0, 0, 0, 0]),
        pid
    );
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETSID, [0, 0, 0, 0, 0, 0]),
        pid
    );
    // Querying by the guest's own pid is the same as querying 0.
    let pid_arg = u64::try_from(pid).unwrap();
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETPGID, [pid_arg, 0, 0, 0, 0, 0]),
        pid
    );
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETSID, [pid_arg, 0, 0, 0, 0, 0]),
        pid
    );
}

#[test]
fn querying_another_pid_is_esrch() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 8];
    let mut mem = region(&mut buf);
    let pid = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    let other = u64::try_from(pid).unwrap().wrapping_add(1);
    // No other process exists in the single-process model.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETPGID, [other, 0, 0, 0, 0, 0]),
        ESRCH
    );
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETSID, [other, 0, 0, 0, 0, 0]),
        ESRCH
    );
}

#[test]
fn hierarchy_queries_are_stable_and_side_effect_free() {
    let mut ctx = SyscallContext::new();
    let fds_before = ctx.fds.open_count();
    ctx.signals.raise(9_u32.wrapping_add(1)); // raise signal 10 as a state marker
    let blocked_before = ctx.signals.blocked();

    let mut buf = [0u8; 8];
    let mut mem = region(&mut buf);
    // Run the whole hierarchy twice — answers are stable and nothing is touched.
    let first = dispatch(&mut ctx, &mut mem, SYS_GETPGRP, [0; 6]);
    for _ in 0..2 {
        for nr in [SYS_GETPGRP, SYS_GETPGID, SYS_GETSID, SYS_GETPID, SYS_GETTID] {
            assert_eq!(dispatch(&mut ctx, &mut mem, nr, [0; 6]), first);
        }
    }
    assert_eq!(ctx.fds.open_count(), fds_before);
    assert_eq!(ctx.signals.blocked(), blocked_before);
    assert!(ctx.signals.pending_set().contains(10));
}
