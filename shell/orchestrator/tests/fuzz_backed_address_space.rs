//! Property-based robustness fuzzing for the byte-backed `BackedAddressSpace`.
//!
//! This is the memory the guest actually reads and writes, so its load-bearing
//! safety property is strong: a `read`/`write` the type accepts must stay
//! strictly inside one region's bytes (never a host out-of-bounds), and
//! `mmap_anon` must never place a region that aliases an existing mapping — a
//! second region writing through the same address would corrupt the first.
//! These assert exactly that over arbitrary inputs.

use proptest::prelude::*;

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;

/// Build a space from a list of (base, len) read-write maps, ignoring rejects.
fn build(maps: &[(u64, u64)]) -> BackedAddressSpace {
    let mut space = BackedAddressSpace::new();
    for &(base, len) in maps {
        let _ = space.map(base, len, Protection::ReadWrite);
    }
    space
}

proptest! {
    /// The headline memory-safety property: a `read` the type accepts returns
    /// exactly `len` bytes from inside one mapping — it can never run off the
    /// end into a host out-of-bounds, and never panics for any input.
    #[test]
    fn accepted_read_returns_exact_len_and_never_panics(
        maps in prop::collection::vec((0u64..0x4000, 1u64..0x800), 0..16),
        addr in 0u64..0x6000,
        len in 0usize..0x400,
    ) {
        let space = build(&maps);
        let result = space.read(addr, len); // must not panic
        if let Ok(bytes) = result {
            prop_assert_eq!(bytes.len(), len);
        }
    }

    /// `write` then `read` of the same range round-trips — a write the type
    /// accepts lands exactly where read sees it.
    #[test]
    fn write_then_read_round_trips(
        maps in prop::collection::vec((0u64..0x4000, 1u64..0x800), 0..16),
        addr in 0u64..0x6000,
        payload in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut space = build(&maps);
        if space.write(addr, &payload).is_ok() {
            prop_assert_eq!(space.read(addr, payload.len()).unwrap(), &payload[..]);
        }
    }

    /// `mmap_anon` never aliases: a sentinel written through a freshly-mapped
    /// region leaves a pre-existing region's bytes untouched. (If the two
    /// aliased, the second write would corrupt the first.)
    #[test]
    fn mmap_anon_does_not_alias_existing_memory(
        first_base in 0x1_0000u64..0x2_0000,
        first_len in 0x100u64..0x1000,
        len in 1u64..0x1000,
    ) {
        let mut space = BackedAddressSpace::new();
        space.map(first_base, first_len, Protection::ReadWrite).unwrap();
        // Mark the first region with 0x11s.
        let marker = vec![0x11u8; usize::try_from(first_len).unwrap()];
        space.write(first_base, &marker).unwrap();
        // Place an anonymous mapping somewhere free and fill it with 0x22s.
        if let Ok(base) = space.mmap_anon(len, 0x1_0000, Protection::ReadWrite) {
            let other = vec![0x22u8; usize::try_from(len).unwrap()];
            space.write(base, &other).unwrap();
            // The first region's bytes are still all 0x11 — no aliasing.
            prop_assert!(space.read(first_base, usize::try_from(first_len).unwrap())
                .unwrap()
                .iter()
                .all(|&b| b == 0x11));
        }
    }

    /// `map` then `unmap` of each region drains the space to empty and never
    /// panics; the count tracks exactly.
    #[test]
    fn map_unmap_balance(maps in prop::collection::vec((0u64..0x4000, 1u64..0x800), 0..24)) {
        let mut space = build(&maps);
        let mut count = space.region_count();
        // Unmap by re-deriving bases from the accepted maps (dedup via a probe).
        let mut bases: Vec<u64> = maps.iter().map(|&(b, _)| b).collect();
        bases.sort_unstable();
        bases.dedup();
        for base in bases {
            if space.unmap(base).is_ok() {
                count -= 1;
                prop_assert_eq!(space.region_count(), count);
            }
        }
        prop_assert!(space.is_empty());
    }
}
