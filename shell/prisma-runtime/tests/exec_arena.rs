//! End-to-end: a translated block reads and writes a real `GuestArena` through
//! `CpuStateFrame::mem_base` (RFC 0020). This is the contiguous-arena backing
//! Danny chose for Stage 2B: one host mapping, a single offset (`mem_base`) maps
//! every guest VA in the window to its host address, and the JIT loads/stores
//! land in the arena.
//!
//! Execution is gated to `aarch64` (the `ffi-link-arm64` runner); on x86 the
//! translate + W^X install path runs and execution is skipped with
//! `ExecError::WrongArch`, leaving the arena untouched.

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError};
use prisma_runtime::guest_arena::GuestArena;
use prisma_translator::Translator;

#[test]
fn block_loads_from_guest_arena() {
    // mov rax, [rbx]   (48 8B 03)
    let program = [0x48u8, 0x8B, 0x03];
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x1000, &program, 64)
        .expect("translate mov rax,[rbx]");

    // A guest window at VA 0x40_0000 with a sentinel the guest will load.
    let guest_base: u64 = 0x40_0000;
    let mut arena = GuestArena::new(guest_base, 64 * 1024).unwrap();
    let load_va = guest_base + 0x20;
    arena
        .write(load_va, &0xCAFE_BABE_DEAD_BEEFu64.to_le_bytes())
        .unwrap();

    let mut state = CpuStateFrame::default();
    state.mem_base = arena.mem_base();
    state.gpr[gpr::RBX] = load_va;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RAX],
            0xCAFE_BABE_DEAD_BEEF,
            "rax = *(mem_base + rbx), the arena sentinel"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
    }
}

#[test]
fn block_stores_into_guest_arena() {
    // mov [rbx], rax   (48 89 03)
    let program = [0x48u8, 0x89, 0x03];
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x2000, &program, 64)
        .expect("translate mov [rbx],rax");

    let guest_base: u64 = 0x50_0000;
    let arena = GuestArena::new(guest_base, 64 * 1024).unwrap();
    let store_va = guest_base + 0x40;

    let mut state = CpuStateFrame::default();
    state.mem_base = arena.mem_base();
    state.gpr[gpr::RBX] = store_va;
    state.gpr[gpr::RAX] = 0x1122_3344_5566_7788;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        // Read the stored value back out of the arena through its safe API.
        assert_eq!(
            arena.read(store_va, 8),
            Some(&0x1122_3344_5566_7788u64.to_le_bytes()[..]),
            "the store landed in the arena at the guest VA"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(arena.read(store_va, 8), Some(&[0u8; 8][..]));
    }
}

/// Store then load through the arena in one block: prove a round-trip lands in
/// real host memory the guest addressed by its own VA.
#[test]
fn block_round_trips_through_guest_arena() {
    // mov [rbx], rax ; mov rcx, [rbx]   (48 89 03 ; 48 8B 0B)
    let mut program = Vec::new();
    program.extend_from_slice(&[0x48, 0x89, 0x03]); // mov [rbx], rax
    program.extend_from_slice(&[0x48, 0x8B, 0x0B]); // mov rcx, [rbx]
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x3000, &program, 64)
        .expect("translate store+load");

    let guest_base: u64 = 0x60_0000;
    let arena = GuestArena::new(guest_base, 64 * 1024).unwrap();
    let va = guest_base + 0x80;

    let mut state = CpuStateFrame::default();
    state.mem_base = arena.mem_base();
    state.gpr[gpr::RBX] = va;
    state.gpr[gpr::RAX] = 0x0F0F_0F0F_F0F0_F0F0;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RCX],
            0x0F0F_0F0F_F0F0_F0F0,
            "rcx = *(mem_base + rbx) read back what rax stored"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
    }
    drop(arena);
}
