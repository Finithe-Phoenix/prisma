//! Property-based robustness fuzzing for the guest `AddressSpace`.
//!
//! Region bases/sizes can come from a PE's (untrusted) section table, so the
//! address space must stay coherent under arbitrary inputs: never panic, never
//! admit overlapping mappings, and keep translate consistent with what was
//! mapped. These properties assert exactly that over random sequences of ops.

use proptest::prelude::*;

use prisma_orchestrator::address_space::{AddressSpace, Protection};

/// Build a space from a list of (base, size) maps, ignoring rejects. The
/// invariant under test is that whatever the space accepts stays coherent.
fn build(maps: &[(u64, u64)]) -> AddressSpace {
    let mut space = AddressSpace::new();
    for (i, &(base, size)) in maps.iter().enumerate() {
        // Name each region by index; map() rejects zero-size / overflow / overlap.
        let _ = space.map(base, size, Protection::ReadOnly, format!("r{i}"));
    }
    space
}

proptest! {
    /// `map` over arbitrary inputs never panics, and accepted regions are
    /// pairwise non-overlapping (the core safety invariant).
    #[test]
    fn accepted_regions_never_overlap(maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..64)) {
        let space = build(&maps);
        let regions = space.regions();
        for (i, a) in regions.iter().enumerate() {
            for b in &regions[i + 1..] {
                // Disjoint: one ends at or before the other begins (saturating end).
                prop_assert!(a.end() <= b.base || b.end() <= a.base);
            }
        }
    }

    /// Regions stay sorted by base, so translate's binary search is valid.
    #[test]
    fn regions_stay_sorted(maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..64)) {
        let space = build(&maps);
        let bases: Vec<u64> = space.regions().iter().map(|r| r.base).collect();
        let mut sorted = bases.clone();
        sorted.sort_unstable();
        prop_assert_eq!(bases, sorted);
    }

    /// `translate` never panics on an arbitrary address, and when it resolves,
    /// the address really lies inside the returned region at the reported offset.
    #[test]
    fn translate_is_consistent(
        maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..32),
        probe in any::<u64>(),
    ) {
        let space = build(&maps);
        if let Some((region, offset)) = space.translate(probe) {
            prop_assert!(region.contains(probe));
            prop_assert_eq!(region.base + offset, probe);
        }
    }

    /// The base of every mapped region translates back to that region at
    /// offset 0 — map and translate agree.
    #[test]
    fn mapped_base_translates_to_itself(maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..32)) {
        let space = build(&maps);
        // Collect bases first (translate borrows the space immutably anyway).
        let bases: Vec<u64> = space.regions().iter().map(|r| r.base).collect();
        for base in bases {
            let (_region, offset) = space.translate(base).expect("a mapped base resolves");
            prop_assert_eq!(offset, 0);
        }
    }

    /// The load-bearing safety property of the syscall boundary: if
    /// `validate_range` accepts a non-empty range, then EVERY byte of it is
    /// mapped — an unmapped byte can never slip through to a host dereference.
    /// Bounded bases/sizes/probe so accepted ranges actually occur.
    #[test]
    fn validate_range_ok_implies_every_byte_mapped(
        maps in prop::collection::vec((0u64..0x4000, 0u64..0x1000), 0..16),
        addr in 0u64..0x4000,
        len in 0u64..0x800,
        need_write in any::<bool>(),
    ) {
        let space = build(&maps);
        if space.validate_range(addr, len, need_write).is_ok() {
            // Every byte in [addr, addr+len) resolves (no gap, no overrun).
            for off in 0..len {
                let byte = addr + off; // no overflow: Ok implies addr+len didn't wrap
                let (region, _) = space.translate(byte)
                    .expect("validate_range Ok but a byte is unmapped");
                if need_write {
                    prop_assert!(region.prot.is_writable());
                }
            }
        }
    }

    /// Contrapositive headline: an unmapped start address is never accepted for
    /// a non-empty range, whatever the permission flag.
    #[test]
    fn validate_range_rejects_unmapped_start(
        maps in prop::collection::vec((0u64..0x4000, 0u64..0x1000), 0..16),
        addr in 0u64..0x8000,
        len in 1u64..0x800,
    ) {
        let space = build(&maps);
        if space.translate(addr).is_none() {
            prop_assert!(space.validate_range(addr, len, false).is_err());
        }
    }

    /// A zero-length range dereferences nothing and is always valid; and
    /// `validate_range` never panics on arbitrary inputs.
    #[test]
    fn validate_range_zero_len_ok_and_never_panics(
        maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..16),
        addr in any::<u64>(),
        len in any::<u64>(),
        need_write in any::<bool>(),
    ) {
        let space = build(&maps);
        prop_assert!(space.validate_range(addr, 0, need_write).is_ok());
        let _ = space.validate_range(addr, len, need_write); // must not panic
    }

    /// Unmapping every region in turn drains the space to empty and never
    /// panics; each unmap removes exactly one region.
    #[test]
    fn unmap_drains_to_empty(maps in prop::collection::vec((any::<u64>(), any::<u64>()), 0..32)) {
        let mut space = build(&maps);
        let mut expected = space.len();
        let bases: Vec<u64> = space.regions().iter().map(|r| r.base).collect();
        for base in bases {
            prop_assert!(space.unmap(base).is_ok());
            expected -= 1;
            prop_assert_eq!(space.len(), expected);
        }
        prop_assert!(space.is_empty());
    }
}

