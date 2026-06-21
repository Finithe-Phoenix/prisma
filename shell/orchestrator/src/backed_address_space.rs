//! Backed guest address space: the region map *together with* the bytes behind
//! it.
//!
//! [`crate::address_space::AddressSpace`] tracks region metadata (base / size /
//! protection) but owns no memory; [`crate::guest_memory::GuestRegion`] is a
//! single byte window. Neither can model a guest whose address space *grows* at
//! runtime — `brk` extends the heap, `mmap` adds anonymous regions — because
//! that needs both the placement bookkeeping and freshly-allocated backing
//! bytes in one place. This type is that place: an overlap-free, base-sorted set
//! of regions, each carrying its own `Vec<u8>`.
//!
//! It is the reversible core of RFC 0019: a self-contained component that `brk`/
//! `mmap` handlers (and, later, a unified `dispatch` memory argument) build on.
//! Wiring it into the dispatcher and executor is deliberately out of scope here.
//!
//! Resource discipline (the standing clause): a region's bytes are owned by its
//! entry and freed the moment [`BackedAddressSpace::unmap`] removes it or
//! [`BackedAddressSpace::clear`] drops the whole space — guest memory is released
//! deterministically, never leaked across a restart.

use crate::address_space::{AddressSpaceError, Protection, RangeError};

/// One mapped region and the bytes behind it. `size` is implicit in
/// `bytes.len()`, so the map and the backing can never disagree.
#[derive(Debug)]
struct MemRegion {
    base: u64,
    prot: Protection,
    bytes: Vec<u8>,
}

impl MemRegion {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn end(&self) -> u64 {
        self.base.saturating_add(self.size())
    }

    fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.end()
    }
}

/// A guest address space that owns its backing memory: an overlap-free,
/// base-sorted set of byte-backed regions that can be mapped, unmapped, grown,
/// shrunk and re-protected at runtime.
#[derive(Debug, Default)]
pub struct BackedAddressSpace {
    regions: Vec<MemRegion>,
}

impl BackedAddressSpace {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Index of the region containing `addr`, if any. Regions are sorted and
    /// disjoint, so the only candidate is the last whose base is `<= addr`.
    fn region_idx(&self, addr: u64) -> Option<usize> {
        let idx = self.regions.partition_point(|r| r.base <= addr);
        let i = idx.checked_sub(1)?;
        if self.regions[i].contains(addr) {
            Some(i)
        } else {
            None
        }
    }

    /// Insert a region with the given backing `bytes`, rejecting an empty
    /// region, an address-space overflow, and any overlap. The shared core of
    /// [`map`](Self::map) and [`map_with_bytes`](Self::map_with_bytes).
    fn insert_region(
        &mut self,
        base: u64,
        prot: Protection,
        bytes: Vec<u8>,
    ) -> Result<(), AddressSpaceError> {
        let len = bytes.len() as u64;
        if len == 0 {
            return Err(AddressSpaceError::ZeroSize);
        }
        let end = base
            .checked_add(len)
            .ok_or(AddressSpaceError::Overflow(base, len))?;
        if let Some(r) = self.regions.iter().find(|r| r.base < end && base < r.end()) {
            return Err(AddressSpaceError::Overlap {
                base,
                existing: r.base,
            });
        }
        let idx = self.regions.partition_point(|r| r.base < base);
        self.regions.insert(idx, MemRegion { base, prot, bytes });
        Ok(())
    }

    /// Map `len` zeroed bytes at `base` with `prot`. Rejects a zero length, an
    /// address-space overflow, and any overlap with an existing mapping.
    ///
    /// # Errors
    /// [`AddressSpaceError::ZeroSize`], [`AddressSpaceError::Overflow`], or
    /// [`AddressSpaceError::Overlap`].
    pub fn map(&mut self, base: u64, len: u64, prot: Protection) -> Result<(), AddressSpaceError> {
        let size = usize::try_from(len).map_err(|_| AddressSpaceError::Overflow(base, len))?;
        self.insert_region(base, prot, vec![0u8; size])
    }

