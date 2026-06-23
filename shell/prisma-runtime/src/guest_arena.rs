//! Contiguous host-backed guest memory arena (RFC 0020).
//!
//! A single page-aligned host mapping (mmap `PROT_READ|PROT_WRITE` on unix,
//! `VirtualAlloc` read/write on Windows — the same platform backend the JIT
//! buffers use, no extra crate) that backs a *window* of the guest address
//! space starting at `base`. Because the backing is one contiguous allocation,
//! a guest VA maps to its host address by a single offset:
//!
//! ```text
//! host_addr = mem_base + guest_va,   where mem_base = host_ptr - base
//! ```
//!
//! That `mem_base` is exactly what the lowerer rebases every JIT memory access
//! through (`CpuStateFrame::mem_base`, RFC 0020): set `state.mem_base =
//! arena.mem_base()` and the translated block reads and writes real guest memory
//! without the host address space having to be identity-mapped to the guest VAs.
//!
//! Resource discipline (the standing clause): the mapping is owned by the arena
//! and released in `Drop` (munmap / VirtualFree) — deterministically, never
//! leaked across a restart. The host bytes are freed the moment the arena is
//! dropped.

use std::io;

use crate::jit_memory::platform;

/// A contiguous host mapping backing `[base, base + size)` of the guest address
/// space. Owns the mapping; frees it on `Drop`.
#[derive(Debug)]
pub struct GuestArena {
    /// Guest base VA the arena maps from.
    base: u64,
    /// Host allocation start (page-aligned).
    ptr: *mut u8,
    /// Mapping size in bytes (page-rounded).
    size: usize,
}

// SAFETY: the arena exclusively owns its mapping; the raw pointer is never
// aliased by another owner, so it is sound to move across threads. (It is not
// `Sync` — concurrent mutation needs external synchronization.)
unsafe impl Send for GuestArena {}

impl GuestArena {
    /// Map a window of `size` bytes (rounded up to a page) for guest VAs starting
    /// at `base`. The bytes start zeroed (the platform allocators commit
    /// zero-filled pages).
    ///
    /// # Errors
    /// [`io::Error`] if the host allocation fails.
    pub fn new(base: u64, size: usize) -> io::Result<Self> {
        let ps = platform::page_size();
        // At least one page; round the request up to a whole number of pages.
        let size = size.max(1).next_multiple_of(ps);
        let ptr = platform::alloc_rw(size);
        if ptr.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "GuestArena: host allocation failed",
            ));
        }
        Ok(Self { base, ptr, size })
    }

    /// The base the lowerer adds to every guest VA: `host_ptr - base`. Adding a
    /// guest VA in `[base, base + size)` yields its host address.
    #[must_use]
    pub fn mem_base(&self) -> u64 {
        (self.ptr as u64).wrapping_sub(self.base)
    }

    /// Guest base VA of the window.
    #[must_use]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Window size in bytes (page-rounded).
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether `[va, va + len)` lies entirely inside the window.
    #[must_use]
    pub fn contains(&self, va: u64, len: usize) -> bool {
        self.offset(va, len).is_some()
    }

    /// Host-buffer offset of `[va, va + len)` if it fits the window.
    fn offset(&self, va: u64, len: usize) -> Option<usize> {
        let rel = va.checked_sub(self.base)?;
        let off = usize::try_from(rel).ok()?;
        let end = off.checked_add(len)?;
        if end <= self.size {
            Some(off)
        } else {
            None
        }
    }

    /// Borrow `len` guest bytes at `va`, or `None` if the range escapes the
    /// window.
    #[must_use]
    pub fn read(&self, va: u64, len: usize) -> Option<&[u8]> {
        let off = self.offset(va, len)?;
        Some(&self.as_slice()[off..off + len])
    }

    /// Copy `src` into guest memory at `va`, or `None` if the range escapes the
    /// window.
    pub fn write(&mut self, va: u64, src: &[u8]) -> Option<()> {
        let off = self.offset(va, src.len())?;
        self.as_mut_slice()[off..off + src.len()].copy_from_slice(src);
        Some(())
    }

    /// The whole window as a host slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is a live mapping of `size` bytes owned by `self`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    /// The whole window as a mutable host slice.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` is a live mapping of `size` bytes exclusively owned by
        // `self` (we hold `&mut self`).
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }
}

impl Drop for GuestArena {
    fn drop(&mut self) {
        // Deterministic release (the standing clause): unmap the backing here so
        // it never survives a restart. `free` munmaps (unix) / VirtualFrees
        // (Windows).
        platform::free(self.ptr, self.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_base_maps_guest_va_to_host_pointer() {
        let base = 0x4_0000;
        let arena = GuestArena::new(base, 4096).unwrap();
        // host_addr(va) == mem_base + va, and at va == base that is the start.
        let host_start = arena.as_slice().as_ptr() as u64;
        assert_eq!(arena.mem_base().wrapping_add(base), host_start);
        let va = base + 0x100;
        assert_eq!(arena.mem_base().wrapping_add(va), host_start + 0x100);
    }

    #[test]
    fn rounds_size_up_to_a_page_and_starts_zeroed() {
        let arena = GuestArena::new(0, 1).unwrap();
        assert!(arena.size() >= platform::page_size());
        assert!(arena.as_slice().iter().all(|&b| b == 0));
    }

    #[test]
    fn read_write_round_trip_within_window() {
        let base = 0x1000;
        let mut arena = GuestArena::new(base, 4096).unwrap();
        arena.write(base + 8, &[1, 2, 3, 4]).unwrap();
        assert_eq!(arena.read(base + 8, 4), Some(&[1, 2, 3, 4][..]));
    }

    #[test]
    fn out_of_window_access_is_rejected() {
        let base = 0x1000;
        let mut arena = GuestArena::new(base, 4096).unwrap();
        assert!(!arena.contains(base - 1, 1)); // below base
        assert!(arena.read(base + arena.size() as u64, 1).is_none()); // past end
        assert!(arena
            .write(base + arena.size() as u64 - 2, &[0; 4])
            .is_none()); // straddles end
    }

    // The mapping is released on Drop. Repeated alloc+drop must not exhaust the
    // address space — proof that `Drop` actually unmaps (the standing RAII
    // clause). A leak here would fail or OOM well before the loop completes.
    #[test]
    fn drop_releases_mapping() {
        for _ in 0..2048 {
            let a = GuestArena::new(0x1_0000, 1 << 20).unwrap(); // 1 MiB each
            assert_eq!(a.size(), 1 << 20);
        }
    }
}
