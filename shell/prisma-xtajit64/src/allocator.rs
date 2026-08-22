const ALLOCATION_HEADER_BYTES: usize = size_of::<*mut u8>();

fn allocation_span(size: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    size.max(1)
        .checked_add(align.checked_sub(1)?)?
        .checked_add(ALLOCATION_HEADER_BYTES)
}

fn aligned_user_address(raw_address: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    raw_address
        .checked_add(ALLOCATION_HEADER_BYTES)?
        .checked_add(align - 1)
        .map(|address| address & !(align - 1))
}

#[cfg(all(windows, target_arch = "arm64ec"))]
mod arm64ec {
    use super::ALLOCATION_HEADER_BYTES;
    use super::{aligned_user_address, allocation_span};
    use std::alloc::{GlobalAlloc, Layout};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

    const HEAP_ZERO_MEMORY: u32 = 0x0000_0008;

    unsafe extern "system" {
        fn HeapCreate(options: u32, initial_size: usize, maximum_size: usize) -> *mut c_void;
        fn HeapDestroy(heap: *mut c_void) -> i32;
        fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut c_void;
        fn HeapFree(heap: *mut c_void, flags: u32, memory: *mut c_void) -> i32;
        fn HeapValidate(heap: *mut c_void, flags: u32, memory: *const c_void) -> i32;
    }

    /// Provider-private heap isolated behind an ARM64EC preservation boundary.
    ///
    /// Rust's `System` allocator shares Wine's process heap with the x64 guest
    /// runtime. A mixed-architecture failure can therefore poison the same
    /// free-list metadata that the next translator allocation must traverse.
    /// This serialized private heap keeps Prisma's allocation metadata separate
    /// while preserving normal `HeapAlloc` performance and deallocation.
    pub struct Arm64EcPrivateHeapAllocator;

    #[global_allocator]
    static GLOBAL_ALLOCATOR: Arm64EcPrivateHeapAllocator = Arm64EcPrivateHeapAllocator;

    static PRIVATE_HEAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static TRACE_ALLOCATOR: AtomicBool = AtomicBool::new(false);

