//! End-to-end: translate MOV-immediate guest programs with the full Rust
//! pipeline (`prisma-translator`: decode -> optimize -> lower), then
//! JIT-execute the ARM64 output and verify the destination registers.
//!
//! Mirrors `tests/exec_e2e.rs`: the execution half is gated to `aarch64`
//! (the translated bytes are ARM64 machine code), so the GPR asserts only run
//! on the `ffi-link-arm64` CI runner. On the x86 dev host the test still
//! translates and drives the W^X install path, asserting `ExecError::WrongArch`.
//!
//! Covers the broadened MOV-immediate family:
//!   - MOV r64, imm64  (REX.W B8+r) on an extended register (R8)
//!   - MOV r/m64, imm32 sign-extended (C7 /0)
//!   - MOV r8, imm8     (B0+r)
//!   - MOV r/m8, imm8   (C6 /0, register form)

#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};
use prisma_translator::Translator;

#[test]
fn translate_then_execute_mov_r8_imm64_then_copy() {
    // mov r8, 0xDEAD   (49 B8 AD DE 00 00 00 00 00 00)  REX.W+REX.B, B8+0
    // mov rax, r8      (4C 89 C0)                       REX.W+REX.R, 89 /r
    let mut program = Vec::new();
    program.extend_from_slice(&[0x49, 0xB8, 0xAD, 0xDE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    program.extend_from_slice(&[0x4C, 0x89, 0xC0]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x1000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 2);
    assert!(!block.code.is_empty());
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");

    let mut state = CpuStateFrame::default();
    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::R8], 0xDEAD, "mov r8, imm64");
        assert_eq!(state.gpr[gpr::RAX], 0xDEAD, "rax = r8");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(state.gpr[gpr::R8], 0, "no execution -> r8 unchanged");
        assert_eq!(state.gpr[gpr::RAX], 0, "no execution -> rax unchanged");
    }
}

#[test]
fn translate_then_execute_mov_rm64_imm32_sign_extends() {
    // mov rbx, -1   (48 C7 C3 FF FF FF FF)  C7 /0, imm32 sign-extended to I64
    let mut program = Vec::new();
    program.extend_from_slice(&[0x48, 0xC7, 0xC3, 0xFF, 0xFF, 0xFF, 0xFF]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x2000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 1);

    let mut state = CpuStateFrame::default();
    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RBX],
            u64::MAX,
            "C7 /0 imm32 0xFFFFFFFF sign-extends to all-ones in I64"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(state.gpr[gpr::RBX], 0, "no execution -> rbx unchanged");
    }
}

#[test]
fn translate_then_execute_mov_r8l_imm8() {
    // mov cl, 0x7F   (B1 7F)  B0+1, imm8 into the low byte of rcx
    let mut program = Vec::new();
    program.extend_from_slice(&[0xB1, 0x7F]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x3000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 1);

    // Pre-seed the upper bytes of rcx; an 8-bit move must leave them intact.
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0xAABB_CC00;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RCX],
            0xAABB_CC7F,
            "mov cl, 0x7F writes only the low byte"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(
            state.gpr[gpr::RCX],
            0xAABB_CC00,
            "no execution -> rcx unchanged"
        );
    }
}

#[test]
fn translate_then_execute_mov_rm8_imm8_register_form() {
    // mov dl, 0x42   (C6 C2 42)  C6 /0 register form (modrm 11 000 010 = rdx)
    let mut program = Vec::new();
    program.extend_from_slice(&[0xC6, 0xC2, 0x42]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x4000, &program, 64)
        .expect("translate the block");
    assert_eq!(block.instruction_count, 1);

    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RDX] = 0x1122_3300;

    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RDX],
            0x1122_3342,
            "mov dl, 0x42 writes only the low byte"
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(result, Err(ExecError::WrongArch)));
        assert_eq!(
            state.gpr[gpr::RDX],
            0x1122_3300,
            "no execution -> rdx unchanged"
        );
    }
}
