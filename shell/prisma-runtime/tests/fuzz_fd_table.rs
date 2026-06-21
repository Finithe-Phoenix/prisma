//! Property-based robustness fuzzing for [`FdTable`] — the I/O keystone the
//! `read`/`write`/`close`/`dup` syscalls resolve guest fds through.
//!
//! A guest fd is an arbitrary untrusted `i32`, so the table must hold for every
//! value: never panic, allocate only ever returns a fresh open fd, and
//! dup/close keep the open-set consistent.

use proptest::prelude::*;

use prisma_runtime::fd_table::{FdEntry, FdTable};

proptest! {
    /// get / is_open / dup / dup2 / close never panic on an arbitrary fd.
    #[test]
    fn arbitrary_fd_ops_never_panic(fd in any::<i32>()) {
        let mut t = FdTable::new();
        let _ = t.get(fd);
        let _ = t.is_open(fd);
        let _ = t.dup(fd);
        let _ = t.dup2(fd, fd.wrapping_add(1));
        let _ = t.close(fd);
    }

    /// Every `allocate` returns a fresh, now-open fd, distinct from the ones
    /// already handed out, and the open count rises by exactly one each time.
    #[test]
    fn allocate_returns_a_fresh_open_fd(n in 1usize..64) {
        let mut t = FdTable::new();
        let base = t.open_count();
        let mut handed_out = Vec::new();
        for _ in 0..n {
            let fd = t.allocate(FdEntry::Stderr).expect("allocate within range");
            prop_assert!(t.is_open(fd));
            prop_assert!(!handed_out.contains(&fd), "allocate reused an open fd");
            handed_out.push(fd);
        }
        prop_assert_eq!(t.open_count(), base + n);
    }

    /// `dup2(1, target)` opens exactly `target`; closing it frees exactly it.
    #[test]
    fn dup2_then_close_is_consistent(target in 0i32..2048) {
        let mut t = FdTable::new();
        prop_assert!(t.dup2(1, target)); // stdout -> target
        prop_assert!(t.is_open(target));
        prop_assert!(t.close(target));
        prop_assert!(!t.is_open(target));
        // Closing it again (now unopen) is EBADF.
        prop_assert!(!t.close(target));
    }

    /// `dup` of an open fd yields a new, distinct, open fd; the original stays
    /// open.
    #[test]
    fn dup_yields_a_distinct_open_fd(_seed in 0u8..16) {
        let mut t = FdTable::new();
        let new = t.dup(1).expect("dup stdout");
        prop_assert!(new != 1);
        prop_assert!(t.is_open(new) && t.is_open(1));
    }
}
