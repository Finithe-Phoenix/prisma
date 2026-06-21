//! Guest file-descriptor table.
//!
//! Maps the small integer fds a guest holds to host resources. Descriptors
//! 0/1/2 are the standard streams; `open` allocates the lowest free fd (POSIX).
//! Closing an fd drops its host resource — a `File`'s host fd is released the
//! moment its slot is cleared, and the whole table closes on drop — so no host
//! descriptor outlives the guest fd or survives a restart, per the
//! resource-discipline clause.

use std::fs::File;

/// What a guest fd refers to on the host.
#[derive(Debug)]
pub enum FdEntry {
    /// Standard input.
    Stdin,
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// An opened host file; its descriptor is closed when this drops.
    File(File),
}

/// A guest file-descriptor table: fd (a small `i32`) → host resource.
#[derive(Debug, Default)]
pub struct FdTable {
    /// Slot `i` holds the entry for fd `i`, or `None` if that fd is free.
    entries: Vec<Option<FdEntry>>,
}

impl FdTable {
    /// A table with the three standard streams installed at fds 0, 1, 2.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: vec![
                Some(FdEntry::Stdin),
                Some(FdEntry::Stdout),
                Some(FdEntry::Stderr),
            ],
        }
    }

    /// Install `entry` at the lowest free fd (POSIX `open` semantics) and return
    /// that fd. `None` (guest `EMFILE`) if no fd within `i32` range is free —
    /// in which case the table is left unchanged.
    pub fn allocate(&mut self, entry: FdEntry) -> Option<i32> {
        let idx = self
            .entries
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.entries.len());
        let fd = i32::try_from(idx).ok()?;
        if idx == self.entries.len() {
            self.entries.push(Some(entry));
        } else {
            self.entries[idx] = Some(entry);
        }
        Some(fd)
    }

    /// The entry for `fd`, or `None` if the fd is not open.
    #[must_use]
    pub fn get(&self, fd: i32) -> Option<&FdEntry> {
        usize::try_from(fd)
            .ok()
            .and_then(|i| self.entries.get(i))
            .and_then(Option::as_ref)
    }

    /// Whether `fd` is currently open.
    #[must_use]
    pub fn is_open(&self, fd: i32) -> bool {
        self.get(fd).is_some()
    }

    /// Close `fd`, dropping its host resource. Returns `false` (guest `EBADF`)
    /// if it was not open. A standard stream can be closed like any other fd.
    pub fn close(&mut self, fd: i32) -> bool {
        let slot = usize::try_from(fd)
            .ok()
            .and_then(|i| self.entries.get_mut(i));
        match slot {
            Some(entry @ Some(_)) => {
                *entry = None; // drops the FdEntry — a File's host fd closes here
                true
            }
            _ => false,
        }
    }

    /// Number of currently-open fds.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::{FdEntry, FdTable};

    #[test]
    fn new_table_has_the_three_standard_streams() {
        let t = FdTable::new();
        assert!(matches!(t.get(0), Some(FdEntry::Stdin)));
        assert!(matches!(t.get(1), Some(FdEntry::Stdout)));
        assert!(matches!(t.get(2), Some(FdEntry::Stderr)));
        assert_eq!(t.open_count(), 3);
        assert!(t.get(3).is_none());
    }

    #[test]
    fn allocate_uses_the_lowest_free_fd() {
        let mut t = FdTable::new();
        // No holes yet: the next fd is 3.
        assert_eq!(t.allocate(FdEntry::Stderr), Some(3));
        // Free fd 1, then allocate — it must reuse 1, not 4 (POSIX lowest-free).
        assert!(t.close(1));
        assert_eq!(t.allocate(FdEntry::Stdout), Some(1));
        assert_eq!(t.allocate(FdEntry::Stderr), Some(4));
    }

    #[test]
    fn close_frees_the_fd_and_reports_ebadf_for_unopen() {
        let mut t = FdTable::new();
        assert!(t.close(1));
        assert!(!t.is_open(1));
        assert_eq!(t.open_count(), 2);
        // Closing an already-closed or never-opened fd is EBADF.
        assert!(!t.close(1));
        assert!(!t.close(99));
        assert!(!t.close(-1));
    }

    #[test]
    fn negative_and_oversized_fds_resolve_to_nothing() {
        let t = FdTable::new();
        assert!(t.get(-1).is_none());
        assert!(!t.is_open(-5));
    }
}
