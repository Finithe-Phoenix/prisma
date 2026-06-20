//! ARM64 execution e2e for RCL/RCR by 1 (D1 /2, /3) through the persistent CF.
//!
//! Rotate-through-carry: RCL shifts the MSB into CF and CF into bit 0; RCR
//! shifts the LSB into CF and CF into the MSB. The tests seed `state.cf`, run
//! the rotate, and check the result + new CF. aarch64-gated; off-target the
//! translate + W^X install path runs and the call reports `WrongArch`.

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
fn rcl_shifts_msb_into_carry() {
    // rcl rax, 1 (48 D1 D0): MSB set -> new CF = 1; (dst<<1)|CF(0).
    let code = translate(0x1000, &[0x48, 0xD1, 0xD0]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0x8000_0000_0000_0001;
    state.cf = 0;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0x2, "(MSB|1) << 1 with CF-in 0");
        assert_eq!(state.cf, 1, "old MSB -> CF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn rcl_rotates_carry_into_bit0() {
    // rcl rax, 1 with rax=0, CF=1 -> dst = 1, new CF = 0.
    let code = translate(0x2000, &[0x48, 0xD1, 0xD0]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 1, "CF rotates into bit 0");
        assert_eq!(state.cf, 0, "MSB of 0 -> CF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn rcr_shifts_lsb_into_carry() {
    // rcr rax, 1 (48 D1 D8): rax=1, CF=0 -> dst=0, new CF=1.
    let code = translate(0x3000, &[0x48, 0xD1, 0xD8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 1;
    state.cf = 0;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 0, "(1 >> 1) | (CF(0) << 63)");
        assert_eq!(state.cf, 1, "old LSB -> CF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn rcr_rotates_carry_into_msb() {
    // rcr rax, 1 with rax=0, CF=1 -> dst = 1<<63, new CF = 0.
    let code = translate(0x4000, &[0x48, 0xD1, 0xD8]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RAX] = 0;
    state.cf = 1;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(
            state.gpr[gpr::RAX],
            0x8000_0000_0000_0000,
            "CF rotates into MSB"
        );
        assert_eq!(state.cf, 0, "LSB of 0 -> CF");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
