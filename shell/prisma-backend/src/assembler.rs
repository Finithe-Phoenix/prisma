//! ARM64 assembler abstractions.
//!
//! This module is intentionally minimal during this migration stage and
//! exposes the public surface required by the backend crate API.

use prisma_ir::{CondCode, FenceKind};

/// Backend-side assembler handle for ARM64 emission.
#[derive(Debug, Default, Clone)]
pub struct Arm64Assembler {
    instructions: Vec<u32>,
    labels: Vec<Option<usize>>,
    branch_fixups: Vec<BranchFixup>,
}

/// Opaque label handle used for PC-relative branch fixups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BranchFixup {
    label: Label,
    instruction_index: usize,
    kind: BranchFixupKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchFixupKind {
    B,
    BCond { cond: CondCode },
    CbzX { rt: u8 },
    CbnzX { rt: u8 },
}

impl Arm64Assembler {
    /// Creates a new, empty assembler.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            instructions: Vec::new(),
            labels: Vec::new(),
            branch_fixups: Vec::new(),
        }
    }

    /// Appends a placeholder ARM64 word.
    pub fn emit_word(&mut self, word: u32) {
        self.instructions.push(word);
    }

    /// Current byte offset from the beginning of the instruction stream.
    #[must_use]
    pub fn cursor_offset(&self) -> usize {
        self.instructions.len() * 4
    }

    /// Number of emitted instructions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    /// Whether the assembler has emitted no instructions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Emits `RET`.
    pub fn ret(&mut self) {
        self.emit_word(ret());
    }

    /// Emits `BR Xn`.
    pub fn br_x(&mut self, target: u8) {
        self.emit_word(br_x(target));
    }

    /// Emits `BLR Xn`.
    pub fn blr_x(&mut self, target: u8) {
        self.emit_word(blr_x(target));
    }

    /// Emits `NOP`.
    pub fn nop(&mut self) {
        self.emit_word(nop());
    }

    /// Emits `MRS Xd, CNTVCT_EL0`.
    pub fn mrs_cntvct(&mut self, dst: u8) {
        self.emit_word(mrs_cntvct(dst));
    }

    /// Emits `MSR NZCV, Xt`.
    pub fn msr_nzcv(&mut self, src: u8) {
        self.emit_word(msr_nzcv(src));
    }

    /// Emits the ARM64 barrier corresponding to an x86 fence.
    pub fn fence(&mut self, kind: FenceKind) {
        self.emit_word(fence(kind));
    }

    /// Emits `B imm26`.
    ///
    /// `offset_bytes` is PC-relative and must be 4-byte aligned.
    pub fn b(&mut self, offset_bytes: i32) {
        self.emit_word(b(offset_bytes));
    }

    /// Emits `B.cond imm19`.
    ///
    /// `offset_bytes` is PC-relative and must be 4-byte aligned.
    pub fn b_cond(&mut self, cond: CondCode, offset_bytes: i32) {
        self.emit_word(b_cond(cond, offset_bytes));
    }

    /// Creates a new unbound label.
    pub fn create_label(&mut self) -> Label {
        let label = Label(self.labels.len());
        self.labels.push(None);
        label
    }

    /// Binds a label at the current cursor position.
    ///
    /// # Panics
    ///
    /// Panics if the label does not belong to this assembler or was already
    /// bound.
    pub fn bind_label(&mut self, label: Label) {
        let cursor_offset = self.cursor_offset();
        let slot = self
            .labels
            .get_mut(label.0)
            .expect("label does not belong to this assembler");
        assert!(slot.is_none(), "label was already bound");
        *slot = Some(cursor_offset);
    }

    /// Emits `B label`, patched when the assembler is finished.
    pub fn b_label(&mut self, label: Label) {
        let instruction_index = self.instructions.len();
        self.emit_word(b(0));
        self.branch_fixups.push(BranchFixup {
            label,
            instruction_index,
            kind: BranchFixupKind::B,
        });
    }

    /// Emits `B.cond label`, patched when the assembler is finished.
    pub fn b_cond_label(&mut self, cond: CondCode, label: Label) {
        let instruction_index = self.instructions.len();
        self.emit_word(b_cond(cond, 0));
        self.branch_fixups.push(BranchFixup {
            label,
            instruction_index,
            kind: BranchFixupKind::BCond { cond },
        });
    }

    /// Emits `CBZ Xt, label`, patched when the assembler is finished.
    pub fn cbz_x_label(&mut self, rt: u8, label: Label) {
        let instruction_index = self.instructions.len();
        self.emit_word(cbz_x(rt, 0));
        self.branch_fixups.push(BranchFixup {
            label,
            instruction_index,
            kind: BranchFixupKind::CbzX { rt },
        });
    }

    /// Emits `CBNZ Xt, label`, patched when the assembler is finished.
    pub fn cbnz_x_label(&mut self, rt: u8, label: Label) {
        let instruction_index = self.instructions.len();
        self.emit_word(cbnz_x(rt, 0));
        self.branch_fixups.push(BranchFixup {
            label,
            instruction_index,
            kind: BranchFixupKind::CbnzX { rt },
        });
    }

    /// Emits `MOV Xd, Xn` as `ORR Xd, XZR, Xn`.
    pub fn mov_x(&mut self, dst: u8, src: u8) {
        self.emit_word(mov_x(dst, src));
    }

    /// Emits `MOVZ Xd, #imm16, LSL #shift`.
    pub fn movz_x(&mut self, dst: u8, imm16: u16, shift: u8) {
        self.emit_word(movz_x(dst, imm16, shift));
    }

    /// Emits `MOVK Xd, #imm16, LSL #shift`.
    pub fn movk_x(&mut self, dst: u8, imm16: u16, shift: u8) {
        self.emit_word(movk_x(dst, imm16, shift));
    }

    /// Emits `ADD Xd, Xn, #imm12`.
    pub fn add_x_imm(&mut self, dst: u8, src: u8, imm12: u16) {
        self.emit_word(add_x_imm(dst, src, imm12));
    }

    /// Emits `ADD Xd, Xn, Xm`.
    pub fn add_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(add_x(dst, lhs, rhs));
    }

    /// Emits `ADDS Xd, Xn, Xm`.
    pub fn adds_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(adds_x(dst, lhs, rhs));
    }

    /// Emits `SUB Xd, Xn, #imm12`.
    pub fn sub_x_imm(&mut self, dst: u8, src: u8, imm12: u16) {
        self.emit_word(sub_x_imm(dst, src, imm12));
    }

    /// Emits `SUB Xd, Xn, Xm`.
    pub fn sub_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(sub_x(dst, lhs, rhs));
    }

    /// Emits `AND Xd, Xn, Xm`.
    pub fn and_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(and_x(dst, lhs, rhs));
    }

    /// Emits `ANDS Xd, Xn, Xm`.
    pub fn ands_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(ands_x(dst, lhs, rhs));
    }

    /// Emits `ORR Xd, Xn, Xm`.
    pub fn orr_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(orr_x(dst, lhs, rhs));
    }

    /// Emits `EOR Xd, Xn, Xm`.
    pub fn eor_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(eor_x(dst, lhs, rhs));
    }

    /// Emits `LSLV Xd, Xn, Xm`.
    pub fn lsl_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(lsl_x(dst, lhs, rhs));
    }

    /// Emits `LSRV Xd, Xn, Xm`.
    pub fn lsr_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(lsr_x(dst, lhs, rhs));
    }

    /// Emits `ASRV Xd, Xn, Xm`.
    pub fn asr_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(asr_x(dst, lhs, rhs));
    }

    /// Emits `RORV Xd, Xn, Xm`.
    pub fn ror_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(ror_x(dst, lhs, rhs));
    }

    /// Emits `CRC32CB Wd, Wn, Wm`.
    pub fn crc32cb(&mut self, dst: u8, crc: u8, data: u8) {
        self.emit_word(crc32cb(dst, crc, data));
    }

    /// Emits `CRC32CH Wd, Wn, Wm`.
    pub fn crc32ch(&mut self, dst: u8, crc: u8, data: u8) {
        self.emit_word(crc32ch(dst, crc, data));
    }

    /// Emits `CRC32CW Wd, Wn, Wm`.
    pub fn crc32cw(&mut self, dst: u8, crc: u8, data: u8) {
        self.emit_word(crc32cw(dst, crc, data));
    }

    /// Emits `CRC32CX Wd, Wn, Xm`.
    pub fn crc32cx(&mut self, dst: u8, crc: u8, data: u8) {
        self.emit_word(crc32cx(dst, crc, data));
    }

    /// Emits `CLZ Xd, Xn`.
    pub fn clz_x(&mut self, dst: u8, src: u8) {
        self.emit_word(clz_x(dst, src));
    }

    /// Emits `CLZ Wd, Wn`.
    pub fn clz_w(&mut self, dst: u8, src: u8) {
        self.emit_word(clz_w(dst, src));
    }

    /// Emits `RBIT Xd, Xn`.
    pub fn rbit_x(&mut self, dst: u8, src: u8) {
        self.emit_word(rbit_x(dst, src));
    }

    /// Emits `RBIT Wd, Wn`.
    pub fn rbit_w(&mut self, dst: u8, src: u8) {
        self.emit_word(rbit_w(dst, src));
    }

    /// Emits `REV Xd, Xn`.
    pub fn rev_x(&mut self, dst: u8, src: u8) {
        self.emit_word(rev_x(dst, src));
    }

    /// Emits `REV Wd, Wn`.
    pub fn rev_w(&mut self, dst: u8, src: u8) {
        self.emit_word(rev_w(dst, src));
    }

    /// Emits `SXTB Xd, Wn`.
    pub fn sxtb_x(&mut self, dst: u8, src: u8) {
        self.emit_word(sxtb_x(dst, src));
    }

    /// Emits `SXTH Xd, Wn`.
    pub fn sxth_x(&mut self, dst: u8, src: u8) {
        self.emit_word(sxth_x(dst, src));
    }

    /// Emits `SXTW Xd, Wn`.
    pub fn sxtw_x(&mut self, dst: u8, src: u8) {
        self.emit_word(sxtw_x(dst, src));
    }

    /// Emits a zero-extension of the low byte into `Xd`.
    pub fn uxtb_x(&mut self, dst: u8, src: u8) {
        self.emit_word(uxtb_x(dst, src));
    }

    /// Emits a zero-extension of the low halfword into `Xd`.
    pub fn uxth_x(&mut self, dst: u8, src: u8) {
        self.emit_word(uxth_x(dst, src));
    }

    /// Emits a zero-extension of the low word into `Xd`.
    pub fn uxtw_x(&mut self, dst: u8, src: u8) {
        self.emit_word(uxtw_x(dst, src));
    }

    /// Emits `MUL Xd, Xn, Xm`.
    pub fn mul_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(mul_x(dst, lhs, rhs));
    }

    /// Emits `UMULH Xd, Xn, Xm`.
    pub fn umulh_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(umulh_x(dst, lhs, rhs));
    }

    /// Emits `SMULH Xd, Xn, Xm`.
    pub fn smulh_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(smulh_x(dst, lhs, rhs));
    }

    /// Emits `UDIV Xd, Xn, Xm`.
    pub fn udiv_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(udiv_x(dst, lhs, rhs));
    }

    /// Emits `SDIV Xd, Xn, Xm`.
    pub fn sdiv_x(&mut self, dst: u8, lhs: u8, rhs: u8) {
        self.emit_word(sdiv_x(dst, lhs, rhs));
    }

    /// Emits `MSUB Xd, Xn, Xm, Xa`.
    pub fn msub_x(&mut self, dst: u8, lhs: u8, rhs: u8, minuend: u8) {
        self.emit_word(msub_x(dst, lhs, rhs, minuend));
    }

    /// Emits `CMP Xn, Xm`.
    pub fn cmp_x(&mut self, lhs: u8, rhs: u8) {
        self.emit_word(cmp_x(lhs, rhs));
    }

    /// Emits `CSET Xd, cond`.
    pub fn cset_x(&mut self, dst: u8, cond: CondCode) {
        self.emit_word(cset_x(dst, cond));
    }

    /// Emits `CSEL Xd, Xn, Xm, cond` (Xd = cond ? Xn : Xm).
    pub fn csel_x(&mut self, dst: u8, if_true: u8, if_false: u8, cond: CondCode) {
        self.emit_word(csel_x(dst, if_true, if_false, cond));
    }

    /// Emits `LDR Xt, [Xn, #imm]`.
    pub fn ldr_x_unsigned(&mut self, dst: u8, base: u8, imm_bytes: u16) {
        self.emit_word(ldr_x_unsigned(dst, base, imm_bytes));
    }

    /// Emits `LDR Wt, [Xn, #imm]`.
    pub fn ldr_w_unsigned(&mut self, dst: u8, base: u8, imm_bytes: u16) {
        self.emit_word(ldr_w_unsigned(dst, base, imm_bytes));
    }

    /// Emits `LDRH Wt, [Xn, #imm]`.
    pub fn ldrh_unsigned(&mut self, dst: u8, base: u8, imm_bytes: u16) {
        self.emit_word(ldrh_unsigned(dst, base, imm_bytes));
    }

    /// Emits `LDRB Wt, [Xn, #imm]`.
    pub fn ldrb_unsigned(&mut self, dst: u8, base: u8, imm_bytes: u16) {
        self.emit_word(ldrb_unsigned(dst, base, imm_bytes));
    }

    /// Emits `STR Xt, [Xn, #imm]`.
    pub fn str_x_unsigned(&mut self, src: u8, base: u8, imm_bytes: u16) {
        self.emit_word(str_x_unsigned(src, base, imm_bytes));
    }

    /// Emits `STR Wt, [Xn, #imm]`.
    pub fn str_w_unsigned(&mut self, src: u8, base: u8, imm_bytes: u16) {
        self.emit_word(str_w_unsigned(src, base, imm_bytes));
    }

    /// Emits `STRH Wt, [Xn, #imm]`.
    pub fn strh_unsigned(&mut self, src: u8, base: u8, imm_bytes: u16) {
        self.emit_word(strh_unsigned(src, base, imm_bytes));
    }

    /// Emits `STRB Wt, [Xn, #imm]`.
    pub fn strb_unsigned(&mut self, src: u8, base: u8, imm_bytes: u16) {
        self.emit_word(strb_unsigned(src, base, imm_bytes));
    }

    /// Emits `STP Xt1, Xt2, [SP, #imm]!`.
    pub fn stp_x_pre_sp(&mut self, rt: u8, rt2: u8, imm_bytes: i16) {
        self.emit_word(stp_x_pre_sp(rt, rt2, imm_bytes));
    }

    /// Emits `LDP Xt1, Xt2, [SP], #imm`.
    pub fn ldp_x_post_sp(&mut self, rt: u8, rt2: u8, imm_bytes: i16) {
        self.emit_word(ldp_x_post_sp(rt, rt2, imm_bytes));
    }

    /// Returns the current instruction stream and resets the assembler.
    #[must_use]
    pub fn finish(mut self) -> Vec<u32> {
        self.resolve_branch_fixups();
        self.instructions
    }

    /// Returns the instruction stream as little-endian bytes.
    #[must_use]
    pub fn finish_bytes(mut self) -> Vec<u8> {
        self.resolve_branch_fixups();
        let mut out = Vec::with_capacity(self.instructions.len() * 4);
        for word in self.instructions {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    fn resolve_branch_fixups(&mut self) {
        for fixup in &self.branch_fixups {
            let target_offset = self
                .labels
                .get(fixup.label.0)
                .and_then(|label| *label)
                .expect("branch label was not bound");
            let branch_offset = fixup.instruction_index * 4;
            let relative = i32::try_from(target_offset)
                .expect("target offset fits i32")
                .checked_sub(i32::try_from(branch_offset).expect("branch offset fits i32"))
                .expect("branch relative offset fits i32");
            self.instructions[fixup.instruction_index] = match fixup.kind {
                BranchFixupKind::B => b(relative),
                BranchFixupKind::BCond { cond } => b_cond(cond, relative),
                BranchFixupKind::CbzX { rt } => cbz_x(rt, relative),
                BranchFixupKind::CbnzX { rt } => cbnz_x(rt, relative),
            };
        }
        self.branch_fixups.clear();
    }
}

/// Encodes `NOP`.
#[must_use]
pub const fn nop() -> u32 {
    0xD503_201F
}

/// Encodes `MRS Xd, CNTVCT_EL0`.
///
/// # Panics
///
/// Panics if `dst` is outside the `AArch64` register range `0..32`.
#[must_use]
pub fn mrs_cntvct(dst: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    0xD53B_E040 | u32::from(dst)
}

/// Encodes `MSR NZCV, Xt`.
///
/// # Panics
///
/// Panics if `src` is outside the `AArch64` register range `0..32`.
#[must_use]
pub fn msr_nzcv(src: u8) -> u32 {
    assert!(src < 32, "source register out of range");
    0xD51B_4200 | u32::from(src)
}

/// Encodes an ARM64 `DMB` for the supplied x86 fence kind.
#[must_use]
pub const fn fence(kind: FenceKind) -> u32 {
    match kind {
        FenceKind::Mfence => 0xD503_3BBF,
        FenceKind::Lfence => 0xD503_39BF,
        FenceKind::Sfence => 0xD503_3ABF,
    }
}

/// Encodes `RET`.
#[must_use]
pub const fn ret() -> u32 {
    0xD65F_03C0
}

/// Encodes `BR Xn`.
///
/// # Panics
///
/// Panics if `target` is outside the `AArch64` register range `0..32`.
#[must_use]
pub fn br_x(target: u8) -> u32 {
    assert!(target < 32, "target register out of range");
    0xD61F_0000 | (u32::from(target) << 5)
}

/// Encodes `BLR Xn`.
///
/// # Panics
///
/// Panics if `target` is outside the `AArch64` register range `0..32`.
#[must_use]
pub fn blr_x(target: u8) -> u32 {
    assert!(target < 32, "target register out of range");
    0xD63F_0000 | (u32::from(target) << 5)
}

/// Encodes an unconditional branch.
///
/// # Panics
///
/// Panics if `offset_bytes` is not 4-byte aligned.
#[must_use]
pub fn b(offset_bytes: i32) -> u32 {
    assert!(
        offset_bytes % 4 == 0,
        "branch offset must be 4-byte aligned"
    );
    let imm26 = (offset_bytes >> 2).cast_unsigned() & 0x03FF_FFFF;
    0x1400_0000 | imm26
}

/// Encodes a conditional branch.
///
/// # Panics
///
/// Panics if `offset_bytes` is not 4-byte aligned or if the offset cannot fit
/// the signed 19-bit branch immediate.
#[must_use]
pub fn b_cond(cond: CondCode, offset_bytes: i32) -> u32 {
    assert!(
        offset_bytes % 4 == 0,
        "branch offset must be 4-byte aligned"
    );
    let imm19 = offset_bytes >> 2;
    assert!(
        (-(1 << 18)..(1 << 18)).contains(&imm19),
        "conditional branch offset out of range"
    );
    0x5400_0000 | ((imm19.cast_unsigned() & 0x7_FFFF) << 5) | u32::from(arm_condition(cond))
}

/// Encodes `CBZ Xt, label`.
///
/// # Panics
///
/// Panics if `rt` is outside the `AArch64` register range `0..32`, if
/// `offset_bytes` is not 4-byte aligned, or if the offset cannot fit the
/// signed 19-bit branch immediate.
#[must_use]
pub fn cbz_x(rt: u8, offset_bytes: i32) -> u32 {
    encode_compare_branch_x(0xB400_0000, rt, offset_bytes)
}

/// Encodes `CBNZ Xt, label`.
///
/// # Panics
///
/// Panics if `rt` is outside the `AArch64` register range `0..32`, if
/// `offset_bytes` is not 4-byte aligned, or if the offset cannot fit the
/// signed 19-bit branch immediate.
#[must_use]
pub fn cbnz_x(rt: u8, offset_bytes: i32) -> u32 {
    encode_compare_branch_x(0xB500_0000, rt, offset_bytes)
}

/// Encodes `MOV Xd, Xn` as `ORR Xd, XZR, Xn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn mov_x(dst: u8, src: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(src < 32, "source register out of range");
    0xAA00_03E0 | (u32::from(src) << 16) | u32::from(dst)
}