    unsafe impl GlobalAlloc for Arm64EcPrivateHeapAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the preserving entry forwards this valid layout.
            unsafe { preserving_alloc(layout.size(), layout.align()) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: pointer/layout are the pair returned by this allocator.
            unsafe { preserving_dealloc(pointer, layout.size(), layout.align()) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the preserving entry forwards this valid layout.
            unsafe { preserving_alloc_zeroed(layout.size(), layout.align()) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: pointer/layout came from this allocator and `new_size`
            // is the replacement size requested by `GlobalAlloc`.
            unsafe { preserving_realloc(pointer, layout.size(), layout.align(), new_size) }
        }
    }

    unsafe fn private_heap() -> *mut c_void {
        let existing = PRIVATE_HEAP.load(Ordering::Acquire);
        if !existing.is_null() {
            return existing;
        }

        // SAFETY: zero sizes create a growable, serialized private heap.
        let provider_mapping = crate::dispatch::ProviderOwnedMappingGuard::enter_if_available();
        let candidate = unsafe { HeapCreate(0, 0, 0) };
        drop(provider_mapping);
        if candidate.is_null() {
            return std::ptr::null_mut();
        }
        match PRIVATE_HEAP.compare_exchange(
            std::ptr::null_mut(),
            candidate,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => candidate,
            Err(winner) => {
                // SAFETY: this racing candidate has served no allocations.
                let _ = unsafe { HeapDestroy(candidate) };
                winner
            }
        }
    }

    unsafe fn heap_alloc(size: usize, align: usize, flags: u32) -> *mut u8 {
        let trace = TRACE_ALLOCATOR.load(Ordering::Acquire);
        if trace {
            crate::phase_marker(b"prisma-phase: allocator-alloc-enter\n");
            crate::phase_value(b"prisma-value: allocator-request-size=", size as u64);
            crate::phase_value(b"prisma-value: allocator-request-align=", align as u64);
        }
        let Some(span) = allocation_span(size, align) else {
            return std::ptr::null_mut();
        };
        if trace {
            crate::phase_value(b"prisma-value: allocator-span=", span as u64);
        }
        // SAFETY: initialization is allocation-free and publishes one process-
        // lifetime private heap handle.
        let heap = unsafe { private_heap() };
        if heap.is_null() {
            return std::ptr::null_mut();
        }
        if trace {
            crate::phase_value(b"prisma-value: allocator-heap=", heap.addr() as u64);
            crate::phase_marker(b"prisma-phase: allocator-heap-alloc-enter\n");
        }
        // SAFETY: `span` is nonzero and overflow-checked.
        let provider_mapping = crate::dispatch::ProviderOwnedMappingGuard::enter_if_available();
        let raw = unsafe { HeapAlloc(heap, flags, span) }.cast::<u8>();
        drop(provider_mapping);
        if trace {
            crate::phase_value(
                b"prisma-value: allocator-heap-alloc-result=",
                raw.addr() as u64,
            );
        }
        if raw.is_null() {
            return std::ptr::null_mut();
        }
        let Some(user_address) = aligned_user_address(raw.addr(), align) else {
            // SAFETY: `raw` came from this exact heap and has not escaped.
            let _ = unsafe { HeapFree(heap, 0, raw.cast()) };
            return std::ptr::null_mut();
        };
        let user = user_address as *mut u8;
        // SAFETY: the checked span reserves one pointer-sized header before the
        // aligned user region; HeapAlloc is at least pointer-aligned on Win64.
        unsafe {
            user.sub(ALLOCATION_HEADER_BYTES)
                .cast::<*mut u8>()
                .write(raw);
        }
        if trace {
            crate::phase_value(b"prisma-value: allocator-user=", user.addr() as u64);
            crate::phase_marker(b"prisma-phase: allocator-alloc-returned\n");
        }
        user
    }

    unsafe fn heap_dealloc(pointer: *mut u8) -> i32 {
        let trace = TRACE_ALLOCATOR.load(Ordering::Acquire);
        if trace {
            crate::phase_marker(b"prisma-phase: allocator-dealloc-enter\n");
            crate::phase_value(b"prisma-value: allocator-user=", pointer.addr() as u64);
        }
        if pointer.is_null() {
            return 1;
        }
        let heap = PRIVATE_HEAP.load(Ordering::Acquire);
        if heap.is_null() {
            return 1;
        }
        // SAFETY: every non-null pointer returned by `heap_alloc` owns this
        // initialized header immediately before its user region.
        let raw = unsafe {
            pointer
                .sub(ALLOCATION_HEADER_BYTES)
                .cast::<*mut u8>()
                .read()
        };
        if trace {
            crate::phase_value(b"prisma-value: allocator-heap=", heap.addr() as u64);
            crate::phase_value(b"prisma-value: allocator-raw=", raw.addr() as u64);
            crate::phase_marker(b"prisma-phase: allocator-block-validate-enter\n");
            // SAFETY: the diagnostic asks this exact heap to validate the raw
            // base recovered from the allocation header without mutating it.
            let valid = unsafe { HeapValidate(heap, 0, raw.cast()) };
            crate::phase_value(b"prisma-value: allocator-block-valid=", valid as u64);
            crate::phase_marker(b"prisma-phase: allocator-heap-free-enter\n");
        }
        // SAFETY: `raw` is the exact HeapAlloc base from this private heap.
        let freed = unsafe { HeapFree(heap, 0, raw.cast()) };
        if trace {
            crate::phase_value(b"prisma-value: allocator-heap-free-result=", freed as u64);
            crate::phase_marker(b"prisma-phase: allocator-heap-free-returned\n");
        }
        freed
    }

    macro_rules! preserve_arm64ec_nonvolatiles {
        ($helper:path; $($operands:tt)*) => {
            // SAFETY: the assembly owns one balanced, aligned save area. The
            // direct helper call uses the compiler-generated ARM64EC convention;
            // every usable register requiring preservation at this native EC
            // boundary is restored before LLVM regains control of this function.
            unsafe {
                core::arch::asm!(
                    "sub sp, sp, #0xa0",
                    "stp d8, d9, [sp, #0x00]",
                    "stp d10, d11, [sp, #0x10]",
                    "stp d12, d13, [sp, #0x20]",
                    "stp d14, d15, [sp, #0x30]",
                    "stp x19, x20, [sp, #0x40]",
                    "stp x21, x22, [sp, #0x50]",
                    "stp x25, x26, [sp, #0x60]",
                    "stp x27, x29, [sp, #0x70]",
                    "stp x30, xzr, [sp, #0x80]",
                    "mrs x15, fpcr",
                    "str x15, [sp, #0x90]",
                    "bl {helper}",
                    "ldr x15, [sp, #0x90]",
                    "msr fpcr, x15",
                    "ldp x30, xzr, [sp, #0x80]",
                    "ldp x27, x29, [sp, #0x70]",
                    "ldp x25, x26, [sp, #0x60]",
                    "ldp x21, x22, [sp, #0x50]",
                    "ldp x19, x20, [sp, #0x40]",
                    "ldp d14, d15, [sp, #0x30]",
                    "ldp d12, d13, [sp, #0x20]",
                    "ldp d10, d11, [sp, #0x10]",
                    "ldp d8, d9, [sp, #0x00]",
                    "add sp, sp, #0xa0",
                    helper = sym $helper,
                    $($operands)*
                    clobber_abi("C"),
                )
            }
        };
    }

    #[inline(never)]
    unsafe extern "C" fn preserving_alloc(size: usize, align: usize) -> *mut u8 {
        let result: *mut u8;
        preserve_arm64ec_nonvolatiles!(private_alloc;
            inlateout("x0") size => result,
            in("x1") align,
        );
        result
    }

    #[inline(never)]
    unsafe extern "C" fn preserving_dealloc(pointer: *mut u8, size: usize, align: usize) {
        preserve_arm64ec_nonvolatiles!(private_dealloc;
            inlateout("x0") pointer => _,
            in("x1") size,
            in("x2") align,
        );
    }

    #[inline(never)]
    unsafe extern "C" fn preserving_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
        let result: *mut u8;
        preserve_arm64ec_nonvolatiles!(private_alloc_zeroed;
            inlateout("x0") size => result,
            in("x1") align,
        );
        result
    }

