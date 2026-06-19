/// OpCode category for one-byte dispatch (0x00..0xFF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneByteOpcode {
    /// MOV r/m8, r8 (88 /r)
    MovRmR8,
    /// MOV r/m64, r64 (REX.W 89 /r)
    MovRmR,
    /// MOV r8, r/m8 (8A /r)
    MovR8Rm,
    /// MOV r64, r/m64 (REX.W 8B /r)
    MovRRm,
    /// POP r/m64 (8F /0)
    PopRm,
    /// LEA r64, m (REX.W 8D /r)
    Lea,
    /// MOV r64, imm64 (REX.W B8+rd)
    MovRI,
    /// MOV r8, imm8 (B0+rb)
    MovR8I8,
    /// MOV AL/AX/EAX/RAX, moffs (A0/A1)
    MovMoffsToAcc,
    /// MOV moffs, AL/AX/EAX/RAX (A2/A3)
    MovAccToMoffs,
    /// MOVS (A4/A5)
    Movsb,
    /// CMPS (A6/A7)
    Cmpsb,
    /// STOS (AA/AB)
    Stosb,
    /// LODS (AC/AD)
    Lodsb,
    /// SCAS (AE/AF)
    Scasb,
    /// ADD r/m64, r64 (REX.W 01 /r)
    AddRmR,
    /// ADD r/m8, r8 (00 /r)
    AddRmR8,
    /// ADD r64, r/m64 (REX.W 03 /r)
    AddRRm,
    /// ADD r8, r/m8 (02 /r)
    AddR8Rm,
    /// ALU/test/cmp accumulator immediate forms.
    AccImm,
    /// ADD/SUB/... r/m, full-width immediate (80/81 /digit).
    AluRmImm,
    /// ADD/SUB/... r/m, imm8 sign-extended (83 /digit ib)
    AluRmImm8,
    /// Shift/rotate r/m by immediate or CL (C0/C1/D0/D1/D2/D3 /digit).
    Group2,
    /// SUB r/m64, r64 (REX.W 29 /r)
    SubRmR,
    /// SUB r/m8, r8 (28 /r)
    SubRmR8,
    /// SUB r64, r/m64 (REX.W 2B /r)
    SubRRm,
    /// SUB r8, r/m8 (2A /r)
    SubR8Rm,
    /// AND r/m64, r64 (REX.W 21 /r)
    AndRmR,
    /// AND r/m8, r8 (20 /r)
    AndRmR8,
    /// AND r8, r/m8 (22 /r)
    AndR8Rm,
    /// AND r64, r/m64 (REX.W 23 /r)
    AndRRm,
    /// OR r/m64, r64 (REX.W 09 /r)
    OrRmR,
    /// OR r/m8, r8 (08 /r)
    OrRmR8,
    /// OR r8, r/m8 (0A /r)
    OrR8Rm,
    /// OR r64, r/m64 (REX.W 0B /r)
    OrRRm,
    /// XOR r/m64, r64 (REX.W 31 /r)
    XorRmR,
    /// XOR r/m8, r8 (30 /r)
    XorRmR8,
    /// XOR r8, r/m8 (32 /r)
    XorR8Rm,
    /// XOR r64, r/m64 (REX.W 33 /r)
    XorRRm,
    /// CMP r/m64, r64 (REX.W 39 /r)
    CmpRmR,
    /// CMP r/m8, r8 (38 /r)
    CmpRmR8,
    /// CMP r8, r/m8 (3A /r)
    CmpR8Rm,
    /// CMP r64, r/m64 (REX.W 3B /r)
    CmpRRm,
    /// TEST r/m64, r/m64 (REX.W 85 /r)
    TestRmR,
    /// TEST r/m8, r8 (84 /r)
    TestRmR8,
    /// Legacy instructions invalid in 64-bit mode; emit SIGILL trap.
    LegacyInvalidTrap,
    /// Legacy invalid instruction with imm8 operand; emit SIGILL trap.
    LegacyInvalidImm8,
    /// MOVSXD r64, r/m32 (REX.W 63 /r)
    Movsxd,
    /// IMUL r, r/m, imm (69 /r iw/id)
    ImulRmImm,
    /// IMUL r, r/m, imm8 (6B /r ib)
    ImulRmImm8,
    /// MOV r/m, imm (C6/C7 /0)
    MovRmImm,
    /// CALL rel32 (E8 cd)
    CallRel32,
    /// NOP if no REX.W prefix.
    Nop,
    /// FWAIT/WAIT x87 synchronization no-op (9B).
    Fwait,
    /// CLD clears the direction flag; string ops are modeled forward-only.
    Cld,
    /// XCHG rAX, r (91..97)
    XchgAcc,
    /// CBW/CWDE/CDQE (98)
    SignExtendAcc,
    /// CWD/CDQ/CQO (99)
    SignExtendAccToDx,
    /// XLAT/XLATB (D7)
    Xlat,
    /// x87 D9 group, currently exact FNOP (D9 D0).
    X87D9,
    /// x87 DB group, currently exact FNCLEX/FNINIT (DB E2/E3).
    X87DB,
    /// RET (C3)
    Ret,
    /// RET imm16 (C2 iw)
    RetImm16,
    /// IRET/IRETD/IRETQ privileged return from interrupt (CF).
    IretTrap,
    /// ENTER imm16, imm8 (C8 iw ib)
    Enter,
    /// LEAVE (C9)
    Leave,
    /// INT3 breakpoint trap (CC)
    Int3,
    /// INT imm8 software interrupt trap (CD ib)
    IntImm8,
    /// JMP rel8 (EB cb)
    JumpRel8,
    /// LOOP rel8 (E2 cb)
    LoopRel8,
    /// JRCXZ/JECXZ rel8 (E3 cb)
    JrcxzRel8,
    /// JMP rel32 (E9 cd)
    JumpRel32,
    /// IN/OUT with immediate port (E4..E7 ib)
    IoTrapImm8,
    /// IN/OUT with DX port (EC..EF)
    IoTrapDx,
    /// JCC rel8 (0x70..0x7F).
    CondJumpRel8,
    /// XCHG r64, r/m64 (REX.W 87 /r)
    Xchg,
    /// PUSH r64 (50+rd)
    PushReg,
    /// PUSH imm16/imm32/imm8 (68/6A)
    PushImm,
    /// POP r64 (58+rd)
    PopReg,
    /// ICEBP/INT1 breakpoint trap (F1)
    Icebp,
    /// HLT privileged halt trap (F4)
    Hlt,
    /// CLI/STI privileged interrupt flag instructions (FA/FB)
    PrivilegedTrap,
    /// 0xF6/0xF7 group 3
    Group3,
    /// 0xFE group 4
    Group4,
    /// 0xFF group 5
    Group5,
    /// 0x0F escape.
    TwoBytePrefix,
    Unsupported,
}

