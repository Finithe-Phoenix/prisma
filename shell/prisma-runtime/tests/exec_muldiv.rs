//! ARM64 execution e2e for one-operand MUL/IMUL/DIV/IDIV (Group 3 /4../7).
//!
//! Each test fuses a single `<op> rcx` instruction with `prisma-translator` and
//! runs it through the canonical [`prisma_runtime::executor`] path, checking the
//! RAX (low/quotient) and RDX (high/remainder) results. Mirrors the C++ MVP: the
//! operand is RAX-only (full 128-bit RDX:RAX is deferred). GPR assertions are
//! gated to aarch64; off-target the translate + W^X install path runs and the
//! call is skipped with `WrongArch`.

#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};
use prisma_translator::Translator;

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
fn mul_rcx_writes_rdx_rax() {
    // mul rcx  (REX.W F7 /4): RDX:RAX = RAX * RCX (unsigned).
    let code = translate(0x1000, &[0x48, 0xF7, 0xE1]);
    let mut state = CpuStateFrame::default();
    // 2^32 * 2^32 = 2^64 -> low 0, high 1.
    state.gpr[gpr::RAX] = 0x1_0000_0000;
    state.gpr[gpr::RCX] = 0x1_0000_0000;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0, "low 64 bits of the product");
        assert_eq!(state.gpr[gpr::RDX], 1, "high 64 bits of the product");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn imul_rcx_sign_extends_high() {
    // imul rcx  (REX.W F7 /5): signed RDX:RAX = RAX * RCX.
    let code = translate(0x2000, &[0x48, 0xF7, 0xE9]);
    let mut state = CpuStateFrame::default();
    // (-1) * 2 = -2 -> low = 0xFFFF...FE, high = 0xFFFF...FF (sign extension).
    state.gpr[gpr::RAX] = (-1i64) as u64;
    state.gpr[gpr::RCX] = 2;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], (-2i64) as u64, "low half");
        assert_eq!(state.gpr[gpr::RDX], (-1i64) as u64, "signed high half");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn div_rcx_writes_quotient_and_remainder() {
    // div rcx  (REX.W F7 /6): RAX = RAX / RCX, RDX = RAX % RCX (unsigned).
    let code = translate(0x3000, &[0x48, 0xF7, 0xF1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 100;
    state.gpr[gpr::RCX] = 7;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 14, "quotient");
        assert_eq!(state.gpr[gpr::RDX], 2, "remainder");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn idiv_rcx_signed_quotient_and_remainder() {
    // idiv rcx  (REX.W F7 /7): signed RAX = RAX / RCX, RDX = RAX % RCX.
    let code = translate(0x4000, &[0x48, 0xF7, 0xF9]);
    let mut state = CpuStateFrame::default();
    // -100 / 7 = -14 (truncated toward zero), remainder -2.
    state.gpr[gpr::RAX] = (-100i64) as u64;
    state.gpr[gpr::RCX] = 7;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], (-14i64) as u64, "signed quotient");
        assert_eq!(state.gpr[gpr::RDX], (-2i64) as u64, "signed remainder");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
