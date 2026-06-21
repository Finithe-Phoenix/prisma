//! Marshalling for the small fixed-layout structs the guest passes to syscalls.
//!
//! F2-SY-034 / F2-SY-035. The guest is an x86-64 Linux ABI (LP64), so each
//! field is little-endian and 8-byte wide; ARM64 Linux shares the identical
//! layout, so this is the canonical wire form on both sides. The structs are
//! defined explicitly (not via `libc`) so the marshalling is host-independent
//! and testable anywhere. Each type round-trips through exactly its on-wire
//! size, and decoding rejects a short buffer rather than reading past it.

use std::time::Duration;

/// `struct iovec { void *iov_base; size_t iov_len; }` — the scatter/gather
/// element for `readv`/`writev`. 16 bytes on x86-64 Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Iovec {
    /// Guest virtual address of the buffer.
    pub base: u64,
    /// Length of the buffer in bytes.
    pub len: u64,
}

impl Iovec {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 16;

    /// Decode one `iovec` from the front of `bytes`, or `None` if fewer than
    /// [`Iovec::SIZE`] bytes are available.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            base: u64::from_le_bytes(raw[0..8].try_into().ok()?),
            len: u64::from_le_bytes(raw[8..16].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.base.to_le_bytes());
        out[8..16].copy_from_slice(&self.len.to_le_bytes());
        out
    }
}

/// `struct timespec { time_t tv_sec; long tv_nsec; }` — `nanosleep`,
/// `clock_gettime`. Both fields are signed 64-bit on x86-64 Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timespec {
    /// Whole seconds.
    pub sec: i64,
    /// Nanoseconds within the second.
    pub nsec: i64,
}

impl Timespec {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 16;

    /// Decode one `timespec` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            sec: i64::from_le_bytes(raw[0..8].try_into().ok()?),
            nsec: i64::from_le_bytes(raw[8..16].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.sec.to_le_bytes());
        out[8..16].copy_from_slice(&self.nsec.to_le_bytes());
        out
    }

    /// This `timespec` as a host [`Duration`] — the form `nanosleep` /
    /// `clock_nanosleep` sleep for. `None` if either field is negative (a
    /// `Duration` cannot represent time before zero) or the nanoseconds do not
    /// fit a `u32` (a malformed request the kernel would reject as `EINVAL`).
    #[must_use]
    pub fn to_duration(self) -> Option<Duration> {
        if self.sec < 0 || self.nsec < 0 {
            return None;
        }
        let sec = u64::try_from(self.sec).ok()?;
        let nsec = u32::try_from(self.nsec).ok()?;
        Some(Duration::new(sec, nsec))
    }

    /// A host [`Duration`] as a `timespec`, for reporting a clock value back to
    /// the guest. Seconds beyond `i64::MAX` saturate (they cannot occur for a
    /// real clock).
    #[must_use]
    pub fn from_duration(d: Duration) -> Self {
        Self {
            sec: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            nsec: i64::from(d.subsec_nanos()),
        }
    }
}

/// `struct timeval { time_t tv_sec; suseconds_t tv_usec; }` —
/// `gettimeofday`, `select`. Both fields are signed 64-bit on x86-64 Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeval {
    /// Whole seconds.
    pub sec: i64,
    /// Microseconds within the second.
    pub usec: i64,
}

impl Timeval {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 16;

    /// Decode one `timeval` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            sec: i64::from_le_bytes(raw[0..8].try_into().ok()?),
            usec: i64::from_le_bytes(raw[8..16].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.sec.to_le_bytes());
        out[8..16].copy_from_slice(&self.usec.to_le_bytes());
        out
    }
}

/// The terminal size filled by `ioctl(TIOCGWINSZ)`.
///
/// `struct winsize { unsigned short ws_row, ws_col, ws_xpixel, ws_ypixel; }` —
/// four 16-bit fields, 8 bytes on x86-64 Linux. The pixel fields are usually
/// zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Winsize {
    /// Rows (character cells).
    pub row: u16,
    /// Columns (character cells).
    pub col: u16,
    /// Width in pixels (0 when unknown).
    pub xpixel: u16,
    /// Height in pixels (0 when unknown).
    pub ypixel: u16,
}