/// OpCode category for two-byte dispatch (0x0F 00..0xFF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoByteOpcode {
    /// 0F 00 descriptor-table sub-opcode group.
    DescriptorGroup,
    /// 0F 01 system sub-opcode group.
    SystemGroup,
    /// Privileged fixed two-byte system instruction trap.
    SystemTrap,
    /// Privileged two-byte system instruction with ModRM trap.
    SystemTrapRm,
    /// SYSCALL (0F 05).
    Syscall,
    /// UD2 undefined instruction trap (0F 0B).
    Ud2,
    /// INVD/WBINVD privileged cache instructions (0F 08/09).
    CacheTrap,
    /// Multi-byte NOP (0F 1F /0).
    NopRm,
    /// RDTSC (0F 31).
    Rdtsc,
    /// PREFETCH hint (0F 18 /0..3).
    Prefetch,
    /// AMD PREFETCH/PREFETCHW hint (0F 0D /0..1).
    Prefetchw,
    /// EMMS/FEMMS MMX state cleanup no-op for integer IR (0F 77, 0F 0E).
    MmxStateNoop,
    /// ENDBR32/ENDBR64 (F3 0F 1E FB/FA).
    Endbr,
    /// CMOVcc r, r/m (0F 40..0F 4F).
    Cmov,
    /// JCC rel32 (0F 80..0F 8F).
    CondJumpRel32,
    /// SETcc r/m8 (0F 90..0F 9F).
    Setcc,
    /// CPUID (0F A2).
    Cpuid,
    /// MOVZX r64, r/m8 (0F B6)
    MovzxI8,
    /// MOVZX r64, r/m16 (0F B7)
    MovzxI16,
    /// UD1/UD0 undefined instruction with ModRM (0F B9 /r, 0F FF /r).
    UndefinedRm,
    /// MOVSX r64, r/m8 (0F BE)
    MovsxI8,
    /// MOVSX r64, r/m16 (0F BF)
    MovsxI16,
    /// IMUL r, r/m (0F AF /r)
    ImulRm,
    /// POPCNT (F3 0F B8 /r)
    Popcnt,
    /// XADD r/m, r (0F C0/C1 /r)
    Xadd,
    /// MOVNTI m, r (0F C3 /r)
    Movnti,
    /// LFENCE/MFENCE/SFENCE (0F AE E8/F0/F8)
    Fence,
    /// LZCNT (F3 0F BD /r)
    Lzcnt,
    /// TZCNT (F3 0F BC /r)
    Tzcnt,
    /// BSWAP r32/r64 (0F C8+rd)
    Bswap,
    /// CMPXCHG r/m8, r8 (0F B0) and r/m, r (0F B1). The opcode byte is
    /// threaded through so the decoder picks I8 (0xB0) vs. prefix-sized
    /// (0xB1) operands.
    Cmpxchg,
    /// Three-byte 0F 38 escape map.
    ThreeByte0F38,
    Unsupported,
}