/// Encodes `MOVZ Xd, #imm16, LSL #shift`.
///
/// # Panics
///
/// Panics if `dst` is outside the `AArch64` register range `0..32`, or if
/// `shift` is not one of `0`, `16`, `32`, or `48`.
#[must_use]
pub fn movz_x(dst: u8, imm16: u16, shift: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(
        shift <= 48 && shift.is_multiple_of(16),
        "invalid MOVZ shift"
    );
    let hw = u32::from(shift / 16);
    0xD280_0000 | (hw << 21) | (u32::from(imm16) << 5) | u32::from(dst)
}

/// Encodes `MOVK Xd, #imm16, LSL #shift`.
///
/// # Panics
///
/// Panics if `dst` is outside the `AArch64` register range `0..32`, or if
/// `shift` is not one of `0`, `16`, `32`, or `48`.
#[must_use]
pub fn movk_x(dst: u8, imm16: u16, shift: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(
        shift <= 48 && shift.is_multiple_of(16),
        "invalid MOVK shift"
    );
    let hw = u32::from(shift / 16);
    0xF280_0000 | (hw << 21) | (u32::from(imm16) << 5) | u32::from(dst)
}

/// Encodes `ADD Xd, Xn, #imm12`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`, or if `imm12` cannot fit the unshifted 12-bit immediate field.
#[must_use]
pub fn add_x_imm(dst: u8, src: u8, imm12: u16) -> u32 {
    encode_add_sub_x_imm(0x9100_0000, dst, src, imm12)
}

