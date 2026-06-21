//! Guest virtual address space: an overlap-free set of mapped regions.
//!
//! The PE loader's `map_image` produces a flat buffer at one base; a real guest
//! needs more than that — the image, the stack, TLS blocks and later mmap'd
//! regions all coexist in one virtual space and must never alias. This module
//! is the bookkeeping layer: regions are mapped with overlap rejection, a guest
//! virtual address translates to its region + offset, and teardown is explicit
//! (`unmap` / `clear`) so no mapping survives a container restart per the
//! resource-discipline clause.

use thiserror::Error;

/// Page protection of a mapped guest region. Mirrors the PE section flags the
/// loader resolves (execute/read/write) and the W^X policy the JIT enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    ReadOnly,
    ReadExecute,
    ReadWrite,
    ReadWriteExecute,
}

impl Protection {
    /// Whether the guest may execute from a region with this protection.
    #[must_use]
    pub const fn is_executable(self) -> bool {
        matches!(self, Self::ReadExecute | Self::ReadWriteExecute)
    }

    /// Whether the guest may write to a region with this protection.
    #[must_use]
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::ReadWrite | Self::ReadWriteExecute)
    }
}

/// One contiguous mapped region of the guest address space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub base: u64,
    pub size: u64,
    pub prot: Protection,
    /// Human label (e.g. the PE section name or "stack"); diagnostics only.
    pub name: String,
}

impl Region {
    /// One-past-the-end address, saturating. A region that would overflow the
    /// space is treated as reaching the top rather than wrapping — a wrap would
    /// let it falsely appear disjoint from a low region.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.base.saturating_add(self.size)
    }

    /// Whether `addr` falls inside this region (`[base, base+size)`).
    #[must_use]
    pub const fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.end()
    }

    /// Whether this region intersects `[base, base+size)`.
    const fn overlaps(&self, base: u64, size: u64) -> bool {
        let end = base.saturating_add(size);
        self.base < end && base < self.end()
    }
}

/// A guest virtual address space: an overlap-free set of regions, kept sorted
/// by base address so translation is a binary search.
#[derive(Debug, Default)]
pub struct AddressSpace {
    regions: Vec<Region>,
}

impl AddressSpace {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Map a region. Rejects a zero-size region, an address-space overflow, and
    /// any overlap with an existing mapping — the guest must never have two
    /// mappings aliasing the same address. Regions stay sorted by base.
    pub fn map(
        &mut self,
        base: u64,
        size: u64,
        prot: Protection,
        name: impl Into<String>,
    ) -> Result<(), AddressSpaceError> {
        if size == 0 {
            return Err(AddressSpaceError::ZeroSize);
        }
        if base.checked_add(size).is_none() {
            return Err(AddressSpaceError::Overflow(base, size));
        }
        if let Some(r) = self.regions.iter().find(|r| r.overlaps(base, size)) {
            return Err(AddressSpaceError::Overlap {
                base,
                existing: r.base,
            });
        }
        let idx = self.regions.partition_point(|r| r.base < base);
        self.regions.insert(
            idx,
            Region {
                base,
                size,
                prot,
                name: name.into(),
            },
        );
        Ok(())
    }

    /// Translate a guest virtual address to its region and the offset within
    /// it, or `None` if the address is unmapped.
    #[must_use]
    pub fn translate(&self, addr: u64) -> Option<(&Region, u64)> {
        // Regions are sorted and disjoint, so the only candidate is the last
        // one whose base is <= addr.
        let idx = self.regions.partition_point(|r| r.base <= addr);
        let region = self.regions.get(idx.checked_sub(1)?)?;
        if region.contains(addr) {
            Some((region, addr - region.base))
        } else {
            None
        }
    }

