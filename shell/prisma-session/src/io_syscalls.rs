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
use prisma_runtime::guest_structs::{Flock, Iovec, PollFd, Stat, Timespec};

/// `fcntl` commands we model.
const F_DUPFD: i32 = 0;
const F_DUPFD_CLOEXEC: i32 = 1030;
const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;
const F_GETLK: i32 = 5;
const F_SETLK: i32 = 6;
const F_SETLKW: i32 = 7;
/// `O_RDWR` — the access mode `F_GETFL` reports (the fd table does not track the
/// guest's open flags, so a read/write file is the honest default).
const O_RDWR: i64 = 2;
/// `F_UNLCK` — the `l_type` `F_GETLK` reports: no conflicting lock exists in the
/// single-process model.
const F_UNLCK: i16 = 2;

/// `poll` event bits we model: data-to-read, space-to-write, and the
/// invalid-fd error the kernel reports for a closed descriptor.
const POLLIN: i16 = 0x001;
const POLLOUT: i16 = 0x004;
const POLLNVAL: i16 = 0x020;

/// `S_IFREG` — the `st_mode` bit marking a regular file.
const S_IFREG: u32 = 0o100_000;
/// `S_IFCHR` — the `st_mode` bit marking a character device (the std streams).
const S_IFCHR: u32 = 0o020_000;
/// The uid/gid `fstat` reports — the single unprivileged guest user.
const GUEST_ID: u32 = 1000;

/// Kernel cap on the number of `iovec`s a single vectored call accepts
/// (`IOV_MAX` / `UIO_MAXIOV` on Linux). A larger count is `EINVAL`.
const IOV_MAX: usize = 1024;

/// Why a file-I/O syscall failed (each maps to a guest errno).
#[derive(Debug)]
pub enum IoError {
    /// The fd is not open, or not valid for the operation — guest `EBADF`.
    BadFd,
    /// An argument is out of range (e.g. an unknown `whence`) — guest `EINVAL`.
    Invalid,
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

/// `lseek(fd, offset, whence)`: reposition a file's offset, returning the new
/// absolute offset. `whence` is SEEK_SET (0), SEEK_CUR (1), or SEEK_END (2).
/// The standard streams are not seekable (`EBADF`).
///
/// # Errors
/// [`IoError::Invalid`] for an unknown `whence` or a negative SEEK_SET offset,
/// [`IoError::BadFd`] for an unopen fd or a non-seekable stream,
/// [`IoError::Host`] if the host seek fails.
pub fn lseek(fds: &FdTable, fd: i32, offset: i64, whence: i32) -> Result<u64, IoError> {
    use std::io::{Seek, SeekFrom};
    let from = match whence {
        0 => SeekFrom::Start(u64::try_from(offset).map_err(|_| IoError::Invalid)?),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Err(IoError::Invalid),
    };
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => {
            // `&File` implements `Seek`, so no mutable fd handle is needed.
            let mut handle: &std::fs::File = file;
            handle.seek(from).map_err(IoError::Host)
        }
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Err(IoError::BadFd),
    }
}

/// `fcntl(fd, cmd, arg)`: the descriptor-control multiplexer. The subset modelled
/// here returns the value the guest reads in `rax`:
///
/// * `F_GETFD`/`F_SETFD` — the close-on-exec flag is not tracked, so getting it
///   is `0` and setting it is an accepted no-op;
/// * `F_GETFL` — the open flags are not tracked, so a read/write file (`O_RDWR`)
///   is reported; `F_SETFL` is an accepted no-op;
/// * `F_GETLK` — reads the `flock` request at `arg`, reports `F_UNLCK` (no
///   conflicting lock can exist for the single-process guest), and writes it
///   back; `F_SETLK`/`F_SETLKW` always succeed for the same reason.
///
/// Any other command is `EINVAL`. `fd` must be open.
///
/// # Errors
/// [`IoError::BadFd`] for an unopen fd, [`IoError::Invalid`] for an unmodelled
/// command, [`IoError::Fault`] if a lock command's `arg` pointer is not
/// accessible guest memory.
pub fn fcntl(
    fds: &mut FdTable,
    mem: &mut GuestRegion,
    fd: i32,
    cmd: i32,
    arg: u64,
) -> Result<i64, IoError> {
    if !fds.is_open(fd) {
        return Err(IoError::BadFd);
    }
    match cmd {
        // F_DUPFD / F_DUPFD_CLOEXEC: duplicate fd onto the lowest free fd >= arg
        // (close-on-exec is not modelled, so the CLOEXEC variant is the same).
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let min_fd = arg as i32;
            if min_fd < 0 {
                return Err(IoError::Invalid);
            }
            fds.dup_from(fd, min_fd)
                .map(i64::from)
                .ok_or(IoError::BadFd)
        }
        F_GETFD => Ok(0),
        F_SETFD | F_SETFL => Ok(0),
        F_GETFL => Ok(O_RDWR),
        F_SETLK | F_SETLKW => Ok(0),
        F_GETLK => {
            let bytes = mem.read(arg, Flock::SIZE).map_err(IoError::Fault)?;
            let mut lk =
                Flock::from_guest_bytes(bytes).ok_or(IoError::Fault(RangeError::Unmapped))?;
            // No other process can hold a conflicting lock.
            lk.typ = F_UNLCK;
            mem.write(arg, &lk.to_guest_bytes())
                .map_err(IoError::Fault)?;
            Ok(0)
        }
        _ => Err(IoError::Invalid),
    }
}

