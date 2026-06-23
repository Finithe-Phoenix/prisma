use crate::prefixes::PrefixSet;
use prisma_ir::*;

/// Parsed ModR/M byte: addressing mode, opcode-extension, and r/m operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModRm {
    pub mod_: u8,
    pub reg: u8,
    pub rm: u8,
}

/// Parsed SIB byte: scale, index, and base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sib {
    pub scale: u8,
    pub index: u8,
    pub base: u8,
}

/// Describes the effective address from ModR/M + SIB + displacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrMode {
    /// Direct register (mod=3)
    #[allow(dead_code)]
    Register(Gpr),
    /// [base + disp]
    BaseDisp { base: Gpr, disp: i64 },
    /// [base + index*scale + disp]
    Indexed {
        base: Option<Gpr>,
        index: Gpr,
        scale: u8,
        disp: i64,
    },
    /// [rip + disp32]
    RipRelative { disp: i64 },
    /// [disp] with no base and no index — a SIB with base field 0b101 (mod=0)
    /// and the no-index encoding. An absolute address.
    Absolute { disp: i64 },
}

/// Parse a ModR/M byte from the instruction stream.
pub fn parse_modrm(bytes: &[u8], offset: usize) -> Result<(ModRm, usize), super::DecodeError> {
    let b = *bytes.get(offset).ok_or(super::DecodeError::Truncated)?;
    Ok((
        ModRm {
            mod_: (b >> 6) & 0x3,
            reg: (b >> 3) & 0x7,
            rm: b & 0x7,
        },
        offset + 1,
    ))
}

/// Parse a SIB byte from the instruction stream.
pub fn parse_sib(bytes: &[u8], offset: usize) -> Result<(Sib, usize), super::DecodeError> {
    let b = *bytes.get(offset).ok_or(super::DecodeError::Truncated)?;
    Ok((
        Sib {
            scale: 1u8.wrapping_shl(((b >> 6) & 0x3) as u32),
            index: (b >> 3) & 0x7,
            base: b & 0x7,
        },
        offset + 1,
    ))
}

/// Decode the effective address from ModR/M + optional SIB + displacement.
///
/// Returns `None` for register-direct mode (mod=3).
pub fn decode_addr(
    modrm: &ModRm,
    sib: Option<&Sib>,
    rex: &PrefixSet,
    bytes: &[u8],
    offset: usize,
) -> Result<(AddrMode, usize), super::DecodeError> {
    if modrm.mod_ == 3 {
        return Err(super::DecodeError::InvalidModRm(offset));
    }
    let addr_size = if rex.addr_override { 32 } else { 64 };

    match (modrm.mod_, modrm.rm) {
        // [SIB]
        (0, 4) => {
            let sib = sib.ok_or(super::DecodeError::InvalidSib(offset))?;
            if sib.base == 5 {
                let disp = read_disp32(bytes, offset)?;
                Ok(decode_sib_addr(sib, rex, None, disp, offset + 4))
            } else {
                let base = Some(gpr_from_sib_base(sib.base, rex));
                Ok(decode_sib_addr(sib, rex, base, 0, offset))
            }
        }
        // [rip+disp32]
        (0, 5) => {
            let disp = read_disp32(bytes, offset)?;
            Ok((AddrMode::RipRelative { disp }, offset + 4))
        }
        // [reg]
        (0, rm) => {
            let reg = gpr_from_rm(rm, rex, addr_size);
            Ok((AddrMode::BaseDisp { base: reg, disp: 0 }, offset))
        }
        // [SIB+disp8]
        (1, 4) => {
            let disp = read_disp8(bytes, offset)? as i64;
            let sib = sib.ok_or(super::DecodeError::InvalidSib(offset))?;
            let base = Some(gpr_from_sib_base(sib.base, rex));
            Ok(decode_sib_addr(sib, rex, base, disp, offset + 1))
        }
        // [reg+disp8]
        (1, rm) => {
            let disp = read_disp8(bytes, offset)? as i64;
            let reg = gpr_from_rm(rm, rex, addr_size);
            Ok((AddrMode::BaseDisp { base: reg, disp }, offset + 1))
        }
        // [SIB+disp32]
        (2, 4) => {
            let disp = read_disp32(bytes, offset)?;
            let sib = sib.ok_or(super::DecodeError::InvalidSib(offset))?;
            let base = Some(gpr_from_sib_base(sib.base, rex));
            Ok(decode_sib_addr(sib, rex, base, disp, offset + 4))
        }
        // [reg+disp32]
        (2, rm) => {
            let disp = read_disp32(bytes, offset)?;
            let reg = gpr_from_rm(rm, rex, addr_size);
            Ok((AddrMode::BaseDisp { base: reg, disp }, offset + 4))
        }
        _ => Err(super::DecodeError::InvalidModRm(offset)),
    }
}

