//! ARM64 execution e2e for BT/BTS/BTR/BTC r/m64, imm8 (0F BA /4../7).
//!
//! Each test fuses a single bit-test instruction with `prisma-translator` and
//! runs it through the canonical [`prisma_runtime::executor`] path, checking the
//! affected GPR. Mirrors the C++ MVP: register-direct only, always 64-bit, no
//! real flags bank yet (CF materialization is a CmpFlags placeholder). GPR
//! assertions are gated to aarch64; off-target the translate + W^X install path
//! runs and the call is skipped with `WrongArch`.

#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};
use prisma_translator::Translator;

fn translate(addr: u64, program: &[u8]) -> Vec<u8> {
    let code = translate_allow_empty(addr, program);
    assert!(!code.is_empty(), "lowered to no code");
    code
}

// Plain BT has no architectural side effect we model yet (no register write,
// no flags bank), so the optimizer legitimately lowers it to no code. This
// variant still exercises the full decode -> optimize -> lower -> install
// path without asserting non-empty output.
fn translate_allow_empty(addr: u64, program: &[u8]) -> Vec<u8> {
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(addr, program, 64)
        .expect("fused block translation");
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
    block.code
}

#[test]
fn bts_rcx_4_sets_bit() {
    // bts rcx, 4  (0F BA E9 04: modrm 0xE9 = 11 101 001 -> reg=5(BTS), rm=1(rcx)).
    let code = translate(0x1000, &[0x0F, 0xBA, 0xE9, 0x04]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RCX], 1 << 4, "bit 4 set -> 0x10");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn btr_rcx_4_clears_bit() {
    // btr rcx, 4  (0F BA F1 04: modrm 0xF1 = 11 110 001 -> reg=6(BTR), rm=1(rcx)).
    let code = translate(0x2000, &[0x0F, 0xBA, 0xF1, 0x04]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0x10;
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RCX], 0, "bit 4 cleared -> 0");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn bt_rcx_4_leaves_register_unchanged() {
    // bt rcx, 4  (0F BA E1 04: modrm 0xE1 = 11 100 001 -> reg=4(BT), rm=1(rcx)).
    // Plain BT only tests; it must not write back to the register. With no flags
    // bank yet the optimizer may lower it to no code, which is correct.
    let code = translate_allow_empty(0x3000, &[0x0F, 0xBA, 0xE1, 0x04]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0x10;

    #[cfg(target_arch = "aarch64")]
    {
        if !code.is_empty() {
            execute_block(&code, &mut state).expect("execute on the ARM64 host");
        }
        assert_eq!(state.gpr[gpr::RCX], 0x10, "BT leaves rcx unchanged");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        if code.is_empty() {
            // Nothing to install; RCX trivially unchanged.
            assert_eq!(state.gpr[gpr::RCX], 0x10);
        } else {
            assert!(matches!(
                execute_block(&code, &mut state),
                Err(ExecError::WrongArch)
            ));
        }
    }
}

#[test]
fn btc_rcx_4_toggles_bit() {
    // btc rcx, 4  (0F BA F9 04: modrm 0xF9 = 11 111 001 -> reg=7(BTC), rm=1(rcx)).
    let code = translate(0x4000, &[0x0F, 0xBA, 0xF9, 0x04]);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0x10; // bit 4 already set -> toggles to clear.
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RCX], 0, "bit 4 toggled off");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
