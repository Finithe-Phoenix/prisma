//! End-to-end integration of the syscall layer: a realistic sequence driven
//! through `dispatch`, checking that the `SyscallContext` state persists across
//! calls and that pointer arguments produce the right guest-memory effects.
//!
//! The per-handler unit tests cover each syscall in isolation; this exercises
//! them composed — the entry point, the shared fd table / signal state, and the
//! `GuestRegion` together.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_runtime::guest_signal::Sigset;
use prisma_runtime::guest_structs::Timespec;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_WRITE: u64 = 1;
const SYS_DUP: u64 = 32;
const SYS_CLOSE: u64 = 3;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_RT_SIGPENDING: u64 = 127;
const CLOCK_REALTIME: u64 = 0;
const SIG_BLOCK: u64 = 0;

#[test]
fn an_io_and_time_sequence_persists_fd_state_and_writes_memory() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 64];
    buf[..5].copy_from_slice(b"hello");
    let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };

    // write(1, "hello", 5) -> 5 (stdout, captured by the harness).
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_WRITE, [1, 0x1000, 5, 0, 0, 0]),
        5
    );

    // clock_gettime(REALTIME, 0x1010) -> 0; a plausible wall-clock time lands.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_REALTIME, 0x1010, 0, 0, 0, 0]
        ),
        0
    );
    let ts = Timespec::from_guest_bytes(mem.read(0x1010, 16).unwrap()).unwrap();
    assert!(ts.sec > 1_600_000_000);

    // dup(1) -> 3; the fd table in the context persists across calls.
    let new_fd = dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]);
    assert_eq!(new_fd, 3);
    let fd_arg = u64::try_from(new_fd).unwrap();
    // close(3) -> 0; closing again -> -EBADF (-9): the state persisted.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_CLOSE, [fd_arg, 0, 0, 0, 0, 0]),
        0
    );
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_CLOSE, [fd_arg, 0, 0, 0, 0, 0]),
        -9
    );
}

#[test]
fn blocking_a_signal_then_querying_pending_round_trips_through_state() {
    let mut ctx = SyscallContext::new();
    // Raise signal 11 against the thread (pending).
    ctx.signals.raise(11);

    let mut buf = [0u8; 16];
    // A mask blocking signal 11 at guest 0x1000 for rt_sigprocmask(SIG_BLOCK).
    let mut block = Sigset::empty();
    block.insert(11);
    buf[0..8].copy_from_slice(&block.to_guest_bytes());
    let mut mem = { let mut s = BackedAddressSpace::new(); s.map_with_bytes(0x1000, &buf, Protection::ReadWrite).unwrap(); s };

    // rt_sigprocmask(SIG_BLOCK, set=0x1000, oldset=0) -> 0: the mask persists.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPROCMASK,
            [SIG_BLOCK, 0x1000, 0, 0, 0, 0]
        ),
        0
    );

    // rt_sigpending(0x1008) -> 0; the reported set includes the still-pending 11.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_RT_SIGPENDING,
            [0x1008, 0, 0, 0, 0, 0]
        ),
        0
    );
    let pending = Sigset::from_guest_bytes(mem.read(0x1008, 8).unwrap()).unwrap();
    assert!(pending.contains(11));
}