impl Winsize {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 8;

    /// Decode one `winsize` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            row: u16::from_le_bytes(raw[0..2].try_into().ok()?),
            col: u16::from_le_bytes(raw[2..4].try_into().ok()?),
            xpixel: u16::from_le_bytes(raw[4..6].try_into().ok()?),
            ypixel: u16::from_le_bytes(raw[6..8].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..2].copy_from_slice(&self.row.to_le_bytes());
        out[2..4].copy_from_slice(&self.col.to_le_bytes());
        out[4..6].copy_from_slice(&self.xpixel.to_le_bytes());
        out[6..8].copy_from_slice(&self.ypixel.to_le_bytes());
        out
    }
}

/// `struct termios` — the terminal attributes `ioctl(TCGETS/TCSETS)` and
/// `tcgetattr`/`tcsetattr` read and write.
///
/// The x86-64 Linux layout is four 32-bit mode words, the 8-bit line
/// discipline, and the 19-byte control-char array (`NCCS`), 36 bytes with no
/// trailing padding. F2-SY-036.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Termios {
    /// Input modes (`c_iflag`).
    pub iflag: u32,
    /// Output modes (`c_oflag`).
    pub oflag: u32,
    /// Control modes (`c_cflag`).
    pub cflag: u32,
    /// Local modes (`c_lflag`).
    pub lflag: u32,
    /// Line discipline (`c_line`).
    pub line: u8,
    /// Control characters (`c_cc[NCCS]`).
    pub cc: [u8; Self::NCCS],
}

impl Termios {
    /// Number of control characters (`NCCS` on x86-64 Linux).
    pub const NCCS: usize = 19;

    /// On-wire size in guest memory (4×u32 + u8 + 19-byte `c_cc`).
    pub const SIZE: usize = 36;

    /// Decode one `termios` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        let mut cc = [0u8; Self::NCCS];
        cc.copy_from_slice(&raw[17..36]);
        Some(Self {
            iflag: u32::from_le_bytes(raw[0..4].try_into().ok()?),
            oflag: u32::from_le_bytes(raw[4..8].try_into().ok()?),
            cflag: u32::from_le_bytes(raw[8..12].try_into().ok()?),
            lflag: u32::from_le_bytes(raw[12..16].try_into().ok()?),
            line: raw[16],
            cc,
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.iflag.to_le_bytes());
        out[4..8].copy_from_slice(&self.oflag.to_le_bytes());
        out[8..12].copy_from_slice(&self.cflag.to_le_bytes());
        out[12..16].copy_from_slice(&self.lflag.to_le_bytes());
        out[16] = self.line;
        out[17..36].copy_from_slice(&self.cc);
        out
    }
}

/// `struct sigaltstack` / `stack_t` — the alternate signal stack `sigaltstack`
/// reads and writes.
///
/// The x86-64 Linux layout is the 64-bit `ss_sp` pointer, the 32-bit `ss_flags`
/// (followed by 4 bytes of padding to align the next field), and the 64-bit
/// `ss_size`, 24 bytes total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigAltStack {
    /// Base of the alternate stack (`ss_sp`).
    pub sp: u64,
    /// Flags (`ss_flags`): SS_ONSTACK / SS_DISABLE / SS_AUTODISARM.
    pub flags: i32,
    /// Size of the alternate stack in bytes (`ss_size`).
    pub size: u64,
}

impl SigAltStack {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 24;

    /// Decode one `stack_t` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            sp: u64::from_le_bytes(raw[0..8].try_into().ok()?),
            flags: i32::from_le_bytes(raw[8..12].try_into().ok()?),
            // raw[12..16] is alignment padding, ignored.
            size: u64::from_le_bytes(raw[16..24].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form. The 4 padding bytes after `ss_flags` are
    /// written as zero.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.sp.to_le_bytes());
        out[8..12].copy_from_slice(&self.flags.to_le_bytes());
        out[16..24].copy_from_slice(&self.size.to_le_bytes());
        out
    }
}

