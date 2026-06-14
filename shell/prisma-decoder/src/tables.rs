/// OpCode category for one-byte dispatch (0x00..0xFF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneByteOpcode {
    /// MOV r/m64, r64 (REX.W 89 /r)
    MovRmR,
    /// MOV r64, r/m64 (REX.W 8B /r)
    MovRRm,
    /// MOV r64, imm64 (REX.W B8+rd)
    MovRI,
    /// ADD r/m64, r64 (REX.W 01 /r)
    AddRmR,
    /// ADD r64, r/m64 (REX.W 03 /r)
    AddRRm,
    /// ADD/SUB/... r/m, imm8 sign-extended (83 /digit ib)
    AluRmImm8,
    /// SUB r/m64, r64 (REX.W 29 /r)
    SubRmR,
    /// SUB r64, r/m64 (REX.W 2B /r)
    SubRRm,
    /// AND r/m64, r64 (REX.W 21 /r)
    AndRmR,
    /// OR r/m64, r64 (REX.W 09 /r)
    OrRmR,
    /// XOR r/m64, r64 (REX.W 31 /r)
    XorRmR,
    /// CMP r/m64, r64 (REX.W 39 /r)
    CmpRmR,
    /// TEST r/m64, r/m64 (REX.W 85 /r)
    TestRmR,
    /// NOP if no REX.W prefix.
    Nop,
    /// JCC rel8 (0x70..0x7F).
    CondJumpRel8,
    /// XCHG r64, r/m64 (REX.W 87 /r)
    Xchg,
    /// PUSH r64 (50+rd)
    PushReg,
    /// POP r64 (58+rd)
    PopReg,
    /// 0x0F escape.
    TwoBytePrefix,
    Unsupported,
}

/// OpCode category for two-byte dispatch (0x0F 00..0xFF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoByteOpcode {
    /// JCC rel32 (0F 80..0F 8F).
    CondJumpRel32,
    /// MOVZX r64, r/m8 (0F B6)
    MovzxI8,
    /// MOVZX r64, r/m16 (0F B7)
    MovzxI16,
    /// MOVSX r64, r/m8 (0F BE)
    MovsxI8,
    /// MOVSX r64, r/m16 (0F BF)
    MovsxI16,
    /// POPCNT (F3 0F B8 /r)
    Popcnt,
    /// LZCNT (F3 0F BD /r)
    Lzcnt,
    /// TZCNT (F3 0F BC /r)
    Tzcnt,
    Unsupported,
}

/// Classify a one-byte opcode using explicit tables by value.
pub const fn classify_one_byte(opcode: u8) -> OneByteOpcode {
    match opcode {
        0x89u8 => OneByteOpcode::MovRmR,
        0x8Bu8 => OneByteOpcode::MovRRm,
        0xB8u8..=0xBFu8 => OneByteOpcode::MovRI,
        0x01u8 => OneByteOpcode::AddRmR,
        0x03u8 => OneByteOpcode::AddRRm,
        0x83u8 => OneByteOpcode::AluRmImm8,
        0x29u8 => OneByteOpcode::SubRmR,
        0x2Bu8 => OneByteOpcode::SubRRm,
        0x21u8 => OneByteOpcode::AndRmR,
        0x09u8 => OneByteOpcode::OrRmR,
        0x31u8 => OneByteOpcode::XorRmR,
        0x39u8 => OneByteOpcode::CmpRmR,
        0x85u8 => OneByteOpcode::TestRmR,
        0x90u8 => OneByteOpcode::Nop,
        0x70u8..=0x7Fu8 => OneByteOpcode::CondJumpRel8,
        0x87u8 => OneByteOpcode::Xchg,
        0x50u8..=0x57u8 => OneByteOpcode::PushReg,
        0x58u8..=0x5Fu8 => OneByteOpcode::PopReg,
        0x0Fu8 => OneByteOpcode::TwoBytePrefix,
        _ => OneByteOpcode::Unsupported,
    }
}

/// Classify a two-byte opcode using explicit tables by value.
pub const fn classify_two_byte(opcode: u8) -> TwoByteOpcode {
    match opcode {
        0x80u8..=0x8Fu8 => TwoByteOpcode::CondJumpRel32,
        0xB6u8 => TwoByteOpcode::MovzxI8,
        0xB7u8 => TwoByteOpcode::MovzxI16,
        0xBEu8 => TwoByteOpcode::MovsxI8,
        0xBFu8 => TwoByteOpcode::MovsxI16,
        0xB8u8 => TwoByteOpcode::Popcnt,
        0xBDu8 => TwoByteOpcode::Lzcnt,
        0xBCu8 => TwoByteOpcode::Tzcnt,
        _ => TwoByteOpcode::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_byte_dispatch_table_classifies_common_cases() {
        assert_eq!(classify_one_byte(0x89), OneByteOpcode::MovRmR);
        assert_eq!(classify_one_byte(0x8B), OneByteOpcode::MovRRm);
        assert_eq!(classify_one_byte(0x83), OneByteOpcode::AluRmImm8);
        assert_eq!(classify_one_byte(0x0F), OneByteOpcode::TwoBytePrefix);
        assert_eq!(classify_one_byte(0x70), OneByteOpcode::CondJumpRel8);
        assert_eq!(classify_one_byte(0x90), OneByteOpcode::Nop);
        assert_eq!(classify_one_byte(0x20), OneByteOpcode::Unsupported);
    }

    #[test]
    fn two_byte_dispatch_table_classifies_common_cases() {
        assert_eq!(classify_two_byte(0x80), TwoByteOpcode::CondJumpRel32);
        assert_eq!(classify_two_byte(0xB6), TwoByteOpcode::MovzxI8);
        assert_eq!(classify_two_byte(0xBE), TwoByteOpcode::MovsxI8);
        assert_eq!(classify_two_byte(0xB8), TwoByteOpcode::Popcnt);
        assert_eq!(classify_two_byte(0x20), TwoByteOpcode::Unsupported);
    }
}
