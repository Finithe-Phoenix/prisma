//! Integration check that the file-I/O handlers compose over one real file.
//!
//! The unit tests exercise `fstat` / `ftruncate` / `pwrite` / `pread` in
//! isolation. This drives them together against a single host-backed fd to
//! confirm they agree on the file's state: a positional write grows it, `fstat`
//! reflects each size change, `ftruncate` resizes it both ways, and a positional
//! read sees exactly what was written — the composed behaviour a real guest
//! relies on.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::fd_table::{FdEntry, FdTable};
use prisma_runtime::guest_structs::Stat;
use prisma_session::io_syscalls::{fstat, ftruncate, pread, pwrite};

fn read_size(fds: &FdTable, fd: i32) -> i64 {
    let mut buf = [0u8; 160];
    let mut mem = GuestRegion::new(0x2000, Protection::ReadWrite, &mut buf);
    fstat(fds, &mut mem, fd, 0x2000).expect("fstat");
    Stat::from_guest_bytes(mem.read(0x2000, Stat::SIZE).unwrap())
        .unwrap()
        .size
}

#[test]
fn fstat_tracks_pwrite_and_ftruncate_on_one_file() {
    let path = std::env::temp_dir().join(format!("prisma_fh_{}.tmp", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .expect("temp file");
    let mut fds = FdTable::new();
    let fd = fds.allocate(FdEntry::File(file)).expect("allocate");

    // A fresh file is empty.
    assert_eq!(read_size(&fds, fd), 0);

    // pwrite 5 bytes at offset 0 -> the file (and fstat) grow to 5.
    let mut wbuf = [0u8; 16];
    wbuf[0..5].copy_from_slice(b"hello");
    let wmem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut wbuf);
    assert_eq!(pwrite(&fds, &wmem, fd, 0x1000, 5, 0).unwrap(), 5);
    assert_eq!(read_size(&fds, fd), 5);

    // ftruncate out to 100 -> fstat reflects the grown size.
    ftruncate(&fds, fd, 100).expect("grow");
    assert_eq!(read_size(&fds, fd), 100);

    // ftruncate back to 3 -> fstat shrinks, and a positional read sees the
    // surviving prefix "hel".
    ftruncate(&fds, fd, 3).expect("shrink");
    assert_eq!(read_size(&fds, fd), 3);
    let mut rbuf = [0u8; 16];
    let mut rmem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut rbuf);
    assert_eq!(pread(&fds, &mut rmem, fd, 0x1000, 8, 0).unwrap(), 3);
    assert_eq!(rmem.read(0x1000, 3).unwrap(), b"hel");

    // Resource discipline: drop the table (closes the fd) then unlink.
    drop(fds);
    let _ = std::fs::remove_file(&path);
}
