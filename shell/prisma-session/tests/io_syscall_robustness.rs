//! Property-fuzz coverage for the file-I/O syscalls.
//!
//! A guest drives `write` / `read` / `close` / `dup` / `dup2` / `fsync` /
//! `fdatasync` / `lseek` with untrusted fds, pointers, counts, and offsets. The
//! invariants under test:
//!
//! * no input ever panics — a bad pointer is a guest `Fault`, an unopen fd is
//!   `BadFd`, never an out-of-bounds host access or an index panic;
//! * `write`/`read` validate the guest buffer and the fd before any host I/O, so
//!   an unopen fd never reaches a host stream (the harness keeps fds in the
//!   unopen range, so the fuzz never emits to stdout nor blocks on stdin);
//! * the fd-table bookkeeping (`dup`/`dup2`/`close`) stays internally consistent
//!   under arbitrary fd arguments.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::fd_table::FdTable;
use prisma_session::io_syscalls::{
    close, dup, dup2, fdatasync, fsync, lseek, read, write, IoError,
};
use proptest::prelude::*;

const BASE: u64 = 0x1000;
const LEN: usize = 64;

// fds at or above this are never open in a fresh table, so write/read on them
// fault on the fd lookup *before* touching a host stream — no stdout emission,
// no stdin block.
const UNOPEN: i32 = 3;

proptest! {
    /// `write` to an unopen fd never panics and never performs host I/O: it is
    /// either `Fault` (bad buffer, caught first) or `BadFd` (the fd lookup).
    #[test]
    fn write_to_unopen_fd_never_panics(
        fd in UNOPEN..i32::MAX,
        buf in any::<u64>(),
        count in 0usize..256,
    ) {
        let fds = FdTable::new();
        let mut backing = [0u8; LEN];
        let mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut backing);
        match write(&fds, &mem, fd, buf, count) {
            Err(IoError::Fault(_) | IoError::BadFd) => {}
            other => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `read` from an unopen fd never panics and never blocks on stdin: a bad
    /// buffer faults, otherwise the unopen fd is `BadFd` — input is never read.
    #[test]
    fn read_from_unopen_fd_never_panics(
        fd in UNOPEN..i32::MAX,
        buf in any::<u64>(),
        count in 0usize..256,
    ) {
        let fds = FdTable::new();
        let mut backing = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut backing);
        match read(&fds, &mut mem, fd, buf, count) {
            Err(IoError::Fault(_) | IoError::BadFd) => {}
            other => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `lseek` over arbitrary fds, offsets, and whence values never panics; the
    /// standard streams are not seekable (`BadFd`) and an unknown whence is
    /// `Invalid`, checked the same way regardless of fd.
    #[test]
    fn lseek_never_panics(fd in any::<i32>(), offset in any::<i64>(), whence in any::<i32>()) {
        let fds = FdTable::new();
        match lseek(&fds, fd, offset, whence) {
            Ok(_) | Err(IoError::BadFd | IoError::Invalid) => {}
            other => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// `fsync`/`fdatasync` over arbitrary fds never panic: an open stream is a
    /// successful no-op, an unopen fd is `BadFd`.
    #[test]
    fn fsync_family_never_panics(fd in any::<i32>()) {
        let fds = FdTable::new();
        for r in [fsync(&fds, fd), fdatasync(&fds, fd)] {
            match r {
                Ok(()) | Err(IoError::BadFd) => {}
                other => prop_assert!(false, "unexpected {other:?}"),
            }
        }
    }

    /// `close`/`dup`/`dup2` over arbitrary fds never panic and keep the table
    /// consistent: a successful `dup` raises the open count by one and yields a
    /// genuinely open fd; `close` of that fd lowers it back.
    #[test]
    fn dup_close_keep_the_table_consistent(oldfd in any::<i32>(), newfd in any::<i32>()) {
        let mut fds = FdTable::new();
        let before = fds.open_count();
        match dup(&mut fds, oldfd) {
            Ok(nfd) => {
                prop_assert!(fds.is_open(nfd));
                prop_assert_eq!(fds.open_count(), before + 1);
                // Closing the fresh fd restores the count.
                prop_assert!(close(&mut fds, nfd).is_ok());
                prop_assert_eq!(fds.open_count(), before);
            }
            Err(IoError::BadFd) => prop_assert_eq!(fds.open_count(), before),
            other => prop_assert!(false, "unexpected {other:?}"),
        }
        // dup2 with untrusted fds must also never panic.
        match dup2(&mut fds, oldfd, newfd) {
            Ok(_) | Err(IoError::BadFd | IoError::Invalid) => {}
            other => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// A zero-count `write`/`read` is safe for *any* fd — including the standard
    /// streams — because no bytes move: it never emits, never blocks, never
    /// panics.
    #[test]
    fn zero_count_io_is_safe_for_any_fd(fd in any::<i32>()) {
        let fds = FdTable::new();
        let mut backing = [0u8; LEN];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut backing);
        // write(fd, buf, 0): an empty slice, so even stdout emits nothing.
        let _ = write(&fds, &mem, fd, BASE, 0);
        let _ = read(&fds, &mut mem, fd, BASE, 0);
        prop_assert!(true); // reaching here means neither call panicked
    }
}
