//! ARM64 execution e2e for ADC/SBB with real carry (0F-free 0x11/0x19, r/m,r).
//!
//! These prove the persistent-CF subsystem on real hardware: the test seeds
//! `state.cf`, runs an `adc`/`sbb`, and checks both the result register and the
//! new `state.cf`. The CF lives in a dedicated `CpuStateFrame` slot (offset 808)
//! so multi-precision chains survive between instructions. GPR/CF assertions are
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
fn adc_adds_carry_in_no_carry_out() {
    // adc rax, rcx (48 11 C8): rax = rax + rcx + CF. 10 + 20 + 1 = 31, no carry.
    let code = translate(0x1000, &[0x48, 0x11, 0xC8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 10;
    state.gpr[gpr::RCX] = 20;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 31, "10 + 20 + carry(1)");
        assert_eq!(state.cf, 0, "no carry out");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn adc_carry_out_on_overflow() {
    // adc rax, rcx: u64::MAX + 0 + CF(1) wraps to 0 with carry out.
    let code = translate(0x2000, &[0x48, 0x11, 0xC8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = u64::MAX;
    state.gpr[gpr::RCX] = 0;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0, "MAX + 1 wraps to 0");
        assert_eq!(state.cf, 1, "carry out set");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn sbb_subtracts_borrow_in() {
    // sbb rax, rcx (48 19 C8): rax = rax - rcx - CF. 10 - 3 - 1 = 6, no borrow.
    let code = translate(0x3000, &[0x48, 0x19, 0xC8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 10;
    state.gpr[gpr::RCX] = 3;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 6, "10 - 3 - borrow(1)");
        assert_eq!(state.cf, 0, "no borrow out");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn sbb_borrow_out_on_underflow() {
    // sbb rax, rcx: 0 - 0 - CF(1) underflows to -1 with borrow out.
    let code = translate(0x4000, &[0x48, 0x19, 0xC8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0;
    state.gpr[gpr::RCX] = 0;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], u64::MAX, "0 - borrow(1) = -1");
        assert_eq!(state.cf, 1, "borrow out set");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