    /// Map a region at `base` initialised with `bytes` (size = `bytes.len()`) —
    /// how the PE loader installs a section's contents into the guest space.
    /// Same rejection rules as [`map`](Self::map).
    ///
    /// # Errors
    /// [`AddressSpaceError::ZeroSize`], [`AddressSpaceError::Overflow`], or
    /// [`AddressSpaceError::Overlap`].
    pub fn map_with_bytes(
        &mut self,
        base: u64,
        bytes: &[u8],
        prot: Protection,
    ) -> Result<(), AddressSpaceError> {
        self.insert_region(base, prot, bytes.to_vec())
    }

    /// Unmap the region based exactly at `base`, dropping its bytes. Explicit,
    /// deterministic release per the resource-discipline clause.
    ///
    /// # Errors
    /// [`AddressSpaceError::NotMapped`] if no region begins at `base`.
    pub fn unmap(&mut self, base: u64) -> Result<(), AddressSpaceError> {
        let idx = self
            .regions
            .iter()
            .position(|r| r.base == base)
            .ok_or(AddressSpaceError::NotMapped(base))?;
        self.regions.remove(idx); // drops the Vec<u8> — bytes freed here
        Ok(())
    }

    /// Borrow `len` bytes at guest address `addr`. The range must lie within a
    /// single mapped region.
    ///
    /// # Errors
    /// [`RangeError::Unmapped`] if `addr` is unmapped or the range runs off the
    /// region end, [`RangeError::Overflow`] if `addr + len` wraps `usize`.
    pub fn read(&self, addr: u64, len: usize) -> Result<&[u8], RangeError> {
        let i = self.region_idx(addr).ok_or(RangeError::Unmapped)?;
        let r = &self.regions[i];
        let off = usize::try_from(addr - r.base).map_err(|_| RangeError::Overflow)?;
        let end = off.checked_add(len).ok_or(RangeError::Overflow)?;
        if end > r.bytes.len() {
            return Err(RangeError::Unmapped); // runs past the region
        }
        Ok(&r.bytes[off..end])
    }