/// Encodes `SUB Xd, Xn, #imm12`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`, or if `imm12` cannot fit the unshifted 12-bit immediate field.
#[must_use]
pub fn sub_x_imm(dst: u8, src: u8, imm12: u16) -> u32 {
    encode_add_sub_x_imm(0xD100_0000, dst, src, imm12)
}

/// Encodes `ADD Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn add_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0x8B00_0000, dst, lhs, rhs)
}

/// Encodes `ADDS Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn adds_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0xAB00_0000, dst, lhs, rhs)
}

/// Encodes `SUB Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn sub_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0xCB00_0000, dst, lhs, rhs)
}

/// Encodes `AND Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn and_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0x8A00_0000, dst, lhs, rhs)
}

/// Encodes `ANDS Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn ands_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0xEA00_0000, dst, lhs, rhs)
}

/// Encodes `ORR Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn orr_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0xAA00_0000, dst, lhs, rhs)
}

/// Encodes `EOR Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn eor_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_logical_x_reg(0xCA00_0000, dst, lhs, rhs)
}

/// Encodes `LSLV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn lsl_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_shift_x_reg(0x9AC0_2000, dst, lhs, rhs)
}

/// Encodes `LSRV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn lsr_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_shift_x_reg(0x9AC0_2400, dst, lhs, rhs)
}

