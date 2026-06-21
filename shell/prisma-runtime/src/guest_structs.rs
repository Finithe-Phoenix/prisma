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

#[cfg(test)]
mod tests {
    use super::{Iovec, SigAltStack, Termios, Timespec, Timeval, Winsize};

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
}