fn decode_sib_addr(
    sib: &Sib,
    prefixes: &PrefixSet,
    base: Option<Gpr>,
    disp: i64,
    next_offset: usize,
) -> (AddrMode, usize) {
    // x86: a SIB index field of 0b100 means *no index register* — unless REX.X
    // is set, which makes it r12. Without this, `[rsp+disp]` (and any no-index
    // SIB) wrongly decodes the base as the index too, yielding base+base+disp.
    let has_index = sib.index != 4 || prefixes.rex.x;
    let mode = if has_index {
        AddrMode::Indexed {
            base,
            index: gpr_from_sib_index(sib.index, prefixes),
            scale: sib.scale,
            disp,
        }
    } else {
        base.map_or(AddrMode::Absolute { disp }, |base| AddrMode::BaseDisp {
            base,
            disp,
        })
    };
    (mode, next_offset)
}

fn read_disp8(bytes: &[u8], offset: usize) -> Result<i8, super::DecodeError> {
    let b = *bytes.get(offset).ok_or(super::DecodeError::Truncated)?;
    Ok(b as i8)
}

fn read_disp32(bytes: &[u8], offset: usize) -> Result<i64, super::DecodeError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(super::DecodeError::Truncated)?;
    Ok(i64::from(i32::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3],
    ])))
}

/// Map a raw 3-bit register field to a GPR.
fn gpr_from_index(reg: u8) -> Gpr {
    match reg & 0xF {
        0 => Gpr::Rax,
        1 => Gpr::Rcx,
        2 => Gpr::Rdx,
        3 => Gpr::Rbx,
        4 => Gpr::Rsp,
        5 => Gpr::Rbp,
        6 => Gpr::Rsi,
        7 => Gpr::Rdi,
        8 => Gpr::R8,
        9 => Gpr::R9,
        10 => Gpr::R10,
        11 => Gpr::R11,
        12 => Gpr::R12,
        13 => Gpr::R13,
        14 => Gpr::R14,
        15 => Gpr::R15,
        _ => unreachable!(),
    }
}

fn gpr_from_raw(reg: u8) -> Gpr {
    gpr_from_index(reg & 0x7)
}

fn gpr_from_rm(rm: u8, prefixes: &PrefixSet, _addr_size: u32) -> Gpr {
    let rex_ext = if prefixes.rex.b { 8 } else { 0 };
    gpr_from_index((rm & 0x7) | rex_ext)
}

fn gpr_from_sib_base(base: u8, prefixes: &PrefixSet) -> Gpr {
    let rex_ext = if prefixes.rex.b { 8 } else { 0 };
    gpr_from_index((base & 0x7) | rex_ext)
}