/// A `SystemTime` as a guest `timespec` (seconds + nanos since the Unix epoch);
/// times before the epoch or that fail to read clamp to zero.
fn systime_to_timespec(t: std::io::Result<std::time::SystemTime>) -> Timespec {
    let dur = t
        .ok()
        .and_then(|st| st.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    Timespec {
        sec: i64::try_from(dur.as_secs()).unwrap_or(i64::MAX),
        nsec: i64::from(dur.subsec_nanos()),
    }
}

/// Map host file metadata to the guest `stat`. The host is not POSIX, so mode,
/// ownership and block accounting are synthesised: a regular file with mode
/// `0644`, the single guest user, and 512-byte blocks.
fn stat_from_metadata(m: &std::fs::Metadata) -> Stat {
    let size = i64::try_from(m.len()).unwrap_or(i64::MAX);
    Stat {
        dev: 0,
        ino: 0,
        nlink: 1,
        mode: S_IFREG | 0o644,
        uid: GUEST_ID,
        gid: GUEST_ID,
        rdev: 0,
        size,
        blksize: 512,
        blocks: (size + 511) / 512,
        atime: systime_to_timespec(m.accessed()),
        mtime: systime_to_timespec(m.modified()),
        ctime: systime_to_timespec(m.created()),
    }
}

/// The `stat` reported for a standard stream — a character device, no size.
fn stat_for_stream() -> Stat {
    Stat {
        dev: 0,
        ino: 0,
        nlink: 1,
        mode: S_IFCHR | 0o620,
        uid: GUEST_ID,
        gid: GUEST_ID,
        rdev: 0,
        size: 0,
        blksize: 1024,
        blocks: 0,
        atime: Timespec { sec: 0, nsec: 0 },
        mtime: Timespec { sec: 0, nsec: 0 },
        ctime: Timespec { sec: 0, nsec: 0 },
    }
}

/// `fstat(fd, statbuf)`: fill the guest `stat` at `statbuf` with the metadata of
/// the file (or character device) behind `fd`. The struct is written through the
/// range-checked [`GuestRegion`].
///
/// # Errors
/// [`IoError::BadFd`] for an unopen fd, [`IoError::Fault`] if `statbuf` is not
/// writable guest memory, [`IoError::Host`] if reading the host metadata fails.
pub fn fstat(fds: &FdTable, mem: &mut GuestRegion, fd: i32, statbuf: u64) -> Result<(), IoError> {
    let stat = match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => stat_from_metadata(&file.metadata().map_err(IoError::Host)?),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => stat_for_stream(),
    };
    mem.write(statbuf, &stat.to_guest_bytes())
        .map_err(IoError::Fault)
}

/// The events ready on `fd` from those `events` requested. In this model a
/// regular file is always ready to read and write, stdin is readable, and
/// stdout/stderr are writable; a closed fd is `POLLNVAL`. A negative fd is the
/// caller's job to skip (the kernel ignores it).
fn poll_revents(fds: &FdTable, fd: i32, events: i16) -> i16 {
    match fds.get(fd) {
        None => POLLNVAL,
        Some(FdEntry::File(_)) => events & (POLLIN | POLLOUT),
        Some(FdEntry::Stdin) => events & POLLIN,
        Some(FdEntry::Stdout | FdEntry::Stderr) => events & POLLOUT,
    }
}

/// `poll(fds, nfds, timeout)`: for each `pollfd` in the guest array, compute the
/// ready `revents`, write them back, and return how many entries are ready.
///
/// Readiness is immediate in this model — regular files and the standard streams
/// are always ready in their natural direction — so `timeout` never causes a
/// block (a real blocking wait needs pipe/socket support that does not exist
/// yet). A negative fd is skipped with `revents = 0`, per the kernel.
///
/// # Errors
/// [`IoError::Invalid`] if `nfds * sizeof(pollfd)` overflows, [`IoError::Fault`]
/// if the array is not accessible guest memory.
pub fn poll(
    fds: &FdTable,
    mem: &mut GuestRegion,
    array_ptr: u64,
    nfds: u32,
) -> Result<usize, IoError> {
    let count = nfds as usize;
    if count == 0 {
        // nfds == 0 never dereferences the array pointer (the poll(NULL, 0, ...)
        // idiom), so nothing is ready.
        return Ok(0);
    }
    let array_len = count.checked_mul(PollFd::SIZE).ok_or(IoError::Invalid)?;
    // Own a copy so the `revents` can be written back after the read borrow ends.
    let array = mem
        .read(array_ptr, array_len)
        .map_err(IoError::Fault)?
        .to_vec();

    let mut ready = 0usize;
    let mut out = Vec::with_capacity(array_len);
    for i in 0..count {
        let mut pfd = PollFd::from_guest_bytes(&array[i * PollFd::SIZE..])
            .ok_or(IoError::Fault(RangeError::Unmapped))?;
        pfd.revents = if pfd.fd < 0 {
            0
        } else {
            poll_revents(fds, pfd.fd, pfd.events)
        };
        if pfd.revents != 0 {
            ready += 1;
        }
        out.extend_from_slice(&pfd.to_guest_bytes());
    }
    mem.write(array_ptr, &out).map_err(IoError::Fault)?;
    Ok(ready)
}