    #[inline(never)]
    unsafe extern "C" fn preserving_realloc(
        pointer: *mut u8,
        size: usize,
        align: usize,
        new_size: usize,
    ) -> *mut u8 {
        let result: *mut u8;
        preserve_arm64ec_nonvolatiles!(private_realloc;
            inlateout("x0") pointer => result,
            in("x1") size,
            in("x2") align,
            in("x3") new_size,
        );
        result
    }

    unsafe extern "C" fn private_alloc(size: usize, align: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc supplies a valid size/alignment pair.
        unsafe { heap_alloc(size, align, 0) }
    }

    unsafe extern "C" fn private_dealloc(pointer: *mut u8, _size: usize, _align: usize) -> usize {
        // Keep a real ARM64EC return boundary after HeapFree. Returning the
        // i32 status as usize requires a post-call extension, which prevents
        // LLVM from tail-branching directly across Wine's hybrid heap thunk.
        // SAFETY: pointer came from this allocator.
        unsafe { heap_dealloc(pointer) as usize }
    }

    unsafe extern "C" fn private_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc supplies a valid size/alignment pair and the
        // platform zeroes the complete raw allocation before the header write.
        unsafe { heap_alloc(size, align, HEAP_ZERO_MEMORY) }
    }

    unsafe extern "C" fn private_realloc(
        pointer: *mut u8,
        size: usize,
        align: usize,
        new_size: usize,
    ) -> *mut u8 {
        // SAFETY: GlobalAlloc supplies the original pointer/layout pair.
        let replacement = unsafe { heap_alloc(new_size, align, 0) };
        if replacement.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: both regions are live, distinct allocations and each has at
        // least the corresponding requested user size.
        unsafe {
            std::ptr::copy_nonoverlapping(pointer, replacement, size.min(new_size));
            heap_dealloc(pointer);
        }
        replacement
    }

    #[inline(never)]
    unsafe extern "C" fn private_validate(heap: *mut c_void) -> usize {
        // SAFETY: a null block asks Windows to validate the complete private heap.
        unsafe { HeapValidate(heap, 0, std::ptr::null()) as usize }
    }

    pub(super) fn private_heap_is_valid() -> bool {
        let heap = PRIVATE_HEAP.load(Ordering::Acquire);
        if heap.is_null() {
            return true;
        }
        let valid: usize;
        preserve_arm64ec_nonvolatiles!(private_validate;
            inlateout("x0") heap => valid,
        );
        valid != 0
    }

    pub(super) fn set_allocator_trace(enabled: bool) {
        TRACE_ALLOCATOR.store(enabled, Ordering::Release);
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub(crate) fn private_heap_is_valid() -> bool {
    arm64ec::private_heap_is_valid()
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub(crate) fn set_allocator_trace(enabled: bool) {
    arm64ec::set_allocator_trace(enabled);
}

#[cfg(test)]
mod tests {
    use super::ALLOCATION_HEADER_BYTES;
    use super::{aligned_user_address, allocation_span};

    #[test]
    fn private_heap_span_covers_header_alignment_and_zero_size() {
        assert_eq!(allocation_span(0, 1), Some(ALLOCATION_HEADER_BYTES + 1));
        assert_eq!(
            allocation_span(31, 64),
            Some(31 + 63 + ALLOCATION_HEADER_BYTES)
        );
        assert_eq!(allocation_span(8, 0), None);
        assert_eq!(allocation_span(8, 3), None);
        assert_eq!(allocation_span(usize::MAX, 8), None);
    }

    #[test]
    fn private_heap_user_region_is_aligned_and_inside_the_span() {
        let raw = 0x1_0000_usize;
        for align in [1, 2, 4, 8, 16, 64, 4096, 65_536] {
            let size = 257;
            let span = allocation_span(size, align).unwrap();
            let user = aligned_user_address(raw, align).unwrap();
            assert_eq!(user % align, 0);
            assert!(user >= raw + ALLOCATION_HEADER_BYTES);
            assert_eq!((user - ALLOCATION_HEADER_BYTES) % align_of::<*mut u8>(), 0);
            assert!(user + size <= raw + span);
        }

        let raw = 0x1008_usize;
        let user = aligned_user_address(raw, 64).unwrap();
        assert_eq!(user, 0x1040);
        assert_eq!(user - ALLOCATION_HEADER_BYTES, 0x1038);
        assert!(user + 128 <= raw + allocation_span(128, 64).unwrap());
    }

    #[test]
    fn private_heap_address_math_rejects_overflow() {
        assert_eq!(aligned_user_address(usize::MAX, 8), None);
        assert_eq!(aligned_user_address(0x1000, 0), None);
        assert_eq!(aligned_user_address(0x1000, 6), None);
    }
}
