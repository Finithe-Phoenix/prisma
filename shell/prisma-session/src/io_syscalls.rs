//! Pointer-checked file-I/O syscalls.
//!
//! `write` reads the guest buffer through a [`GuestRegion`] (so a bad pointer is
//! a guest `EFAULT`, never an out-of-bounds host read), resolves the fd against
//! the [`FdTable`], and forwards the bytes to the host stream/file. This is the
//! first syscall that composes the memory keystone with the I/O keystone into
//! real host I/O.

use std::io::Write;

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
}