/// Encodes `ASRV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn asr_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_shift_x_reg(0x9AC0_2800, dst, lhs, rhs)
}

/// Encodes `RORV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn ror_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_shift_x_reg(0x9AC0_2C00, dst, lhs, rhs)
}

/// Encodes `CRC32CB Wd, Wn, Wm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn crc32cb(dst: u8, crc: u8, data: u8) -> u32 {
    encode_data_processing_two_source(0x1AC0_5000, dst, crc, data)
}

/// Encodes `CRC32CH Wd, Wn, Wm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn crc32ch(dst: u8, crc: u8, data: u8) -> u32 {
    encode_data_processing_two_source(0x1AC0_5400, dst, crc, data)
}

/// Encodes `CRC32CW Wd, Wn, Wm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn crc32cw(dst: u8, crc: u8, data: u8) -> u32 {
    encode_data_processing_two_source(0x1AC0_5800, dst, crc, data)
}

/// Encodes `CRC32CX Wd, Wn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn crc32cx(dst: u8, crc: u8, data: u8) -> u32 {
    encode_data_processing_two_source(0x9AC0_5C00, dst, crc, data)
}

/// Encodes `CLZ Xd, Xn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn clz_x(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0xDAC0_1000, dst, src)
}

/// Encodes `CLZ Wd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn clz_w(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0x5AC0_1000, dst, src)
}