/// `struct rlimit` — the soft/hard resource limit pair `getrlimit` / `setrlimit`
/// / `prlimit64` read and write.
///
/// Two 64-bit fields on x86-64 Linux, 16 bytes. `RLIM_INFINITY` is `u64::MAX`
/// ("no limit").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rlimit {
    /// Soft limit (`rlim_cur`): the value the kernel enforces.
    pub cur: u64,
    /// Hard limit (`rlim_max`): the ceiling the soft limit may be raised to.
    pub max: u64,
}

impl Rlimit {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 16;

    /// `RLIM_INFINITY` — the sentinel for "no limit".
    pub const INFINITY: u64 = u64::MAX;

    /// Decode one `rlimit` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            cur: u64::from_le_bytes(raw[0..8].try_into().ok()?),
            max: u64::from_le_bytes(raw[8..16].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.cur.to_le_bytes());
        out[8..16].copy_from_slice(&self.max.to_le_bytes());
        out
    }
}

/// `struct stat` — the file metadata `fstat` / `stat` / `lstat` / `fstatat`
/// fill in.
///
/// The x86-64 Linux layout is 144 bytes with the field offsets below (the
/// 4-byte `__pad0` after `st_gid` and the trailing 24-byte `__unused[3]` are
/// reserved and written zero). The three timestamps are embedded `timespec`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    /// Device id (`st_dev`).
    pub dev: u64,
    /// Inode number (`st_ino`).
    pub ino: u64,
    /// Hard-link count (`st_nlink`).
    pub nlink: u64,
    /// File type and mode bits (`st_mode`).
    pub mode: u32,
    /// Owning user id (`st_uid`).
    pub uid: u32,
    /// Owning group id (`st_gid`).
    pub gid: u32,
    /// Device id for a special file (`st_rdev`).
    pub rdev: u64,
    /// Size in bytes (`st_size`).
    pub size: i64,
    /// Preferred I/O block size (`st_blksize`).
    pub blksize: i64,
    /// Number of 512-byte blocks allocated (`st_blocks`).
    pub blocks: i64,
    /// Last access time (`st_atim`).
    pub atime: Timespec,
    /// Last modification time (`st_mtim`).
    pub mtime: Timespec,
    /// Last status-change time (`st_ctim`).
    pub ctime: Timespec,
}

impl Stat {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 144;

    /// Decode one `stat` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        let word = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap());
        let signed = |o: usize| i64::from_le_bytes(raw[o..o + 8].try_into().unwrap());
        let half = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
        Some(Self {
            dev: word(0),
            ino: word(8),
            nlink: word(16),
            mode: half(24),
            uid: half(28),
            gid: half(32),
            // raw[36..40] is __pad0, ignored.
            rdev: word(40),
            size: signed(48),
            blksize: signed(56),
            blocks: signed(64),
            atime: Timespec::from_guest_bytes(&raw[72..88])?,
            mtime: Timespec::from_guest_bytes(&raw[88..104])?,
            ctime: Timespec::from_guest_bytes(&raw[104..120])?,
            // raw[120..144] is __unused[3], ignored.
        })
    }

    /// Encode to the guest wire form. The reserved `__pad0` and `__unused` bytes
    /// are written zero.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.dev.to_le_bytes());
        out[8..16].copy_from_slice(&self.ino.to_le_bytes());
        out[16..24].copy_from_slice(&self.nlink.to_le_bytes());
        out[24..28].copy_from_slice(&self.mode.to_le_bytes());
        out[28..32].copy_from_slice(&self.uid.to_le_bytes());
        out[32..36].copy_from_slice(&self.gid.to_le_bytes());
        out[40..48].copy_from_slice(&self.rdev.to_le_bytes());
        out[48..56].copy_from_slice(&self.size.to_le_bytes());
        out[56..64].copy_from_slice(&self.blksize.to_le_bytes());
        out[64..72].copy_from_slice(&self.blocks.to_le_bytes());
        out[72..88].copy_from_slice(&self.atime.to_guest_bytes());
        out[88..104].copy_from_slice(&self.mtime.to_guest_bytes());
        out[104..120].copy_from_slice(&self.ctime.to_guest_bytes());
        out
    }
}