    /// Copy `src` to guest address `addr`. The range must lie within a single
    /// writable region.
    ///
    /// # Errors
    /// [`RangeError::Unmapped`] / [`RangeError::Overflow`] as [`read`], plus
    /// [`RangeError::NotWritable`] if the covering region is read-only.
    pub fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), RangeError> {
        let i = self.region_idx(addr).ok_or(RangeError::Unmapped)?;
        let r = &mut self.regions[i];
        if !r.prot.is_writable() {
            return Err(RangeError::NotWritable);
        }
        let off = usize::try_from(addr - r.base).map_err(|_| RangeError::Overflow)?;
        let end = off.checked_add(src.len()).ok_or(RangeError::Overflow)?;
        if end > r.bytes.len() {
            return Err(RangeError::Unmapped);
        }
        r.bytes[off..end].copy_from_slice(src);
        Ok(())
    }

    /// Validate that `[addr, addr+len)` is a writable range within a single
    /// region, without copying — the `read`-before-`write` guard a syscall like
    /// `read(2)` uses so a bad destination faults before any bytes are consumed.
    /// Mirrors [`crate::guest_memory::GuestRegion::ensure_writable`].
    ///
    /// # Errors
    /// [`RangeError::Unmapped`] if `addr` is unmapped or the range runs off the
    /// region, [`RangeError::Overflow`] on a `usize` wrap, or
    /// [`RangeError::NotWritable`] if the covering region is read-only.
    pub fn ensure_writable(&self, addr: u64, len: usize) -> Result<(), RangeError> {
        let i = self.region_idx(addr).ok_or(RangeError::Unmapped)?;
        let r = &self.regions[i];
        if !r.prot.is_writable() {
            return Err(RangeError::NotWritable);
        }
        let off = usize::try_from(addr - r.base).map_err(|_| RangeError::Overflow)?;
        let end = off.checked_add(len).ok_or(RangeError::Overflow)?;
        if end > r.bytes.len() {
            return Err(RangeError::Unmapped);
        }
        Ok(())
    }

    /// The lowest base at or above `min_addr` where `[base, base+len)` fits
    /// without overlapping a mapping (first-fit) — a hint-less `mmap` placement.
    #[must_use]
    pub fn find_free_range(&self, len: u64, min_addr: u64) -> Option<u64> {
        if len == 0 {
            return None;
        }
        let mut cursor = min_addr;
        for r in &self.regions {
            if r.end() <= cursor {
                continue;
            }
            if r.base >= cursor && r.base - cursor >= len {
                return cursor.checked_add(len).map(|_| cursor);
            }
            cursor = cursor.max(r.end());
        }
        cursor.checked_add(len).map(|_| cursor)
    }

    /// Map an anonymous region of `len` zeroed bytes at the lowest free address
    /// at or above `min_addr`, returning its base.
    ///
    /// # Errors
    /// [`AddressSpaceError::ZeroSize`] or [`AddressSpaceError::NoSpace`].
    pub fn mmap_anon(
        &mut self,
        len: u64,
        min_addr: u64,
        prot: Protection,
    ) -> Result<u64, AddressSpaceError> {
        if len == 0 {
            return Err(AddressSpaceError::ZeroSize);
        }
        let base = self
            .find_free_range(len, min_addr)
            .ok_or(AddressSpaceError::NoSpace { len })?;
        self.map(base, len, prot)?;
        Ok(base)
    }

    /// Resize the region at `base` to `new_size` bytes — the `brk` primitive.
    /// Growing zero-fills the new tail and must not collide with the next
    /// mapping; shrinking truncates (freeing the tail bytes).
    ///
    /// # Errors
    /// [`AddressSpaceError::ZeroSize`], [`AddressSpaceError::NotMapped`],
    /// [`AddressSpaceError::Overflow`], or [`AddressSpaceError::Overlap`].
    pub fn resize(&mut self, base: u64, new_size: u64) -> Result<(), AddressSpaceError> {
        if new_size == 0 {
            return Err(AddressSpaceError::ZeroSize);
        }
        let idx = self
            .regions
            .iter()
            .position(|r| r.base == base)
            .ok_or(AddressSpaceError::NotMapped(base))?;
        let new_end = base
            .checked_add(new_size)
            .ok_or(AddressSpaceError::Overflow(base, new_size))?;
        if let Some(next) = self.regions.get(idx + 1) {
            if new_end > next.base {
                return Err(AddressSpaceError::Overlap {
                    base,
                    existing: next.base,
                });
            }
        }
        let size =
            usize::try_from(new_size).map_err(|_| AddressSpaceError::Overflow(base, new_size))?;
        self.regions[idx].bytes.resize(size, 0); // grow zero-fills, shrink frees
        Ok(())
    }

    /// Re-protect the region based exactly at `base`, returning the prior
    /// protection.
    ///
    /// # Errors
    /// [`AddressSpaceError::NotMapped`] if no region begins at `base`.
    pub fn mprotect(
        &mut self,
        base: u64,
        prot: Protection,
    ) -> Result<Protection, AddressSpaceError> {
        let r = self
            .regions
            .iter_mut()
            .find(|r| r.base == base)
            .ok_or(AddressSpaceError::NotMapped(base))?;
        let prev = r.prot;
        r.prot = prot;
        Ok(prev)
    }

    /// Drop every mapping (and its bytes) — container teardown.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Number of mapped regions.
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Whether the space has no mappings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::BackedAddressSpace;
    use crate::address_space::{AddressSpaceError, Protection, RangeError};

    fn space() -> BackedAddressSpace {
        let mut s = BackedAddressSpace::new();
        s.map(0x1000, 0x1000, Protection::ReadExecute).unwrap();
        s.map(0x3000, 0x2000, Protection::ReadWrite).unwrap();
        s
    }

    #[test]
    fn map_allocates_zeroed_bytes_and_rejects_overlap() {
        let s = space();
        // Freshly mapped bytes are zero.
        assert_eq!(s.read(0x1000, 4).unwrap(), &[0, 0, 0, 0]);
        // Overlapping the existing .text is rejected.
        let mut s2 = space();
        assert!(matches!(
            s2.map(0x1800, 0x1000, Protection::ReadWrite),
            Err(AddressSpaceError::Overlap { .. })
        ));
    }

    #[test]
    fn write_then_read_round_trips_within_a_region() {
        let mut s = space();
        s.write(0x3010, &[1, 2, 3, 4]).unwrap();
        assert_eq!(s.read(0x3010, 4).unwrap(), &[1, 2, 3, 4]);
        // A write to read-only .text faults.
        assert_eq!(s.write(0x1000, &[9]), Err(RangeError::NotWritable));
        // A read that runs off the region end is Unmapped, not a host overrun.
        assert_eq!(s.read(0x1ffc, 8), Err(RangeError::Unmapped));
    }

    #[test]
    fn unmap_frees_bytes_and_translate_stops_resolving() {
        let mut s = space();
        assert_eq!(s.region_count(), 2);
        s.unmap(0x1000).unwrap();
        assert_eq!(s.region_count(), 1);
        assert_eq!(s.read(0x1000, 1), Err(RangeError::Unmapped));
        assert!(matches!(
            s.unmap(0x1000),
            Err(AddressSpaceError::NotMapped(0x1000))
        ));
    }

    #[test]
    fn mmap_anon_places_writable_bytes_in_free_space() {
        let mut s = space();
        // Floor at .text base targets the gap [0x2000, 0x3000).
        let base = s.mmap_anon(0x1000, 0x1000, Protection::ReadWrite).unwrap();
        assert_eq!(base, 0x2000);
        s.write(0x2000, &[7, 7]).unwrap();
        assert_eq!(s.read(0x2000, 2).unwrap(), &[7, 7]);
    }

    #[test]
    fn resize_grows_zero_filled_then_shrinks_freeing_the_tail() {
        let mut s = space();
        // Grow .text into the gap; the new tail is zero and readable.
        s.resize(0x1000, 0x1800).unwrap();
        assert!(s.read(0x2000, 0x800).unwrap().iter().all(|&b| b == 0));
        // Growing into .data collides.
        assert!(matches!(
            s.resize(0x1000, 0x2001),
            Err(AddressSpaceError::Overlap { .. })
        ));
        // Shrink frees the tail: a read there is now Unmapped.
        s.resize(0x1000, 0x800).unwrap();
        assert_eq!(s.read(0x1800, 1), Err(RangeError::Unmapped));
    }

    #[test]
    fn mprotect_flips_writability() {
        let mut s = space();
        let prev = s.mprotect(0x3000, Protection::ReadOnly).unwrap();
        assert_eq!(prev, Protection::ReadWrite);
        assert_eq!(s.write(0x3000, &[1]), Err(RangeError::NotWritable));
    }

    #[test]
    fn map_with_bytes_installs_section_content() {
        let mut s = BackedAddressSpace::new();
        // Install a ".rodata"-like section with real bytes at a fixed base.
        s.map_with_bytes(0x4_0000, &[0xDE, 0xAD, 0xBE, 0xEF], Protection::ReadOnly)
            .unwrap();
        assert_eq!(s.read(0x4_0000, 4).unwrap(), &[0xDE, 0xAD, 0xBE, 0xEF]);
        // It is read-only: a write faults.
        assert_eq!(s.write(0x4_0000, &[0]), Err(RangeError::NotWritable));
        // Empty content is rejected (zero-size region).
        assert!(matches!(
            s.map_with_bytes(0x5_0000, &[], Protection::ReadWrite),
            Err(AddressSpaceError::ZeroSize)
        ));
    }

    #[test]
    fn ensure_writable_matches_write_acceptance_without_copying() {
        let s = space(); // .text RX [0x1000,0x2000), .data RW [0x3000,0x5000)
                         // A writable range inside .data validates.
        assert!(s.ensure_writable(0x3000, 0x10).is_ok());
        // Read-only .text is NotWritable.
        assert_eq!(s.ensure_writable(0x1000, 1), Err(RangeError::NotWritable));
        // A range running off the region end is Unmapped, not a host overrun.
        assert_eq!(s.ensure_writable(0x4ffc, 8), Err(RangeError::Unmapped));
        // An unmapped address is Unmapped.
        assert_eq!(s.ensure_writable(0x9000, 1), Err(RangeError::Unmapped));
    }

    #[test]
    fn clear_drops_everything() {
        let mut s = space();
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.read(0x1000, 1), Err(RangeError::Unmapped));
    }
}
