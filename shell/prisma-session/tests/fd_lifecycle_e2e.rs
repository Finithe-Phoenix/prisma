//! End-to-end fd-table lifecycle through the dup family.
//!
//! Drive `dup` / `dup2` / `dup3` / `close` through the public `dispatch` entry
//! point to validate the composed fd-table behaviour a real guest relies on:
//! `dup` returns the lowest free descriptor, `dup2`/`dup3` place onto a chosen
//! one, `dup3` enforces its stricter argument rules, every open is balanced by a
//! `close` that releases it (the resource-discipline theme), and a freed
//! descriptor is reused by the next allocation.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_CLOSE: u64 = 3;
const SYS_DUP: u64 = 32;
const SYS_DUP2: u64 = 33;
const SYS_DUP3: u64 = 292;

const O_CLOEXEC: u64 = 0o2_000_000;

fn region(buf: &[u8]) -> BackedAddressSpace {
    let mut s = BackedAddressSpace::new();
    s.map_with_bytes(0x1000, buf, Protection::ReadWrite)
        .unwrap();
    s
}

#[test]
fn dup_family_allocates_places_and_releases() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);
    let start = ctx.fds.open_count();

    // dup(stdout) takes the lowest free fd (3, after stdio 0/1/2).
    let d = dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]);
    assert_eq!(d, 3);
    // dup2 and dup3 place onto explicitly chosen descriptors.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP2, [1, 10, 0, 0, 0, 0]),
        10
    );
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 11, O_CLOEXEC, 0, 0, 0]),
        11
    );
    assert_eq!(ctx.fds.open_count(), start + 3);
    assert!(ctx.fds.is_open(3) && ctx.fds.is_open(10) && ctx.fds.is_open(11));

    // Each open is balanced by a close that releases the descriptor.
    for fd in [3, 10, 11] {
        assert_eq!(
            dispatch(&mut ctx, &mut mem, SYS_CLOSE, [fd, 0, 0, 0, 0, 0]),
            0
        );
    }
    assert_eq!(ctx.fds.open_count(), start);
    // Closing an already-closed fd is -EBADF.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_CLOSE, [10, 0, 0, 0, 0, 0]),
        -9
    );
}

#[test]
fn a_freed_descriptor_is_reused_by_the_next_dup() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);

    // Take fd 3, free it, then the next dup must hand 3 back (lowest free).
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]), 3);
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_CLOSE, [3, 0, 0, 0, 0, 0]),
        0
    );
    assert!(!ctx.fds.is_open(3));
    assert_eq!(dispatch(&mut ctx, &mut mem, SYS_DUP, [1, 0, 0, 0, 0, 0]), 3);
    assert!(ctx.fds.is_open(3));
}

#[test]
fn dup3_enforces_its_stricter_rules_in_a_live_table() {
    let mut ctx = SyscallContext::new();
    let buf = [0u8; 8];
    let mut mem = region(&buf);
    let start = ctx.fds.open_count();

    // dup3 rejects oldfd == newfd (where dup2 would silently succeed)...
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 1, 0, 0, 0, 0]),
        -22
    );
    // ...and any flag bit other than O_CLOEXEC.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP3, [1, 9, 0x4, 0, 0, 0]),
        -22
    );
    // Neither rejected call touched the table.
    assert_eq!(ctx.fds.open_count(), start);
    // dup2 onto the same fd, by contrast, is allowed and returns it unchanged.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_DUP2, [1, 1, 0, 0, 0, 0]),
        1
    );
}
