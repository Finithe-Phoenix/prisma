//! ARM64 execution e2e for bare BSF/BSR (0F BC/BD).
//!
//! BSF = trailing-zero count (index of the lowest set bit); BSR = the high
//! set-bit index = (bit_width-1) - lzcnt. When the source is zero, x86 sets
//! ZF=1 and leaves the destination unchanged — exercised here via a seeded
//! destination. Each test fuses one instruction with `prisma-translator` and
//! runs it through `prisma_runtime::executor`; GPR asserts gated to aarch64,
//! `WrongArch` off-target.

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
fn bsf_finds_lowest_set_bit() {
    // bsf rax, rcx  (48 0F BC C1): rcx = 0xFF00 -> lowest set bit = 8.
    let code = translate(0x1000, &[0x48, 0x0F, 0xBC, 0xC1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0xFF00;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 8, "index of the lowest set bit");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn bsr_finds_highest_set_bit() {
    // bsr rax, rcx  (48 0F BD C1): rcx = 0xFF00 -> highest set bit = 15.
    let code = translate(0x2000, &[0x48, 0x0F, 0xBD, 0xC1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0xFF00;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 15, "index of the highest set bit");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn bsf_zero_source_leaves_destination_unchanged() {
    // bsf rax, rcx with rcx = 0: x86 leaves rax unchanged (ZF=1). The Select
    // keeps the old destination value.
    let code = translate(0x3000, &[0x48, 0x0F, 0xBC, 0xC1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0;
    state.gpr[gpr::RAX] = 0xDEAD;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0xDEAD, "zero source -> dst unchanged");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