/// Classify a one-byte opcode using explicit tables by value.
pub const fn classify_one_byte(opcode: u8) -> OneByteOpcode {
    match opcode {
        0x88u8 => OneByteOpcode::MovRmR8,
        0x89u8 => OneByteOpcode::MovRmR,
        0x8Au8 => OneByteOpcode::MovR8Rm,
        0x8Bu8 => OneByteOpcode::MovRRm,
        0x8Fu8 => OneByteOpcode::PopRm,
        0x8Du8 => OneByteOpcode::Lea,
        0xA0u8 | 0xA1u8 => OneByteOpcode::MovMoffsToAcc,
        0xA2u8 | 0xA3u8 => OneByteOpcode::MovAccToMoffs,
        0xA4u8 | 0xA5u8 => OneByteOpcode::Movsb,
        0xA6u8 | 0xA7u8 => OneByteOpcode::Cmpsb,
        0xAAu8 | 0xABu8 => OneByteOpcode::Stosb,
        0xACu8 | 0xADu8 => OneByteOpcode::Lodsb,
        0xAEu8 | 0xAFu8 => OneByteOpcode::Scasb,
        0xB0u8..=0xB7u8 => OneByteOpcode::MovR8I8,
        0xB8u8..=0xBFu8 => OneByteOpcode::MovRI,
        0x04u8 | 0x05u8 | 0x0Cu8 | 0x0Du8 | 0x14u8 | 0x15u8 | 0x1Cu8 | 0x1Du8 | 0x24u8 | 0x25u8
        | 0x2Cu8 | 0x2Du8 | 0x34u8 | 0x35u8 | 0x3Cu8 | 0x3Du8 | 0xA8u8 | 0xA9u8 => {
            OneByteOpcode::AccImm
        }
        0x00u8 => OneByteOpcode::AddRmR8,
        0x01u8 => OneByteOpcode::AddRmR,
        0x02u8 => OneByteOpcode::AddR8Rm,
        0x03u8 => OneByteOpcode::AddRRm,
        0x10u8 => OneByteOpcode::AddRmR8,
        0x11u8 => OneByteOpcode::AddRmR,
        0x12u8 => OneByteOpcode::AddR8Rm,
        0x13u8 => OneByteOpcode::AddRRm,
        0x08u8 => OneByteOpcode::OrRmR8,
        0x0Au8 => OneByteOpcode::OrR8Rm,
        0x0Bu8 => OneByteOpcode::OrRRm,
        0x80u8 | 0x81u8 => OneByteOpcode::AluRmImm,
        0x83u8 => OneByteOpcode::AluRmImm8,
        0xC0u8 | 0xC1u8 | 0xD0u8 | 0xD1u8 | 0xD2u8 | 0xD3u8 => OneByteOpcode::Group2,
        0x20u8 => OneByteOpcode::AndRmR8,
        0x29u8 => OneByteOpcode::SubRmR,
        0x28u8 => OneByteOpcode::SubRmR8,
        0x2Au8 => OneByteOpcode::SubR8Rm,
        0x2Bu8 => OneByteOpcode::SubRRm,
        0x18u8 => OneByteOpcode::SubRmR8,
        0x19u8 => OneByteOpcode::SubRmR,
        0x1Au8 => OneByteOpcode::SubR8Rm,
        0x1Bu8 => OneByteOpcode::SubRRm,
        0x21u8 => OneByteOpcode::AndRmR,
        0x22u8 => OneByteOpcode::AndR8Rm,
        0x23u8 => OneByteOpcode::AndRRm,
        0x09u8 => OneByteOpcode::OrRmR,
        0x30u8 => OneByteOpcode::XorRmR8,
        0x31u8 => OneByteOpcode::XorRmR,
        0x32u8 => OneByteOpcode::XorR8Rm,
        0x33u8 => OneByteOpcode::XorRRm,
        0x38u8 => OneByteOpcode::CmpRmR8,
        0x39u8 => OneByteOpcode::CmpRmR,
        0x3Au8 => OneByteOpcode::CmpR8Rm,
        0x3Bu8 => OneByteOpcode::CmpRRm,
        0x84u8 => OneByteOpcode::TestRmR8,
        0x85u8 => OneByteOpcode::TestRmR,
        0x06u8 | 0x07u8 | 0x0Eu8 | 0x16u8 | 0x17u8 | 0x1Eu8 | 0x1Fu8 | 0x27u8 | 0x2Fu8 | 0x37u8
        | 0x3Fu8 | 0x60u8 | 0x61u8 | 0x9Au8 | 0xCEu8 | 0xD6u8 | 0xEAu8 => {
            OneByteOpcode::LegacyInvalidTrap
        }
        0xD4u8 | 0xD5u8 => OneByteOpcode::LegacyInvalidImm8,
        0x63u8 => OneByteOpcode::Movsxd,
        0x69u8 => OneByteOpcode::ImulRmImm,
        0x6Bu8 => OneByteOpcode::ImulRmImm8,
        0xC6u8 | 0xC7u8 => OneByteOpcode::MovRmImm,
        0xE8u8 => OneByteOpcode::CallRel32,
        0x90u8 => OneByteOpcode::Nop,
        0x9Bu8 => OneByteOpcode::Fwait,
        0x91u8..=0x97u8 => OneByteOpcode::XchgAcc,
        0x98u8 => OneByteOpcode::SignExtendAcc,
        0x99u8 => OneByteOpcode::SignExtendAccToDx,
        0xD7u8 => OneByteOpcode::Xlat,
        0xD9u8 => OneByteOpcode::X87D9,
        0xDBu8 => OneByteOpcode::X87DB,
        0xC3u8 => OneByteOpcode::Ret,
        0xC2u8 => OneByteOpcode::RetImm16,
        0xCFu8 => OneByteOpcode::IretTrap,
        0xC8u8 => OneByteOpcode::Enter,
        0xC9u8 => OneByteOpcode::Leave,
        0xCCu8 => OneByteOpcode::Int3,
        0xCDu8 => OneByteOpcode::IntImm8,
        0xE2u8 => OneByteOpcode::LoopRel8,
        0xE3u8 => OneByteOpcode::JrcxzRel8,
        0xE4u8..=0xE7u8 => OneByteOpcode::IoTrapImm8,
        0xE9u8 => OneByteOpcode::JumpRel32,
        0xEBu8 => OneByteOpcode::JumpRel8,
        0xECu8..=0xEFu8 => OneByteOpcode::IoTrapDx,
        0x70u8..=0x7Fu8 => OneByteOpcode::CondJumpRel8,
        0x86u8 | 0x87u8 => OneByteOpcode::Xchg,
        0x50u8..=0x57u8 => OneByteOpcode::PushReg,
        0x68u8 | 0x6Au8 => OneByteOpcode::PushImm,
        0x58u8..=0x5Fu8 => OneByteOpcode::PopReg,
        0xF1u8 => OneByteOpcode::Icebp,
        0xF4u8 => OneByteOpcode::Hlt,
        0xFAu8 | 0xFBu8 => OneByteOpcode::PrivilegedTrap,
        0xFCu8 => OneByteOpcode::Cld,
        0xF6u8 | 0xF7u8 => OneByteOpcode::Group3,
        0xFEu8 => OneByteOpcode::Group4,
        0xFFu8 => OneByteOpcode::Group5,
        0x0Fu8 => OneByteOpcode::TwoBytePrefix,
        _ => OneByteOpcode::Unsupported,
    }
}

