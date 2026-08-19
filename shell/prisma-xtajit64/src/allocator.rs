use std::alloc::{GlobalAlloc, Layout, System};

pub struct Arm64EcSystemAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: Arm64EcSystemAllocator = Arm64EcSystemAllocator;

unsafe impl GlobalAlloc for Arm64EcSystemAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding preserves System's allocation contract.
        let allocation = unsafe { System.alloc(layout) };
        preserve_nonvolatile_registers();
        allocation
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout are the pair previously returned by
        // this allocator, which delegates every allocation to System.
        unsafe { System.dealloc(pointer, layout) };
        preserve_nonvolatile_registers();
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding preserves System's allocation contract.
        let allocation = unsafe { System.alloc_zeroed(layout) };
        preserve_nonvolatile_registers();
        allocation
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and layout came from this System-backed
        // allocator, and GlobalAlloc permits the requested replacement size.
        let allocation = unsafe { System.realloc(pointer, layout, new_size) };
        preserve_nonvolatile_registers();
        allocation
    }
}

#[inline(always)]
fn preserve_nonvolatile_registers() {
    // Wine's ARM64EC heap boundary has been observed returning with a damaged
    // nonvolatile register. Declaring every AAPCS64 nonvolatile GPR as clobbered
    // makes LLVM spill and restore them in each allocator entrypoint while the
    // empty barrier itself neither reads memory nor changes the stack.
    // SAFETY: all outputs are discarded and the compiler owns their saves.
    unsafe {
        core::arch::asm!(
            "",
            lateout("x19") _,
            lateout("x20") _,
            lateout("x21") _,
            lateout("x22") _,
            lateout("x23") _,
            lateout("x24") _,
            lateout("x25") _,
            lateout("x26") _,
            lateout("x27") _,
            lateout("x28") _,
            options(nomem, nostack, preserves_flags),
        );
    }
}
