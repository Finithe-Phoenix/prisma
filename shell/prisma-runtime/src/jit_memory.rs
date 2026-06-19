//! JIT memory allocation primitives.
//!
//! [`JitBuffer`] is a plain growable byte buffer used by the buffer-pool
//! accounting shim. [`ExecBuffer`] is the real W^X executable-memory allocator
//! mirroring C++ `prisma::runtime::JitBuffer`: allocate read/write, copy code,
//! flip to read/execute, invalidate the I-cache. Writable XOR executable is
//! enforced — [`ExecBuffer::write`] fails once the page is executable.

use std::io;

// ---------------------------------------------------------------------------
// Platform backends (no extra crates: raw kernel32 FFI on Windows, libc unix).
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    const MEM_COMMIT: u32 = 0x0000_1000;
    const MEM_RESERVE: u32 = 0x0000_2000;
    const MEM_RELEASE: u32 = 0x0000_8000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_EXECUTE_READ: u32 = 0x20;

    #[repr(C)]
    struct SystemInfo {
        w_processor_architecture: u16,
        w_reserved: u16,
        dw_page_size: u32,
        lp_minimum_application_address: *mut c_void,
        lp_maximum_application_address: *mut c_void,
        dw_active_processor_mask: usize,
        dw_number_of_processors: u32,
        dw_processor_type: u32,
        dw_allocation_granularity: u32,
        w_processor_level: u16,
        w_processor_revision: u16,
    }

    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, typ: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
        fn VirtualFree(addr: *mut c_void, size: usize, typ: u32) -> i32;
        fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn GetSystemInfo(info: *mut SystemInfo);
    }

    pub fn page_size() -> usize {
        // SAFETY: GetSystemInfo writes a fully-initialized SystemInfo.
        let mut info: SystemInfo = unsafe { core::mem::zeroed() };
        unsafe { GetSystemInfo(&raw mut info) };
        info.dw_page_size as usize
    }

    /// Reserve+commit `size` bytes as read/write. Returns null on failure.
    pub fn alloc_rw(size: usize) -> *mut u8 {
        // SAFETY: standard VirtualAlloc; null addr lets the OS choose.
        unsafe {
            VirtualAlloc(
                core::ptr::null_mut(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        }
        .cast::<u8>()
    }

    /// Flip `[ptr, ptr+size)` to read/execute. Returns false on failure.
    pub fn protect_rx(ptr: *mut u8, size: usize) -> bool {
        let mut old: u32 = 0;
        // SAFETY: ptr/size came from a prior alloc_rw of the same region.
        unsafe { VirtualProtect(ptr.cast(), size, PAGE_EXECUTE_READ, &raw mut old) != 0 }
    }

    pub fn free(ptr: *mut u8, _size: usize) {
        // SAFETY: ptr came from alloc_rw; MEM_RELEASE requires size 0.
        unsafe {
            VirtualFree(ptr.cast(), 0, MEM_RELEASE);
        }
    }

    pub fn flush_icache(ptr: *const u8, size: usize) {
        // SAFETY: process handle pseudo-handle is always valid.
        unsafe {
            FlushInstructionCache(GetCurrentProcess(), ptr.cast(), size);
        }
    }
}

#[cfg(unix)]
mod platform {
    pub fn page_size() -> usize {
        // SAFETY: sysconf with a valid name returns the page size or -1.
        let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if ps > 0 {
            usize::try_from(ps).unwrap_or(4096)
        } else {
            4096
        }
    }

    pub fn alloc_rw(size: usize) -> *mut u8 {
        // SAFETY: anonymous private mapping, RW, kernel chooses the address.
        let p = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            core::ptr::null_mut()
        } else {
            p.cast::<u8>()
        }
    }

    pub fn protect_rx(ptr: *mut u8, size: usize) -> bool {
        // SAFETY: ptr/size came from a prior alloc_rw of the same region.
        unsafe { libc::mprotect(ptr.cast(), size, libc::PROT_READ | libc::PROT_EXEC) == 0 }
    }

    pub fn free(ptr: *mut u8, size: usize) {
        // SAFETY: ptr/size came from alloc_rw. munmap failure on a valid
        // mapping is not expected; assert in debug to surface programming
        // errors without aborting release builds on a best-effort cleanup.
        let rc = unsafe { libc::munmap(ptr.cast(), size) };
        debug_assert_eq!(rc, 0, "munmap failed on a JIT mapping");
    }

    pub fn flush_icache(ptr: *const u8, size: usize) {
        // x86 has coherent instruction/data caches; nothing to do.
        #[cfg(target_arch = "x86_64")]
        {
            let _ = (ptr, size);
        }
        // ARM64 caches are NOT coherent w.r.t. JIT writes. We must clean the
        // D-cache to the Point of Unification, then invalidate the I-cache,
        // over [ptr, ptr+size), with barriers — the canonical __clear_cache
        // sequence. Line sizes come from CTR_EL0 (DminLine/IminLine).
        #[cfg(target_arch = "aarch64")]
        {
            use core::arch::asm;
            if size == 0 {
                return;
            }
            let start = ptr as usize;
            let end = start + size;

            let ctr: u64;
            // SAFETY: reading CTR_EL0 is unprivileged and side-effect free.
            unsafe {
                asm!("mrs {0}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags));
            }
            // DminLine = bits [19:16], IminLine = bits [3:0]; line bytes =
            // 4 (bytes/word) << field.
            let d_line = 4usize << ((ctr >> 16) & 0xf);
            let i_line = 4usize << (ctr & 0xf);

            // Clean D-cache lines to PoU.
            let mut addr = start & !(d_line - 1);
            while addr < end {
                // SAFETY: dc cvau over a mapped, owned range.
                unsafe {
                    asm!("dc cvau, {0}", in(reg) addr, options(nostack, preserves_flags));
                }
                addr += d_line;
            }
            // SAFETY: barrier to order the cleans before the invalidates.
            unsafe {
                asm!("dsb ish", options(nostack, preserves_flags));
            }
            // Invalidate I-cache lines to PoU.
            let mut addr = start & !(i_line - 1);
            while addr < end {
                // SAFETY: ic ivau over a mapped, owned range.
                unsafe {
                    asm!("ic ivau, {0}", in(reg) addr, options(nostack, preserves_flags));
                }
                addr += i_line;
            }
            // SAFETY: ensure invalidates complete and refetch.
            unsafe {
                asm!("dsb ish", options(nostack, preserves_flags));
                asm!("isb", options(nostack, preserves_flags));
            }
        }
        // Other arches: conservatively do nothing (covered hosts are x86_64
        // and aarch64). Add explicit handling before targeting a new arch.
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = (ptr, size);
        }
    }
}

