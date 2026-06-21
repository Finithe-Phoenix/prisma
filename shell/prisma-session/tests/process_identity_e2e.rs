//! End-to-end checks for the process-identity and credential syscalls.
//!
//! These drive the public `dispatch` entry point (not the in-crate unit tests'
//! view) to validate the *composed* behaviour a real guest sees: the identity
//! syscalls answer consistently across repeated calls, the four credential
//! syscalls agree on one unprivileged id, and none of them disturbs the fd
//! table or the signal mask — they are pure reads of process state.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_GETPID: u64 = 39;
const SYS_GETUID: u64 = 102;
const SYS_GETGID: u64 = 104;
const SYS_GETEUID: u64 = 107;
const SYS_GETEGID: u64 = 108;
const SYS_GETTID: u64 = 186;

const GUEST_UID: i64 = 1000;

fn region(buf: &[u8]) -> BackedAddressSpace {
    let mut s = BackedAddressSpace::new();
    s.map_with_bytes(0x1000, buf, Protection::ReadWrite)
        .unwrap();
    s
}

#[test]
fn identity_syscalls_are_stable_and_consistent() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);

    let pid = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert!(pid > 0, "pid is the live host pid");
    // A single-threaded guest's tid equals its pid, and both are stable across
    // repeated calls (the kernel guarantees identity does not drift).
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETTID, [0; 6]), pid);
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]), pid);
}

#[test]
fn the_four_credentials_agree_on_one_unprivileged_id() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);
    // Real and effective uid/gid all coincide: the guest is one fixed user.
    for nr in [SYS_GETUID, SYS_GETEUID, SYS_GETGID, SYS_GETEGID] {
        assert_eq!(dispatch(&mut ctx, &mut mem, nr, [0; 6]), GUEST_UID);
    }
}

#[test]
fn identity_syscalls_do_not_disturb_fd_or_signal_state() {
    let mut ctx = SyscallContext::new();
    // Establish a known starting state: stdio open, signal 11 raised.
    let fds_before = ctx.fds.open_count();
    ctx.signals.raise(11);
    let blocked_before = ctx.signals.blocked();

    let buf = [0u8; 8];
    let mut mem = region(&buf);
    // Run every identity/credential syscall — all pure reads of process state.
    for nr in [
        SYS_GETPID,
        SYS_GETTID,
        SYS_GETUID,
        SYS_GETEUID,
        SYS_GETGID,
        SYS_GETEGID,
    ] {
        let _ = dispatch(&mut ctx, &mut mem, nr, [0; 6]);
    }

    // Neither the fd table nor the signal mask moved, and the raised signal is
    // still pending: the identity syscalls touched no shared state.
    assert_eq!(ctx.fds.open_count(), fds_before);
    assert_eq!(ctx.signals.blocked(), blocked_before);
    assert!(ctx.signals.pending_set().contains(11));
}

#[test]
fn identity_syscalls_ignore_their_argument_registers() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);
    // Garbage in every argument register must not change the answer: these
    // syscalls take no arguments, so the result is argument-independent.
    let pid = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_GETPID, [u64::MAX; 6]), pid);
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_GETUID, [0xDEAD, 0xBEEF, 1, 2, 3, 4]),
        GUEST_UID
    );
}
