//! End-to-end check that a long, interleaved syscall workload keeps each
//! subsystem's state independent.
//!
//! The per-family e2e tests each exercise one subsystem in isolation. This one
//! drives a realistic *mixed* sequence — fd I/O, signals, clocks, and identity
//! syscalls interleaved — through the public `dispatch` entry point, and proves
//! that `SyscallContext` partitions its state with no cross-talk: a signal
//! operation never perturbs the fd table, an fd operation never perturbs the
//! signal mask, and the clock keeps advancing throughout.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_runtime::guest_signal::Sigset;
use prisma_runtime::guest_structs::Timespec;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_WRITE: u64 = 1;
const SYS_CLOSE: u64 = 3;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_DUP: u64 = 32;
const SYS_DUP2: u64 = 33;
const SYS_GETPID: u64 = 39;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_RT_SIGPENDING: u64 = 127;

const SIG_SETMASK: u64 = 2;
const CLOCK_MONOTONIC: u64 = 1;

fn region(buf: &[u8]) -> BackedAddressSpace {
    let mut s = BackedAddressSpace::new();
    s.map_with_bytes(0x1000, buf, Protection::ReadWrite).unwrap();
    s
}

#[test]
fn interleaved_workload_keeps_subsystems_independent() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 64];
    let mut mem = region(&buf);

    // --- fd subsystem: dup stdout, dup2 it onto a high fd ---
    let fds_start = ctx.fds.open_count();
    let new_fd = dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]);
    assert!(new_fd >= 3);
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP2, [1, 20, 0, 0, 0, 0]),
        20
    );
    assert_eq!(ctx.fds.open_count(), fds_start + 2);

    // --- signal subsystem: block signal 10, raise + report pending ---
    let mut mask = Sigset::empty();
    mask.insert(10);
    buf[0..8].copy_from_slice(&mask.to_guest_bytes());
    let mut mem = region(&buf);
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
    ctx.signals.raise(12);

    // --- clock subsystem: first monotonic sample ---
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_MONOTONIC, 0x1010, 0, 0, 0, 0]
        ),
        0
    );
    let t0 = Timespec::from_guest_bytes(mem.read(0x1010, 16).unwrap()).unwrap();

    // --- identity syscall in the middle must perturb nothing ---
    let pid = dispatch(&mut ctx, &mut mem, SYS_GETPID, [0; 6]);
    assert!(pid > 0);

    // --- fd op (close the dup2'd fd) must not touch the signal mask ---
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_CLOSE, [20, 0, 0, 0, 0, 0]),
        0
    );
    assert!(
        ctx.signals.blocked().contains(10),
        "fd op perturbed the signal mask"
    );
    assert_eq!(ctx.fds.open_count(), fds_start + 1);

    // --- signal report (rt_sigpending) must not touch the fd table ---
    let fds_now = ctx.fds.open_count();
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPENDING,
            [0x1018, 0, 0, 0, 0, 0]
        ),
        0
    );
    let pending = Sigset::from_guest_bytes(mem.read(0x1018, 8).unwrap()).unwrap();
    assert!(pending.contains(12));
    assert_eq!(
        ctx.fds.open_count(),
        fds_now,
        "signal op perturbed the fd table"
    );

    // --- clock subsystem: second sample is not before the first ---
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_MONOTONIC, 0x1010, 0, 0, 0, 0]
        ),
        0
    );
    let t1 = Timespec::from_guest_bytes(mem.read(0x1010, 16).unwrap()).unwrap();
    assert!(
        (t1.sec, t1.nsec) >= (t0.sec, t0.nsec),
        "monotonic regressed across the workload"
    );

    // --- a final write still routes correctly after all the interleaving ---
    let msg = *b"done";
    let mut wbuf = [0u8; 8];
    wbuf[0..4].copy_from_slice(&msg);
    let mut wmem = region(&wbuf);
    assert_eq!(
        dispatch(&mut ctx, &mut wmem, SYS_WRITE, [1, 0x1000, 4, 0, 0, 0]),
        4
    );

    // Final state: stdio + the surviving dup are open; signal 10 still blocked.
    assert_eq!(ctx.fds.open_count(), fds_start + 1);
    assert!(ctx.signals.blocked().contains(10));
}
