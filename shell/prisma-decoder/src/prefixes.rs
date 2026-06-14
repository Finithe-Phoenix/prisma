use prisma_ir::*;

/// Parsed x86 prefix bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefixSet {
    /// REP / REPE / REPNE (F3 / F2)
    pub rep: Option<u8>,
    /// Operand-size override (0x66)
    pub operand_override: bool,
    /// Address-size override (0x67)
    pub addr_override: bool,
    /// Lock prefix (0xF0)
    pub lock: bool,
    /// Segment override (0x26, 0x2E, 0x36, 0x3E, 0x64, 0x65)
    pub segment: Option<SegmentReg>,
    /// REX prefix (0x40-0x4F)
    pub rex: RexPrefix,
    /// VEX prefix (C4 or C5)
    pub vex: Option<VexPrefix>,
}

/// Parsed REX prefix byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RexPrefix {
    /// True when any REX prefix byte is present.
    pub present: bool,
    pub w: bool, // 64-bit operand size
    pub r: bool, // extends reg field
    pub x: bool, // extends SIB index field
    pub b: bool, // extends rm/base field
}

/// Parsed VEX prefix (2-byte C5 or 3-byte C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VexPrefix {
    pub p: u8,    // implied 0x66/F3/F2 prefix
    pub l: bool,  // 256-bit (L=1)
    pub vvvv: u8, // dest/src register (inverted)
    pub pp: u8,   // 2-bit opcode extension
    pub mmmm: u8, // 5-bit map select
}

/// Parse prefix bytes before the opcode.
///
/// Returns the prefix set and the offset of the first non-prefix byte.
pub fn parse_prefixes(bytes: &[u8], start: usize) -> (PrefixSet, usize) {
    let mut p = PrefixSet::default();
    let mut cursor = start;

    loop {
        let b = match bytes.get(cursor) {
            Some(&b) => b,
            None => return (p, cursor),
        };

        match b {
            // REX prefix (40-4F)
            0x40..=0x4F => {
                p.rex = RexPrefix {
                    present: true,
                    w: b & 0x08 != 0,
                    r: b & 0x04 != 0,
                    x: b & 0x02 != 0,
                    b: b & 0x01 != 0,
                };
                cursor += 1;
            }
            // VEX 2-byte (C5)
            0xC5 => {
                let b2 = match bytes.get(cursor + 1) {
                    Some(&b) => b,
                    None => return (p, cursor + 1),
                };
                p.vex = Some(VexPrefix {
                    p: 0,
                    l: b2 & 0x04 != 0,
                    vvvv: (!b2 >> 3) & 0x0F,
                    pp: b2 & 0x03,
                    mmmm: 1,
                });
                cursor += 2;
            }
            // VEX 3-byte (C4)
            0xC4 => {
                let b2 = match bytes.get(cursor + 1) {
                    Some(&b) => b,
                    None => return (p, cursor + 1),
                };
                let b3 = match bytes.get(cursor + 2) {
                    Some(&b) => b,
                    None => return (p, cursor + 2),
                };
                p.vex = Some(VexPrefix {
                    p: b3 & 0x03,
                    l: b3 & 0x04 != 0,
                    vvvv: (!b2 >> 3) & 0x0F,
                    pp: b3 & 0x03,
                    mmmm: b2 & 0x1F,
                });
                cursor += 3;
            }
            // Lock prefix
            0xF0 => {
                p.lock = true;
                cursor += 1;
            }
            // REPNE / REP
            0xF2 => {
                p.rep = Some(0xF2);
                cursor += 1;
            }
            // REP / REPE
            0xF3 => {
                p.rep = Some(0xF3);
                cursor += 1;
            }
            // Operand-size override
            0x66 => {
                p.operand_override = true;
                cursor += 1;
            }
            // Address-size override
            0x67 => {
                p.addr_override = true;
                cursor += 1;
            }
            // Segment overrides
            0x26 => {
                p.segment = Some(SegmentReg::Es);
                cursor += 1;
            }
            0x2E => {
                p.segment = Some(SegmentReg::Cs);
                cursor += 1;
            }
            0x36 => {
                p.segment = Some(SegmentReg::Ss);
                cursor += 1;
            }
            0x3E => {
                p.segment = Some(SegmentReg::Ds);
                cursor += 1;
            }
            0x64 => {
                p.segment = Some(SegmentReg::Fs);
                cursor += 1;
            }
            0x65 => {
                p.segment = Some(SegmentReg::Gs);
                cursor += 1;
            }
            // Not a prefix — stop
            _ => return (p, cursor),
        }
    }
}

/// Determine the operand size from prefixes and default size.
pub fn operand_size(prefixes: &PrefixSet, _default_64: bool) -> OpSize {
    if prefixes.rex.w {
        return OpSize::I64;
    }
    if prefixes.operand_override {
        return OpSize::I16;
    }
    OpSize::I32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_prefixes() {
        let (p, cursor) = parse_prefixes(b"\x90", 0);
        assert_eq!(cursor, 0);
        assert!(!p.operand_override);
        assert!(!p.rex.w);
    }

    #[test]
    fn rex_w_prefix() {
        let (p, cursor) = parse_prefixes(b"\x48\x90", 0);
        assert_eq!(cursor, 1);
        assert!(p.rex.w);
        assert!(!p.rex.r);
        assert!(!p.rex.x);
        assert!(!p.rex.b);
    }

    #[test]
    fn operand_size_override() {
        let (p, cursor) = parse_prefixes(b"\x66\x90", 0);
        assert_eq!(cursor, 1);
        assert!(p.operand_override);
    }

    #[test]
    fn multiple_prefixes() {
        let (p, cursor) = parse_prefixes(b"\x66\x48\x90", 0);
        assert_eq!(cursor, 2);
        assert!(p.operand_override);
        assert!(p.rex.w);
    }

    #[test]
    fn complex_prefix_set_tracks_last_rep_segment_overrides_and_rex() {
        let (p, cursor) = parse_prefixes(b"\xF2\xF3\x64\x66\x67\x4D\x90", 0);
        assert_eq!(cursor, 6);
        assert_eq!(p.rep, Some(0xF3));
        assert_eq!(p.segment, Some(SegmentReg::Fs));
        assert!(p.operand_override);
        assert!(p.addr_override);
        assert!(p.rex.w);
        assert!(p.rex.r);
        assert!(!p.rex.x);
        assert!(p.rex.b);
    }

    #[test]
    fn lock_prefix() {
        let (p, cursor) = parse_prefixes(b"\xF0\x90", 0);
        assert_eq!(cursor, 1);
        assert!(p.lock);
    }

    #[test]
    fn operand_size_64bit() {
        let p = PrefixSet {
            rex: RexPrefix {
                w: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(operand_size(&p, true), OpSize::I64);
    }

    #[test]
    fn operand_size_16bit() {
        let p = PrefixSet {
            operand_override: true,
            ..Default::default()
        };
        assert_eq!(operand_size(&p, true), OpSize::I16);
    }

    #[test]
    fn operand_size_default_32() {
        let p = PrefixSet::default();
        assert_eq!(operand_size(&p, true), OpSize::I32);
    }
}