/// Encodes `RBIT Xd, Xn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn rbit_x(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0xDAC0_0000, dst, src)
}

/// Encodes `RBIT Wd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn rbit_w(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0x5AC0_0000, dst, src)
}

/// Encodes `REV Xd, Xn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn rev_x(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0xDAC0_0C00, dst, src)
}

/// Encodes `REV Wd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn rev_w(dst: u8, src: u8) -> u32 {
    encode_data_processing_one_source(0x5AC0_0800, dst, src)
}

/// Encodes `SXTB Xd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn sxtb_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0x9340_0000, dst, src, 0, 7)
}

/// Encodes `SXTH Xd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn sxth_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0x9340_0000, dst, src, 0, 15)
}

/// Encodes `SXTW Xd, Wn`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn sxtw_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0x9340_0000, dst, src, 0, 31)
}

/// Encodes a zero-extension of the low byte into `Xd`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn uxtb_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0xD340_0000, dst, src, 0, 7)
}

/// Encodes a zero-extension of the low halfword into `Xd`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn uxth_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0xD340_0000, dst, src, 0, 15)
}

/// Encodes a zero-extension of the low word into `Xd`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn uxtw_x(dst: u8, src: u8) -> u32 {
    encode_bitfield_x(0xD340_0000, dst, src, 0, 31)
}

/// Encodes `MUL Xd, Xn, Xm` as `MADD Xd, Xn, Xm, XZR`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn mul_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_madd_x(0x9B00_0000, dst, lhs, rhs, 31)
}

/// Encodes `UMULH Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn umulh_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_multiply_high_x(0x9BC0_7C00, dst, lhs, rhs)
}

/// Encodes `SMULH Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn smulh_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_multiply_high_x(0x9B40_7C00, dst, lhs, rhs)
}

/// Encodes `UDIV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn udiv_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_divide_x(0x9AC0_0800, dst, lhs, rhs)
}

/// Encodes `SDIV Xd, Xn, Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn sdiv_x(dst: u8, lhs: u8, rhs: u8) -> u32 {
    encode_divide_x(0x9AC0_0C00, dst, lhs, rhs)
}

/// Encodes `MSUB Xd, Xn, Xm, Xa`.
///
/// Computes `Xd = Xa - Xn * Xm`.
///
/// # Panics
///
/// Panics if any register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn msub_x(dst: u8, lhs: u8, rhs: u8, minuend: u8) -> u32 {
    encode_madd_x(0x9B00_8000, dst, lhs, rhs, minuend)
}

/// Encodes `CMP Xn, Xm` as `SUBS XZR, Xn, Xm`.
///
/// # Panics
///
/// Panics if either register number is outside the `AArch64` register range
/// `0..32`.
#[must_use]
pub fn cmp_x(lhs: u8, rhs: u8) -> u32 {
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    0xEB00_001F | (u32::from(rhs) << 16) | (u32::from(lhs) << 5)
}

/// Encodes `CSET Xd, cond` as `CSINC Xd, XZR, XZR, !cond`.
///
/// # Panics
///
/// Panics if `dst` is outside the `AArch64` register range `0..32`.
#[must_use]
pub fn cset_x(dst: u8, cond: CondCode) -> u32 {
    assert!(dst < 32, "destination register out of range");
    let inverted = invert_condition(cond);
    0x9A80_0400
        | (31 << 16)
        | (u32::from(arm_condition(inverted)) << 12)
        | (31 << 5)
        | u32::from(dst)
}

/// Encodes `CSEL Xd, Xn, Xm, cond` — `Xd = if cond then Xn else Xm`.
///
/// # Panics
///
/// Panics if any register is outside `0..32`.
#[must_use]
pub fn csel_x(dst: u8, if_true: u8, if_false: u8, cond: CondCode) -> u32 {
    assert!(
        dst < 32 && if_true < 32 && if_false < 32,
        "register out of range"
    );
    // CSEL (64-bit): sf=1, Rm at [20:16], cond at [15:12], Rn at [9:5], Rd[4:0].
    0x9A80_0000
        | (u32::from(if_false) << 16)
        | (u32::from(arm_condition(cond)) << 12)
        | (u32::from(if_true) << 5)
        | u32::from(dst)
}

/// Encodes `LDR Xt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 8-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn ldr_x_unsigned(dst: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0xF940_0000, dst, base_reg, imm_bytes, 8)
}

/// Encodes `LDR Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 4-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn ldr_w_unsigned(dst: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0xB940_0000, dst, base_reg, imm_bytes, 4)
}

/// Encodes `LDRH Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 2-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn ldrh_unsigned(dst: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0x7940_0000, dst, base_reg, imm_bytes, 2)
}

/// Encodes `LDRB Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is outside the
/// unsigned 12-bit unscaled byte offset range.
#[must_use]
pub fn ldrb_unsigned(dst: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0x3940_0000, dst, base_reg, imm_bytes, 1)
}

/// Encodes `STR Xt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 8-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn str_x_unsigned(src: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0xF900_0000, src, base_reg, imm_bytes, 8)
}

/// Encodes `STR Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 4-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn str_w_unsigned(src: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0xB900_0000, src, base_reg, imm_bytes, 4)
}

/// Encodes `STRH Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is not 2-byte
/// aligned in the unsigned 12-bit scaled offset range.
#[must_use]
pub fn strh_unsigned(src: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0x7900_0000, src, base_reg, imm_bytes, 2)
}

