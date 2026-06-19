//! ARM64 execution e2e for CMPXCHG r/m, r register-direct (0F B0/B1).
//!
//! Each test fuses a single `cmpxchg rcx, rdx` instruction with
//! `prisma-translator` and runs it through the canonical
//! [`prisma_runtime::executor`] path, checking RAX/RCX. Mirrors the C++ MVP:
//! register-direct only (memory/LOCK forms deferred). x86 semantics:
//!   if RAX == dst (rcx): dst = src (rdx), RAX unchanged  (ZF=1, success)
//!   else:                RAX = dst (rcx), dst unchanged  (ZF=0, failure)
//! GPR assertions are gated to aarch64; off-target the translate + W^X install
//! path runs and the call is skipped with `WrongArch`.

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
fn cmpxchg_rcx_rdx_success_writes_dst() {
    // cmpxchg rcx, rdx  (REX.W 0F B1 modrm 0xD1 = reg=rdx, rm=rcx).
    // RAX == RCX (5 == 5) -> success: RCX = RDX (99), RAX unchanged (5).
    let code = translate(0x1000, &[0x48, 0x0F, 0xB1, 0xD1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 5;
    state.gpr[gpr::RCX] = 5;
    state.gpr[gpr::RDX] = 99;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RCX], 99, "dst := src on match");
        assert_eq!(state.gpr[gpr::RAX], 5, "accumulator unchanged on success");
        assert_eq!(state.gpr[gpr::RDX], 99, "source untouched");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn cmpxchg_rcx_rdx_failure_writes_accumulator() {
    // RAX != RCX (5 != 7) -> failure: RAX = RCX (7), RCX unchanged (7).
    let code = translate(0x2000, &[0x48, 0x0F, 0xB1, 0xD1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 5;
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = 99;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 7, "accumulator := dst on mismatch");
        assert_eq!(state.gpr[gpr::RCX], 7, "dst unchanged on failure");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