/// A page-aligned region of executable memory with a W^X lifecycle.
#[derive(Debug)]
pub struct ExecBuffer {
    base: *mut u8,
    capacity: usize,
    written: usize,
    executable: bool,
}

impl ExecBuffer {
    /// Allocate at least `min_bytes` of read/write memory (page-rounded).
    ///
    /// # Errors
    /// Returns an error if `min_bytes` is so large the page-rounded size
    /// overflows `usize`, or if the OS allocation fails.
    pub fn alloc(min_bytes: usize) -> io::Result<Self> {
        let ps = platform::page_size();
        let want = if min_bytes == 0 { 1 } else { min_bytes };
        // Overflow-safe page rounding (guards against absurd min_bytes).
        let capacity = want
            .div_ceil(ps)
            .checked_mul(ps)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "JIT size overflow"))?;
        let base = platform::alloc_rw(capacity);
        if base.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            base,
            capacity,
            written: 0,
            executable: false,
        })
    }

    /// Copy `code` into the buffer. Fails if already executable (W^X) or if
    /// `code` exceeds the capacity.
    pub fn write(&mut self, code: &[u8]) -> bool {
        if self.executable || code.len() > self.capacity {
            return false;
        }
        // SAFETY: base is a valid RW mapping of `capacity >= code.len()` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(code.as_ptr(), self.base, code.len());
        }
        self.written = code.len();
        true
    }

    /// Flip the region to read/execute and invalidate the I-cache.
    ///
    /// # Errors
    /// Returns an error if the OS protection change fails.
    pub fn make_executable(&mut self) -> io::Result<()> {
        if self.executable {
            return Ok(());
        }
        if !platform::protect_rx(self.base, self.capacity) {
            return Err(io::Error::last_os_error());
        }
        platform::flush_icache(self.base.cast_const(), self.written);
        self.executable = true;
        Ok(())
    }

    /// Pointer to the start of the code region.
    ///
    /// # Safety contract
    /// The returned pointer is valid only while `self` is alive. Calling it as
    /// a function after the `ExecBuffer` (or its owning [`ExecPool`]) is dropped
    /// is a use-after-free of executable memory. Callers must keep the owner
    /// alive for the entire duration of any call through this pointer.
    #[must_use]
    pub const fn as_ptr(&self) -> *const u8 {
        self.base.cast_const()
    }

    /// Total page-rounded capacity in bytes.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of code bytes written.
    #[must_use]
    pub const fn written_len(&self) -> usize {
        self.written
    }

    /// Whether the region has been flipped to executable.
    #[must_use]
    pub const fn is_executable(&self) -> bool {
        self.executable
    }
}