/// `ppoll(fds, nfds, tmo, sigmask, sigsetsize)`: the modern `poll` that takes a
/// `timespec` timeout and an optional signal mask. Readiness is immediate in
/// this model, so the call never blocks; a non-null `tmo` pointer is still
/// range-checked (a bad one is `EFAULT`) and the atomic `sigmask` is not applied
/// (there is no blocking window for it to guard). Delegates the per-fd work to
/// [`poll`].
///
/// # Errors
/// [`IoError::Invalid`] for an `nfds` overflow, [`IoError::Fault`] if the `tmo`
/// timespec or the pollfd array is not accessible guest memory.
pub fn ppoll(
    fds: &FdTable,
    mem: &mut GuestRegion,
    array_ptr: u64,
    nfds: u32,
    tmo_ptr: u64,
) -> Result<usize, IoError> {
    if tmo_ptr != 0 {
        // Validate the timeout struct is readable, even though we do not block.
        mem.read(tmo_ptr, Timespec::SIZE).map_err(IoError::Fault)?;
    }
    poll(fds, mem, array_ptr, nfds)
}

/// `ftruncate(fd, length)`: set the size of the file behind `fd` to `length`
/// bytes — shrinking discards the tail, growing extends with zeros. A negative
/// length is `EINVAL`; the standard streams are not regular files (`EINVAL`).
///
/// # Errors
/// [`IoError::Invalid`] for a negative length or a non-file fd,
/// [`IoError::BadFd`] for an unopen fd, [`IoError::Host`] if the host resize
/// fails.
pub fn ftruncate(fds: &FdTable, fd: i32, length: i64) -> Result<(), IoError> {
    let len = u64::try_from(length).map_err(|_| IoError::Invalid)?;
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => file.set_len(len).map_err(IoError::Host),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Err(IoError::Invalid),
    }
}

/// Read into `buf` from `file` starting at byte `offset`, without relying on (or
/// preserving) the file's own cursor. Uses the platform's positional read.
#[cfg(unix)]
fn pread_file(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}
#[cfg(windows)]
fn pread_file(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

/// Write `bytes` to `file` starting at byte `offset`. Uses the platform's
/// positional write.
#[cfg(unix)]
fn pwrite_file(file: &std::fs::File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::write_at(file, bytes, offset)
}
#[cfg(windows)]
fn pwrite_file(file: &std::fs::File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_write(file, bytes, offset)
}

/// `pread(fd, buf, count, offset)`: read up to `count` bytes from `fd` at the
/// absolute `offset` into the guest buffer, without using the file's cursor.
/// Only regular files are positional; the standard streams are not seekable
/// (reported as `EINVAL` here, the closest of the routed errnos).
///
/// # Errors
/// [`IoError::Fault`] if `buf` is not writable guest memory, [`IoError::BadFd`]
/// for an unopen fd, [`IoError::Invalid`] for a non-seekable stream,
/// [`IoError::Host`] if the host read fails.
pub fn pread(
    fds: &FdTable,
    mem: &mut GuestRegion,
    fd: i32,
    buf: u64,
    count: usize,
    offset: u64,
) -> Result<usize, IoError> {
    mem.ensure_writable(buf, count).map_err(IoError::Fault)?;
    let mut host = vec![0u8; count];
    let n = match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => pread_file(file, &mut host, offset).map_err(IoError::Host)?,
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => return Err(IoError::Invalid),
    };
    mem.write(buf, &host[..n]).map_err(IoError::Fault)?;
    Ok(n)
}

/// `pwrite(fd, buf, count, offset)`: write `count` bytes from the guest buffer to
/// `fd` at the absolute `offset`, without using the file's cursor.
///
/// # Errors
/// [`IoError::Fault`] if `buf` is not readable guest memory, [`IoError::BadFd`]
/// for an unopen fd, [`IoError::Invalid`] for a non-seekable stream,
/// [`IoError::Host`] if the host write fails.
pub fn pwrite(
    fds: &FdTable,
    mem: &GuestRegion,
    fd: i32,
    buf: u64,
    count: usize,
    offset: u64,
) -> Result<usize, IoError> {
    let bytes = mem.read(buf, count).map_err(IoError::Fault)?;
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => pwrite_file(file, bytes, offset).map_err(IoError::Host),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Err(IoError::Invalid),
    }
}

/// Write `bytes` to the host resource behind `fd`, returning the count written.
/// Shared by `writev`; mirrors the fd handling of [`write`]. Stdin is not
/// writable (`EBADF`).
fn write_bytes_to_fd(fds: &FdTable, fd: i32, bytes: &[u8]) -> Result<usize, IoError> {
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
            let mut handle: &std::fs::File = file;
            handle.write_all(bytes).map_err(IoError::Host)?;
            Ok(bytes.len())
        }
    }
}