/// `struct pollfd { int fd; short events; short revents; }` — the per-fd request
/// element `poll` / `ppoll` read (`fd`, `events`) and write back (`revents`).
///
/// 8 bytes on x86-64 Linux: a 32-bit fd then two 16-bit event masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollFd {
    /// File descriptor to poll (a negative fd is ignored by the kernel).
    pub fd: i32,
    /// Requested events (`POLLIN`/`POLLOUT`/… bitmask).
    pub events: i16,
    /// Returned events the kernel fills in.
    pub revents: i16,
}

impl PollFd {
    /// On-wire size in guest memory.
    pub const SIZE: usize = 8;

    /// Decode one `pollfd` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            fd: i32::from_le_bytes(raw[0..4].try_into().ok()?),
            events: i16::from_le_bytes(raw[4..6].try_into().ok()?),
            revents: i16::from_le_bytes(raw[6..8].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.fd.to_le_bytes());
        out[4..6].copy_from_slice(&self.events.to_le_bytes());
        out[6..8].copy_from_slice(&self.revents.to_le_bytes());
        out
    }
}

/// `struct epoll_event { uint32_t events; epoll_data_t data; }` — the event
/// descriptor `epoll_ctl` registers and `epoll_wait` returns.
///
/// On x86-64 Linux the struct is `__packed`, so it is 12 bytes (a 32-bit event
/// mask immediately followed by the 64-bit user data, no alignment padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpollEvent {
    /// Event mask (`EPOLLIN`/`EPOLLOUT`/… bits).
    pub events: u32,
    /// Opaque user data the kernel echoes back on `epoll_wait`.
    pub data: u64,
}

impl EpollEvent {
    /// On-wire size in guest memory (packed: 4-byte mask + 8-byte data).
    pub const SIZE: usize = 12;