impl Drop for ExecBuffer {
    fn drop(&mut self) {
        if !self.base.is_null() {
            platform::free(self.base, self.capacity);
            self.base = core::ptr::null_mut();
        }
    }
}

// ExecBuffer owns a unique OS mapping; sending it across threads is sound.
// SAFETY: the raw pointer is an exclusively-owned mapping, freed once on Drop.
unsafe impl Send for ExecBuffer {}

/// A handle to code installed in an [`ExecPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecAllocation {
    /// Index of the owning buffer in the pool.
    pub index: usize,
    /// Executable entry pointer. Valid only while the owning [`ExecPool`] is
    /// alive — using it after the pool is dropped is a use-after-free of
    /// executable memory.
    pub entry: *const u8,
    /// Number of code bytes.
    pub code_size: usize,
    /// Page-rounded capacity of the backing buffer.
    pub capacity: usize,
}

/// Owns a set of executable buffers, one per installed block.
///
/// Mirrors C++ `JitBufferPool::add`: each `add` allocates an [`ExecBuffer`],
/// writes the code, flips it executable, and hands back an [`ExecAllocation`]
/// whose `entry` is directly callable.
#[derive(Debug, Default)]
pub struct ExecPool {
    buffers: Vec<ExecBuffer>,
    min_bytes: usize,
}

impl ExecPool {
    /// Create a pool whose buffers are at least `min_bytes` each.
    #[must_use]
    pub const fn new(min_bytes: usize) -> Self {
        Self {
            buffers: Vec::new(),
            min_bytes,
        }
    }

    /// Install `code` as a fresh executable buffer and return its handle.
    /// Returns `None` for empty input or on allocation/protection failure.
    pub fn add(&mut self, code: &[u8]) -> Option<ExecAllocation> {
        if code.is_empty() {
            return None;
        }
        let want = self.min_bytes.max(code.len());
        let mut buffer = ExecBuffer::alloc(want).ok()?;
        if !buffer.write(code) {
            return None;
        }
        buffer.make_executable().ok()?;
        let alloc = ExecAllocation {
            index: self.buffers.len(),
            entry: buffer.as_ptr(),
            code_size: code.len(),
            capacity: buffer.capacity(),
        };
        self.buffers.push(buffer);
        Some(alloc)
    }

