//! Pointer-checked file-I/O syscalls.
//!
//! `write` reads the guest buffer through a [`GuestRegion`] (so a bad pointer is
//! a guest `EFAULT`, never an out-of-bounds host read), resolves the fd against
//! the [`FdTable`], and forwards the bytes to the host stream/file. This is the
//! first syscall that composes the memory keystone with the I/O keystone into
//! real host I/O.

use std::io::{Read, Write};

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::fd_table::{FdEntry, FdTable};

/// Why a file-I/O syscall failed (each maps to a guest errno).
#[derive(Debug)]
pub enum IoError {
    /// The fd is not open, or not valid for the operation — guest `EBADF`.
    BadFd,
    /// The guest buffer pointer is not accessible — guest `EFAULT`.
    Fault(RangeError),
    /// The host write failed — surfaced as the guest errno by the caller.
    Host(std::io::Error),
}

/// `write(fd, buf, count)`: copy `count` bytes from the guest buffer at `buf` to
/// the host resource behind `fd`, returning the number of bytes written.
///
/// The guest bytes are range-checked before the host reads them; `fd` 0 (stdin)
/// is not writable (`EBADF`).
///
/// # Errors
/// [`IoError::Fault`] if `buf`/`count` is not readable guest memory,
/// [`IoError::BadFd`] for an unopen fd or a write to stdin, [`IoError::Host`] if
/// the underlying host write fails.
pub fn write(
    fds: &FdTable,
    mem: &GuestRegion,
    fd: i32,
    buf: u64,
    count: usize,
) -> Result<usize, IoError> {
    let bytes = mem.read(buf, count).map_err(IoError::Fault)?;
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::Stdin => Err(IoError::BadFd),
        FdEntry::Stdout => {
            std::io::stdout().write_all(bytes).map_err(IoError::Host)?;
            Ok(bytes.len())
        }
        FdEntry::Stderr => {
            std::io::stderr().write_all(bytes).map_err(IoError::Host)?;
            Ok(bytes.len())
        }
        FdEntry::File(file) => {
            // `&File` implements `Write`, so no mutable fd handle is needed.
            let mut handle: &std::fs::File = file;
            handle.write_all(bytes).map_err(IoError::Host)?;
            Ok(bytes.len())
        }
    }
}

/// `read(fd, buf, count)`: read up to `count` bytes from `fd` into the guest
/// buffer at `buf`, returning the number actually read (0 at EOF).
///
/// The guest buffer is checked writable *before* any bytes are pulled off the
/// fd, so a bad pointer faults (`EFAULT`) without consuming input. Reading from
/// a write-only stream (stdout/stderr) is `EBADF`.
///
/// # Errors
/// [`IoError::Fault`] if `buf`/`count` is not writable guest memory,
/// [`IoError::BadFd`] for an unopen fd or a read from stdout/stderr,
/// [`IoError::Host`] if the underlying host read fails.
pub fn read(
    fds: &FdTable,
    mem: &mut GuestRegion,
    fd: i32,
    buf: u64,
    count: usize,
) -> Result<usize, IoError> {
    // Validate the destination before consuming the fd, so a bad buffer cannot
    // discard already-read bytes.
    mem.ensure_writable(buf, count).map_err(IoError::Fault)?;
    let mut host = vec![0u8; count];
    let n = match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::Stdin => std::io::stdin().read(&mut host).map_err(IoError::Host)?,
        FdEntry::Stdout | FdEntry::Stderr => return Err(IoError::BadFd),
        FdEntry::File(file) => {
            // `&File` implements `Read`, so no mutable fd handle is needed.
            let mut handle: &std::fs::File = file;
            handle.read(&mut host).map_err(IoError::Host)?
        }
    };
    mem.write(buf, &host[..n]).map_err(IoError::Fault)?;
    Ok(n)
}

/// `close(fd)`: close `fd`, releasing its host resource (a `File`'s descriptor
/// is closed deterministically here, per the resource-discipline clause).
///
/// # Errors
/// [`IoError::BadFd`] if `fd` was not open.
pub fn close(fds: &mut FdTable, fd: i32) -> Result<(), IoError> {
    if fds.close(fd) {
        Ok(())
    } else {
        Err(IoError::BadFd)
    }
}

