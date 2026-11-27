//! ARM64 execution e2e for one-operand MUL/IMUL/DIV/IDIV (Group 3 /4../7).
//!
//! Each test fuses a single `<op> rcx` instruction with `prisma-translator` and
//! runs it through the canonical [`prisma_runtime::executor`] path, checking the
//! RAX (low/quotient) and RDX (high/remainder) results. Mirrors the C++ MVP: the
//! narrow MUL/IMUL and DIV/IDIV forms use AX, DX:AX, or EDX:EAX, and the qword
//! forms use the full RDX:RAX dividend. GPR assertions are gated to aarch64;
//! off-target the translate + W^X install path runs and the call is skipped
//! with `WrongArch`.

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
fn mul_cl_writes_ax_without_touching_rdx() {
    // mul cl (F6 /4): AX = AL * CL. RDX is not an architectural output.
    let code = translate(0x1100, &[0xF6, 0xE1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1122_3344_5566_00ff;
    state.gpr[gpr::RCX] = 2;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_2211;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1122_3344_5566_01fe);
        assert_eq!(state.gpr[gpr::RDX], 0x8877_6655_4433_2211);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn mul_cx_writes_dx_ax_halves() {
    // mul cx (66 F7 /4): DX:AX = AX * CX.
    let code = translate(0x1200, &[0x66, 0xF7, 0xE1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1122_3344_5566_8000;
    state.gpr[gpr::RCX] = 2;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_2222;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1122_3344_5566_0000);
        assert_eq!(state.gpr[gpr::RDX], 0x8877_6655_4433_0001);
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
fn imul_ecx_writes_edx_eax_halves() {
    // imul ecx (F7 /5): signed EDX:EAX = EAX * ECX.
    let code = translate(0x2100, &[0xF7, 0xE9]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0xffff_ffff;
    state.gpr[gpr::RCX] = 2;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_2222;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0xffff_fffe);
        assert_eq!(state.gpr[gpr::RDX], 0xffff_ffff);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn div_rcx_uses_rdx_rax_dividend() {
    // div rcx (REX.W F7 /6): unsigned RDX:RAX / RCX -> RAX quotient, RDX remainder.
    let code = translate(0x3000, &[0x48, 0xF7, 0xF1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0;
    state.gpr[gpr::RCX] = 0x1_0000_0000;
    state.gpr[gpr::RDX] = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1_0000_0000);
        assert_eq!(state.gpr[gpr::RDX], 0);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn div_cl_writes_ah_al_without_touching_rdx() {
    // div cl (F6 /6): AX / CL -> AL quotient, AH remainder.
    let code = translate(0x3100, &[0xF6, 0xF1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1122_3344_5566_0123;
    state.gpr[gpr::RCX] = 0x12;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_2211;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1122_3344_5566_0310);
        assert_eq!(state.gpr[gpr::RDX], 0x8877_6655_4433_2211);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn div_cx_uses_dx_ax_dividend() {
    // div cx (66 F7 /6): DX:AX / CX -> AX quotient, DX remainder.
    let code = translate(0x3200, &[0x66, 0xF7, 0xF1]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1122_3344_5566_0000;
    state.gpr[gpr::RCX] = 0x100;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_0001;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1122_3344_5566_0100);
        assert_eq!(state.gpr[gpr::RDX], 0x8877_6655_4433_0000);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn idiv_cl_writes_ah_al_signed_without_touching_rdx() {
    // idiv cl (F6 /7): signed AX / CL -> AL quotient, AH remainder.
    let code = translate(0x3300, &[0xF6, 0xF9]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x1122_3344_5566_ffe2;
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = 0x8877_6655_4433_2211;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x1122_3344_5566_fefc);
        assert_eq!(state.gpr[gpr::RDX], 0x8877_6655_4433_2211);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn idiv_ecx_uses_edx_eax_signed_dividend() {
    // idiv ecx (F7 /7): signed EDX:EAX / ECX -> EAX quotient, EDX remainder.
    let code = translate(0x3400, &[0xF7, 0xF9]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0xffff_ff9c;
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = 0xffff_ffff;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0xffff_fff2);
        assert_eq!(state.gpr[gpr::RDX], 0xffff_fffe);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn idiv_rcx_uses_signed_rdx_rax_dividend() {
    // idiv rcx (REX.W F7 /7): signed RDX:RAX / RCX -> RAX quotient, RDX remainder.
    let code = translate(0x4000, &[0x48, 0xF7, 0xF9]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = (-100i64) as u64;
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = u64::MAX;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], (-14i64) as u64);
        assert_eq!(state.gpr[gpr::RDX], (-2i64) as u64);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
