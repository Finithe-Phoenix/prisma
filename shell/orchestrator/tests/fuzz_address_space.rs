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