/// `dup(oldfd)`: duplicate `oldfd` onto the lowest free fd, returning the new fd.
///
/// # Errors
/// [`IoError::BadFd`] if `oldfd` is not open, the host `dup` fails, or no fd is
/// free.
pub fn dup(fds: &mut FdTable, oldfd: i32) -> Result<i32, IoError> {
    fds.dup(oldfd).ok_or(IoError::BadFd)
}

/// `dup2(oldfd, newfd)`: duplicate `oldfd` onto exactly `newfd` (closing
/// whatever it held), returning `newfd`.
///
/// # Errors
/// [`IoError::BadFd`] if `oldfd` is not open, the host `dup` fails, or `newfd`
/// is out of range.
pub fn dup2(fds: &mut FdTable, oldfd: i32, newfd: i32) -> Result<i32, IoError> {
    if fds.dup2(oldfd, newfd) {
        Ok(newfd)
    } else {
        Err(IoError::BadFd)
    }
}

/// `fsync(fd)`: flush a file's data and metadata to the host disk. The standard
/// streams have nothing to durably sync, so it is a no-op for them (rather than
/// the kernel's `EINVAL` — a benign relaxation a guest can only over-rely on).
///
/// # Errors
/// [`IoError::BadFd`] if `fd` is not open, [`IoError::Host`] if the host sync
/// fails.
pub fn fsync(fds: &FdTable, fd: i32) -> Result<(), IoError> {
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => file.sync_all().map_err(IoError::Host),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Ok(()),
    }
}

