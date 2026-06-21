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

impl FdEntry {
    /// Duplicate this entry, as `dup`/`dup2` do: the standard streams clone as
    /// markers; a `File` duplicates its host descriptor (`File::try_clone`,
    /// which is a `dup` at the OS level — the copy is independently closeable).
    ///
    /// # Errors
    /// The host `dup` of a `File` can fail (e.g. fd-table exhaustion).
    fn try_clone(&self) -> std::io::Result<Self> {
        Ok(match self {
            Self::Stdin => Self::Stdin,
            Self::Stdout => Self::Stdout,
            Self::Stderr => Self::Stderr,
            Self::File(f) => Self::File(f.try_clone()?),
        })
    }
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

    /// `dup(oldfd)`: duplicate `oldfd` onto the lowest free fd, returning the new
    /// fd. `None` for an unopen `oldfd` (guest `EBADF`), a failed host `dup`, or
    /// no free fd in range.
    pub fn dup(&mut self, oldfd: i32) -> Option<i32> {
        let entry = self.get(oldfd)?.try_clone().ok()?;
        self.allocate(entry)
    }

    /// `dup2(oldfd, newfd)`: duplicate `oldfd` onto exactly `newfd`, closing
    /// whatever `newfd` held first. `dup2(fd, fd)` is a no-op that succeeds iff
    /// `fd` is open. Returns `false` for an unopen `oldfd`, a failed host `dup`,
    /// or a `newfd` outside the representable range (guest `EBADF`).
    pub fn dup2(&mut self, oldfd: i32, newfd: i32) -> bool {
        if oldfd == newfd {
            return self.is_open(oldfd);
        }
        let Some(entry) = self.get(oldfd).and_then(|e| e.try_clone().ok()) else {
            return false;
        };
        let Ok(idx) = usize::try_from(newfd) else {
            return false;
        };
        if idx >= self.entries.len() {
            self.entries.resize_with(idx + 1, || None);
        }
        self.entries[idx] = Some(entry); // drops any prior newfd entry (closes it)
        true
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

    #[test]
    fn dup_duplicates_onto_the_lowest_free_fd() {
        let mut t = FdTable::new();
        // dup(1) -> fd 3, also referring to stdout.
        let new = t.dup(1).expect("dup stdout");
        assert_eq!(new, 3);
        assert!(matches!(t.get(new), Some(FdEntry::Stdout)));
        assert!(t.is_open(1)); // the original stays open
                               // dup of an unopen fd is EBADF.
        assert!(t.dup(7).is_none());
    }

    #[test]
    fn dup2_targets_an_exact_fd_and_grows_the_table() {
        let mut t = FdTable::new();
        // dup2(1, 5): fd 5 becomes stdout; the table grows, fds 3/4 stay free.
        assert!(t.dup2(1, 5));
        assert!(matches!(t.get(5), Some(FdEntry::Stdout)));
        assert!(t.get(3).is_none() && t.get(4).is_none());
        // dup2(fd, fd) on an open fd is a successful no-op.
        assert!(t.dup2(1, 1));
        // dup2 from an unopen source fails.
        assert!(!t.dup2(9, 6));
    }

    #[test]
    fn dup2_closes_the_existing_target_first() {
        let mut t = FdTable::new();
        // 0/1/2 open -> 3 open. dup2(1, 0) overwrites stdin with stdout; the
        // count is unchanged (one closed, one installed).
        assert_eq!(t.open_count(), 3);
        assert!(t.dup2(1, 0));
        assert!(matches!(t.get(0), Some(FdEntry::Stdout)));
        assert_eq!(t.open_count(), 3);
    }

    #[test]
    fn dup_of_a_file_fd_duplicates_the_host_descriptor() {
        // A real File-backed fd: dup it, then close the original; the dup must
        // still be usable (independent host descriptor).
        let path = std::env::temp_dir().join(format!("prisma_dup_{}.tmp", std::process::id()));
        let mut t = FdTable::new();
        {
            let file = std::fs::File::create(&path).expect("create temp");
            let fd = t.allocate(FdEntry::File(file)).expect("allocate");
            let dup = t.dup(fd).expect("dup the file");
            assert!(matches!(t.get(dup), Some(FdEntry::File(_))));
            // Closing the original leaves the dup open (separate host fd).
            assert!(t.close(fd));
            assert!(t.is_open(dup));
            assert!(t.close(dup));
        }
        let _ = std::fs::remove_file(&path);
    }
}
