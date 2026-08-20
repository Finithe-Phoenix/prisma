//! ARM64 execution e2e for CMPXCHG r/m, r register-direct (0F B1).
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
fn cmpxchg_ecx_edx_failure_zero_extends_rax() {
    // 32-bit form (no REX.W): cmpxchg ecx, edx (0F B1 D1). On failure RAX gets
    // the 32-bit dst zero-extended (upper 32 bits cleared).
    let code = translate(0x1500, &[0x0F, 0xB1, 0xD1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0xFFFF_FFFF_0000_0005; // eax = 5, upper set
    state.gpr[gpr::RCX] = 0x0000_0007; // ecx = 7 (mismatch)
    state.gpr[gpr::RDX] = 99;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RAX],
            7,
            "32-bit failure write zero-extends RAX"
        );
        assert_eq!(state.gpr[gpr::RCX], 7, "dst unchanged on failure");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn cmpxchg_rax_aliased_dst_lets_dest_write_win() {
    // cmpxchg rax, rcx (REX.W 0F B1 modrm 0xC8 = reg=rcx, rm=rax): the r/m
    // operand aliases RAX so the compare always succeeds; the accumulator-then-
    // DEST store order means the dst write (rax := rcx) must win.
    let code = translate(0x2500, &[0x48, 0x0F, 0xB1, 0xC8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 42;
    state.gpr[gpr::RCX] = 99;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 99, "dest write wins for the RAX alias");
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

#[repr(align(16))]
struct AlignedPair([u64; 2]);

#[test]
fn cmpxchg16b_success_replaces_the_aligned_memory_pair() {
    // cmpxchg16b [rsi]: compare RDX:RAX with memory and replace it with
    // RCX:RBX. RSI is a guest offset rebased through CpuStateFrame::mem_base.
    let code = translate(0x3000, &[0x48, 0x0F, 0xC7, 0x0E]);
    let mut memory = AlignedPair([5, 6]);
    let mut state = CpuStateFrame::default();
    state.mem_base = memory.0.as_mut_ptr() as u64;
    state.gpr[gpr::RSI] = 0;
    state.gpr[gpr::RAX] = 5;
    state.gpr[gpr::RDX] = 6;
    state.gpr[gpr::RBX] = 9;
    state.gpr[gpr::RCX] = 10;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute pair compare-exchange on the ARM64 host");
        assert_eq!(memory.0, [9, 10]);
        assert_eq!(state.gpr[gpr::RAX], 5);
        assert_eq!(state.gpr[gpr::RDX], 6);
        assert_ne!(state.rflags & (1 << 6), 0, "successful pair CAS sets ZF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn cmpxchg16b_failure_loads_the_observed_pair_and_clears_zf() {
    let code = translate(0x3100, &[0x48, 0x0F, 0xC7, 0x0E]);
    let mut memory = AlignedPair([7, 8]);
    let mut state = CpuStateFrame::default();
    state.mem_base = memory.0.as_mut_ptr() as u64;
    state.gpr[gpr::RSI] = 0;
    state.gpr[gpr::RAX] = 5;
    state.gpr[gpr::RDX] = 6;
    state.gpr[gpr::RBX] = 9;
    state.gpr[gpr::RCX] = 10;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute pair compare-exchange on the ARM64 host");
        assert_eq!(memory.0, [7, 8], "failed pair CAS leaves memory intact");
        assert_eq!(state.gpr[gpr::RAX], 7);
        assert_eq!(state.gpr[gpr::RDX], 8);
        assert_eq!(state.rflags & (1 << 6), 0, "failed pair CAS clears ZF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
