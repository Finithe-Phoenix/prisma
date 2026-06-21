//! Memory-management syscalls: `brk`, `mmap`, `munmap`, `mprotect`.
//!
//! These operate on a [`BackedAddressSpace`] — the one component that owns both
//! the region map and the bytes, so a mapping can actually be created or grown.
//! They are *pure handlers*: the address space and the program break are
//! caller-owned (passed in), so the module stays decoupled from the dispatcher.
//! Unifying `dispatch`'s memory argument so these can be routed is the
//! review-gated step of RFC 0019; the logic itself is correct and tested here.
//!
//! Resource discipline: `munmap` and a heap-shrinking `brk` release their bytes
//! through [`BackedAddressSpace::unmap`] / `resize`, deterministically.

use prisma_orchestrator::address_space::{AddressSpaceError, Protection};
use prisma_orchestrator::backed_address_space::BackedAddressSpace;

/// `mmap` protection bits (`PROT_*`). `PROT_READ` (0x1) is implied by every
/// modelled region, so only write/exec select the protection.
const PROT_WRITE: i32 = 0x2;
const PROT_EXEC: i32 = 0x4;

/// The lowest address `mmap` will place a hint-less mapping at — above the NULL
/// page so a guest never receives 0 as a valid mapping.
const MMAP_MIN_ADDR: u64 = 0x1_0000;

/// Why a memory-management syscall failed (each maps to a guest errno at routing
/// time).
#[derive(Debug, PartialEq, Eq)]
pub enum MemError {
    /// No free address-space range / out of memory — guest `ENOMEM`.
    NoMemory,
    /// An argument is invalid (e.g. unmapping an address that is not a mapping
    /// base) — guest `EINVAL`.
    Invalid,
}

/// Translate `PROT_*` bits to a region [`Protection`]. `PROT_NONE` and any
/// bit-set without write/exec map to the nearest modelled protection
/// (read-only); the model has no no-access region.
fn protection_from_prot(prot: i32) -> Protection {
    let w = prot & PROT_WRITE != 0;
    let x = prot & PROT_EXEC != 0;
    match (w, x) {
        (true, true) => Protection::ReadWriteExecute,
        (true, false) => Protection::ReadWrite,
        (false, true) => Protection::ReadExecute,
        (false, false) => Protection::ReadOnly,
    }
}

/// `brk(addr)`: query or move the program break (the top of the heap).
///
/// `addr` of 0 (or any value below `heap_base`) is a query: the current break is
/// returned unchanged — this is how a runtime reads the initial break. Otherwise
/// the single heap region based at `heap_base` is grown/shrunk so its top is
/// `addr`; on success the break advances to `addr`, on failure (a collision with
/// the next mapping) the break is returned unchanged, exactly as Linux `brk`
/// does. The break value is caller-owned (`current_break`).
pub fn brk(
    mem: &mut BackedAddressSpace,
    heap_base: u64,
    current_break: &mut u64,
    addr: u64,
) -> u64 {
    if addr < heap_base {
        return *current_break; // query / invalid shrink below the heap base
    }
    let new_size = addr - heap_base;
    let outcome = if new_size == 0 {
        // The heap is emptied: drop the region if one exists.
        if *current_break > heap_base {
            let _ = mem.unmap(heap_base);
        }
        Ok(())
    } else if *current_break <= heap_base {
        // First allocation: no heap region yet.
        mem.map(heap_base, new_size, Protection::ReadWrite)
    } else {
        mem.resize(heap_base, new_size)
    };
    if outcome.is_ok() {
        *current_break = addr;
    }
    *current_break
}

/// `mmap(addr, len, prot, ...)`: place an anonymous mapping of `len` bytes and
/// return its base. A fixed `addr` hint is honoured when free; otherwise (the
/// common case) the lowest free address at or above [`MMAP_MIN_ADDR`] is used.
///
/// The model serves anonymous mappings (what `malloc` needs); file-backed
/// `mmap` is out of scope.
///
/// # Errors
/// [`MemError::Invalid`] for a zero length, [`MemError::NoMemory`] if no free
/// range fits.
pub fn mmap(mem: &mut BackedAddressSpace, addr: u64, len: u64, prot: i32) -> Result<u64, MemError> {
    if len == 0 {
        return Err(MemError::Invalid);
    }
    let protection = protection_from_prot(prot);
    // A non-zero hint that is actually free is honoured; otherwise fall back to
    // a first-fit placement above the NULL page.
    if addr >= MMAP_MIN_ADDR && mem.map(addr, len, protection).is_ok() {
        return Ok(addr);
    }
    mem.mmap_anon(len, MMAP_MIN_ADDR, protection)
        .map_err(|e| match e {
            AddressSpaceError::NoSpace { .. } => MemError::NoMemory,
            _ => MemError::Invalid,
        })
}