/// Read and validate `iovcnt` guest `iovec`s from the array at `iov_ptr`. The
/// array itself is range-checked through `mem`; a negative count or one past
/// `IOV_MAX` is `EINVAL`. The named buffers are *not* validated here — the
/// caller checks them in the direction it needs.
fn read_iovecs(mem: &GuestRegion, iov_ptr: u64, iovcnt: i32) -> Result<Vec<Iovec>, IoError> {
    let count = usize::try_from(iovcnt).map_err(|_| IoError::Invalid)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > IOV_MAX {
        return Err(IoError::Invalid);
    }
    let array_len = count.checked_mul(Iovec::SIZE).ok_or(IoError::Invalid)?;
    let array = mem.read(iov_ptr, array_len).map_err(IoError::Fault)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let iov = Iovec::from_guest_bytes(&array[i * Iovec::SIZE..])
            .ok_or(IoError::Fault(RangeError::Unmapped))?;
        out.push(iov);
    }
    Ok(out)
}

/// `writev(fd, iov, iovcnt)`: gather the bytes from the guest buffers named by
/// the `iovec` array (in order) and write them to `fd`, returning the total
/// written. Every source buffer is read through the range-checked [`GuestRegion`].
///
/// # Errors
/// [`IoError::Invalid`] for a bad `iovcnt`, [`IoError::Fault`] if the array or
/// any buffer is not readable guest memory, [`IoError::BadFd`] for an unopen fd
/// or stdin, [`IoError::Host`] on a host write failure.
pub fn writev(
    fds: &FdTable,
    mem: &GuestRegion,
    fd: i32,
    iov_ptr: u64,
    iovcnt: i32,
) -> Result<usize, IoError> {
    let iovs = read_iovecs(mem, iov_ptr, iovcnt)?;
    let mut gathered = Vec::new();
    for iov in &iovs {
        let len = usize::try_from(iov.len).map_err(|_| IoError::Invalid)?;
        let buf = mem.read(iov.base, len).map_err(IoError::Fault)?;
        gathered.extend_from_slice(buf);
    }
    write_bytes_to_fd(fds, fd, &gathered)
}

/// `readv(fd, iov, iovcnt)`: read from `fd` and scatter the bytes into the guest
/// buffers named by the `iovec` array (in order), returning the number read.
///
/// Every destination buffer is checked writable *before* the fd is read, so a
/// bad buffer faults (`EFAULT`) without consuming input. Reading from a
/// write-only stream (stdout/stderr) is `EBADF`.
///
/// # Errors
/// [`IoError::Invalid`] for a bad `iovcnt` / overflowing total, [`IoError::Fault`]
/// if the array or any buffer is not writable guest memory, [`IoError::BadFd`]
/// for an unopen fd or stdout/stderr, [`IoError::Host`] on a host read failure.
pub fn readv(
    fds: &FdTable,
    mem: &mut GuestRegion,
    fd: i32,
    iov_ptr: u64,
    iovcnt: i32,
) -> Result<usize, IoError> {
    let iovs = read_iovecs(mem, iov_ptr, iovcnt)?;
    // Validate every destination is writable and total up the capacity first.
    let mut total = 0usize;
    for iov in &iovs {
        let len = usize::try_from(iov.len).map_err(|_| IoError::Invalid)?;
        mem.ensure_writable(iov.base, len).map_err(IoError::Fault)?;
        total = total.checked_add(len).ok_or(IoError::Invalid)?;
    }
    // Pull up to `total` bytes off the fd into a host staging buffer.
    let mut host = vec![0u8; total];
    let nread = match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::Stdin => std::io::stdin().read(&mut host).map_err(IoError::Host)?,
        FdEntry::Stdout | FdEntry::Stderr => return Err(IoError::BadFd),
        FdEntry::File(file) => {
            let mut handle: &std::fs::File = file;
            handle.read(&mut host).map_err(IoError::Host)?
        }
    };
    // Scatter the bytes actually read across the buffers, in order.
    let mut off = 0usize;
    for iov in &iovs {
        if off >= nread {
            break;
        }
        let take = usize::try_from(iov.len).unwrap_or(0).min(nread - off);
        mem.write(iov.base, &host[off..off + take])
            .map_err(IoError::Fault)?;
        off += take;
    }
    Ok(nread)
}

/// `pwritev(fd, iov, iovcnt, offset)`: vectored positional write — gather the
/// guest buffers and write them to `fd` at the absolute `offset`, without using
/// the file cursor. Only regular files are positional (`EINVAL` otherwise).
///
/// # Errors
/// [`IoError::Invalid`] for a bad `iovcnt` or a non-file fd, [`IoError::Fault`]
/// if the array or a buffer is unreadable, [`IoError::BadFd`] for an unopen fd,
/// [`IoError::Host`] on a host write failure.
pub fn pwritev(
    fds: &FdTable,
    mem: &GuestRegion,
    fd: i32,
    iov_ptr: u64,
    iovcnt: i32,
    offset: u64,
) -> Result<usize, IoError> {
    let iovs = read_iovecs(mem, iov_ptr, iovcnt)?;
    let mut gathered = Vec::new();
    for iov in &iovs {
        let len = usize::try_from(iov.len).map_err(|_| IoError::Invalid)?;
        let buf = mem.read(iov.base, len).map_err(IoError::Fault)?;
        gathered.extend_from_slice(buf);
    }
    match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => pwrite_file(file, &gathered, offset).map_err(IoError::Host),
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => Err(IoError::Invalid),
    }
}

