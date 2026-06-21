//! Checked read/write over one contiguous guest memory region.
//!
//! A syscall handler that copies a buffer to/from the guest must never index
//! the backing store with an unchecked guest pointer. [`GuestRegion`] pairs the
//! backing bytes with the base address they are mapped at and their protection,
//! and bounds- and permission-checks every access before touching them — a bad
//! pointer yields a [`RangeError`] (guest `EFAULT`) instead of an out-of-bounds
//! host access. A full guest address space is a set of these regions; this is
//! the per-region primitive they share.

use crate::address_space::{Protection, RangeError};

/// One contiguous, protection-tagged span of guest memory with checked access.
#[derive(Debug)]
pub struct GuestRegion<'a> {
    base: u64,
    prot: Protection,
    bytes: &'a mut [u8],
}

impl<'a> GuestRegion<'a> {
    /// A region of `bytes` mapped at guest address `base` with `prot`.
    #[must_use]
    pub fn new(base: u64, prot: Protection, bytes: &'a mut [u8]) -> Self {
        Self { base, prot, bytes }
    }

    /// Guest base address of the region.
    #[must_use]
    pub const fn base(&self) -> u64 {
        self.base
    }

    /// Size of the region in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the region is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Byte offset of `[addr, addr+len)` within this region, or a [`RangeError`]
    /// if the access runs outside the region, overflows, or needs write access
    /// to read-only memory. A zero-length access anywhere inside (or exactly at
    /// the end of) the region is valid.
    fn checked_offset(&self, addr: u64, len: u64, need_write: bool) -> Result<usize, RangeError> {
        if need_write && !self.prot.is_writable() {
            return Err(RangeError::NotWritable);
        }
        let end = addr.checked_add(len).ok_or(RangeError::Overflow)?;
        let region_end = self
            .base
            .checked_add(self.bytes.len() as u64)
            .ok_or(RangeError::Overflow)?;
        if addr < self.base || end > region_end {
            return Err(RangeError::Unmapped);
        }
        usize::try_from(addr - self.base).map_err(|_| RangeError::Unmapped)
    }

    /// Borrow `len` bytes at guest address `addr`, or a [`RangeError`] if the
    /// range is not fully inside the region.
    ///
    /// # Errors
    /// [`RangeError::Unmapped`] / [`RangeError::Overflow`] when the range leaves
    /// the region.
    pub fn read(&self, addr: u64, len: usize) -> Result<&[u8], RangeError> {
        let off = self.checked_offset(addr, len as u64, false)?;
        Ok(&self.bytes[off..off + len])
    }

    /// Copy `src` into guest memory at `addr`, or a [`RangeError`] if the range
    /// is not fully inside the region or the region is read-only.
    ///
    /// # Errors
    /// [`RangeError::NotWritable`] for a read-only region, otherwise
    /// [`RangeError::Unmapped`] / [`RangeError::Overflow`] when the range leaves
    /// the region.
    pub fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), RangeError> {
        let off = self.checked_offset(addr, src.len() as u64, true)?;
        self.bytes[off..off + src.len()].copy_from_slice(src);
        Ok(())
    }

    /// Verify `[addr, addr+len)` is writable guest memory without writing — the
    /// check a syscall runs before consuming an external source (e.g. `read`
    /// validates the destination before pulling bytes off an fd, so a bad
    /// pointer faults without losing data).
    ///
    /// # Errors
    /// [`RangeError::NotWritable`] for a read-only region, otherwise
    /// [`RangeError::Unmapped`] / [`RangeError::Overflow`] when the range leaves
    /// the region.
    pub fn ensure_writable(&self, addr: u64, len: usize) -> Result<(), RangeError> {
        self.checked_offset(addr, len as u64, true).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::GuestRegion;
    use crate::address_space::{Protection, RangeError};

    #[test]
    fn read_inside_the_region_returns_the_bytes() {
        let mut buf = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let r = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(r.read(0x1000, 3).unwrap(), &[1, 2, 3]);
        assert_eq!(r.read(0x1004, 2).unwrap(), &[5, 6]);
        // Exactly the last byte.
        assert_eq!(r.read(0x1007, 1).unwrap(), &[8]);
    }

    #[test]
    fn read_past_the_end_is_rejected_not_overrun() {
        let mut buf = [0u8; 8];
        let r = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(r.read(0x1006, 4), Err(RangeError::Unmapped)); // runs 2 past
        assert_eq!(r.read(0x0fff, 1), Err(RangeError::Unmapped)); // below base
        assert_eq!(r.read(0x2000, 1), Err(RangeError::Unmapped)); // far past
    }

    #[test]
    fn write_into_the_region_updates_the_backing_bytes() {
        let mut buf = [0u8; 8];
        {
            let mut r = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
            r.write(0x1002, &[0xAA, 0xBB]).unwrap();
        }
        assert_eq!(buf, [0, 0, 0xAA, 0xBB, 0, 0, 0, 0]);
    }

    #[test]
    fn write_to_read_only_region_is_rejected() {
        let mut buf = [0u8; 8];
        let mut r = GuestRegion::new(0x1000, Protection::ReadExecute, &mut buf);
        assert_eq!(r.write(0x1000, &[1]), Err(RangeError::NotWritable));
        // Reading the read-only region is still fine.
        assert_eq!(r.read(0x1000, 1).unwrap(), &[0]);
    }

    #[test]
    fn zero_length_access_at_the_end_is_valid() {
        let mut buf = [0u8; 8];
        let r = GuestRegion::new(0x1000, Protection::ReadOnly, &mut buf);
        assert_eq!(r.read(0x1008, 0).unwrap(), &[] as &[u8]); // one past, len 0
        assert!(r.read(0x1009, 0).is_err()); // beyond even an empty access
    }
}
