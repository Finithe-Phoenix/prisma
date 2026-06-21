//! Marshalling for the small fixed-layout structs the guest passes to syscalls.
//!
//! F2-SY-034 / F2-SY-035. The guest is an x86-64 Linux ABI (LP64), so each
//! field is little-endian and 8-byte wide; ARM64 Linux shares the identical
//! layout, so this is the canonical wire form on both sides. The structs are
//! defined explicitly (not via `libc`) so the marshalling is host-independent
//! and testable anywhere. Each type round-trips through exactly its on-wire
//! size, and decoding rejects a short buffer rather than reading past it.

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

#[cfg(test)]
mod tests {
    use super::{Iovec, Timespec, Timeval};

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
    fn short_buffers_are_rejected_not_overrun() {
        assert!(Iovec::from_guest_bytes(&[0u8; 15]).is_none());
        assert!(Timespec::from_guest_bytes(&[]).is_none());
        assert!(Timeval::from_guest_bytes(&[0u8; 8]).is_none());
        // An over-long buffer decodes from the front and ignores the tail.
        let mut buf = [0u8; 32];
        buf[0] = 7;
        assert_eq!(Iovec::from_guest_bytes(&buf).map(|iv| iv.base), Some(7));
    }
}
