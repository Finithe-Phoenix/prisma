//! Guest initial-thread stack allocation.
//!
//! A loaded image is not runnable until its initial thread has a stack. This
//! reserves a read/write region in the guest [`AddressSpace`] and reports its
//! top — the value the runtime seeds RSP with (`GuestThread::initial`). The
//! region is overlap- and overflow-checked by the address space, so a stack can
//! never alias the image or run off the end of the space.

use crate::address_space::{AddressSpace, AddressSpaceError, Protection};

/// Default initial-thread stack size (1 MiB).
///
/// Matches the typical Windows default reserve for the main thread.
pub const DEFAULT_STACK_SIZE: u64 = 1 << 20;

/// A mapped guest stack: its base (lowest address) and one-past-the-top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestStack {
    pub base: u64,
    /// One-past-the-highest address; RSP starts here and grows down.
    pub top: u64,
}

impl GuestStack {
    /// Size of the stack region in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.top - self.base
    }
}

/// Map a `size`-byte read/write stack at `base` into `space`, returning its
/// descriptor.
///
/// Fails (via the address space) on a zero size, an overflow, or an overlap
/// with an existing mapping — the stack must not alias the image.
pub fn map_stack(
    space: &mut AddressSpace,
    base: u64,
    size: u64,
) -> Result<GuestStack, AddressSpaceError> {
    space.map(base, size, Protection::ReadWrite, "stack")?;
    // map() already rejected base + size overflow, so this cannot wrap.
    Ok(GuestStack {
        base,
        top: base + size,
    })
}

/// Map a default-sized ([`DEFAULT_STACK_SIZE`]) stack at `base`.
pub fn map_default_stack(
    space: &mut AddressSpace,
    base: u64,
) -> Result<GuestStack, AddressSpaceError> {
    map_stack(space, base, DEFAULT_STACK_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_stack_and_reports_top() {
        let mut space = AddressSpace::new();
        let stack = map_stack(&mut space, 0x10_0000, 0x4000).expect("map");
        assert_eq!(stack.base, 0x10_0000);
        assert_eq!(stack.top, 0x10_4000);
        assert_eq!(stack.size(), 0x4000);
        // The whole region is mapped read/write in the space.
        let (region, _) = space.translate(0x10_0000).expect("base mapped");
        assert!(region.prot.is_writable());
        assert!(space.translate(0x10_3FFF).is_some()); // last byte
    }

    #[test]
    fn default_stack_is_one_mib() {
        let mut space = AddressSpace::new();
        let stack = map_default_stack(&mut space, 0x20_0000).unwrap();
        assert_eq!(stack.size(), 1 << 20);
    }

    #[test]
    fn stack_cannot_overlap_an_existing_mapping() {
        let mut space = AddressSpace::new();
        // Pretend the image occupies [0x40_0000, +0x10000).
        space
            .map(0x40_0000, 0x10000, Protection::ReadExecute, "image")
            .unwrap();
        // A stack straddling it is rejected — no aliasing the image.
        assert!(matches!(
            map_stack(&mut space, 0x40_8000, 0x10000),
            Err(AddressSpaceError::Overlap { .. })
        ));
    }

    #[test]
    fn zero_size_and_overflow_are_rejected() {
        let mut space = AddressSpace::new();
        assert!(map_stack(&mut space, 0x1000, 0).is_err());
        assert!(map_stack(&mut space, u64::MAX, 0x1000).is_err());
    }
}