fn gpr_from_sib_index(index: u8, prefixes: &PrefixSet) -> Gpr {
    let rex_ext = if prefixes.rex.x { 8 } else { 0 };
    gpr_from_index((index & 0x7) | rex_ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modrm_basic() {
        let (m, next) = parse_modrm(b"\x01\x02", 0).unwrap();
        assert_eq!(m.mod_, 0);
        assert_eq!(m.reg, 0);
        assert_eq!(m.rm, 1);
        assert_eq!(next, 1);
    }

    #[test]
    fn parse_modrm_regmem() {
        let (m, next) = parse_modrm(b"\xC3", 0).unwrap();
        assert_eq!(m.mod_, 3);
        assert_eq!(m.reg, 0);
        assert_eq!(m.rm, 3);
        assert_eq!(next, 1);
    }

    #[test]
    fn parse_sib_basic() {
        // 0x12 = 0001_0010 → scale=00(1), index=010(2), base=010(2)
        let (s, _) = parse_sib(b"\x12", 0).unwrap();
        assert_eq!(s.scale, 1);
        assert_eq!(s.index, 2);
        assert_eq!(s.base, 2);
    }

    #[test]
    fn parse_sib_scale_values() {
        // scale=01(2), index=111(7), base=000(0)
        let (s, _) = parse_sib(b"\x78", 0).unwrap();
        assert_eq!(s.scale, 2);
        assert_eq!(s.index, 7);
        assert_eq!(s.base, 0);
        // scale=10(4), index=000(0), base=001(1)
        let (s, _) = parse_sib(b"\x81", 0).unwrap();
        assert_eq!(s.scale, 4);
        assert_eq!(s.index, 0);
        assert_eq!(s.base, 1);
        // scale=11(8), index=100(4), base=011(3)
        let (s, _) = parse_sib(b"\xE3", 0).unwrap();
        assert_eq!(s.scale, 8);
        assert_eq!(s.index, 4);
        assert_eq!(s.base, 3);
    }

    #[test]
    fn decode_addr_rip_relative() {
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 5,
        };
        let rex = PrefixSet::default();
        let (addr, used) = decode_addr(&m, None, &rex, b"\x00\x10\x00\x00", 0).unwrap();
        assert_eq!(used, 4);
        assert_eq!(addr, AddrMode::RipRelative { disp: 0x1000 });
    }

    #[test]
    fn decode_addr_base_disp8() {
        let m = ModRm {
            mod_: 1,
            reg: 0,
            rm: 0,
        };
        let rex = PrefixSet::default();
        let (addr, used) = decode_addr(&m, None, &rex, b"\x7F", 0).unwrap();
        assert_eq!(used, 1);
        assert_eq!(
            addr,
            AddrMode::BaseDisp {
                base: Gpr::Rax,
                disp: 0x7F
            }
        );
    }

    #[test]
    fn decode_addr_base_disp8_sign_extends() {
        let m = ModRm {
            mod_: 1,
            reg: 0,
            rm: 0,
        };
        let prefixes = PrefixSet::default();
        let (addr, used) = decode_addr(&m, None, &prefixes, b"\x80", 0).unwrap();
        assert_eq!(used, 1);
        assert_eq!(
            addr,
            AddrMode::BaseDisp {
                base: Gpr::Rax,
                disp: -128,
            }
        );
    }

    #[test]
    fn decode_addr_rex_b_extends_base_register() {
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 0,
        };
        let prefixes = PrefixSet {
            rex: crate::prefixes::RexPrefix {
                b: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let (addr, used) = decode_addr(&m, None, &prefixes, b"", 0).unwrap();
        assert_eq!(used, 0);
        assert_eq!(
            addr,
            AddrMode::BaseDisp {
                base: Gpr::R8,
                disp: 0,
            }
        );
    }

    #[test]
    fn decode_addr_sib_disp8_tracks_offset_and_components() {
        let m = ModRm {
            mod_: 1,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 4,
            index: 1,
            base: 0,
        };
        let prefixes = PrefixSet::default();
        let (addr, used) = decode_addr(&m, Some(&sib), &prefixes, b"\xFC", 0).unwrap();
        assert_eq!(used, 1);
        assert_eq!(
            addr,
            AddrMode::Indexed {
                base: Some(Gpr::Rax),
                index: Gpr::Rcx,
                scale: 4,
                disp: -4,
            }
        );
    }

    #[test]
    fn decode_addr_sib_base5_disp32_is_indexed_without_base() {
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 2,
            index: 1,
            base: 5,
        };
        let prefixes = PrefixSet::default();
        let (addr, used) = decode_addr(&m, Some(&sib), &prefixes, b"\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(used, 4);
        assert_eq!(
            addr,
            AddrMode::Indexed {
                base: None,
                index: Gpr::Rcx,
                scale: 2,
                disp: 0x1234_5678,
            }
        );
    }

    #[test]
    fn decode_addr_sib_rex_x_extends_index_register() {
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 2,
            index: 1,
            base: 0,
        };
        let prefixes = PrefixSet {
            rex: crate::prefixes::RexPrefix {
                x: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let (addr, used) = decode_addr(&m, Some(&sib), &prefixes, b"", 0).unwrap();
        assert_eq!(used, 0);
        assert_eq!(
            addr,
            AddrMode::Indexed {
                base: Some(Gpr::Rax),
                index: Gpr::R9,
                scale: 2,
                disp: 0,
            }
        );
    }

    #[test]
    fn decode_addr_sib_no_index_is_base_disp_not_doubled() {
        // [rsp - 8]: SIB index field 0b100 = no index. Must be BaseDisp{Rsp,-8},
        // NOT Indexed (which doubled the base into rsp+rsp-8).
        let m = ModRm {
            mod_: 1,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 1,
            index: 4, // no index
            base: 4,  // rsp
        };
        let prefixes = PrefixSet::default();
        let (addr, used) = decode_addr(&m, Some(&sib), &prefixes, b"\xF8", 0).unwrap();
        assert_eq!(used, 1);
        assert_eq!(
            addr,
            AddrMode::BaseDisp {
                base: Gpr::Rsp,
                disp: -8,
            }
        );
    }

    #[test]
    fn decode_addr_sib_no_index_with_rex_x_is_r12_index() {
        // index 0b100 + REX.X = r12 is a real index register (not "no index").
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 2,
            index: 4,
            base: 0,
        };
        let prefixes = PrefixSet {
            rex: crate::prefixes::RexPrefix {
                x: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let (addr, _used) = decode_addr(&m, Some(&sib), &prefixes, b"", 0).unwrap();
        assert_eq!(
            addr,
            AddrMode::Indexed {
                base: Some(Gpr::Rax),
                index: Gpr::R12,
                scale: 2,
                disp: 0,
            }
        );
    }

    #[test]
    fn decode_addr_sib_no_base_no_index_is_absolute() {
        // mod=0, base field 0b101 (no base) + no index → [disp32] absolute.
        let m = ModRm {
            mod_: 0,
            reg: 0,
            rm: 4,
        };
        let sib = Sib {
            scale: 1,
            index: 4, // no index
            base: 5,  // no base (mod=0)
        };
        let prefixes = PrefixSet::default();
        let (addr, used) = decode_addr(&m, Some(&sib), &prefixes, b"\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(used, 4);
        assert_eq!(addr, AddrMode::Absolute { disp: 0x1234_5678 });
    }
}
