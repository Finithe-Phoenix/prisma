use std::alloc::{GlobalAlloc, Layout, System};

/// System allocator isolated behind an ARM64EC-native preservation boundary.
///
/// Wine's x64 heap implementation is reached through an ARM64EC exit thunk.
/// Keeping the live Rust caller state outside that mixed-architecture call
/// prevents a damaged nonvolatile register from poisoning a later Vec drop.
pub struct Arm64EcSystemAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: Arm64EcSystemAllocator = Arm64EcSystemAllocator;

unsafe impl GlobalAlloc for Arm64EcSystemAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the preserving entry forwards this valid layout to System.
        unsafe { preserving_alloc(layout.size(), layout.align()) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer/layout are the pair returned by this allocator.
        unsafe { preserving_dealloc(pointer, layout.size(), layout.align()) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the preserving entry forwards this valid layout to System.
        unsafe { preserving_alloc_zeroed(layout.size(), layout.align()) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: pointer/layout came from this allocator and new_size is the
        // replacement size requested under GlobalAlloc's contract.
        unsafe { preserving_realloc(pointer, layout.size(), layout.align(), new_size) }
    }
}

macro_rules! preserve_arm64ec_nonvolatiles {
    ($helper:path; $($operands:tt)*) => {
        // SAFETY: the assembly owns one balanced, aligned save area. The
        // direct helper call uses the compiler-generated ARM64EC convention;
        // every register that survives an EC call is restored before LLVM
        // regains control of this function.
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
    preserve_arm64ec_nonvolatiles!(system_alloc;
        inlateout("x0") size => result,
        in("x1") align,
    );
    result
}

#[inline(never)]
unsafe extern "C" fn preserving_dealloc(pointer: *mut u8, size: usize, align: usize) {
    preserve_arm64ec_nonvolatiles!(system_dealloc;
        inlateout("x0") pointer => _,
        in("x1") size,
        in("x2") align,
    );
}

#[inline(never)]
unsafe extern "C" fn preserving_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
    let result: *mut u8;
    preserve_arm64ec_nonvolatiles!(system_alloc_zeroed;
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
    preserve_arm64ec_nonvolatiles!(system_realloc;
        inlateout("x0") pointer => result,
        in("x1") size,
        in("x2") align,
        in("x3") new_size,
    );
    result
}

unsafe extern "C" fn system_alloc(size: usize, align: usize) -> *mut u8 {
    // SAFETY: GlobalAlloc only supplies valid Layout size/alignment pairs.
    let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
    // SAFETY: System receives the same valid layout.
    unsafe { System.alloc(layout) }
}

unsafe extern "C" fn system_dealloc(pointer: *mut u8, size: usize, align: usize) {
    // SAFETY: GlobalAlloc supplies the layout paired with this pointer.
    let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
    // SAFETY: pointer/layout came from System through this allocator.
    unsafe { System.dealloc(pointer, layout) }
}

unsafe extern "C" fn system_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
    // SAFETY: GlobalAlloc only supplies valid Layout size/alignment pairs.
    let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
    // SAFETY: System receives the same valid layout.
    unsafe { System.alloc_zeroed(layout) }
}

unsafe extern "C" fn system_realloc(
    pointer: *mut u8,
    size: usize,
    align: usize,
    new_size: usize,
) -> *mut u8 {
    // SAFETY: GlobalAlloc supplies the layout paired with this pointer.
    let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
    // SAFETY: pointer/layout came from System and new_size is forwarded intact.
    unsafe { System.realloc(pointer, layout, new_size) }
}