/// Classify a two-byte opcode using explicit tables by value.
pub const fn classify_two_byte(opcode: u8) -> TwoByteOpcode {
    match opcode {
        0x00u8 => TwoByteOpcode::DescriptorGroup,
        0x01u8 => TwoByteOpcode::SystemGroup,
        0x06u8 | 0x07u8 => TwoByteOpcode::SystemTrap,
        0x05u8 => TwoByteOpcode::Syscall,
        0x08u8 | 0x09u8 => TwoByteOpcode::CacheTrap,
        0x0Bu8 => TwoByteOpcode::Ud2,
        0x0Du8 => TwoByteOpcode::Prefetchw,
        0x0Eu8 => TwoByteOpcode::MmxStateNoop,
        0x18u8 => TwoByteOpcode::Prefetch,
        0x1Eu8 => TwoByteOpcode::Endbr,
        0x1Fu8 => TwoByteOpcode::NopRm,
        0x20u8..=0x23u8 => TwoByteOpcode::SystemTrapRm,
        0x37u8 => TwoByteOpcode::SystemTrap,
        0x30u8 | 0x32u8 | 0x33u8 | 0x34u8 | 0x35u8 => TwoByteOpcode::SystemTrap,
        0x31u8 => TwoByteOpcode::Rdtsc,
        0x40u8..=0x4Fu8 => TwoByteOpcode::Cmov,
        0x77u8 => TwoByteOpcode::MmxStateNoop,
        0x78u8 | 0x79u8 => TwoByteOpcode::SystemTrapRm,
        0x80u8..=0x8Fu8 => TwoByteOpcode::CondJumpRel32,
        0x90u8..=0x9Fu8 => TwoByteOpcode::Setcc,
        0xA2u8 => TwoByteOpcode::Cpuid,
        0xAAu8 => TwoByteOpcode::SystemTrap,
        0xB6u8 => TwoByteOpcode::MovzxI8,
        0xB7u8 => TwoByteOpcode::MovzxI16,
        0xB9u8 | 0xFFu8 => TwoByteOpcode::UndefinedRm,
        0xAFu8 => TwoByteOpcode::ImulRm,
        0xBEu8 => TwoByteOpcode::MovsxI8,
        0xBFu8 => TwoByteOpcode::MovsxI16,
        0xB8u8 => TwoByteOpcode::Popcnt,
        0xAEu8 => TwoByteOpcode::Fence,
        0xC0u8 | 0xC1u8 => TwoByteOpcode::Xadd,
        0xC3u8 => TwoByteOpcode::Movnti,
        0xBDu8 => TwoByteOpcode::Lzcnt,
        0xBCu8 => TwoByteOpcode::Tzcnt,
        // Only 0F B1 (CMPXCHG r/m,r). The C++ reference does not decode the
        // r/m8,r8 form (0F B0), so leave it Unsupported to keep the differential.
        0xB1u8 => TwoByteOpcode::Cmpxchg,
        0xC8u8..=0xCFu8 => TwoByteOpcode::Bswap,
        0x38u8 => TwoByteOpcode::ThreeByte0F38,
        _ => TwoByteOpcode::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_byte_dispatch_table_classifies_common_cases() {
        assert_eq!(classify_one_byte(0x88), OneByteOpcode::MovRmR8);
        assert_eq!(classify_one_byte(0x89), OneByteOpcode::MovRmR);
        assert_eq!(classify_one_byte(0x8A), OneByteOpcode::MovR8Rm);
        assert_eq!(classify_one_byte(0x8B), OneByteOpcode::MovRRm);
        assert_eq!(classify_one_byte(0x8F), OneByteOpcode::PopRm);
        assert_eq!(classify_one_byte(0xA0), OneByteOpcode::MovMoffsToAcc);
        assert_eq!(classify_one_byte(0xA1), OneByteOpcode::MovMoffsToAcc);
        assert_eq!(classify_one_byte(0xA2), OneByteOpcode::MovAccToMoffs);
        assert_eq!(classify_one_byte(0xA3), OneByteOpcode::MovAccToMoffs);
        assert_eq!(classify_one_byte(0xA4), OneByteOpcode::Movsb);
        assert_eq!(classify_one_byte(0xA5), OneByteOpcode::Movsb);
        assert_eq!(classify_one_byte(0xA6), OneByteOpcode::Cmpsb);
        assert_eq!(classify_one_byte(0xA7), OneByteOpcode::Cmpsb);
        assert_eq!(classify_one_byte(0xAA), OneByteOpcode::Stosb);
        assert_eq!(classify_one_byte(0xAB), OneByteOpcode::Stosb);
        assert_eq!(classify_one_byte(0xAC), OneByteOpcode::Lodsb);
        assert_eq!(classify_one_byte(0xAD), OneByteOpcode::Lodsb);
        assert_eq!(classify_one_byte(0xAE), OneByteOpcode::Scasb);
        assert_eq!(classify_one_byte(0xAF), OneByteOpcode::Scasb);
        assert_eq!(classify_one_byte(0xB0), OneByteOpcode::MovR8I8);
        assert_eq!(classify_one_byte(0xB7), OneByteOpcode::MovR8I8);
        assert_eq!(classify_one_byte(0x00), OneByteOpcode::AddRmR8);
        assert_eq!(classify_one_byte(0x01), OneByteOpcode::AddRmR);
        assert_eq!(classify_one_byte(0x02), OneByteOpcode::AddR8Rm);
        assert_eq!(classify_one_byte(0x03), OneByteOpcode::AddRRm);
        assert_eq!(classify_one_byte(0x05), OneByteOpcode::AccImm);
        assert_eq!(classify_one_byte(0x08), OneByteOpcode::OrRmR8);
        assert_eq!(classify_one_byte(0x0A), OneByteOpcode::OrR8Rm);
        assert_eq!(classify_one_byte(0x0B), OneByteOpcode::OrRRm);
        assert_eq!(classify_one_byte(0xA9), OneByteOpcode::AccImm);
        assert_eq!(classify_one_byte(0x81), OneByteOpcode::AluRmImm);
        assert_eq!(classify_one_byte(0x83), OneByteOpcode::AluRmImm8);
        assert_eq!(classify_one_byte(0xC0), OneByteOpcode::Group2);
        assert_eq!(classify_one_byte(0xC1), OneByteOpcode::Group2);
        assert_eq!(classify_one_byte(0xD3), OneByteOpcode::Group2);
        assert_eq!(classify_one_byte(0x20), OneByteOpcode::AndRmR8);
        assert_eq!(classify_one_byte(0x22), OneByteOpcode::AndR8Rm);
        assert_eq!(classify_one_byte(0x23), OneByteOpcode::AndRRm);
        assert_eq!(classify_one_byte(0x28), OneByteOpcode::SubRmR8);
        assert_eq!(classify_one_byte(0x2A), OneByteOpcode::SubR8Rm);
        assert_eq!(classify_one_byte(0x30), OneByteOpcode::XorRmR8);
        assert_eq!(classify_one_byte(0x32), OneByteOpcode::XorR8Rm);
        assert_eq!(classify_one_byte(0x33), OneByteOpcode::XorRRm);
        assert_eq!(classify_one_byte(0x38), OneByteOpcode::CmpRmR8);
        assert_eq!(classify_one_byte(0x3A), OneByteOpcode::CmpR8Rm);
        assert_eq!(classify_one_byte(0x3B), OneByteOpcode::CmpRRm);
        assert_eq!(classify_one_byte(0x84), OneByteOpcode::TestRmR8);
        assert_eq!(classify_one_byte(0x06), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x07), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x0E), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x16), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x17), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x1E), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x1F), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x27), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x2F), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x37), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x3F), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x60), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x61), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0x9A), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0xCE), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0xD6), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0xEA), OneByteOpcode::LegacyInvalidTrap);
        assert_eq!(classify_one_byte(0xD4), OneByteOpcode::LegacyInvalidImm8);
        assert_eq!(classify_one_byte(0xD5), OneByteOpcode::LegacyInvalidImm8);
        assert_eq!(classify_one_byte(0x0F), OneByteOpcode::TwoBytePrefix);
        assert_eq!(classify_one_byte(0xE8), OneByteOpcode::CallRel32);
        assert_eq!(classify_one_byte(0x70), OneByteOpcode::CondJumpRel8);
        assert_eq!(classify_one_byte(0x90), OneByteOpcode::Nop);
        assert_eq!(classify_one_byte(0x9B), OneByteOpcode::Fwait);
        assert_eq!(classify_one_byte(0x86), OneByteOpcode::Xchg);
        assert_eq!(classify_one_byte(0x91), OneByteOpcode::XchgAcc);
        assert_eq!(classify_one_byte(0x97), OneByteOpcode::XchgAcc);
        assert_eq!(classify_one_byte(0x98), OneByteOpcode::SignExtendAcc);
        assert_eq!(classify_one_byte(0x99), OneByteOpcode::SignExtendAccToDx);
        assert_eq!(classify_one_byte(0xD7), OneByteOpcode::Xlat);
        assert_eq!(classify_one_byte(0xD9), OneByteOpcode::X87D9);
        assert_eq!(classify_one_byte(0xDB), OneByteOpcode::X87DB);
        assert_eq!(classify_one_byte(0x8D), OneByteOpcode::Lea);
        assert_eq!(classify_one_byte(0x63), OneByteOpcode::Movsxd);
        assert_eq!(classify_one_byte(0x69), OneByteOpcode::ImulRmImm);
        assert_eq!(classify_one_byte(0x6B), OneByteOpcode::ImulRmImm8);
        assert_eq!(classify_one_byte(0xC7), OneByteOpcode::MovRmImm);
        assert_eq!(classify_one_byte(0xC3), OneByteOpcode::Ret);
        assert_eq!(classify_one_byte(0xC2), OneByteOpcode::RetImm16);
        assert_eq!(classify_one_byte(0xCF), OneByteOpcode::IretTrap);
        assert_eq!(classify_one_byte(0xC8), OneByteOpcode::Enter);
        assert_eq!(classify_one_byte(0xC9), OneByteOpcode::Leave);
        assert_eq!(classify_one_byte(0xCC), OneByteOpcode::Int3);
        assert_eq!(classify_one_byte(0xCD), OneByteOpcode::IntImm8);
        assert_eq!(classify_one_byte(0xE2), OneByteOpcode::LoopRel8);
        assert_eq!(classify_one_byte(0xE3), OneByteOpcode::JrcxzRel8);
        assert_eq!(classify_one_byte(0xE4), OneByteOpcode::IoTrapImm8);
        assert_eq!(classify_one_byte(0xE7), OneByteOpcode::IoTrapImm8);
        assert_eq!(classify_one_byte(0xE9), OneByteOpcode::JumpRel32);
        assert_eq!(classify_one_byte(0xEB), OneByteOpcode::JumpRel8);
        assert_eq!(classify_one_byte(0xEC), OneByteOpcode::IoTrapDx);
        assert_eq!(classify_one_byte(0xEF), OneByteOpcode::IoTrapDx);
        assert_eq!(classify_one_byte(0x68), OneByteOpcode::PushImm);
        assert_eq!(classify_one_byte(0x6A), OneByteOpcode::PushImm);
        assert_eq!(classify_one_byte(0xF1), OneByteOpcode::Icebp);
        assert_eq!(classify_one_byte(0xF4), OneByteOpcode::Hlt);
        assert_eq!(classify_one_byte(0xFA), OneByteOpcode::PrivilegedTrap);
        assert_eq!(classify_one_byte(0xFB), OneByteOpcode::PrivilegedTrap);
        assert_eq!(classify_one_byte(0xFC), OneByteOpcode::Cld);
        assert_eq!(classify_one_byte(0xF6), OneByteOpcode::Group3);
        assert_eq!(classify_one_byte(0xF7), OneByteOpcode::Group3);
        assert_eq!(classify_one_byte(0xFE), OneByteOpcode::Group4);
        assert_eq!(classify_one_byte(0xFF), OneByteOpcode::Group5);
        assert_eq!(classify_one_byte(0x10), OneByteOpcode::AddRmR8);
        assert_eq!(classify_one_byte(0x13), OneByteOpcode::AddRRm);
        assert_eq!(classify_one_byte(0x18), OneByteOpcode::SubRmR8);
        assert_eq!(classify_one_byte(0x1B), OneByteOpcode::SubRRm);
    }

    #[test]
    fn two_byte_dispatch_table_classifies_common_cases() {
        assert_eq!(classify_two_byte(0x00), TwoByteOpcode::DescriptorGroup);
        assert_eq!(classify_two_byte(0x01), TwoByteOpcode::SystemGroup);
        assert_eq!(classify_two_byte(0x05), TwoByteOpcode::Syscall);
        assert_eq!(classify_two_byte(0x06), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x07), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x08), TwoByteOpcode::CacheTrap);
        assert_eq!(classify_two_byte(0x09), TwoByteOpcode::CacheTrap);
        assert_eq!(classify_two_byte(0x0B), TwoByteOpcode::Ud2);
        assert_eq!(classify_two_byte(0x0E), TwoByteOpcode::MmxStateNoop);
        assert_eq!(classify_two_byte(0x1F), TwoByteOpcode::NopRm);
        assert_eq!(classify_two_byte(0x0D), TwoByteOpcode::Prefetchw);
        assert_eq!(classify_two_byte(0x18), TwoByteOpcode::Prefetch);
        assert_eq!(classify_two_byte(0x1E), TwoByteOpcode::Endbr);
        assert_eq!(classify_two_byte(0x20), TwoByteOpcode::SystemTrapRm);
        assert_eq!(classify_two_byte(0x23), TwoByteOpcode::SystemTrapRm);
        assert_eq!(classify_two_byte(0x31), TwoByteOpcode::Rdtsc);
        assert_eq!(classify_two_byte(0x30), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x32), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x33), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x34), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x35), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x37), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x77), TwoByteOpcode::MmxStateNoop);
        assert_eq!(classify_two_byte(0x78), TwoByteOpcode::SystemTrapRm);
        assert_eq!(classify_two_byte(0x79), TwoByteOpcode::SystemTrapRm);
        assert_eq!(classify_two_byte(0x80), TwoByteOpcode::CondJumpRel32);
        assert_eq!(classify_two_byte(0x94), TwoByteOpcode::Setcc);
        assert_eq!(classify_two_byte(0xA2), TwoByteOpcode::Cpuid);
        assert_eq!(classify_two_byte(0xAA), TwoByteOpcode::SystemTrap);
        assert_eq!(classify_two_byte(0x44), TwoByteOpcode::Cmov);
        assert_eq!(classify_two_byte(0xAF), TwoByteOpcode::ImulRm);
        assert_eq!(classify_two_byte(0xB6), TwoByteOpcode::MovzxI8);
        assert_eq!(classify_two_byte(0xB9), TwoByteOpcode::UndefinedRm);
        assert_eq!(classify_two_byte(0xBE), TwoByteOpcode::MovsxI8);
        assert_eq!(classify_two_byte(0xB8), TwoByteOpcode::Popcnt);
        assert_eq!(classify_two_byte(0xAE), TwoByteOpcode::Fence);
        assert_eq!(classify_two_byte(0xC1), TwoByteOpcode::Xadd);
        assert_eq!(classify_two_byte(0xC3), TwoByteOpcode::Movnti);
        assert_eq!(classify_two_byte(0xB0), TwoByteOpcode::Unsupported);
        assert_eq!(classify_two_byte(0xB1), TwoByteOpcode::Cmpxchg);
        assert_eq!(classify_two_byte(0xC8), TwoByteOpcode::Bswap);
        assert_eq!(classify_two_byte(0xCF), TwoByteOpcode::Bswap);
        assert_eq!(classify_two_byte(0x38), TwoByteOpcode::ThreeByte0F38);
        assert_eq!(classify_two_byte(0xFF), TwoByteOpcode::UndefinedRm);
        assert_eq!(classify_two_byte(0x24), TwoByteOpcode::Unsupported);
    }
}
