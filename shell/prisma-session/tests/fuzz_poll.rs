//! Property-fuzz the `poll` handler over arbitrary `pollfd` arrays.
//!
//! `poll` reads an untrusted array of `(fd, events)` requests and writes back a
//! `revents` for each. The invariants under test, against a fresh fd table
//! (stdin/stdout/stderr open at 0/1/2):
//!
//! * never panics, and always succeeds when the array is in valid guest memory;
//! * the returned ready count equals the number of entries whose `revents` is
//!   non-zero;
//! * each `revents` is exactly what the model dictates — a negative fd yields 0,
//!   stdin yields `events & POLLIN`, stdout/stderr `events & POLLOUT`, and any
//!   other (unopen) fd yields `POLLNVAL` regardless of the requested events.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::fd_table::FdTable;
use prisma_runtime::guest_structs::PollFd;
use prisma_session::io_syscalls::poll;
use proptest::prelude::*;

const BASE: u64 = 0x1000;
const POLLIN: i16 = 0x1;
const POLLOUT: i16 = 0x4;
const POLLNVAL: i16 = 0x20;

/// The `revents` the model must produce for `fd`/`events` on a fresh table.
fn expected(fd: i32, events: i16) -> i16 {
    match fd {
        _ if fd < 0 => 0,
        0 => events & POLLIN,
        1 | 2 => events & POLLOUT,
        _ => POLLNVAL,
    }
}

proptest! {
    #[test]
    fn poll_revents_match_the_model(
        entries in prop::collection::vec((-2i32..12, any::<i16>()), 0..16),
    ) {
        let fds = FdTable::new();
        // Lay the pollfd array into guest memory at BASE.
        let mut buf = vec![0u8; entries.len().max(1) * PollFd::SIZE];
        for (i, &(fd, events)) in entries.iter().enumerate() {
            let pfd = PollFd { fd, events, revents: 0 };
            buf[i * PollFd::SIZE..(i + 1) * PollFd::SIZE].copy_from_slice(&pfd.to_guest_bytes());
        }
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);

        let nfds = u32::try_from(entries.len()).unwrap();
        let ready = poll(&fds, &mut mem, BASE, nfds).expect("valid array polls");

        // Read each entry's revents back and check it against the model.
        let mut want_ready = 0usize;
        for (i, &(fd, events)) in entries.iter().enumerate() {
            let at = BASE + (i as u64) * PollFd::SIZE as u64;
            let got = PollFd::from_guest_bytes(mem.read(at, PollFd::SIZE).unwrap())
                .unwrap()
                .revents;
            let exp = expected(fd, events);
            prop_assert_eq!(got, exp);
            if exp != 0 {
                want_ready += 1;
            }
        }
        // The ready count equals the number of non-zero revents.
        prop_assert_eq!(ready, want_ready);
    }

    /// A zero-length poll touches nothing and reports nothing ready.
    #[test]
    fn poll_with_no_fds_is_zero(ptr in any::<u64>()) {
        let fds = FdTable::new();
        let mut buf = [0u8; 8];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        // nfds == 0: the array pointer is never dereferenced.
        prop_assert_eq!(poll(&fds, &mut mem, ptr, 0).unwrap(), 0);
    }
}
