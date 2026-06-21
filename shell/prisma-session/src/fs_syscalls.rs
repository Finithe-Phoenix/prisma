//! Filesystem-statistics syscalls (`statfs`/`fstatfs`).
//!
//! The session has no real filesystem, so both report a fixed synthetic
//! [`Statfs`] — plausible non-zero figures (so a guest reading free space sees a
//! sane mount) rather than zero. `fstatfs` still validates its fd. Pointers are
//! checked through a [`GuestRegion`].

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_mem::GuestMem;
use prisma_runtime::fd_table::FdTable;
use prisma_runtime::guest_structs::Statfs;

/// `TMPFS_MAGIC` — the filesystem type the session presents (an in-memory
/// filesystem, which matches the guest's RAM-backed address space).
const TMPFS_MAGIC: u64 = 0x0102_1994;
/// Block size reported (`f_bsize`/`f_frsize`).
const BLOCK_SIZE: u64 = 4096;
/// Total blocks reported (`f_blocks`): 0x40000 * 4 KiB = 1 GiB.
const TOTAL_BLOCKS: u64 = 0x4_0000;
/// Total inodes reported (`f_files`).
const TOTAL_FILES: u64 = 0x10_0000;
/// Maximum filename length (`f_namelen`).
const NAME_MAX: u64 = 255;

/// The fixed synthetic filesystem statistics both calls report (half the blocks
/// and inodes free).
fn synthetic_statfs() -> Statfs {
    Statfs {
        f_type: TMPFS_MAGIC,
        bsize: BLOCK_SIZE,
        blocks: TOTAL_BLOCKS,
        bfree: TOTAL_BLOCKS / 2,
        bavail: TOTAL_BLOCKS / 2,
        files: TOTAL_FILES,
        ffree: TOTAL_FILES / 2,
        fsid: 0,
        namelen: NAME_MAX,
        frsize: BLOCK_SIZE,
        flags: 0,
    }
}

/// `statfs(path, buf)`: write filesystem statistics for the mount containing
/// `path` to the guest `statfs` at `buf`. With no filesystem modelled, the same
/// synthetic stats are reported for any path (the path is not dereferenced).
///
/// # Errors
/// [`RangeError`] if `buf` is not writable guest memory (guest `EFAULT`).
pub fn statfs(mem: &mut impl GuestMem, buf: u64) -> Result<(), RangeError> {
    mem.write(buf, &synthetic_statfs().to_guest_bytes())
}

/// Why `fstatfs` failed (each maps to a guest errno at routing time).
#[derive(Debug)]
pub enum FstatfsError {
    /// `fd` is not open — guest `EBADF`.
    BadFd,
    /// `buf` is not writable guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `fstatfs(fd, buf)`: write filesystem statistics for the mount containing the
/// file behind `fd`. `fd` must be open; the stats are the same synthetic values.
///
/// # Errors
/// [`FstatfsError::BadFd`] if `fd` is not open, [`FstatfsError::Fault`] if `buf`
/// is not writable guest memory.
pub fn fstatfs(
    fds: &FdTable,
    mem: &mut impl GuestMem,
    fd: i32,
    buf: u64,
) -> Result<(), FstatfsError> {
    if !fds.is_open(fd) {
        return Err(FstatfsError::BadFd);
    }
    mem.write(buf, &synthetic_statfs().to_guest_bytes())
        .map_err(FstatfsError::Fault)
}

#[cfg(test)]
mod tests {
    use super::{fstatfs, statfs, FstatfsError};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::fd_table::FdTable;
    use prisma_runtime::guest_structs::Statfs;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn statfs_reports_a_nonzero_mount() {
        let mut buf = [0u8; Statfs::SIZE];
        let mut mem = region(&mut buf);
        statfs(&mut mem, 0x1000).expect("write ok");
        let out = mem.read(0x1000, Statfs::SIZE).unwrap();
        // f_bsize (word 1) is 4096; f_bavail (word 4) is non-zero free space.
        assert_eq!(&out[8..16], &4096u64.to_le_bytes());
        assert_ne!(&out[32..40], &0u64.to_le_bytes());
    }

    #[test]
    fn fstatfs_requires_an_open_fd() {
        let mut fds = FdTable::new();
        let mut buf = [0u8; Statfs::SIZE];
        let mut mem = region(&mut buf);
        // fd 1 (stdout) is open -> ok.
        assert!(fstatfs(&fds, &mut mem, 1, 0x1000).is_ok());
        // an unopened fd -> EBADF.
        assert!(fds.close(1));
        assert!(matches!(
            fstatfs(&fds, &mut mem, 1, 0x1000),
            Err(FstatfsError::BadFd)
        ));
    }

    #[test]
    fn statfs_faults_on_an_unwritable_pointer() {
        let mut buf = [0u8; 8]; // too small for the struct
        let mut mem = region(&mut buf);
        assert!(statfs(&mut mem, 0x1000).is_err());
    }
}