/// Encodes `STRB Wt, [Xn, #imm]`.
///
/// # Panics
///
/// Panics if a register is outside `0..32`, or if `imm_bytes` is outside the
/// unsigned 12-bit unscaled byte offset range.
#[must_use]
pub fn strb_unsigned(src: u8, base_reg: u8, imm_bytes: u16) -> u32 {
    encode_unsigned_offset(0x3900_0000, src, base_reg, imm_bytes, 1)
}

/// Encodes `STP Xt1, Xt2, [SP, #imm]!`.
///
/// # Panics
///
/// Panics if a register number is outside `0..32`, or if `imm_bytes` is not
/// 8-byte aligned in the signed 7-bit scaled pair offset range.
#[must_use]
pub fn stp_x_pre_sp(rt: u8, rt2: u8, imm_bytes: i16) -> u32 {
    encode_pair_sp_indexed(0xA980_0000, rt, rt2, imm_bytes)
}

/// Encodes `LDP Xt1, Xt2, [SP], #imm`.
///
/// # Panics
///
/// Panics if a register number is outside `0..32`, or if `imm_bytes` is not
/// 8-byte aligned in the signed 7-bit scaled pair offset range.
#[must_use]
pub fn ldp_x_post_sp(rt: u8, rt2: u8, imm_bytes: i16) -> u32 {
    encode_pair_sp_indexed(0xA8C0_0000, rt, rt2, imm_bytes)
}

fn encode_add_sub_x_imm(base: u32, dst: u8, src: u8, imm12: u16) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(src < 32, "source register out of range");
    assert!(imm12 < 4096, "immediate out of range");
    base | (u32::from(imm12) << 10) | (u32::from(src) << 5) | u32::from(dst)
}

fn encode_logical_x_reg(base: u32, dst: u8, lhs: u8, rhs: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    base | (u32::from(rhs) << 16) | (u32::from(lhs) << 5) | u32::from(dst)
}

fn encode_shift_x_reg(base: u32, dst: u8, lhs: u8, rhs: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    base | (u32::from(rhs) << 16) | (u32::from(lhs) << 5) | u32::from(dst)
}

fn encode_data_processing_two_source(base: u32, dst: u8, lhs: u8, rhs: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    base | (u32::from(rhs) << 16) | (u32::from(lhs) << 5) | u32::from(dst)
}

// immr/imms are the canonical ARMv8 bitfield encoding field names.
#[allow(clippy::similar_names)]
fn encode_bitfield_x(base: u32, dst: u8, src: u8, immr: u8, imms: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(src < 32, "source register out of range");
    assert!(immr < 64, "immr out of range");
    assert!(imms < 64, "imms out of range");
    base | (u32::from(immr) << 16)
        | (u32::from(imms) << 10)
        | (u32::from(src) << 5)
        | u32::from(dst)
}

fn encode_data_processing_one_source(base: u32, dst: u8, src: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(src < 32, "source register out of range");
    base | (u32::from(src) << 5) | u32::from(dst)
}

fn encode_madd_x(base: u32, dst: u8, lhs: u8, rhs: u8, addend: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    assert!(addend < 32, "addend register out of range");
    base | (u32::from(rhs) << 16)
        | (u32::from(addend) << 10)
        | (u32::from(lhs) << 5)
        | u32::from(dst)
}

fn encode_multiply_high_x(base: u32, dst: u8, lhs: u8, rhs: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    base | (u32::from(rhs) << 16) | (u32::from(lhs) << 5) | u32::from(dst)
}

fn encode_divide_x(base: u32, dst: u8, lhs: u8, rhs: u8) -> u32 {
    assert!(dst < 32, "destination register out of range");
    assert!(lhs < 32, "left register out of range");
    assert!(rhs < 32, "right register out of range");
    base | (u32::from(rhs) << 16) | (u32::from(lhs) << 5) | u32::from(dst)
}

const fn arm_condition(cond: CondCode) -> u8 {
    match cond {
        CondCode::Eq => 0x0,
        CondCode::Ne => 0x1,
        CondCode::Uge | CondCode::Nc => 0x2,
        CondCode::Ult | CondCode::Cc => 0x3,
        CondCode::Mi => 0x4,
        CondCode::Pl => 0x5,
        CondCode::Ov => 0x6,
        CondCode::NoOv => 0x7,
        CondCode::Ugt => 0x8,
        CondCode::Ule => 0x9,
        CondCode::Sge => 0xa,
        CondCode::Slt => 0xb,
        CondCode::Sgt => 0xc,
        CondCode::Sle => 0xd,
    }
}

const fn invert_condition(cond: CondCode) -> CondCode {
    match cond {
        CondCode::Eq => CondCode::Ne,
        CondCode::Ne => CondCode::Eq,
        CondCode::Ult => CondCode::Uge,
        CondCode::Ule => CondCode::Ugt,
        CondCode::Ugt => CondCode::Ule,
        CondCode::Uge => CondCode::Ult,
        CondCode::Slt => CondCode::Sge,
        CondCode::Sle => CondCode::Sgt,
        CondCode::Sgt => CondCode::Sle,
        CondCode::Sge => CondCode::Slt,
        CondCode::Cc => CondCode::Nc,
        CondCode::Nc => CondCode::Cc,
        CondCode::Ov => CondCode::NoOv,
        CondCode::NoOv => CondCode::Ov,
        CondCode::Mi => CondCode::Pl,
        CondCode::Pl => CondCode::Mi,
    }
}

fn encode_compare_branch_x(base: u32, rt: u8, offset_bytes: i32) -> u32 {
    assert!(rt < 32, "test register out of range");
    assert!(
        offset_bytes % 4 == 0,
        "branch offset must be 4-byte aligned"
    );
    let imm19 = offset_bytes >> 2;
    assert!(
        (-(1 << 18)..(1 << 18)).contains(&imm19),
        "conditional branch offset out of range"
    );
    base | ((imm19.cast_unsigned() & 0x7_FFFF) << 5) | u32::from(rt)
}