/// `preadv(fd, iov, iovcnt, offset)`: vectored positional read — read from `fd`
/// at the absolute `offset` and scatter into the guest buffers, without using
/// the file cursor. Destinations are validated writable before the fd is read.
///
/// # Errors
/// [`IoError::Invalid`] for a bad `iovcnt`/overflow or a non-file fd,
/// [`IoError::Fault`] if the array or a buffer is not writable, [`IoError::BadFd`]
/// for an unopen fd, [`IoError::Host`] on a host read failure.
pub fn preadv(
    fds: &FdTable,
    mem: &mut GuestRegion,
    fd: i32,
    iov_ptr: u64,
    iovcnt: i32,
    offset: u64,
) -> Result<usize, IoError> {
    let iovs = read_iovecs(mem, iov_ptr, iovcnt)?;
    let mut total = 0usize;
    for iov in &iovs {
        let len = usize::try_from(iov.len).map_err(|_| IoError::Invalid)?;
        mem.ensure_writable(iov.base, len).map_err(IoError::Fault)?;
        total = total.checked_add(len).ok_or(IoError::Invalid)?;
    }
    let mut host = vec![0u8; total];
    let nread = match fds.get(fd).ok_or(IoError::BadFd)? {
        FdEntry::File(file) => pread_file(file, &mut host, offset).map_err(IoError::Host)?,
        FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr => return Err(IoError::Invalid),
    };
    let mut off = 0usize;
    for iov in &iovs {
        if off >= nread {
            break;
        }
        let take = usize::try_from(iov.len).unwrap_or(0).min(nread - off);
        mem.write(iov.base, &host[off..off + take])
            .map_err(IoError::Fault)?;
        off += take;
    }
    Ok(nread)
}

/// The working directory the session reports. No per-process cwd is tracked yet,
/// so the guest always sees the single root.
const CWD: &[u8] = b"/";

/// Why `getcwd` failed (each maps to a guest errno at routing time).
#[derive(Debug)]
pub enum GetcwdError {
    /// The buffer is smaller than the path plus its NUL terminator — `ERANGE`.
    Range,
    /// The buffer is not writable guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `getcwd(buf, size)`: write the current working directory as a
/// NUL-terminated path into the guest buffer at `buf`, returning its length
/// including the terminator (the Linux `getcwd` return convention).
///
/// # Errors
/// [`GetcwdError::Range`] if `size` cannot hold the path plus its NUL,
/// [`GetcwdError::Fault`] if `buf` is not writable guest memory.
pub fn getcwd(mem: &mut GuestRegion, buf: u64, size: usize) -> Result<usize, GetcwdError> {
    let needed = CWD.len() + 1; // path bytes + the trailing NUL
    if size < needed {
        return Err(GetcwdError::Range);
    }
    let mut out = Vec::with_capacity(needed);
    out.extend_from_slice(CWD);
    out.push(0);
    mem.write(buf, &out).map_err(GetcwdError::Fault)?;
    Ok(needed)
}

#[cfg(test)]
mod tests {
    use super::{
        fcntl, fstat, ftruncate, getcwd, poll, ppoll, pread, preadv, pwrite, pwritev, readv, write,
        writev, GetcwdError, IoError,
    };
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::fd_table::{FdEntry, FdTable};
    use prisma_runtime::guest_structs::Iovec;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    /// Lay an `iovec` array into `buf` at byte offset `off` (guest addr
    /// `0x1000 + off`).
    fn put_iovecs(buf: &mut [u8], off: usize, iovs: &[Iovec]) {
        for (i, iov) in iovs.iter().enumerate() {
            let at = off + i * Iovec::SIZE;
            buf[at..at + Iovec::SIZE].copy_from_slice(&iov.to_guest_bytes());
        }
    }

    #[test]
    fn writev_gathers_buffers_in_order_to_stdout() {
        let fds = FdTable::new();
        let mut buf = [0u8; 64];
        buf[0..3].copy_from_slice(b"foo");
        buf[16..19].copy_from_slice(b"bar");
        // iovec array at guest 0x1020 -> {0x1000,3}, {0x1010,3}.
        put_iovecs(
            &mut buf,
            0x20,
            &[
                Iovec {
                    base: 0x1000,
                    len: 3,
                },
                Iovec {
                    base: 0x1010,
                    len: 3,
                },
            ],
        );
        let mem = region(&mut buf);
        // writev(1, 0x1020, 2) gathers "foo"+"bar" -> 6 bytes to stdout.
        assert_eq!(writev(&fds, &mem, 1, 0x1020, 2).unwrap(), 6);
    }

    #[test]
    fn writev_rejects_bad_iovcnt() {
        let fds = FdTable::new();
        let mut buf = [0u8; 32];
        let mem = region(&mut buf);
        // Negative count and a count past IOV_MAX are both EINVAL.
        assert!(matches!(
            writev(&fds, &mem, 1, 0x1000, -1),
            Err(IoError::Invalid)
        ));
        assert!(matches!(
            writev(&fds, &mem, 1, 0x1000, 4096),
            Err(IoError::Invalid)
        ));
    }

    #[test]
    fn writev_faults_on_an_out_of_range_buffer() {
        let fds = FdTable::new();
        let mut buf = [0u8; 32];
        // One iovec pointing past the 32-byte region.
        put_iovecs(
            &mut buf,
            0,
            &[Iovec {
                base: 0x9000,
                len: 4,
            }],
        );
        let mem = region(&mut buf);
        assert!(matches!(
            writev(&fds, &mem, 1, 0x1000, 1),
            Err(IoError::Fault(_))
        ));
    }