    /// Validate that the whole guest range `[addr, addr+len)` is mapped — and,
    /// when `need_write`, writable — before the host dereferences a guest
    /// pointer argument. A buffer that runs off the end of its mapping, crosses
    /// an unmapped hole, wraps the address space, or points at read-only memory
    /// for a write must be rejected as a guest `EFAULT`, never let the host read
    /// or write out of bounds. A zero-length range is vacuously valid (nothing
    /// is dereferenced). The range may legitimately span several *contiguous*
    /// regions, so the check walks region by region rather than demanding one.
    ///
    /// # Errors
    /// [`RangeError::Overflow`] if `addr + len` wraps, [`RangeError::Unmapped`]
    /// if any byte of the range is not mapped, [`RangeError::NotWritable`] if
    /// `need_write` and any covering region is read-only.
    pub fn validate_range(&self, addr: u64, len: u64, need_write: bool) -> Result<(), RangeError> {
        if len == 0 {
            return Ok(());
        }
        let end = addr.checked_add(len).ok_or(RangeError::Overflow)?;
        let mut cursor = addr;
        while cursor < end {
            let (region, _) = self.translate(cursor).ok_or(RangeError::Unmapped)?;
            if need_write && !region.prot.is_writable() {
                return Err(RangeError::NotWritable);
            }
            // Regions are disjoint and sorted; jump to this region's end. A gap
            // before `end` makes the next translate fail (Unmapped); a
            // contiguous neighbour continues the walk.
            cursor = region.end();
        }
        Ok(())
    }

    /// Unmap the region based exactly at `base`, returning it. Explicit
    /// teardown: releasing guest memory drops the bookkeeping deterministically
    /// rather than leaking it across a restart.
    pub fn unmap(&mut self, base: u64) -> Result<Region, AddressSpaceError> {
        let idx = self
            .regions
            .iter()
            .position(|r| r.base == base)
            .ok_or(AddressSpaceError::NotMapped(base))?;
        Ok(self.regions.remove(idx))
    }

    /// Drop every mapping at once — used on container teardown so no guest
    /// mapping survives the restart.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// The mapped regions, sorted by base address.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressSpaceError {
    #[error("cannot map a zero-size region")]
    ZeroSize,

    #[error("region [{0:#x}, +{1:#x}) overflows the address space")]
    Overflow(u64, u64),

    #[error("region at {base:#x} overlaps an existing mapping at {existing:#x}")]
    Overlap { base: u64, existing: u64 },

    #[error("no region mapped at {0:#x}")]
    NotMapped(u64),
}