    /// Number of installed buffers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// Whether the pool is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

/// Minimal JIT buffer wrapper.
#[derive(Debug, Default, Clone)]
pub struct JitBuffer {
    storage: Vec<u8>,
}

impl JitBuffer {
    /// Creates a new zero-capacity buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            storage: Vec::new(),
        }
    }

    /// Allocates backing storage with the requested capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            storage: Vec::with_capacity(capacity),
        }
    }

    /// Returns current buffer size.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns whether buffer is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Appends bytes to the buffer.
    pub fn append(&mut self, data: &[u8]) {
        self.storage.extend_from_slice(data);
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecBuffer, ExecPool};

    #[test]
    fn alloc_rounds_up_to_page() {
        let b = ExecBuffer::alloc(1).expect("alloc");
        // Page-rounded: at least one page, a multiple of the common 4 KiB page
        // (16 KiB pages, e.g. Apple silicon, are also multiples of 4096).
        assert!(b.capacity() >= 4096);
        assert_eq!(b.capacity() % 4096, 0);
        assert!(!b.is_executable());
    }

    #[test]
    fn write_then_executable_enforces_wxor() {
        let mut b = ExecBuffer::alloc(64).expect("alloc");
        assert!(b.write(&[0x90, 0x90, 0xC3])); // nop;nop;ret
        assert_eq!(b.written_len(), 3);
        b.make_executable().expect("protect rx");
        assert!(b.is_executable());
        // W^X: writes are rejected once executable.
        assert!(!b.write(&[0x00]));
    }

    #[test]
    fn oversized_write_rejected() {
        let mut b = ExecBuffer::alloc(8).expect("alloc");
        let too_big = vec![0u8; b.capacity() + 1];
        assert!(!b.write(&too_big));
    }

    // The crown-jewel test: JIT a real function into executable memory and
    // CALL it. x86-64 only (the encoded bytes are x86-64 machine code).
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn jit_and_execute_returns_42() {
        // mov eax, 42 ; ret   ->  B8 2A 00 00 00 C3
        let code: [u8; 6] = [0xB8, 0x2A, 0x00, 0x00, 0x00, 0xC3];
        let mut buf = ExecBuffer::alloc(code.len()).expect("alloc");
        assert!(buf.write(&code));
        buf.make_executable().expect("rx");

        // SAFETY: the buffer holds a valid, executable `extern "C" fn() -> u32`
        // (System V / Win64 both return u32 in EAX), and stays alive for the
        // duration of the call.
        let f: extern "C" fn() -> u32 = unsafe { core::mem::transmute(buf.as_ptr()) };
        assert_eq!(f(), 42, "JIT-executed function must return 42");
    }

    // ARM64 counterpart of the x86-64 execute test. Exercises mmap + the real
    // dc cvau/ic ivau I-cache maintenance + execution on aarch64 (run the
    // suite under linux/arm64 to validate the JIT path on Prisma's target).
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn jit_and_execute_returns_42_aarch64() {
        // mov w0, #42 ; ret   ->  0x52800540, 0xD65F03C0 (little-endian)
        let code: [u8; 8] = [0x40, 0x05, 0x80, 0x52, 0xC0, 0x03, 0x5F, 0xD6];
        let mut buf = ExecBuffer::alloc(code.len()).expect("alloc");
        assert!(buf.write(&code));
        buf.make_executable().expect("rx");
        // SAFETY: buf holds a valid executable extern "C" fn() -> u32 (AAPCS64
        // returns u32 in w0) kept alive for the call.
        let f: extern "C" fn() -> u32 = unsafe { core::mem::transmute(buf.as_ptr()) };
        assert_eq!(f(), 42, "ARM64 JIT-executed function must return 42");
    }

    #[test]
    fn exec_pool_add_rejects_empty() {
        let mut pool = ExecPool::new(64);
        assert!(pool.add(&[]).is_none());
        assert!(pool.is_empty());
    }

    // Install two functions in the pool and call both through their handles.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn exec_pool_installs_and_runs_two_blocks() {
        let mut pool = ExecPool::new(64);
        // mov eax, 7 ; ret
        let a = pool
            .add(&[0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3])
            .expect("add a");
        // mov eax, 100 ; ret
        let b = pool
            .add(&[0xB8, 0x64, 0x00, 0x00, 0x00, 0xC3])
            .expect("add b");
        assert_eq!(pool.len(), 2);
        assert_eq!((a.index, b.index), (0, 1));

        // SAFETY: each entry is a valid executable extern "C" fn() -> u32 kept
        // alive by `pool` for the duration of the calls.
        let fa: extern "C" fn() -> u32 = unsafe { core::mem::transmute(a.entry) };
        let fb: extern "C" fn() -> u32 = unsafe { core::mem::transmute(b.entry) };
        assert_eq!(fa(), 7);
        assert_eq!(fb(), 100);
    }
}