    #[test]
    fn readv_validates_destinations_before_consuming_the_fd() {
        // A read-only region: readv must fault on the write-into-guest check,
        // never reaching (and blocking on) the fd.
        let mut buf = [0u8; 32];
        put_iovecs(
            &mut buf,
            0,
            &[Iovec {
                base: 0x1010,
                len: 4,
            }],
        );
        let mut mem = GuestRegion::new(0x1000, Protection::ReadOnly, &mut buf);
        let fds = FdTable::new();
        assert!(matches!(
            readv(&fds, &mut mem, 0, 0x1000, 1),
            Err(IoError::Fault(RangeError::NotWritable))
        ));
    }

    #[test]
    fn writev_then_readv_round_trips_through_a_file() {
        // Gather two buffers into a file, then scatter the file back into two
        // different buffers — validates both directions end to end.
        let path = std::env::temp_dir().join(format!("prisma_iov_{}.tmp", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("temp file");
        let mut fds = FdTable::new();
        let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

        let mut buf = [0u8; 128];
        buf[0..3].copy_from_slice(b"foo"); // src1 @ 0x1000
        buf[16..19].copy_from_slice(b"bar"); // src2 @ 0x1010
        put_iovecs(
            &mut buf,
            0x20, // write-iovecs @ 0x1020
            &[
                Iovec {
                    base: 0x1000,
                    len: 3,
                },
                Iovec {
                    base: 0x1010,
                    len: 3,
                },
            ],
        );
        put_iovecs(
            &mut buf,
            0x60, // read-iovecs @ 0x1060 -> dst1 @ 0x1040, dst2 @ 0x1050
            &[
                Iovec {
                    base: 0x1040,
                    len: 3,
                },
                Iovec {
                    base: 0x1050,
                    len: 3,
                },
            ],
        );
        let mut mem = region(&mut buf);

        assert_eq!(writev(&fds, &mem, fd, 0x1020, 2).unwrap(), 6);
        // Rewind to the start, then scatter the file content back out.
        super::lseek(&fds, fd, 0, 0).expect("rewind");
        assert_eq!(readv(&fds, &mut mem, fd, 0x1060, 2).unwrap(), 6);

        // dst1 got "foo", dst2 got "bar": gather+scatter preserved order.
        assert_eq!(mem.read(0x1040, 3).unwrap(), b"foo");
        assert_eq!(mem.read(0x1050, 3).unwrap(), b"bar");

        // Resource discipline: drop the table (closes the host fd) then unlink.
        drop(fds);
        let _ = std::fs::remove_file(&path);
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

    #[test]
    fn lseek_repositions_a_file_and_validates_whence() {
        use super::lseek;
        let path = std::env::temp_dir().join(format!("prisma_lseek_{}.tmp", std::process::id()));
        std::fs::write(&path, b"0123456789").expect("seed");
        let mut fds = FdTable::new();
        {
            let file = std::fs::File::open(&path).expect("open");
            let fd = fds.allocate(FdEntry::File(file)).expect("allocate");
            // SEEK_SET to 4, then SEEK_CUR +2 -> 6, SEEK_END -> 10.
            assert_eq!(lseek(&fds, fd, 4, 0).unwrap(), 4);
            assert_eq!(lseek(&fds, fd, 2, 1).unwrap(), 6);
            assert_eq!(lseek(&fds, fd, 0, 2).unwrap(), 10);
            // A negative SEEK_SET and an unknown whence are EINVAL.
            assert!(matches!(lseek(&fds, fd, -1, 0), Err(IoError::Invalid)));
            assert!(matches!(lseek(&fds, fd, 0, 9), Err(IoError::Invalid)));
            assert!(fds.close(fd));
        }
        // Standard streams are not seekable; an unopen fd is EBADF.
        assert!(matches!(lseek(&fds, 1, 0, 0), Err(IoError::BadFd)));
        assert!(matches!(lseek(&fds, 88, 0, 0), Err(IoError::BadFd)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pread_pwrite_are_positional_and_ignore_the_cursor() {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("prisma_pio_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed"); // 11 bytes
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open rw");
        let mut fds = FdTable::new();
        let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // pread 5 bytes at offset 6 -> "world".
        assert_eq!(pread(&fds, &mut mem, fd, 0x1000, 5, 6).unwrap(), 5);
        assert_eq!(mem.read(0x1000, 5).unwrap(), b"world");

        // pwrite "ABC" at offset 0, then pread it back -> the cursor never moved.
        buf[8..11].copy_from_slice(b"ABC");
        let mut mem = region(&mut buf);
        assert_eq!(pwrite(&fds, &mem, fd, 0x1008, 3, 0).unwrap(), 3);
        assert_eq!(pread(&fds, &mut mem, fd, 0x1000, 3, 0).unwrap(), 3);
        assert_eq!(mem.read(0x1000, 3).unwrap(), b"ABC");

        // pread on a non-seekable stream (stdin) is EINVAL, not a host panic.
        assert!(matches!(
            pread(&fds, &mut mem, 0, 0x1000, 1, 0),
            Err(IoError::Invalid)
        ));

        drop(fds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ftruncate_resizes_a_file_and_rejects_bad_inputs() {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("prisma_trunc_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed"); // 11 bytes
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open rw");
        let mut fds = FdTable::new();
        let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

        // Shrink to 5 bytes.
        ftruncate(&fds, fd, 5).expect("shrink");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 5);
        // Grow to 20 (zero-extended).
        ftruncate(&fds, fd, 20).expect("grow");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 20);

        // A negative length is EINVAL; a non-file fd is EINVAL; an unopen fd EBADF.
        assert!(matches!(ftruncate(&fds, fd, -1), Err(IoError::Invalid)));
        assert!(matches!(ftruncate(&fds, 1, 0), Err(IoError::Invalid)));
        assert!(matches!(ftruncate(&fds, 77, 0), Err(IoError::BadFd)));

        drop(fds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pwritev_then_preadv_round_trip_at_an_offset() {
        use prisma_runtime::guest_structs::Iovec;
        let path = std::env::temp_dir().join(format!("prisma_pv_{}.tmp", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("temp file");
        let mut fds = FdTable::new();
        let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

        let mut buf = [0u8; 128];
        buf[0..3].copy_from_slice(b"foo");
        buf[16..19].copy_from_slice(b"bar");
        // write-iovecs @ 0x1020 -> {0x1000,3},{0x1010,3}; read-iovecs @ 0x1060
        // -> {0x1040,3},{0x1050,3}.
        for (at, iov) in [
            (
                0x20,
                Iovec {
                    base: 0x1000,
                    len: 3,
                },
            ),
            (
                0x30,
                Iovec {
                    base: 0x1010,
                    len: 3,
                },
            ),
            (
                0x60,
                Iovec {
                    base: 0x1040,
                    len: 3,
                },
            ),
            (
                0x70,
                Iovec {
                    base: 0x1050,
                    len: 3,
                },
            ),
        ] {
            buf[at..at + 16].copy_from_slice(&iov.to_guest_bytes());
        }
        let mut mem = region(&mut buf);

        // pwritev at offset 4 gathers "foobar" -> 6 bytes written at offset 4.
        assert_eq!(pwritev(&fds, &mem, fd, 0x1020, 2, 4).unwrap(), 6);
        // preadv at offset 4 scatters it back into dst1/dst2.
        assert_eq!(preadv(&fds, &mut mem, fd, 0x1060, 2, 4).unwrap(), 6);
        assert_eq!(mem.read(0x1040, 3).unwrap(), b"foo");
        assert_eq!(mem.read(0x1050, 3).unwrap(), b"bar");

        // A non-file fd is EINVAL (positional ops need a regular file).
        assert!(matches!(
            pwritev(&fds, &mem, 1, 0x1020, 2, 0),
            Err(IoError::Invalid)
        ));

        drop(fds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fstat_reports_file_and_stream_metadata() {
        use prisma_runtime::guest_structs::Stat;
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("prisma_fstat_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create");
            f.write_all(b"hello world").expect("seed"); // 11 bytes
        }
        let file = std::fs::File::open(&path).expect("open");
        let mut fds = FdTable::new();
        let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

        let mut buf = [0u8; 160];
        let mut mem = region(&mut buf);
        // fstat the file: size 11, regular-file mode bit, link count 1.
        fstat(&fds, &mut mem, fd, 0x1000).expect("fstat file");
        let st = Stat::from_guest_bytes(mem.read(0x1000, Stat::SIZE).unwrap()).unwrap();
        assert_eq!(st.size, 11);
        assert_eq!(st.mode & 0o170_000, 0o100_000); // S_IFREG
        assert_eq!(st.nlink, 1);
        assert_eq!(st.blocks, 1); // ceil(11/512)

        // fstat stdout: a character device, no size.
        fstat(&fds, &mut mem, 1, 0x1000).expect("fstat stream");
        let stream = Stat::from_guest_bytes(mem.read(0x1000, Stat::SIZE).unwrap()).unwrap();
        assert_eq!(stream.mode & 0o170_000, 0o020_000); // S_IFCHR
        assert_eq!(stream.size, 0);

        // A bad statbuf pointer is EFAULT; an unopen fd is EBADF.
        assert!(matches!(
            fstat(&fds, &mut mem, fd, 0x1090),
            Err(IoError::Fault(_))
        ));
        assert!(matches!(
            fstat(&fds, &mut mem, 88, 0x1000),
            Err(IoError::BadFd)
        ));

        drop(fds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_reports_readiness_per_fd() {
        use prisma_runtime::guest_structs::PollFd;
        const POLLIN: i16 = 0x1;
        const POLLOUT: i16 = 0x4;
        const POLLNVAL: i16 = 0x20;
        let fds = FdTable::new();
        // Three pollfds at guest 0x1000: stdin(POLLIN), stdout(POLLOUT), an
        // unopen fd 9 (POLLNVAL), plus a negative fd that must be skipped.
        let entries = [
            PollFd {
                fd: 0,
                events: POLLIN | POLLOUT,
                revents: 0,
            },
            PollFd {
                fd: 1,
                events: POLLIN | POLLOUT,
                revents: 0,
            },
            PollFd {
                fd: 9,
                events: POLLIN,
                revents: 0,
            },
            PollFd {
                fd: -1,
                events: POLLIN,
                revents: 0,
            },
        ];
        let mut buf = [0u8; 64];
        for (i, e) in entries.iter().enumerate() {
            buf[i * 8..i * 8 + 8].copy_from_slice(&e.to_guest_bytes());
        }
        let mut mem = region(&mut buf);
        // Three fds are ready (stdin readable, stdout writable, fd 9 invalid);
        // the negative fd is not counted.
        assert_eq!(poll(&fds, &mut mem, 0x1000, 4).unwrap(), 3);

        let rd = |i: usize| {
            PollFd::from_guest_bytes(mem.read(0x1000 + (i as u64) * 8, 8).unwrap())
                .unwrap()
                .revents
        };
        assert_eq!(rd(0), POLLIN); // stdin: only the readable half
        assert_eq!(rd(1), POLLOUT); // stdout: only the writable half
        assert_eq!(rd(2), POLLNVAL); // unopen fd
        assert_eq!(rd(3), 0); // negative fd skipped
    }

    #[test]
    fn ppoll_validates_the_timeout_and_delegates_to_poll() {
        use prisma_runtime::guest_structs::{PollFd, Timespec};
        const POLLOUT: i16 = 0x4;
        let fds = FdTable::new();
        // One pollfd for stdout at 0x1000; a {1s,0} timeout struct at 0x1010.
        let mut buf = [0u8; 64];
        buf[0..8].copy_from_slice(
            &PollFd {
                fd: 1,
                events: POLLOUT,
                revents: 0,
            }
            .to_guest_bytes(),
        );
        buf[16..32].copy_from_slice(&Timespec { sec: 1, nsec: 0 }.to_guest_bytes());
        let mut mem = region(&mut buf);
        // ppoll with a valid timeout -> delegates to poll -> stdout is writable.
        assert_eq!(ppoll(&fds, &mut mem, 0x1000, 1, 0x1010).unwrap(), 1);
        // A null timeout is the "block forever" form — still works (non-blocking).
        assert_eq!(ppoll(&fds, &mut mem, 0x1000, 1, 0).unwrap(), 1);
        // A bad timeout pointer faults before the poll work.
        assert!(matches!(
            ppoll(&fds, &mut mem, 0x1000, 1, 0x103A),
            Err(IoError::Fault(_))
        ));
    }

    #[test]
    fn fcntl_handles_descriptor_flags_and_locks() {
        use prisma_runtime::guest_structs::Flock;
        const F_GETFD: i32 = 1;
        const F_SETFD: i32 = 2;
        const F_GETFL: i32 = 3;
        const F_GETLK: i32 = 5;
        const F_SETLK: i32 = 6;
        const F_DUPFD: i32 = 0;
        const F_WRLCK: i16 = 1;
        const F_UNLCK: i16 = 2;
        let mut fds = FdTable::new();
        let mut buf = [0u8; 64];
        let mut mem = region(&mut buf);

        // Flag commands on stdout (fd 1).
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_GETFD, 0).unwrap(), 0); // no CLOEXEC
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_SETFD, 1).unwrap(), 0); // accepted
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_GETFL, 0).unwrap(), 2); // O_RDWR

        // F_DUPFD(1, 10) duplicates stdout onto the lowest free fd >= 10.
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_DUPFD, 10).unwrap(), 10);
        assert!(fds.is_open(10));
        // A negative floor is -EINVAL.
        assert!(matches!(
            fcntl(&mut fds, &mut mem, 1, F_DUPFD, u64::MAX),
            Err(IoError::Invalid)
        ));

        // F_SETLK always succeeds for the single-process guest.
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_SETLK, 0x1000).unwrap(), 0);

        // F_GETLK reports F_UNLCK regardless of the requested lock type.
        let req = Flock {
            typ: F_WRLCK,
            whence: 0,
            start: 0,
            len: 0,
            pid: 0,
        };
        buf[0..32].copy_from_slice(&req.to_guest_bytes());
        let mut mem = region(&mut buf);
        assert_eq!(fcntl(&mut fds, &mut mem, 1, F_GETLK, 0x1000).unwrap(), 0);
        let got = Flock::from_guest_bytes(mem.read(0x1000, Flock::SIZE).unwrap()).unwrap();
        assert_eq!(got.typ, F_UNLCK);

        // An unopen fd is EBADF; an unknown command is EINVAL.
        assert!(matches!(
            fcntl(&mut fds, &mut mem, 9, F_GETFD, 0),
            Err(IoError::BadFd)
        ));
        assert!(matches!(
            fcntl(&mut fds, &mut mem, 1, 999, 0),
            Err(IoError::Invalid)
        ));
    }

    #[test]
    fn getcwd_writes_a_nul_terminated_root_path() {
        let mut buf = [0xffu8; 16];
        let mut mem = region(&mut buf);
        // getcwd(buf, 16) -> 2 (the path "/" plus its NUL).
        let n = getcwd(&mut mem, 0x1000, 16).expect("write ok");
        assert_eq!(n, 2);
        assert_eq!(mem.read(0x1000, 2).unwrap(), b"/\0");
    }

    #[test]
    fn getcwd_reports_erange_when_the_buffer_is_too_small() {
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // Need 2 bytes ("/" + NUL); a 1-byte buffer is ERANGE, nothing written.
        assert!(matches!(
            getcwd(&mut mem, 0x1000, 1),
            Err(GetcwdError::Range)
        ));
    }

    #[test]
    fn getcwd_faults_on_an_unwritable_pointer() {
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // A pointer outside the mapped region faults rather than writing OOB.
        assert!(matches!(
            getcwd(&mut mem, 0x9000, 16),
            Err(GetcwdError::Fault(_))
        ));
    }
}
