//! ARM64 execution e2e for the single-operand ALU family (INC/DEC/NEG/NOT).
//!
//! Each test fuses a short straight-line x86-64 block with `prisma-translator`
//! and runs it through the canonical [`prisma_runtime::executor`] path. GPR
//! assertions only run on aarch64 (the lowered bytes are ARM64 machine code); on
//! every other host the translate + W^X install path runs and the call is
//! skipped with `WrongArch` — the same gate the C++ e2e corpus uses.

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError};
use prisma_translator::Translator;

/// Translate `program` at `addr` into a non-empty fused ARM64 block.
fn translate(addr: u64, program: &[u8]) -> Vec<u8> {
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(addr, program, 64)
        .expect("fused block translation");
    assert!(!block.code.is_empty(), "lowered to no code");
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
    block.code
}

#[test]
fn inc_then_not_executes() {
    // mov rax, rcx ; inc rax ; not rax
    //   48 89 C8   |  48 FF C0  |  48 F7 D0
    let code = translate(
        0x1_0000,
        &[0x48, 0x89, 0xC8, 0x48, 0xFF, 0xC0, 0x48, 0xF7, 0xD0],
    );
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0x41;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        // rax = !(0x41 + 1) = !0x42
        assert_eq!(state.gpr[gpr::RAX], !0x42u64);
        assert_eq!(state.gpr[gpr::RCX], 0x41, "rcx untouched");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn neg_then_dec_executes() {
    // mov rax, rcx ; neg rax ; dec rax
    //   48 89 C8   |  48 F7 D8  |  48 FF C8
    let code = translate(
        0x2_0000,
        &[0x48, 0x89, 0xC8, 0x48, 0xF7, 0xD8, 0x48, 0xFF, 0xC8],
    );
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 5;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        // rax = (0 - 5) - 1 = -6
        assert_eq!(state.gpr[gpr::RAX], (-6i64) as u64);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