    /// Decode one `epoll_event` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            events: u32::from_le_bytes(raw[0..4].try_into().ok()?),
            data: u64::from_le_bytes(raw[4..12].try_into().ok()?),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.events.to_le_bytes());
        out[4..12].copy_from_slice(&self.data.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EpollEvent, Iovec, PollFd, Rlimit, SigAltStack, Stat, Termios, Timespec, Timeval, Winsize,
    };

    #[test]
    fn iovec_round_trips_through_exact_layout() {
        let iv = Iovec {
            base: 0x1_4000_2000,
            len: 0x1000,
        };
        let bytes = iv.to_guest_bytes();
        // Field order/endianness: base in [0..8], len in [8..16], little-endian.
        assert_eq!(&bytes[0..8], &0x1_4000_2000u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &0x1000u64.to_le_bytes());
        assert_eq!(Iovec::from_guest_bytes(&bytes), Some(iv));
    }

    #[test]
    fn timespec_and_timeval_round_trip() {
        let ts = Timespec {
            sec: 1_700_000_000,
            nsec: 123_456_789,
        };
        assert_eq!(Timespec::from_guest_bytes(&ts.to_guest_bytes()), Some(ts));
        let tv = Timeval {
            sec: 1_700_000_000,
            usec: 654_321,
        };
        assert_eq!(Timeval::from_guest_bytes(&tv.to_guest_bytes()), Some(tv));
    }

    #[test]
    fn negative_time_fields_are_signed() {
        // time_t is signed: a pre-epoch timestamp must survive the round trip.
        let ts = Timespec { sec: -42, nsec: -1 };
        assert_eq!(Timespec::from_guest_bytes(&ts.to_guest_bytes()), Some(ts));
    }

    #[test]
    fn timespec_duration_conversions() {
        use std::time::Duration;
        // 5.5 seconds.
        let ts = Timespec {
            sec: 5,
            nsec: 500_000_000,
        };
        assert_eq!(ts.to_duration(), Some(Duration::new(5, 500_000_000)));
        // Round trip through Duration.
        assert_eq!(Timespec::from_duration(ts.to_duration().unwrap()), ts);
        // A negative request has no Duration (the kernel would reject it).
        assert_eq!(Timespec { sec: -1, nsec: 0 }.to_duration(), None);
        assert_eq!(Timespec { sec: 0, nsec: -1 }.to_duration(), None);
    }

    #[test]
    fn short_buffers_are_rejected_not_overrun() {
        assert!(Iovec::from_guest_bytes(&[0u8; 15]).is_none());
        assert!(Timespec::from_guest_bytes(&[]).is_none());
        assert!(Timeval::from_guest_bytes(&[0u8; 8]).is_none());
        // An over-long buffer decodes from the front and ignores the tail.
        let mut buf = [0u8; 32];
        buf[0] = 7;
        assert_eq!(Iovec::from_guest_bytes(&buf).map(|iv| iv.base), Some(7));
    }

    #[test]
    fn winsize_round_trips_through_four_u16_fields() {
        let ws = Winsize {
            row: 24,
            col: 80,
            xpixel: 0,
            ypixel: 0,
        };
        let bytes = ws.to_guest_bytes();
        assert_eq!(bytes.len(), Winsize::SIZE);
        // row in [0..2], col in [2..4], little-endian.
        assert_eq!(&bytes[0..2], &24u16.to_le_bytes());
        assert_eq!(&bytes[2..4], &80u16.to_le_bytes());
        assert_eq!(Winsize::from_guest_bytes(&bytes), Some(ws));
        assert!(Winsize::from_guest_bytes(&[0u8; 7]).is_none());
    }

    #[test]
    fn termios_round_trips_through_its_36_byte_layout() {
        let mut cc = [0u8; Termios::NCCS];
        // A few representative control chars (VINTR=^C, VEOF=^D, VMIN, VTIME).
        cc[0] = 3;
        cc[4] = 4;
        cc[6] = 1;
        cc[5] = 0;
        let t = Termios {
            iflag: 0x0000_0500,
            oflag: 0x0000_0005,
            cflag: 0x0000_00bf,
            lflag: 0x0000_8a3b,
            line: 0,
            cc,
        };
        let bytes = t.to_guest_bytes();
        assert_eq!(bytes.len(), Termios::SIZE);
        // Field offsets: four LE u32 mode words, then c_line, then c_cc.
        assert_eq!(&bytes[0..4], &0x0000_0500u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &0x0000_8a3bu32.to_le_bytes());
        assert_eq!(bytes[16], 0); // c_line
        assert_eq!(&bytes[17..36], &cc);
        assert_eq!(Termios::from_guest_bytes(&bytes), Some(t));
        // A buffer one byte short is rejected, not overrun.
        assert!(Termios::from_guest_bytes(&[0u8; 35]).is_none());
    }

    #[test]
    fn sigaltstack_round_trips_and_keeps_flags_padding_clean() {
        let ss = SigAltStack {
            sp: 0x1_4000_8000,
            flags: 1, // SS_ONSTACK
            size: 0x2000,
        };
        let bytes = ss.to_guest_bytes();
        assert_eq!(bytes.len(), SigAltStack::SIZE);
        // ss_sp [0..8], ss_flags [8..12], padding [12..16] is zero, ss_size [16..24].
        assert_eq!(&bytes[0..8], &0x1_4000_8000u64.to_le_bytes());
        assert_eq!(&bytes[8..12], &1i32.to_le_bytes());
        assert_eq!(&bytes[12..16], &[0u8; 4]);
        assert_eq!(&bytes[16..24], &0x2000u64.to_le_bytes());
        assert_eq!(SigAltStack::from_guest_bytes(&bytes), Some(ss));
        // Decoding ignores whatever sits in the padding bytes.
        let mut dirty = bytes;
        dirty[12..16].copy_from_slice(&[0xAA; 4]);
        assert_eq!(SigAltStack::from_guest_bytes(&dirty), Some(ss));
        // A buffer one byte short is rejected.
        assert!(SigAltStack::from_guest_bytes(&[0u8; 23]).is_none());
    }

    #[test]
    fn rlimit_round_trips_through_two_u64_fields() {
        let rl = Rlimit {
            cur: 1024 * 1024,
            max: Rlimit::INFINITY,
        };
        let bytes = rl.to_guest_bytes();
        assert_eq!(bytes.len(), Rlimit::SIZE);
        // rlim_cur [0..8], rlim_max [8..16], little-endian.
        assert_eq!(&bytes[0..8], &(1024u64 * 1024).to_le_bytes());
        assert_eq!(&bytes[8..16], &u64::MAX.to_le_bytes());
        assert_eq!(Rlimit::from_guest_bytes(&bytes), Some(rl));
        // A buffer one byte short is rejected, not overrun.
        assert!(Rlimit::from_guest_bytes(&[0u8; 15]).is_none());
    }

    #[test]
    fn stat_round_trips_through_its_144_byte_layout() {
        let st = Stat {
            dev: 0x0010,
            ino: 0x1234_5678,
            nlink: 1,
            mode: 0o100_644, // regular file, rw-r--r--
            uid: 1000,
            gid: 1000,
            rdev: 0,
            size: 4096,
            blksize: 512,
            blocks: 8,
            atime: Timespec {
                sec: 1_700_000_000,
                nsec: 1,
            },
            mtime: Timespec {
                sec: 1_700_000_001,
                nsec: 2,
            },
            ctime: Timespec {
                sec: 1_700_000_002,
                nsec: 3,
            },
        };
        let bytes = st.to_guest_bytes();
        assert_eq!(bytes.len(), Stat::SIZE);
        // Spot-check a few field offsets.
        assert_eq!(&bytes[24..28], &0o100_644u32.to_le_bytes()); // st_mode
        assert_eq!(&bytes[48..56], &4096i64.to_le_bytes()); // st_size
        assert_eq!(&bytes[72..80], &1_700_000_000i64.to_le_bytes()); // st_atim.sec
                                                                     // Reserved __pad0 and __unused are zero.
        assert_eq!(&bytes[36..40], &[0u8; 4]);
        assert_eq!(&bytes[120..144], &[0u8; 24]);
        assert_eq!(Stat::from_guest_bytes(&bytes), Some(st));
        // A buffer one byte short is rejected, not overrun.
        assert!(Stat::from_guest_bytes(&[0u8; 143]).is_none());
    }

    #[test]
    fn pollfd_round_trips_through_fd_and_event_masks() {
        // POLLIN = 0x1, POLLOUT = 0x4.
        let p = PollFd {
            fd: 7,
            events: 0x1 | 0x4,
            revents: 0,
        };
        let bytes = p.to_guest_bytes();
        assert_eq!(bytes.len(), PollFd::SIZE);
        // fd [0..4], events [4..6], revents [6..8], little-endian.
        assert_eq!(&bytes[0..4], &7i32.to_le_bytes());
        assert_eq!(&bytes[4..6], &5i16.to_le_bytes());
        assert_eq!(PollFd::from_guest_bytes(&bytes), Some(p));
        // A negative fd (ignored by poll) round-trips faithfully.
        let neg = PollFd {
            fd: -1,
            events: 0,
            revents: 0,
        };
        assert_eq!(PollFd::from_guest_bytes(&neg.to_guest_bytes()), Some(neg));
        // A buffer one byte short is rejected.
        assert!(PollFd::from_guest_bytes(&[0u8; 7]).is_none());
    }

    #[test]
    fn epoll_event_round_trips_packed() {
        // EPOLLIN = 0x1, EPOLLOUT = 0x4.
        let e = EpollEvent {
            events: 0x1 | 0x4,
            data: 0xDEAD_BEEF_0000_0001,
        };
        let bytes = e.to_guest_bytes();
        assert_eq!(bytes.len(), EpollEvent::SIZE);
        // events [0..4], data [4..12] — packed, the data is NOT 8-byte aligned.
        assert_eq!(&bytes[0..4], &5u32.to_le_bytes());
        assert_eq!(&bytes[4..12], &0xDEAD_BEEF_0000_0001u64.to_le_bytes());
        assert_eq!(EpollEvent::from_guest_bytes(&bytes), Some(e));
        // A buffer one byte short is rejected, not overrun.
        assert!(EpollEvent::from_guest_bytes(&[0u8; 11]).is_none());
    }
}