/// `munmap(addr, len)`: unmap the region based at `addr`, releasing its bytes.
/// The model unmaps whole regions (a partial-range unmap is not supported), so
/// `addr` must be a mapping base.
///
/// # Errors
/// [`MemError::Invalid`] if `addr` is not the base of a mapping.
pub fn munmap(mem: &mut BackedAddressSpace, addr: u64) -> Result<(), MemError> {
    mem.unmap(addr).map_err(|_| MemError::Invalid)
}

/// `mprotect(addr, len, prot)`: change the protection of the region based at
/// `addr` (whole-region; a sub-range split is not modelled).
///
/// # Errors
/// [`MemError::Invalid`] if `addr` is not the base of a mapping.
pub fn mprotect(mem: &mut BackedAddressSpace, addr: u64, prot: i32) -> Result<(), MemError> {
    mem.mprotect(addr, protection_from_prot(prot))
        .map(|_prev| ())
        .map_err(|_| MemError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::{brk, mmap, mprotect, munmap, MemError, MMAP_MIN_ADDR};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::backed_address_space::BackedAddressSpace;

    const HEAP_BASE: u64 = 0x10_0000;

    #[test]
    fn brk_queries_then_grows_then_shrinks_the_heap() {
        let mut mem = BackedAddressSpace::new();
        let mut brk_val = HEAP_BASE;
        // brk(0) is a query: the initial break is the heap base.
        assert_eq!(brk(&mut mem, HEAP_BASE, &mut brk_val, 0), HEAP_BASE);
        assert!(mem.is_empty());
        // Grow the heap by 0x1000: the region is created and writable.
        let top = HEAP_BASE + 0x1000;
        assert_eq!(brk(&mut mem, HEAP_BASE, &mut brk_val, top), top);
        mem.write(HEAP_BASE, &[1, 2, 3]).unwrap();
        // Grow further: the region is resized in place.
        let top2 = HEAP_BASE + 0x4000;
        assert_eq!(brk(&mut mem, HEAP_BASE, &mut brk_val, top2), top2);
        assert_eq!(mem.read(HEAP_BASE, 3).unwrap(), &[1, 2, 3]); // data preserved
                                                                 // Shrink back to empty: the heap region is released.
        assert_eq!(brk(&mut mem, HEAP_BASE, &mut brk_val, HEAP_BASE), HEAP_BASE);
        assert!(mem.is_empty());
    }

    #[test]
    fn brk_failure_leaves_the_break_unchanged() {
        let mut mem = BackedAddressSpace::new();
        // A mapping right above the heap blocks growth past it.
        mem.map(HEAP_BASE + 0x1000, 0x1000, Protection::ReadOnly)
            .unwrap();
        let mut brk_val = HEAP_BASE;
        // Grow into the blocker: brk returns the unchanged break.
        let asked = HEAP_BASE + 0x2000;
        assert_eq!(brk(&mut mem, HEAP_BASE, &mut brk_val, asked), HEAP_BASE);
        assert_eq!(brk_val, HEAP_BASE);
    }

    #[test]
    fn mmap_places_anonymous_writable_memory() {
        let mut mem = BackedAddressSpace::new();
        let base = mmap(&mut mem, 0, 0x2000, 0x1 | 0x2).unwrap(); // PROT_READ|WRITE
        assert!(base >= MMAP_MIN_ADDR);
        mem.write(base, &[7; 4]).unwrap();
        assert_eq!(mem.read(base, 4).unwrap(), &[7, 7, 7, 7]);
        // A zero length is EINVAL.
        assert_eq!(mmap(&mut mem, 0, 0, 0x3), Err(MemError::Invalid));
    }

    #[test]
    fn munmap_releases_and_rejects_a_non_base() {
        let mut mem = BackedAddressSpace::new();
        let base = mmap(&mut mem, 0, 0x1000, 0x3).unwrap();
        assert!(munmap(&mut mem, base).is_ok());
        assert!(mem.is_empty());
        // Unmapping something that is not a mapping base is EINVAL.
        assert_eq!(munmap(&mut mem, 0xDEAD_0000), Err(MemError::Invalid));
    }

    #[test]
    fn mprotect_flips_writability_and_rejects_unmapped() {
        let mut mem = BackedAddressSpace::new();
        let base = mmap(&mut mem, 0, 0x1000, 0x3).unwrap(); // RW
                                                            // Drop to read-only (PROT_READ): a write now faults inside the space.
        mprotect(&mut mem, base, 0x1).unwrap();
        assert!(mem.write(base, &[1]).is_err());
        assert_eq!(mprotect(&mut mem, 0x1234, 0x1), Err(MemError::Invalid));
    }
}