/// Why a guest pointer range failed validation (each maps to a guest `EFAULT`).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RangeError {
    #[error("range start + length overflows the address space")]
    Overflow,

    #[error("range covers unmapped guest memory")]
    Unmapped,

    #[error("range requires write permission but covers read-only memory")]
    NotWritable,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space() -> AddressSpace {
        let mut s = AddressSpace::new();
        s.map(0x1000, 0x1000, Protection::ReadExecute, ".text")
            .unwrap();
        s.map(0x3000, 0x2000, Protection::ReadWrite, ".data")
            .unwrap();
        s
    }

    #[test]
    fn map_keeps_regions_sorted_by_base() {
        let mut s = AddressSpace::new();
        // Insert out of order; the space must stay base-sorted.
        s.map(0x5000, 0x1000, Protection::ReadOnly, "c").unwrap();
        s.map(0x1000, 0x1000, Protection::ReadOnly, "a").unwrap();
        s.map(0x3000, 0x1000, Protection::ReadOnly, "b").unwrap();
        let bases: Vec<u64> = s.regions().iter().map(|r| r.base).collect();
        assert_eq!(bases, vec![0x1000, 0x3000, 0x5000]);
    }

    #[test]
    fn translate_resolves_addresses_to_region_and_offset() {
        let s = space();
        let (r, off) = s.translate(0x1000).expect("base of .text");
        assert_eq!(r.name, ".text");
        assert_eq!(off, 0);
        let (r, off) = s.translate(0x3100).expect("inside .data");
        assert_eq!(r.name, ".data");
        assert_eq!(off, 0x100);
        // Last byte of .data is mapped; one past the end is not.
        assert!(s.translate(0x4fff).is_some());
        assert!(s.translate(0x5000).is_none());
    }

    #[test]
    fn translate_rejects_gaps_and_below_first() {
        let s = space();
        assert!(s.translate(0x0500).is_none()); // below the first region
        assert!(s.translate(0x2000).is_none()); // in the gap between regions
        assert!(s.translate(0x2fff).is_none()); // just below .data
    }

    #[test]
    fn validate_range_accepts_in_bounds_and_rejects_overruns() {
        let s = space();
        // Fully inside .data, read.
        assert_eq!(s.validate_range(0x3000, 0x100, false), Ok(()));
        // Whole of .data, write (it is RW).
        assert_eq!(s.validate_range(0x3000, 0x2000, true), Ok(()));
        // Runs one byte off the end of .data into unmapped space.
        assert_eq!(
            s.validate_range(0x4000, 0x1001, false),
            Err(RangeError::Unmapped)
        );
        // Starts in the gap between regions.
        assert_eq!(
            s.validate_range(0x2000, 0x10, false),
            Err(RangeError::Unmapped)
        );
        // Spans .text -> gap (the second byte-run is unmapped).
        assert_eq!(
            s.validate_range(0x1f00, 0x200, false),
            Err(RangeError::Unmapped)
        );
    }

    #[test]
    fn validate_range_enforces_write_permission() {
        let s = space();
        // .text is ReadExecute — readable but not writable.
        assert_eq!(s.validate_range(0x1000, 0x10, false), Ok(()));
        assert_eq!(
            s.validate_range(0x1000, 0x10, true),
            Err(RangeError::NotWritable)
        );
    }

    #[test]
    fn validate_range_handles_zero_length_and_overflow() {
        let s = space();
        // Zero length dereferences nothing — vacuously valid even when unmapped.
        assert_eq!(s.validate_range(0x9999_9999, 0, true), Ok(()));
        // addr + len wrapping the address space is rejected, not wrapped.
        assert_eq!(
            s.validate_range(u64::MAX, 2, false),
            Err(RangeError::Overflow)
        );
    }

    #[test]
    fn validate_range_spans_contiguous_regions() {
        let mut s = AddressSpace::new();
        s.map(0x1000, 0x1000, Protection::ReadWrite, "a").unwrap();
        s.map(0x2000, 0x1000, Protection::ReadWrite, "b").unwrap(); // contiguous
                                                                    // A buffer straddling the a|b boundary is valid: no gap, both writable.
        assert_eq!(s.validate_range(0x1800, 0x1000, true), Ok(()));
        // If the second region were read-only the write must fail.
        let mut ro = AddressSpace::new();
        ro.map(0x1000, 0x1000, Protection::ReadWrite, "a").unwrap();
        ro.map(0x2000, 0x1000, Protection::ReadOnly, "b").unwrap();
        assert_eq!(
            ro.validate_range(0x1800, 0x1000, true),
            Err(RangeError::NotWritable)
        );
    }

    #[test]
    fn map_rejects_overlap() {
        let mut s = space();
        // Straddles the end of .text.
        assert!(matches!(
            s.map(0x1800, 0x1000, Protection::ReadOnly, "x"),
            Err(AddressSpaceError::Overlap { .. })
        ));
        // Exactly covers .data.
        assert!(matches!(
            s.map(0x3000, 0x2000, Protection::ReadOnly, "x"),
            Err(AddressSpaceError::Overlap { .. })
        ));
    }

    #[test]
    fn adjacent_regions_do_not_overlap() {
        let mut s = AddressSpace::new();
        s.map(0x1000, 0x1000, Protection::ReadOnly, "a").unwrap();
        // Starts exactly where the first ends — abutting, not overlapping.
        s.map(0x2000, 0x1000, Protection::ReadOnly, "b").unwrap();
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn map_rejects_zero_size_and_overflow() {
        let mut s = AddressSpace::new();
        assert!(matches!(
            s.map(0x1000, 0, Protection::ReadOnly, "z"),
            Err(AddressSpaceError::ZeroSize)
        ));
        assert!(matches!(
            s.map(u64::MAX, 2, Protection::ReadOnly, "o"),
            Err(AddressSpaceError::Overflow(..))
        ));
    }

    #[test]
    fn unmap_releases_a_region_and_is_exact() {
        let mut s = space();
        assert!(matches!(
            s.unmap(0x1234),
            Err(AddressSpaceError::NotMapped(_))
        ));
        let r = s.unmap(0x1000).expect("unmap .text");
        assert_eq!(r.name, ".text");
        assert_eq!(s.len(), 1);
        assert!(s.translate(0x1000).is_none());
    }

    #[test]
    fn clear_releases_the_whole_space() {
        let mut s = space();
        s.clear();
        assert!(s.is_empty());
        assert!(s.translate(0x1000).is_none());
    }

    #[test]
    fn protection_predicates() {
        assert!(Protection::ReadExecute.is_executable());
        assert!(!Protection::ReadWrite.is_executable());
        assert!(Protection::ReadWrite.is_writable());
        assert!(!Protection::ReadOnly.is_writable());
    }
}
