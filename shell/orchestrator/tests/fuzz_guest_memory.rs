//! Property-based robustness fuzzing for [`GuestRegion`] — the memory-safety
//! keystone every syscall handler routes guest pointers through.
//!
//! A guest pointer is arbitrary and untrusted, so the checked accessor must hold
//! for every (base, size, protection, addr, len): never panic, never read/write
//! outside the backing bytes, and keep `ensure_writable` consistent with what an
//! actual `write` would accept.

use proptest::prelude::*;

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;

const fn prot(i: u8) -> Protection {
    match i % 4 {
        0 => Protection::ReadOnly,
        1 => Protection::ReadExecute,
        2 => Protection::ReadWrite,
        _ => Protection::ReadWriteExecute,
    }
}

proptest! {
    /// read / write / ensure_writable never panic on an arbitrary region and
    /// access — the whole point of the checked accessor over untrusted pointers.
    #[test]
    fn accessors_never_panic(
        base in 0u64..0x4000,
        size in 0usize..256,
        addr in 0u64..0x8000,
        len in 0usize..512,
        p in any::<u8>(),
    ) {
        let mut bytes = vec![0u8; size];
        let mut region = GuestRegion::new(base, prot(p), &mut bytes);
        let _ = region.ensure_writable(addr, len);
        let _ = region.read(addr, len);
        let src = vec![0xABu8; len];
        let _ = region.write(addr, &src);
    }

    /// An accepted read returns exactly `len` bytes and the range was wholly
    /// inside the region — no over-read past the backing bytes.
    #[test]
    fn read_ok_is_exact_and_in_bounds(
        base in 0u64..0x4000,
        size in 1usize..256,
        addr in 0u64..0x8000,
        len in 0usize..512,
    ) {
        let mut bytes = vec![1u8; size];
        let region = GuestRegion::new(base, Protection::ReadWrite, &mut bytes);
        if let Ok(slice) = region.read(addr, len) {
            prop_assert_eq!(slice.len(), len);
            prop_assert!(addr >= base);
            prop_assert!(addr + len as u64 <= base + size as u64);
        }
    }

    /// `ensure_writable` accepts a range iff an actual `write` of that range
    /// would — the read syscall relies on this to validate before consuming an
    /// fd.
    #[test]
    fn ensure_writable_agrees_with_write(
        base in 0u64..0x4000,
        size in 0usize..256,
        addr in 0u64..0x8000,
        len in 0usize..256,
    ) {
        let mut a = vec![0u8; size];
        let ensure = {
            let region = GuestRegion::new(base, Protection::ReadWrite, &mut a);
            region.ensure_writable(addr, len).is_ok()
        };
        let mut b = vec![0u8; size];
        let wrote = {
            let mut region = GuestRegion::new(base, Protection::ReadWrite, &mut b);
            region.write(addr, &vec![7u8; len]).is_ok()
        };
        prop_assert_eq!(ensure, wrote);
    }

    /// A read-only region rejects every non-empty write but still allows reads.
    #[test]
    fn read_only_region_never_accepts_a_write(
        base in 0u64..0x4000,
        size in 0usize..256,
        addr in 0u64..0x8000,
        len in 1usize..256,
    ) {
        let mut bytes = vec![0u8; size];
        let mut region = GuestRegion::new(base, Protection::ReadOnly, &mut bytes);
        let src = vec![0u8; len];
        prop_assert!(region.write(addr, &src).is_err());
    }
}