proptest! {
    /// The allocator's load-bearing invariant: when `mmap_anon` succeeds it
    /// places the new region in genuinely free space — every region stays
    /// pairwise non-overlapping and the returned base is mapped + writable end
    /// to end. (Bounded inputs so a free gap actually exists.)
    #[test]
    fn mmap_anon_never_aliases(
        maps in prop::collection::vec((0u64..0x1_0000, 1u64..0x1000), 0..16),
        len in 1u64..0x1000,
        min_addr in 0u64..0x1_0000,
    ) {
        let mut space = build(&maps);
        if let Ok(base) = space.mmap_anon(len, min_addr, Protection::ReadWrite) {
            prop_assert!(base >= min_addr);
            space.validate_range(base, len, true).expect("mmap_anon region usable");
            let regions = space.regions();
            for (i, a) in regions.iter().enumerate() {
                for b in &regions[i + 1..] {
                    prop_assert!(a.end() <= b.base || b.end() <= a.base);
                }
            }
        }
    }

    /// When `find_free_range` returns a base, `[base, base+len)` overlaps no
    /// existing region — the reported placement is genuinely free.
    #[test]
    fn find_free_range_result_is_free(
        maps in prop::collection::vec((0u64..0x1_0000, 1u64..0x1000), 0..16),
        len in 1u64..0x1000,
        min_addr in 0u64..0x1_0000,
    ) {
        let space = build(&maps);
        if let Some(base) = space.find_free_range(len, min_addr) {
            prop_assert!(base >= min_addr);
            let end = base.checked_add(len).expect("bounded inputs do not overflow");
            for r in space.regions() {
                prop_assert!(end <= r.base || r.end() <= base);
            }
        }
    }

    /// `mprotect` changes only a region's protection, never the set of regions
    /// or their bounds.
    #[test]
    fn mprotect_preserves_layout(
        maps in prop::collection::vec((0u64..0x1_0000, 1u64..0x1000), 0..16),
    ) {
        let mut space = build(&maps);
        let before: Vec<(u64, u64)> = space.regions().iter().map(|r| (r.base, r.size)).collect();
        let bases: Vec<u64> = before.iter().map(|&(b, _)| b).collect();
        for base in bases {
            prop_assert!(space.mprotect(base, Protection::ReadWriteExecute).is_ok());
        }
        let after: Vec<(u64, u64)> = space.regions().iter().map(|r| (r.base, r.size)).collect();
        prop_assert_eq!(before, after);
    }
}
