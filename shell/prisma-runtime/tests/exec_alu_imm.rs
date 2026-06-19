//! ARM64 execution e2e for the immediate-ALU family (0x81 group + the
//! accumulator-immediate opcodes).
//!
//! Each test fuses a straight-line x86-64 block with `prisma-translator`
//! (decode -> optimize -> lower) and runs it through the canonical
//! [`prisma_runtime::executor`] path: wrap with the AAPCS64 block
//! prologue/epilogue, install W^X, and call as `extern "C" fn(*mut
//! CpuStateFrame)`. GPR assertions only run on aarch64 (the translated bytes are
//! ARM64 machine code); on every other host the translate + W^X install path
//! runs and the call is skipped with `WrongArch` — the same gate the C++ e2e
//! corpus uses.

#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};
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
fn add_rax_imm32_executes() {
    // add rax, 0x1234  (REX.W 81 /0 id)
    let code = translate(0x1000, &[0x48, 0x81, 0xC0, 0x34, 0x12, 0x00, 0x00]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1111;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1111 + 0x1234);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn and_eax_imm32_zero_extends() {
    // and eax, 0x0F0F  (81 /4 id) — a 32-bit write must zero the upper 32 bits.
    let code = translate(0x2000, &[0x81, 0xE0, 0x0F, 0x0F, 0x00, 0x00]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0xFFFF_FFFF_FFFF_FFFF;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        // The 32-bit AND result is zero-extended into the 64-bit register.
        assert_eq!(state.gpr[gpr::RAX], 0x0F0F);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn add_eax_imm32_accumulator_executes() {
    // add eax, 0x1234  (05 id, accumulator-immediate)
    let code = translate(0x3000, &[0x05, 0x34, 0x12, 0x00, 0x00]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1000;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1000 + 0x1234);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn sub_rax_imm32_accumulator_executes() {
    // sub rax, 0x10  (REX.W 2D id, accumulator-immediate)
    let code = translate(0x4000, &[0x48, 0x2D, 0x10, 0x00, 0x00, 0x00]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x100;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x100 - 0x10);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