/// `fdatasync(fd)`: flush a file's data (not necessarily its metadata) to the
/// host disk. Like [`fsync`] but via `File::sync_data`; a no-op for the standard
/// streams.
///
/// # Errors
/// [`IoError::BadFd`] if `fd` is not open, [`IoError::Host`] if the host sync
/// fails.
pub fn fdatasync(fds: &FdTable, fd: i32) -> Result<(), IoError> {
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => file.sync_data().map_err(IoError::Host),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{write, IoError};
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::fd_table::{FdEntry, FdTable};

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn write_to_stdout_reports_the_byte_count() {
        // The test harness captures stdout, so this produces no visible output.
        let fds = FdTable::new();
        let mut buf = *b"hello!!!";
        let mem = region(&mut buf);
        assert_eq!(write(&fds, &mem, 1, 0x1000, 5).unwrap(), 5);
    }

    #[test]
    fn write_to_stdin_is_ebadf() {
        let fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mem = region(&mut buf);
        assert!(matches!(
            write(&fds, &mem, 0, 0x1000, 1),
            Err(IoError::BadFd)
        ));
    }

    #[test]
    fn write_to_an_unopen_fd_is_ebadf() {
        let fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mem = region(&mut buf);
        assert!(matches!(
            write(&fds, &mem, 7, 0x1000, 1),
            Err(IoError::BadFd)
        ));
    }

    #[test]
    fn an_out_of_range_buffer_is_efault() {
        let fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mem = region(&mut buf);
        // 9 bytes from an 8-byte region runs off the end.
        assert!(matches!(
            write(&fds, &mem, 1, 0x1000, 9),
            Err(IoError::Fault(RangeError::Unmapped))
        ));
    }

    #[test]
    fn write_to_a_file_fd_persists_the_bytes() {
        // A real File-backed fd: write the guest bytes, then read the file back.
        let path = std::env::temp_dir().join(format!("prisma_write_{}.tmp", std::process::id()));
        let mut fds = FdTable::new();
        {
            let file = std::fs::File::create(&path).expect("create temp");
            let fd = fds.allocate(FdEntry::File(file)).expect("allocate fd");
            let mut buf = *b"diskdata";
            let mem = region(&mut buf);
            assert_eq!(write(&fds, &mem, fd, 0x1000, 8).unwrap(), 8);
            // Close the fd -> drops the File -> flushes/closes the host fd.
            assert!(fds.close(fd));
        }
        let read_back = std::fs::read(&path).expect("read temp");
        assert_eq!(&read_back, b"diskdata");
        // Resource cleanup: remove the temp file.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_from_a_file_fd_fills_the_guest_buffer() {
        use super::read;
        let path = std::env::temp_dir().join(format!("prisma_read_{}.tmp", std::process::id()));
        std::fs::write(&path, b"hello world").expect("seed temp");
        let mut fds = FdTable::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        {
            let file = std::fs::File::open(&path).expect("open temp");
            let fd = fds.allocate(FdEntry::File(file)).expect("allocate");
            let n = read(&fds, &mut mem, fd, 0x1000, 5).expect("read ok");
            assert_eq!(n, 5);
            assert!(fds.close(fd));
        }
        assert_eq!(mem.read(0x1000, 5).unwrap(), b"hello");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_from_a_write_only_stream_is_ebadf() {
        use super::read;
        let fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // stdout (1) / stderr (2) are write-only for the guest.
        assert!(matches!(
            read(&fds, &mut mem, 1, 0x1000, 1),
            Err(IoError::BadFd)
        ));
        assert!(matches!(
            read(&fds, &mut mem, 2, 0x1000, 1),
            Err(IoError::BadFd)
        ));
    }

    #[test]
    fn read_into_an_out_of_range_buffer_is_efault_without_consuming() {
        use super::read;
        let path = std::env::temp_dir().join(format!("prisma_readf_{}.tmp", std::process::id()));
        std::fs::write(&path, b"data").expect("seed");
        let mut fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        {
            let file = std::fs::File::open(&path).expect("open");
            let fd = fds.allocate(FdEntry::File(file)).expect("allocate");
            // 9 bytes into an 8-byte region: rejected before the file is read.
            assert!(matches!(
                read(&fds, &mut mem, fd, 0x1000, 9),
                Err(IoError::Fault(RangeError::Unmapped))
            ));
            assert!(fds.close(fd));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn close_releases_the_fd_and_reports_ebadf_for_unopen() {
        use super::close;
        let mut fds = FdTable::new();
        assert!(close(&mut fds, 1).is_ok()); // stdout closes
        assert!(!fds.is_open(1));
        // Closing it again (or an unopen fd) is EBADF.
        assert!(matches!(close(&mut fds, 1), Err(IoError::BadFd)));
        assert!(matches!(close(&mut fds, 99), Err(IoError::BadFd)));
    }

    #[test]
    fn dup_and_dup2_route_to_the_fd_table() {
        use super::{dup, dup2};
        let mut fds = FdTable::new();
        // dup(1) -> the lowest free fd (3).
        assert_eq!(dup(&mut fds, 1).unwrap(), 3);
        // dup2(1, 7) -> 7, growing the table.
        assert_eq!(dup2(&mut fds, 1, 7).unwrap(), 7);
        assert!(fds.is_open(7));
        // dup/dup2 from an unopen source is EBADF.
        assert!(matches!(dup(&mut fds, 50), Err(IoError::BadFd)));
        assert!(matches!(dup2(&mut fds, 50, 8), Err(IoError::BadFd)));
    }

    #[test]
    fn fsync_and_fdatasync_flush_a_file_and_noop_streams() {
        use super::{fdatasync, fsync};
        let path = std::env::temp_dir().join(format!("prisma_fsync_{}.tmp", std::process::id()));
        let mut fds = FdTable::new();
        {
            let file = std::fs::File::create(&path).expect("create temp");
            let fd = fds.allocate(FdEntry::File(file)).expect("allocate");
            assert!(fsync(&fds, fd).is_ok());
            assert!(fdatasync(&fds, fd).is_ok());
            assert!(fds.close(fd));
        }
        // The standard streams are a successful no-op.
        assert!(fsync(&fds, 1).is_ok());
        assert!(fdatasync(&fds, 2).is_ok());
        // An unopen fd is EBADF.
        assert!(matches!(fsync(&fds, 77), Err(IoError::BadFd)));
        assert!(matches!(fdatasync(&fds, 77), Err(IoError::BadFd)));
        let _ = std::fs::remove_file(&path);
    }
}
