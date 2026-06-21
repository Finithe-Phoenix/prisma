//! A common interface for pointer-checked guest memory access.
//!
//! A syscall handler needs to `read`/`write`/`ensure_writable` guest memory, and
//! two types provide exactly that: the single-window [`GuestRegion`] the
//! dispatcher passes today, and the byte-backed [`BackedAddressSpace`] that owns
//! the whole growable address space (RFC 0019). This trait abstracts over both,
//! so a handler can be written once and run against either.
//!
//! It is the staged path to unifying the dispatcher's memory argument: handlers
//! migrate to `impl GuestMem` one module at a time (their tests keep using
//! `GuestRegion`, which implements it), and only the final step swaps what the
//! dispatcher actually holds — no big-bang rewrite.

use crate::address_space::RangeError;
use crate::backed_address_space::BackedAddressSpace;
use crate::guest_memory::GuestRegion;

/// Pointer-checked guest memory: every access is range-checked, so a bad guest
/// pointer is a `RangeError` (the caller's `EFAULT`), never a host out-of-bounds.
pub trait GuestMem {
    /// Borrow `len` bytes at guest address `addr`.
    ///
    /// # Errors
    /// [`RangeError`] if the range is not entirely readable guest memory.
    fn read(&self, addr: u64, len: usize) -> Result<&[u8], RangeError>;

    /// Copy `src` to guest address `addr`.
    ///
    /// # Errors
    /// [`RangeError`] if the range is not entirely writable guest memory.
    fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), RangeError>;

    /// Validate that `[addr, addr+len)` is writable without copying — the
    /// read-before-write guard a syscall uses so a bad destination faults before
    /// any input is consumed.
    ///
    /// # Errors
    /// [`RangeError`] if the range is not entirely writable guest memory.
    fn ensure_writable(&self, addr: u64, len: usize) -> Result<(), RangeError>;
}

// Each impl forwards to the type's inherent method; inherent methods take
// priority over trait methods in resolution, so `self.read(..)` is the concrete
// implementation, not a recursive call back into the trait.
impl GuestMem for GuestRegion<'_> {
    fn read(&self, addr: u64, len: usize) -> Result<&[u8], RangeError> {
        self.read(addr, len)
    }
    fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), RangeError> {
        self.write(addr, src)
    }
    fn ensure_writable(&self, addr: u64, len: usize) -> Result<(), RangeError> {
        self.ensure_writable(addr, len)
    }
}

impl GuestMem for BackedAddressSpace {
    fn read(&self, addr: u64, len: usize) -> Result<&[u8], RangeError> {
        self.read(addr, len)
    }
    fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), RangeError> {
        self.write(addr, src)
    }
    fn ensure_writable(&self, addr: u64, len: usize) -> Result<(), RangeError> {
        self.ensure_writable(addr, len)
    }
}

#[cfg(test)]
mod tests {
    use super::GuestMem;
    use crate::address_space::Protection;
    use crate::backed_address_space::BackedAddressSpace;
    use crate::guest_memory::GuestRegion;

    /// A handler-style routine written once against the trait.
    fn store_then_load<M: GuestMem>(mem: &mut M, addr: u64, src: &[u8]) -> Vec<u8> {
        mem.ensure_writable(addr, src.len()).unwrap();
        mem.write(addr, src).unwrap();
        mem.read(addr, src.len()).unwrap().to_vec()
    }

    #[test]
    fn the_same_generic_code_runs_against_a_guest_region() {
        let mut buf = [0u8; 16];
        let mut region = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert_eq!(
            store_then_load(&mut region, 0x1004, &[1, 2, 3]),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn the_same_generic_code_runs_against_a_backed_address_space() {
        let mut space = BackedAddressSpace::new();
        space.map(0x1000, 0x1000, Protection::ReadWrite).unwrap();
        assert_eq!(
            store_then_load(&mut space, 0x1004, &[4, 5, 6]),
            vec![4, 5, 6]
        );
    }

    #[test]
    fn a_read_only_region_rejects_writes_through_the_trait() {
        let mut buf = [0u8; 8];
        let mut region = GuestRegion::new(0x1000, Protection::ReadOnly, &mut buf);
        assert!(GuestMem::ensure_writable(&region, 0x1000, 1).is_err());
        assert!(GuestMem::write(&mut region, 0x1000, &[1]).is_err());
    }
}
