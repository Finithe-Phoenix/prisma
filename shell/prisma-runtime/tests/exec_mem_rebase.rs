//! End-to-end: a guest memory access reaches a host arena that is *not*
//! identity-mapped to the guest VA. The lowerer rebases every access to
//! `host = mem_base + guest_va` (RFC 0020), so setting `CpuStateFrame::mem_base`
//! to `host_arena_ptr - guest_va` lets the JIT load/store the arena while the
//! guest still uses its own (small, arbitrary) virtual address.
//!
//! Execution is gated to `aarch64` (the `ffi-link-arm64` CI runner); on the x86
//! dev host the translate + W^X install path runs and execution is skipped with
//! `ExecError::WrongArch`, leaving the arena untouched. This mirrors the C++
//! core's `constexpr is_arm64` gate.

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError};
use prisma_translator::Translator;

/// `mem_base` that maps `guest_va` onto the start of `arena`.
fn mem_base_for(arena: &[u64], guest_va: u64) -> u64 {
    (arena.as_ptr() as u64).wrapping_sub(guest_va)
}

#[test]
fn load_reads_rebased_host_arena() {
    // mov rax, [rbx]   (48 8B 03)
    let program = [0x48u8, 0x8B, 0x03];

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x1000, &program, 64)
        .expect("translate mov rax,[rbx]");
    assert!(!block.code.is_empty());

    // A host arena holding a sentinel. The guest reaches it through a small VA.
    let arena: Vec<u64> = vec![0xDEAD_BEEF_CAFE_F00D, 0, 0, 0];
    let guest_va: u64 = 0x4000;

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RBX] = guest_va;
    state.mem_base = mem_base_for(&arena, guest_va);

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RAX],
            0xDEAD_BEEF_CAFE_F00D,
            "rax = *(mem_base + rbx) = arena[0]"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(state.gpr[gpr::RAX], 0, "no execution -> rax unchanged");
    }
    // Keep the arena alive across the call (the JIT dereferenced into it).
    drop(arena);
}

#[test]
fn store_writes_rebased_host_arena() {
    // mov [rbx], rax   (48 89 03)
    let program = [0x48u8, 0x89, 0x03];

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x2000, &program, 64)
        .expect("translate mov [rbx],rax");
    assert!(!block.code.is_empty());

    let arena: Vec<u64> = vec![0u64; 4];
    let guest_va: u64 = 0x8000;

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RBX] = guest_va;
    state.gpr[gpr::RAX] = 0x0123_4567_89AB_CDEF;
    state.mem_base = mem_base_for(&arena, guest_va);

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            arena[0], 0x0123_4567_89AB_CDEF,
            "*(mem_base + rbx) = rax wrote into arena[0]"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(arena[0], 0, "no execution -> arena untouched");
    }
}

/// A `mem_base` of 0 reproduces the legacy `host == guest` behaviour: the access
/// dereferences the guest VA directly. Proven here by pointing the guest at a
/// real host address (the arena's own pointer) with `mem_base == 0`.
#[test]
fn zero_mem_base_is_identity() {
    let program = [0x48u8, 0x8B, 0x03]; // mov rax, [rbx]

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x3000, &program, 64)
        .expect("translate mov rax,[rbx]");

    let arena: Vec<u64> = vec![0x00C0_FFEE_0000_0001, 0, 0, 0];

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RBX] = arena.as_ptr() as u64; // guest VA == host VA
    state.mem_base = 0;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x00C0_FFEE_0000_0001);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
    }
    drop(arena);
}
