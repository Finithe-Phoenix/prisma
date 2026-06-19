//! End-to-end: translate a guest program with the full Rust pipeline
//! (`prisma-translator`: decode -> optimize -> lower), then JIT-execute the
//! ARM64 output against a guest CPU state and verify the result.
//!
//! The execution half is gated to `aarch64` — the translated bytes are ARM64
//! machine code, so they only run on an ARM64 host (the `ffi-link-arm64` CI
//! runner). On the x86 dev host the test still translates and drives the W^X
//! install path, asserting `ExecError::WrongArch`. This mirrors the C++ core's
//! `constexpr is_arm64` execution gate.

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError};
use prisma_translator::Translator;

#[test]
fn translate_then_execute_mov_add() {
    // A two-instruction straight-line block:
    //   mov rax, rcx     (48 89 C8)   -> rax = rcx
    //   add rax, 0x10    (48 83 C0 10) -> rax += 16
    let mut program = Vec::new();
    program.extend_from_slice(&[0x48, 0x89, 0xC8]);
    program.extend_from_slice(&[0x48, 0x83, 0xC0, 0x10]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x1000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 2);
    assert!(!block.code.is_empty());
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 100;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 116, "rax = rcx (100) + 16");
        assert_eq!(state.gpr[gpr::RCX], 100, "rcx is untouched");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        // Off-target: the translate + W^X install path ran; execution skipped.
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(state.gpr[gpr::RAX], 0, "no execution -> rax unchanged");
    }
}

#[test]
fn translate_then_execute_three_alu_ops() {
    // mov rax, rcx ; add rax, 0x05 ; sub rax, 0x02   -> rax = rcx + 3
    let mut program = Vec::new();
    program.extend_from_slice(&[0x48, 0x89, 0xC8]); // mov rax, rcx
    program.extend_from_slice(&[0x48, 0x83, 0xC0, 0x05]); // add rax, 5
    program.extend_from_slice(&[0x48, 0x83, 0xE8, 0x02]); // sub rax, 2

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x2000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 3);

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 40;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 43, "rax = rcx (40) + 5 - 2");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
    }
}
