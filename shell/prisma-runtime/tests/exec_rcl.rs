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

/// Reference the ISA one bit at a time, including count masking before the
/// modulo-9/17 reduction and the single-masked-count definition of OF.
fn expected_rotate(mut word: u64, width: u32, raw: u64, right: bool, flags: u64) -> (u64, u64) {
    let mask = u64::MAX >> (64 - width);
    word &= mask;
    let sign = word >> (width - 1);
    let mut cf = flags & 1;
    let masked = raw & if width == 64 { 63 } else { 31 };
    for _ in 0..masked % u64::from(width + 1) {
        (word, cf) = if right {
            ((word >> 1) | (cf << (width - 1)), word & 1)
        } else {
            (((word << 1) | cf) & mask, word >> (width - 1))
        };
    }
    let mut flags = (flags & !1) | cf;
    if masked == 1 {
        flags = (flags & !(1 << 11)) | ((sign ^ (word >> (width - 1))) << 11);
    }
    (word, flags)
}

#[test]
fn cl_rotates_match_value_and_all_flags() {
    use prisma_translator::interp::{interpret_block, BlockOutcome, GuestRegs};
    for (width, opcode) in [
        (8_u32, vec![0xd2]),
        (16, vec![0x66, 0xd3]),
        (32, vec![0xd3]),
        (64, vec![0x48, 0xd3]),
    ] {
        for right in [false, true] {
            for operand in [0xc0_u8, 0xc4, 0] {
                // register, AH, [rax]
                let high = operand == 0xc4;
                if high && width != 8 {
                    continue;
                }
                let mem = operand == 0;
                let mut program = opcode.clone();
                program.push(operand | if right { 0x18 } else { 0x10 });
                let code = translate(0x1000, &program);
                let decoded = prisma_decoder::decode_one(&program, 0).expect("decode");
                for raw in [0, 1, 2, u64::from(width), u64::from(width + 1), 33, 65, 255] {
                    for flags in [0x246, 0xad7] {
                        let original = 0xfedc_ba91_9837_6789_u64;
                        let offset = if high { 8 } else { 0 };
                        let mask = u64::MAX >> (64 - width) << offset;
                        let (word, expected_flags) =
                            expected_rotate(original >> offset, width, raw, right, flags);
                        let expected = if !mem && width == 32 {
                            word
                        } else {
                            original & !mask | word << offset
                        };
                        if !mem {
                            let mut regs = GuestRegs {
                                cf: flags & 1,
                                rflags: flags,
                                ..GuestRegs::default()
                            };
                            regs.gpr[gpr::RAX] = original;
                            regs.gpr[gpr::RCX] = raw;
                            assert_eq!(
                                interpret_block(&decoded.stmts, &mut regs),
                                BlockOutcome::Fallthrough
                            );
                            assert_eq!(
                                (regs.gpr[gpr::RAX], regs.rflags, regs.cf),
                                (expected, expected_flags, expected_flags & 1)
                            );
                        }
                        #[cfg(target_arch = "aarch64")]
                        {
                            let mut backing = original;
                            let mut state = CpuStateFrame::default();
                            state.gpr[gpr::RAX] = if mem {
                                std::ptr::from_mut(&mut backing) as u64
                            } else {
                                original
                            };
                            state.gpr[gpr::RCX] = raw;
                            state.cf = flags & 1;
                            state.rflags = flags;
                            execute_block(&code, &mut state).unwrap();
                            let value = if mem { backing } else { state.gpr[gpr::RAX] };
                            assert_eq!(
                                (value, state.rflags, state.cf),
                                (expected, expected_flags, expected_flags & 1)
                            );
                        }
                    }
                }
                #[cfg(not(target_arch = "aarch64"))]
                assert!(!code.is_empty());
            }
        }
    }
}