fn encode_unsigned_offset(base: u32, rt: u8, base_reg: u8, imm_bytes: u16, scale: u16) -> u32 {
    assert!(rt < 32, "target/source register out of range");
    assert!(base_reg < 32, "base register out of range");
    assert!(
        imm_bytes.is_multiple_of(scale),
        "memory offset must be aligned to access size"
    );
    let scaled = imm_bytes / scale;
    assert!(scaled < 4096, "memory offset out of range");
    base | (u32::from(scaled) << 10) | (u32::from(base_reg) << 5) | u32::from(rt)
}

fn encode_pair_sp_indexed(base: u32, rt: u8, rt2: u8, imm_bytes: i16) -> u32 {
    assert!(rt < 32, "first register out of range");
    assert!(rt2 < 32, "second register out of range");
    assert!(
        imm_bytes % 8 == 0,
        "64-bit pair offset must be 8-byte aligned"
    );
    let scaled = imm_bytes / 8;
    assert!((-64..=63).contains(&scaled), "pair offset out of range");
    let imm7_signed = i8::try_from(scaled).expect("scaled offset was range-checked");
    let imm7 = u32::from(imm7_signed.cast_unsigned() & 0x7f);
    base | (imm7 << 15) | (u32::from(rt2) << 10) | (31 << 5) | u32::from(rt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_tracks_emitted_bytes() {
        let mut asm = Arm64Assembler::new();
        assert!(asm.is_empty());
        assert_eq!(asm.cursor_offset(), 0);
        asm.ret();
        assert_eq!(asm.len(), 1);
        assert_eq!(asm.cursor_offset(), 4);
    }

    #[test]
    fn encodes_basic_control_flow() {
        assert_eq!(nop(), 0xD503_201F);
        assert_eq!(ret(), 0xD65F_03C0);
        assert_eq!(br_x(0), 0xD61F_0000);
        assert_eq!(br_x(9), 0xD61F_0120);
        assert_eq!(blr_x(9), 0xD63F_0120);
        assert_eq!(b(0), 0x1400_0000);
        assert_eq!(b(8), 0x1400_0002);
        assert_eq!(b(-4), 0x17FF_FFFF);
        assert_eq!(b_cond(CondCode::Eq, 8), 0x5400_0040);
        assert_eq!(b_cond(CondCode::Ne, -4), 0x54FF_FFE1);
        assert_eq!(b_cond(CondCode::Slt, 0), 0x5400_000B);
        assert_eq!(cbz_x(0, 0), 0xB400_0000);
        assert_eq!(cbnz_x(1, 8), 0xB500_0041);
        assert_eq!(cbnz_x(1, -4), 0xB5FF_FFE1);
    }

    #[test]
    fn encodes_system_counter_and_fences() {
        assert_eq!(mrs_cntvct(0), 0xD53B_E040);
        assert_eq!(mrs_cntvct(9), 0xD53B_E049);
        assert_eq!(msr_nzcv(0), 0xD51B_4200);
        assert_eq!(msr_nzcv(9), 0xD51B_4209);
        assert_eq!(fence(FenceKind::Mfence), 0xD503_3BBF);
        assert_eq!(fence(FenceKind::Lfence), 0xD503_39BF);
        assert_eq!(fence(FenceKind::Sfence), 0xD503_3ABF);
    }

    #[test]
    fn encodes_extend_aliases() {
        assert_eq!(sxtb_x(0, 1), 0x9340_1C20);
        assert_eq!(sxth_x(2, 3), 0x9340_3C62);
        assert_eq!(sxtw_x(4, 5), 0x9340_7CA4);
        assert_eq!(uxtb_x(6, 7), 0xD340_1CE6);
        assert_eq!(uxth_x(8, 9), 0xD340_3D28);
        assert_eq!(uxtw_x(10, 11), 0xD340_7D6A);
    }

    #[test]
    fn encodes_count_zero_primitives() {
        assert_eq!(clz_x(0, 1), 0xDAC0_1020);
        assert_eq!(clz_w(2, 3), 0x5AC0_1062);
        assert_eq!(rbit_x(4, 5), 0xDAC0_00A4);
        assert_eq!(rbit_w(6, 7), 0x5AC0_00E6);
    }

    #[test]
    fn encodes_reverse_byte_primitives() {
        assert_eq!(rev_x(0, 1), 0xDAC0_0C20);
        assert_eq!(rev_w(2, 3), 0x5AC0_0862);
    }

    #[test]
    fn resolves_forward_branch_label() {
        let mut asm = Arm64Assembler::new();
        let target = asm.create_label();
        asm.b_label(target);
        asm.nop();
        asm.bind_label(target);
        asm.ret();

        assert_eq!(asm.finish(), vec![0x1400_0002, 0xD503_201F, 0xD65F_03C0]);
    }

    #[test]
    fn resolves_backward_branch_label() {
        let mut asm = Arm64Assembler::new();
        let loop_head = asm.create_label();
        asm.bind_label(loop_head);
        asm.nop();
        asm.b_label(loop_head);

        assert_eq!(asm.finish(), vec![0xD503_201F, 0x17FF_FFFF]);
    }

    #[test]
    fn resolves_compare_branch_labels() {
        let mut asm = Arm64Assembler::new();
        let target = asm.create_label();
        asm.cbnz_x_label(9, target);
        asm.cbz_x_label(10, target);
        asm.nop();
        asm.bind_label(target);
        asm.ret();

        assert_eq!(
            asm.finish(),
            vec![0xB500_0069, 0xB400_004A, 0xD503_201F, 0xD65F_03C0]
        );
    }

    #[test]
    fn resolves_conditional_branch_label() {
        let mut asm = Arm64Assembler::new();
        let target = asm.create_label();
        asm.b_cond_label(CondCode::Ne, target);
        asm.nop();
        asm.bind_label(target);
        asm.ret();

        assert_eq!(asm.finish(), vec![0x5400_0041, 0xD503_201F, 0xD65F_03C0]);
    }

    #[test]
    #[should_panic(expected = "branch label was not bound")]
    fn rejects_unbound_branch_label_on_finish() {
        let mut asm = Arm64Assembler::new();
        let target = asm.create_label();
        asm.b_label(target);
        let _ = asm.finish();
    }

    #[test]
    fn encodes_mov_alias() {
        assert_eq!(mov_x(27, 0), 0xAA00_03FB);
        assert_eq!(mov_x(1, 2), 0xAA02_03E1);
    }

    #[test]
    fn encodes_movz_immediate() {
        assert_eq!(movz_x(0, 0, 0), 0xD280_0000);
        assert_eq!(movz_x(1, 0x1234, 0), 0xD282_4681);
        assert_eq!(movz_x(2, 0xABCD, 16), 0xD2B5_79A2);
    }

    #[test]
    fn encodes_movk_immediate() {
        assert_eq!(movk_x(0, 0, 0), 0xF280_0000);
        assert_eq!(movk_x(1, 0x1234, 0), 0xF282_4681);
        assert_eq!(movk_x(2, 0xABCD, 32), 0xF2D5_79A2);
    }

    #[test]
    fn encodes_add_sub_immediates() {
        assert_eq!(add_x_imm(0, 0, 0), 0x9100_0000);
        assert_eq!(add_x_imm(1, 2, 7), 0x9100_1C41);
        assert_eq!(sub_x_imm(1, 2, 7), 0xD100_1C41);
        assert_eq!(add_x_imm(31, 31, 16), 0x9100_43FF);
    }

    #[test]
    fn encodes_add_sub_register_ops() {
        assert_eq!(add_x(11, 10, 9), 0x8B09_014B);
        assert_eq!(sub_x(11, 10, 9), 0xCB09_014B);
    }

    #[test]
    fn encodes_adds_and_ands_register_ops() {
        assert_eq!(adds_x(11, 10, 9), 0xAB09_014B);
        assert_eq!(ands_x(11, 10, 9), 0xEA09_014B);
    }

    #[test]
    fn encodes_logical_register_ops() {
        assert_eq!(and_x(11, 9, 10), 0x8A0A_012B);
        assert_eq!(orr_x(11, 9, 10), 0xAA0A_012B);
        assert_eq!(eor_x(11, 9, 10), 0xCA0A_012B);
    }

    #[test]
    fn encodes_shift_register_ops() {
        assert_eq!(lsl_x(11, 9, 10), 0x9ACA_212B);
        assert_eq!(lsr_x(11, 9, 10), 0x9ACA_252B);
        assert_eq!(asr_x(11, 9, 10), 0x9ACA_292B);
        assert_eq!(ror_x(11, 9, 10), 0x9ACA_2D2B);
    }

    #[test]
    fn encodes_crc32c_primitives() {
        assert_eq!(crc32cb(0, 1, 2), 0x1AC2_5020);
        assert_eq!(crc32ch(3, 4, 5), 0x1AC5_5483);
        assert_eq!(crc32cw(6, 7, 8), 0x1AC8_58E6);
        assert_eq!(crc32cx(9, 10, 11), 0x9ACB_5D49);
    }

    #[test]
    fn encodes_multiply_divide_register_ops() {
        assert_eq!(mul_x(11, 9, 10), 0x9B0A_7D2B);
        assert_eq!(umulh_x(11, 9, 10), 0x9BCA_7D2B);
        assert_eq!(smulh_x(11, 9, 10), 0x9B4A_7D2B);
        assert_eq!(udiv_x(11, 9, 10), 0x9ACA_092B);
        assert_eq!(sdiv_x(11, 9, 10), 0x9ACA_0D2B);
        assert_eq!(msub_x(11, 20, 10, 9), 0x9B0A_A68B);
    }

    #[test]
    fn encodes_cmp_and_cset() {
        assert_eq!(cmp_x(9, 10), 0xEB0A_013F);
        assert_eq!(cset_x(9, CondCode::Eq), 0x9A9F_17E9);
        assert_eq!(cset_x(9, CondCode::Ne), 0x9A9F_07E9);
        assert_eq!(cset_x(9, CondCode::Ult), 0x9A9F_27E9);
        assert_eq!(cset_x(9, CondCode::Slt), 0x9A9F_A7E9);
    }

    #[test]
    fn encodes_csel() {
        // CSEL x9, x9, x10, eq  (x9 = eq ? x9 : x10)
        assert_eq!(csel_x(9, 9, 10, CondCode::Eq), 0x9A8A_0129);
        // CSEL x0, x1, x2, ne
        assert_eq!(csel_x(0, 1, 2, CondCode::Ne), 0x9A82_1020);
    }

    #[test]
    fn encodes_stack_pair_ops() {
        assert_eq!(stp_x_pre_sp(29, 30, -16), 0xA9BF_7BFD);
        assert_eq!(ldp_x_post_sp(29, 30, 16), 0xA8C1_7BFD);
        assert_eq!(stp_x_pre_sp(19, 20, -16), 0xA9BF_53F3);
        assert_eq!(ldp_x_post_sp(19, 20, 16), 0xA8C1_53F3);
    }

    #[test]
    fn encodes_unsigned_memory_ops() {
        assert_eq!(strb_unsigned(10, 9, 0), 0x3900_012A);
        assert_eq!(ldrb_unsigned(11, 9, 0), 0x3940_012B);
        assert_eq!(ldrb_unsigned(11, 9, 3), 0x3940_0D2B);
        assert_eq!(strh_unsigned(10, 9, 2), 0x7900_052A);
        assert_eq!(ldrh_unsigned(11, 9, 4), 0x7940_092B);
        assert_eq!(str_w_unsigned(10, 9, 4), 0xB900_052A);
        assert_eq!(ldr_w_unsigned(11, 9, 8), 0xB940_092B);
        assert_eq!(str_x_unsigned(10, 9, 0), 0xF900_012A);
        assert_eq!(ldr_x_unsigned(11, 9, 0), 0xF940_012B);
        assert_eq!(ldr_x_unsigned(11, 9, 16), 0xF940_092B);
    }

    #[test]
    fn finishes_as_little_endian_bytes() {
        let mut asm = Arm64Assembler::new();
        asm.nop();
        asm.ret();
        assert_eq!(
            asm.finish_bytes(),
            vec![0x1f, 0x20, 0x03, 0xd5, 0xc0, 0x03, 0x5f, 0xd6]
        );
    }
}
