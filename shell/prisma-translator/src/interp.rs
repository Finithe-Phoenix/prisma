//! A small reference interpreter for the integer + control-flow IR subset.
//!
//! This executes a block's optimized IR directly against a guest register file,
//! independent of the ARM64 backend. Its purpose is differential diagnosis: if a
//! program produces the wrong result on real ARM64 but the interpreter (running
//! the SAME decoded + optimized IR) produces the right one, the defect is in
//! lowering/codegen; if the interpreter is also wrong, the defect is in the
//! decode/optimize pipeline. It is deliberately limited to the ops a
//! straight-line integer block uses; an unsupported op stops the block.
//!
//! Flags are modelled the ARM64 way (NZCV from a `SUBS`-style compare), since
//! that is what `CmpFlags`/`AluFlags`/`CondJumpRel` lower to. At a block entry
//! with no local flag writer, condition consumers restore NZCV from the
//! persistent RFLAGS subset, matching backend lowering.

// Guest integer semantics are wrapping/truncating by definition — in this
// reference interpreter those casts are the behavior under test, not a bug.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;

use prisma_ir::{
    BinOpKind, CondCode, Gpr, Op, OpSize, Ref, RflagsCarryMode, Stmt, TrapKind, WideDivResult,
};

/// Guest integer register file (x86 GPR order, matching `CpuStateFrame::gpr`).
///
/// `cf` mirrors the runtime's dedicated persistent carry slot. `rflags` models
/// the currently persisted RFLAGS subset; bit 1 is always forced, matching the
/// backend's `StoreRflags`/`PUSHFQ` handling.
#[derive(Debug, Clone)]
pub struct GuestRegs {
    pub gpr: [u64; 16],
    pub xmm: [u128; 16],
    pub cf: u64,
    pub rflags: u64,
}

impl Default for GuestRegs {
    fn default() -> Self {
        Self {
            gpr: [0; 16],
            xmm: [0; 16],
            cf: 0,
            rflags: 2,
        }
    }
}

/// NZCV condition flags, as an `SUBS` would set them. The four flags are the
/// architectural NZCV bits — modelling them as one bool each mirrors the ISA.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    n: bool,
    z: bool,
    c: bool,
    v: bool,
}

/// How interpreting a block ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockOutcome {
    /// A relative branch (`JumpRel`/`CondJumpRel`) resolved to this guest PC.
    Branch(u64),
    /// The block ended on a `SYSCALL`.
    Syscall,
    /// The block ended on a `ret` / indirect transfer (a dynamic target).
    DynamicTransfer,
    /// The block triggered a guest trap.
    Trap(TrapKind),
    /// The block had no terminator (cut at the instruction budget).
    Fallthrough,
    /// An op outside the interpreted subset was reached.
    Unsupported(&'static str),
}

fn mask(value: u64, size: OpSize) -> u64 {
    match size {
        OpSize::I64 => value,
        _ => value & ((1u64 << size.bit_width()) - 1),
    }
}

fn msb(value: u64, size: OpSize) -> bool {
    (value >> (size.bit_width() - 1)) & 1 == 1
}

fn sign_extend_to_i128(value: u64, size: OpSize) -> i128 {
    let bits = size.bit_width();
    let masked = i128::from(mask(value, size));
    let sign_bit = 1_i128 << (bits - 1);
    if masked & sign_bit == 0 {
        masked
    } else {
        masked - (1_i128 << bits)
    }
}

fn sign_extend_to_u64(value: u64, size: OpSize) -> u64 {
    sign_extend_to_i128(value, size) as i64 as u64
}

/// NZCV for `a - b` at `size`, exactly as ARM64 `SUBS` would compute them.
fn sub_flags(a: u64, b: u64, size: OpSize) -> Flags {
    let a = mask(a, size);
    let b = mask(b, size);
    let result = mask(a.wrapping_sub(b), size);
    Flags {
        n: msb(result, size),
        z: result == 0,
        c: a >= b, // no borrow
        v: msb(a, size) != msb(b, size) && msb(result, size) != msb(a, size),
    }
}

/// NZCV for `a + b` at `size`, as ARM64 `ADDS` would compute them.
fn add_flags(a: u64, b: u64, size: OpSize) -> Flags {
    let a = mask(a, size);
    let b = mask(b, size);
    let result = mask(a.wrapping_add(b), size);
    Flags {
        n: msb(result, size),
        z: result == 0,
        c: result < a, // unsigned carry-out
        v: msb(a, size) == msb(b, size) && msb(result, size) != msb(a, size),
    }
}

/// Logical-op flags (AND/OR/XOR): N/Z from the result, C and V cleared — x86
/// logical-op flag semantics, which the lowerer mirrors.
fn logic_flags(result: u64, size: OpSize) -> Flags {
    Flags {
        n: msb(result, size),
        z: mask(result, size) == 0,
        c: false,
        v: false,
    }
}

fn flags_from_rflags(rflags: u64) -> Flags {
    Flags {
        n: rflags & (1 << 7) != 0,
        z: rflags & (1 << 6) != 0,
        c: rflags & 1 != 0,
        v: rflags & (1 << 11) != 0,
    }
}

fn eval_cc(cc: CondCode, f: Flags) -> bool {
    match cc {
        CondCode::Eq => f.z,
        CondCode::Ne => !f.z,
        CondCode::Uge | CondCode::Nc => f.c,
        CondCode::Ult | CondCode::Cc => !f.c,
        CondCode::Ugt => f.c && !f.z,
        CondCode::Ule => !f.c || f.z,
        CondCode::Sge => f.n == f.v,
        CondCode::Slt => f.n != f.v,
        CondCode::Sgt => !f.z && (f.n == f.v),
        CondCode::Sle => f.z || (f.n != f.v),
        CondCode::Mi => f.n,
        CondCode::Pl => !f.n,
        CondCode::Ov => f.v,
        CondCode::NoOv => !f.v,
    }
}

fn store_reg(regs: &mut GuestRegs, reg: Gpr, value: u64, size: OpSize) {
    let idx = reg as usize;
    let v = mask(value, size);
    regs.gpr[idx] = match size {
        // 32-bit writes zero-extend to 64; 8/16-bit writes preserve the upper.
        OpSize::I32 | OpSize::I64 => v,
        OpSize::I16 => (regs.gpr[idx] & !0xFFFF) | v,
        OpSize::I8 => (regs.gpr[idx] & !0xFF) | v,
    };
}

fn store_carry(regs: &mut GuestRegs, value: u64) {
    let bit = value & 1;
    regs.cf = bit;
    regs.rflags = (regs.rflags & !1) | 2 | bit;
}

fn store_rflags(regs: &mut GuestRegs, value: u64) {
    regs.rflags = value | 2;
    regs.cf = regs.rflags & 1;
}

fn store_rflags_from_flags(
    regs: &mut GuestRegs,
    flags: Flags,
    carry: RflagsCarryMode,
    pf: Option<u64>,
    af: Option<u64>,
) {
    const CF: u64 = 1 << 0;
    const PF: u64 = 1 << 2;
    const AF: u64 = 1 << 4;
    const ZF: u64 = 1 << 6;
    const SF: u64 = 1 << 7;
    const OF: u64 = 1 << 11;

    let mut clear_mask = ZF | SF | OF;
    if pf.is_some() {
        clear_mask |= PF;
    }
    if af.is_some() {
        clear_mask |= AF;
    }

    let mut rflags = match carry {
        RflagsCarryMode::Preserve => regs.rflags & !clear_mask,
        RflagsCarryMode::ArmCarry | RflagsCarryMode::InvertArmCarry | RflagsCarryMode::Clear => {
            regs.rflags & !(CF | clear_mask)
        }
    } | 2;

    if pf.is_some_and(|v| v & 1 == 1) {
        rflags |= PF;
    }
    if af.is_some_and(|v| v & 1 == 1) {
        rflags |= AF;
    }
    if flags.z {
        rflags |= ZF;
    }
    if flags.n {
        rflags |= SF;
    }
    if flags.v {
        rflags |= OF;
    }
    match carry {
        RflagsCarryMode::ArmCarry if flags.c => rflags |= CF,
        RflagsCarryMode::InvertArmCarry if !flags.c => rflags |= CF,
        RflagsCarryMode::Preserve if regs.cf & 1 == 1 => rflags |= CF,
        RflagsCarryMode::ArmCarry
        | RflagsCarryMode::InvertArmCarry
        | RflagsCarryMode::Clear
        | RflagsCarryMode::Preserve => {}
    }

    regs.rflags = rflags;
    regs.cf = rflags & 1;
}

fn store_rflags_from_bits(
    regs: &mut GuestRegs,
    pf: Option<u64>,
    af: Option<u64>,
    zf: u64,
    sf: u64,
    of: u64,
) {
    const CF: u64 = 1 << 0;
    const PF: u64 = 1 << 2;
    const AF: u64 = 1 << 4;
    const ZF: u64 = 1 << 6;
    const SF: u64 = 1 << 7;
    const OF: u64 = 1 << 11;

    let mut clear_mask = ZF | SF | OF;
    if pf.is_some() {
        clear_mask |= PF;
    }
    if af.is_some() {
        clear_mask |= AF;
    }

    let mut rflags = (regs.rflags & !clear_mask) | 2;
    rflags = (rflags & !CF) | (regs.cf & 1);
    if pf.is_some_and(|v| v & 1 == 1) {
        rflags |= PF;
    }
    if af.is_some_and(|v| v & 1 == 1) {
        rflags |= AF;
    }
    if zf & 1 == 1 {
        rflags |= ZF;
    }
    if sf & 1 == 1 {
        rflags |= SF;
    }
    if of & 1 == 1 {
        rflags |= OF;
    }
    regs.rflags = rflags;
}

fn eval_binop(op: BinOpKind, a: u64, b: u64, size: OpSize) -> Option<u64> {
    let a = mask(a, size);
    let b = mask(b, size);
    let bits = size.bit_width();
    let r = match op {
        BinOpKind::Add => a.wrapping_add(b),
        BinOpKind::Sub => a.wrapping_sub(b),
        BinOpKind::And => a & b,
        BinOpKind::Or => a | b,
        BinOpKind::Xor => a ^ b,
        BinOpKind::Mul => a.wrapping_mul(b),
        BinOpKind::UMulHi => ((u128::from(a) * u128::from(b)) >> bits) as u64,
        BinOpKind::SMulHi => {
            let product = sign_extend_to_i128(a, size) * sign_extend_to_i128(b, size);
            (product >> bits) as u64
        }
        BinOpKind::UDiv => {
            if b == 0 {
                return None;
            }
            a / b
        }
        BinOpKind::SDiv => {
            if b == 0 {
                return None;
            }
            let lhs = sign_extend_to_i128(a, size);
            let rhs = sign_extend_to_i128(b, size);
            if lhs == -(1_i128 << (bits - 1)) && rhs == -1 {
                lhs as u64
            } else {
                (lhs / rhs) as u64
            }
        }
        BinOpKind::UMod => {
            if b == 0 {
                return None;
            }
            a % b
        }
        BinOpKind::SMod => {
            if b == 0 {
                return None;
            }
            let lhs = sign_extend_to_i128(a, size);
            let rhs = sign_extend_to_i128(b, size);
            if lhs == -(1_i128 << (bits - 1)) && rhs == -1 {
                0
            } else {
                (lhs % rhs) as u64
            }
        }
        BinOpKind::Shl => a.wrapping_shl((b & 63) as u32),
        BinOpKind::Shr => a.wrapping_shr((b & 63) as u32),
        BinOpKind::Sar => (sign_extend_to_i128(a, size) >> (b & 63)) as u64,
        BinOpKind::Rol => {
            let count = (b % u64::from(bits)) as u32;
            if count == 0 {
                a
            } else {
                ((a << count) | (a >> (bits - count))) & size.mask()
            }
        }
        BinOpKind::Ror => {
            let count = (b % u64::from(bits)) as u32;
            if count == 0 {
                a
            } else {
                ((a >> count) | (a << (bits - count))) & size.mask()
            }
        }
        _ => return None,
    };
    Some(mask(r, size))
}

fn eval_wide_div(
    high: u64,
    low: u64,
    divisor: u64,
    signed: bool,
    result: WideDivResult,
) -> Option<u64> {
    if divisor == 0 {
        return None;
    }

    if signed {
        let dividend_bits = (u128::from(high) << 64) | u128::from(low);
        let dividend = dividend_bits as i128;
        let divisor = i128::from(divisor as i64);
        let quotient = dividend / divisor;
        if quotient < i128::from(i64::MIN) || quotient > i128::from(i64::MAX) {
            return None;
        }
        let remainder = dividend % divisor;
        return Some(match result {
            WideDivResult::Quotient => quotient as i64 as u64,
            WideDivResult::Remainder => remainder as i64 as u64,
        });
    }

    let dividend = (u128::from(high) << 64) | u128::from(low);
    let quotient = dividend / u128::from(divisor);
    if quotient > u128::from(u64::MAX) {
        return None;
    }
    let remainder = dividend % u128::from(divisor);
    Some(match result {
        WideDivResult::Quotient => quotient as u64,
        WideDivResult::Remainder => remainder as u64,
    })
}

#[derive(Debug, Clone, Copy)]
struct PcmpStrEval {
    intres: u16,
    max_lanes: usize,
    lane_bytes: usize,
    lhs_len: usize,
    rhs_len: usize,
}

fn lane_unsigned(bytes: &[u8; 16], lane: usize, lane_bytes: usize) -> u16 {
    if lane_bytes == 1 {
        u16::from(bytes[lane])
    } else {
        u16::from_le_bytes([bytes[lane * 2], bytes[lane * 2 + 1]])
    }
}

fn lane_signed(bytes: &[u8; 16], lane: usize, lane_bytes: usize) -> i16 {
    if lane_bytes == 1 {
        i16::from(bytes[lane] as i8)
    } else {
        i16::from_le_bytes([bytes[lane * 2], bytes[lane * 2 + 1]])
    }
}

fn pcmp_effective_len(bytes: &[u8; 16], lane_bytes: usize, explicit: Option<u64>) -> usize {
    let max_lanes = 16 / lane_bytes;
    if let Some(raw) = explicit {
        let signed = raw as u32 as i32;
        return signed.unsigned_abs().min(max_lanes as u32) as usize;
    }
    (0..max_lanes)
        .position(|i| lane_unsigned(bytes, i, lane_bytes) == 0)
        .unwrap_or(max_lanes)
}

fn eval_pcmp_str(
    lhs: u128,
    rhs: u128,
    lhs_len: Option<u64>,
    rhs_len: Option<u64>,
    imm8: u8,
) -> PcmpStrEval {
    let lhs_bytes = lhs.to_le_bytes();
    let rhs_bytes = rhs.to_le_bytes();
    let lane_bytes = if imm8 & 1 == 0 { 1 } else { 2 };
    let max_lanes = 16 / lane_bytes;
    let signed = imm8 & 0x02 != 0;
    let aggregation = (imm8 >> 2) & 0x03;
    let polarity = (imm8 >> 4) & 0x03;
    let lhs_len = pcmp_effective_len(&lhs_bytes, lane_bytes, lhs_len);
    let rhs_len = pcmp_effective_len(&rhs_bytes, lane_bytes, rhs_len);

    let mut bits = 0u16;
    for i in 0..max_lanes {
        let lhs_valid = i < lhs_len;
        let mut matched = lhs_valid
            && match aggregation {
                // Equal-any.
                0 => (0..rhs_len).any(|j| {
                    if signed {
                        lane_signed(&lhs_bytes, i, lane_bytes)
                            == lane_signed(&rhs_bytes, j, lane_bytes)
                    } else {
                        lane_unsigned(&lhs_bytes, i, lane_bytes)
                            == lane_unsigned(&rhs_bytes, j, lane_bytes)
                    }
                }),
                // Ranges: RHS pairs form inclusive ranges.
                1 => (0..rhs_len).step_by(2).any(|j| {
                    if j + 1 >= rhs_len {
                        return false;
                    }
                    if signed {
                        let v = lane_signed(&lhs_bytes, i, lane_bytes);
                        let lo = lane_signed(&rhs_bytes, j, lane_bytes);
                        let hi = lane_signed(&rhs_bytes, j + 1, lane_bytes);
                        lo <= v && v <= hi
                    } else {
                        let v = lane_unsigned(&lhs_bytes, i, lane_bytes);
                        let lo = lane_unsigned(&rhs_bytes, j, lane_bytes);
                        let hi = lane_unsigned(&rhs_bytes, j + 1, lane_bytes);
                        lo <= v && v <= hi
                    }
                }),
                // Equal-each.
                2 => {
                    i < rhs_len
                        && if signed {
                            lane_signed(&lhs_bytes, i, lane_bytes)
                                == lane_signed(&rhs_bytes, i, lane_bytes)
                        } else {
                            lane_unsigned(&lhs_bytes, i, lane_bytes)
                                == lane_unsigned(&rhs_bytes, i, lane_bytes)
                        }
                }
                // Equal-ordered: RHS prefix appears at LHS position i.
                _ => {
                    rhs_len > 0
                        && i + rhs_len <= lhs_len
                        && (0..rhs_len).all(|j| {
                            if signed {
                                lane_signed(&lhs_bytes, i + j, lane_bytes)
                                    == lane_signed(&rhs_bytes, j, lane_bytes)
                            } else {
                                lane_unsigned(&lhs_bytes, i + j, lane_bytes)
                                    == lane_unsigned(&rhs_bytes, j, lane_bytes)
                            }
                        })
                }
            };

        let valid_for_polarity = if polarity & 0x02 != 0 {
            lhs_valid
        } else {
            true
        };
        if polarity & 0x01 != 0 && valid_for_polarity {
            matched = !matched;
        }
        if !lhs_valid && polarity & 0x02 != 0 {
            matched = false;
        }
        if matched {
            bits |= 1u16 << i;
        }
    }

    PcmpStrEval {
        intres: bits,
        max_lanes,
        lane_bytes,
        lhs_len,
        rhs_len,
    }
}

fn pcmp_str_index(eval: PcmpStrEval, imm8: u8) -> u64 {
    if imm8 & 0x40 == 0 {
        (0..eval.max_lanes)
            .find(|i| eval.intres & (1u16 << i) != 0)
            .unwrap_or(eval.max_lanes) as u64
    } else {
        (0..eval.max_lanes)
            .rev()
            .find(|i| eval.intres & (1u16 << i) != 0)
            .unwrap_or(eval.max_lanes) as u64
    }
}

fn pcmp_str_mask(eval: PcmpStrEval, imm8: u8) -> u128 {
    if imm8 & 0x40 == 0 {
        return u128::from(eval.intres);
    }

    let mut out = 0u128;
    for i in 0..eval.max_lanes {
        if eval.intres & (1u16 << i) == 0 {
            continue;
        }
        let lane_mask = if eval.lane_bytes == 1 {
            0xffu128
        } else {
            0xffffu128
        };
        out |= lane_mask << (i * eval.lane_bytes * 8);
    }
    out
}

fn pcmp_str_flags(eval: PcmpStrEval) -> u64 {
    let cf = u64::from(eval.intres != 0);
    let zf = u64::from(eval.rhs_len < eval.max_lanes);
    let sf = u64::from(eval.lhs_len < eval.max_lanes);
    let of = u64::from(eval.intres & 1 != 0);
    cf | (zf << 1) | (sf << 2) | (of << 3)
}

fn vec_lane_u64(value: u128, high: bool) -> u64 {
    if high {
        (value >> 64) as u64
    } else {
        value as u64
    }
}

fn vec_unpack(lhs: u128, rhs: u128, lane: prisma_ir::VecLane, high: bool) -> u128 {
    let lane_bits = match lane {
        prisma_ir::VecLane::B16 => 8,
        prisma_ir::VecLane::H8 => 16,
        prisma_ir::VecLane::S4 => 32,
        prisma_ir::VecLane::D2 => 64,
    };
    let lanes = 128 / lane_bits;
    let first = if high { lanes / 2 } else { 0 };
    let lane_mask = if lane_bits == 128 {
        u128::MAX
    } else {
        (1u128 << lane_bits) - 1
    };
    let mut result = 0u128;
    for output_pair in 0..(lanes / 2) {
        let source_lane = first + output_pair;
        let lhs_lane = (lhs >> (source_lane * lane_bits)) & lane_mask;
        let rhs_lane = (rhs >> (source_lane * lane_bits)) & lane_mask;
        result |= lhs_lane << ((output_pair * 2) * lane_bits);
        result |= rhs_lane << ((output_pair * 2 + 1) * lane_bits);
    }
    result
}

fn vec_shuffle_h4(src: u128, control: u8, high: bool) -> u128 {
    let first = if high { 4 } else { 0 };
    let mut result = src;
    for output_lane in 0..4 {
        let source_lane = first + usize::from((control >> (output_lane * 2)) & 0x03);
        let target_lane = first + output_lane;
        let value = (src >> (source_lane * 16)) & 0xffff;
        result &= !(0xffffu128 << (target_lane * 16));
        result |= value << (target_lane * 16);
    }
    result
}

fn vec_cmp(lhs: u128, rhs: u128, lane: prisma_ir::VecLane, kind: prisma_ir::VecCmpKind) -> u128 {
    let lane_bits = match lane {
        prisma_ir::VecLane::B16 => 8,
        prisma_ir::VecLane::H8 => 16,
        prisma_ir::VecLane::S4 => 32,
        prisma_ir::VecLane::D2 => 64,
    };
    let lane_mask = (1u128 << lane_bits) - 1;
    let sign_bit = 1u128 << (lane_bits - 1);
    let mut result = 0u128;
    for lane_index in 0..(128 / lane_bits) {
        let left = (lhs >> (lane_index * lane_bits)) & lane_mask;
        let right = (rhs >> (lane_index * lane_bits)) & lane_mask;
        let matches = match kind {
            prisma_ir::VecCmpKind::Eq => left == right,
            prisma_ir::VecCmpKind::Gt => (left ^ sign_bit) > (right ^ sign_bit),
        };
        if matches {
            result |= lane_mask << (lane_index * lane_bits);
        }
    }
    result
}

fn vec_mask_msb(src: u128) -> u64 {
    let mut result = 0u64;
    for byte in 0..16 {
        result |= u64::try_from((src >> (byte * 8 + 7)) & 1).expect("single mask bit") << byte;
    }
    result
}

fn carryless_mul_u64(lhs: u64, rhs: u64) -> u128 {
    let mut acc = 0u128;
    let lhs = u128::from(lhs);
    for bit in 0..64 {
        if ((rhs >> bit) & 1) != 0 {
            acc ^= lhs << bit;
        }
    }
    acc
}

fn f16_to_f32_bits(h: u16) -> u32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = (h >> 10) & 0x1f;
    let frac = u32::from(h & 0x03ff);
    match exp {
        0 => {
            if frac == 0 {
                sign
            } else {
                let mut mant = frac;
                let mut e = -14i32;
                while (mant & 0x0400) == 0 {
                    mant <<= 1;
                    e -= 1;
                }
                mant &= 0x03ff;
                sign | (u32::try_from(e + 127).expect("normalized f16 exponent") << 23)
                    | (mant << 13)
            }
        }
        0x1f => sign | 0x7f80_0000 | (frac << 13),
        _ => sign | (u32::from(exp + 112) << 23) | (frac << 13),
    }
}

fn f16_overflow(sign: u16, mode: u8) -> u16 {
    let inf = sign | 0x7c00;
    let max = sign | 0x7bff;
    match mode {
        1 => {
            if sign != 0 {
                inf
            } else {
                max
            }
        }
        2 => {
            if sign == 0 {
                inf
            } else {
                max
            }
        }
        3 => max,
        _ => inf,
    }
}

fn f16_should_round(sign: u16, mode: u8, remainder: u64, halfway: u64, lsb: u64) -> bool {
    match mode {
        1 => sign != 0 && remainder != 0, // toward -inf
        2 => sign == 0 && remainder != 0, // toward +inf
        3 => false,                       // toward zero
        _ => remainder > halfway || (remainder == halfway && lsb != 0),
    }
}

fn f32_bits_to_f16(bits: u32, imm8: u8) -> u16 {
    let mode = if imm8 & 0x04 != 0 { 0 } else { imm8 & 0x03 };
    let sign = u16::try_from((bits >> 16) & 0x8000).expect("f16 sign");
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x007f_ffff;

    if exp == 0xff {
        if frac == 0 {
            return sign | 0x7c00;
        }
        let payload = u16::try_from((frac >> 13) & 0x01ff).expect("nan payload");
        return sign | 0x7e00 | payload;
    }

    let half_exp = exp - 127 + 15;
    if half_exp >= 0x1f {
        return f16_overflow(sign, mode);
    }

    if half_exp <= 0 {
        if half_exp < -10 {
            let inc = matches!(mode, 1) && sign != 0 || matches!(mode, 2) && sign == 0;
            return sign | u16::from(inc);
        }
        let mant = u64::from(frac | 0x0080_0000);
        let shift = u32::try_from(14 - half_exp).expect("subnormal shift");
        let q = mant >> shift;
        let rem_mask = (1u64 << shift) - 1;
        let rem = mant & rem_mask;
        let halfway = 1u64 << (shift - 1);
        let rounded = q + u64::from(f16_should_round(sign, mode, rem, halfway, q & 1));
        return sign | u16::try_from(rounded).expect("subnormal f16");
    }

    let mut half_exp_u = u16::try_from(half_exp).expect("positive half exponent");
    let mut mant = u16::try_from(frac >> 13).expect("f16 mantissa");
    let rem = u64::from(frac & 0x1fff);
    if f16_should_round(sign, mode, rem, 0x1000, u64::from(mant & 1)) {
        mant = mant.wrapping_add(1);
        if mant == 0x0400 {
            mant = 0;
            half_exp_u += 1;
            if half_exp_u >= 0x1f {
                return f16_overflow(sign, mode);
            }
        }
    }
    sign | (half_exp_u << 10) | mant
}

fn f16c_ph_to_ps(src: u128) -> u128 {
    let mut out = 0u128;
    for lane in 0..4 {
        let h = ((src >> (lane * 16)) & 0xffff) as u16;
        out |= u128::from(f16_to_f32_bits(h)) << (lane * 32);
    }
    out
}

fn f16c_ps_to_ph(src: u128, imm8: u8) -> u128 {
    let mut out = 0u128;
    for lane in 0..4 {
        let bits = ((src >> (lane * 32)) & 0xffff_ffff) as u32;
        out |= u128::from(f32_bits_to_f16(bits, imm8)) << (lane * 16);
    }
    out
}

/// Interpret one block's statements against `regs`, returning how it ended.
///
/// Operates on the optimized IR (post decode + renumber + pipeline) so it sees
/// exactly what the backend would lower.
// One arm per IR op; splitting the dispatch would scatter the op semantics.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn interpret_block(stmts: &[Stmt], regs: &mut GuestRegs) -> BlockOutcome {
    let mut vals: HashMap<Ref, u64> = HashMap::new();
    let mut vec_vals: HashMap<Ref, u128> = HashMap::new();
    let mut flags = Flags::default();
    let mut nzcv_live = false;
    let get = |vals: &HashMap<Ref, u64>, r: Ref| vals.get(&r).copied().unwrap_or(0);
    let get_vec = |vals: &HashMap<Ref, u128>, r: Ref| vals.get(&r).copied().unwrap_or(0);

    for stmt in stmts {
        match &stmt.op {
            Op::Constant(c) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, mask(c.value, c.size));
                }
            }
            Op::LoadReg(l) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, mask(regs.gpr[l.reg as usize], l.size));
                }
            }
            Op::StoreReg(s) => store_reg(regs, s.reg, get(&vals, s.value), s.size),
            Op::VecConstant(c) => {
                if let Some(d) = stmt.result {
                    vec_vals.insert(d, (u128::from(c.hi) << 64) | u128::from(c.lo));
                }
            }
            Op::LoadVecReg(l) => {
                if let Some(d) = stmt.result {
                    vec_vals.insert(d, regs.xmm[usize::from(l.xmm_index)]);
                }
            }
            Op::StoreVecReg(s) => {
                regs.xmm[usize::from(s.xmm_index)] = get_vec(&vec_vals, s.value);
            }
            Op::VecUnpack(p) => {
                if let Some(d) = stmt.result {
                    vec_vals.insert(
                        d,
                        vec_unpack(
                            get_vec(&vec_vals, p.lhs),
                            get_vec(&vec_vals, p.rhs),
                            p.lane,
                            p.is_high,
                        ),
                    );
                }
            }
            Op::VecShuffleH4(p) => {
                if let Some(d) = stmt.result {
                    vec_vals.insert(
                        d,
                        vec_shuffle_h4(get_vec(&vec_vals, p.src), p.control, p.is_high),
                    );
                }
            }
            Op::VecCmp(p) => {
                if let Some(d) = stmt.result {
                    vec_vals.insert(
                        d,
                        vec_cmp(
                            get_vec(&vec_vals, p.lhs),
                            get_vec(&vec_vals, p.rhs),
                            p.lane,
                            p.kind,
                        ),
                    );
                }
            }
            Op::VecMaskMsb(p) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, vec_mask_msb(get_vec(&vec_vals, p.src_xmm)));
                }
            }
            Op::VecClMul(p) => {
                if let Some(d) = stmt.result {
                    let lhs = vec_lane_u64(get_vec(&vec_vals, p.lhs), p.lhs_high);
                    let rhs = vec_lane_u64(get_vec(&vec_vals, p.rhs), p.rhs_high);
                    vec_vals.insert(d, carryless_mul_u64(lhs, rhs));
                }
            }
            Op::VecF16Cvt(p) => {
                if let Some(d) = stmt.result {
                    let src = get_vec(&vec_vals, p.src);
                    let result = match p.kind {
                        prisma_ir::VecF16CvtKind::PhToPs => f16c_ph_to_ps(src),
                        prisma_ir::VecF16CvtKind::PsToPh => f16c_ps_to_ph(src, p.rounding),
                    };
                    vec_vals.insert(d, result);
                }
            }
            Op::BinOp(b) => {
                let Some(r) = eval_binop(b.op, get(&vals, b.lhs), get(&vals, b.rhs), b.size) else {
                    return BlockOutcome::Unsupported("binop kind");
                };
                if let Some(d) = stmt.result {
                    vals.insert(d, r);
                }
            }
            Op::WideDiv(dv) => {
                let Some(r) = eval_wide_div(
                    get(&vals, dv.high),
                    get(&vals, dv.low),
                    get(&vals, dv.divisor),
                    dv.signed,
                    dv.result,
                ) else {
                    return BlockOutcome::Unsupported("wide div trap");
                };
                if let Some(d) = stmt.result {
                    vals.insert(d, r);
                }
            }
            Op::PcmpStrIndex(p) => {
                let eval = eval_pcmp_str(
                    get_vec(&vec_vals, p.lhs),
                    get_vec(&vec_vals, p.rhs),
                    p.lhs_len.map(|r| get(&vals, r)),
                    p.rhs_len.map(|r| get(&vals, r)),
                    p.imm8,
                );
                if let Some(d) = stmt.result {
                    vals.insert(d, pcmp_str_index(eval, p.imm8));
                }
            }
            Op::PcmpStrMask(p) => {
                let eval = eval_pcmp_str(
                    get_vec(&vec_vals, p.lhs),
                    get_vec(&vec_vals, p.rhs),
                    p.lhs_len.map(|r| get(&vals, r)),
                    p.rhs_len.map(|r| get(&vals, r)),
                    p.imm8,
                );
                if let Some(d) = stmt.result {
                    vec_vals.insert(d, pcmp_str_mask(eval, p.imm8));
                }
            }
            Op::PcmpStrFlags(p) => {
                let eval = eval_pcmp_str(
                    get_vec(&vec_vals, p.lhs),
                    get_vec(&vec_vals, p.rhs),
                    p.lhs_len.map(|r| get(&vals, r)),
                    p.rhs_len.map(|r| get(&vals, r)),
                    p.imm8,
                );
                if let Some(d) = stmt.result {
                    vals.insert(d, pcmp_str_flags(eval));
                }
            }
            Op::Extend(e) => {
                if let Some(d) = stmt.result {
                    let value = get(&vals, e.value);
                    let extended = if e.is_signed {
                        sign_extend_to_u64(value, e.from_size)
                    } else {
                        mask(value, e.from_size)
                    };
                    vals.insert(d, mask(extended, e.to_size));
                }
            }
            Op::Truncate(t) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, mask(get(&vals, t.value), t.to_size));
                }
            }
            Op::Compare(c) => {
                if let Some(d) = stmt.result {
                    let f = sub_flags(get(&vals, c.lhs), get(&vals, c.rhs), c.size);
                    vals.insert(d, u64::from(eval_cc(c.cc, f)));
                    flags = f;
                }
                nzcv_live = true;
            }
            Op::CmpFlags(c) => {
                flags = sub_flags(get(&vals, c.lhs), get(&vals, c.rhs), c.size);
                nzcv_live = true;
            }
            Op::WriteFlags(w) => {
                let (l, r) = (get(&vals, w.lhs), get(&vals, w.rhs));
                flags = match w.op {
                    BinOpKind::Sub => sub_flags(l, r, w.size),
                    BinOpKind::Add => add_flags(l, r, w.size),
                    BinOpKind::And => logic_flags(l & r, w.size),
                    BinOpKind::Or => logic_flags(l | r, w.size),
                    BinOpKind::Xor => logic_flags(l ^ r, w.size),
                    _ => return BlockOutcome::Unsupported("writeflags kind"),
                };
                nzcv_live = true;
            }
            Op::AluFlags(a) => {
                let (l, r) = (get(&vals, a.lhs), get(&vals, a.rhs));
                flags = match a.op {
                    BinOpKind::Sub => sub_flags(l, r, a.size),
                    BinOpKind::Add => add_flags(l, r, a.size),
                    BinOpKind::And => logic_flags(l & r, a.size),
                    BinOpKind::Or => logic_flags(l | r, a.size),
                    BinOpKind::Xor => logic_flags(l ^ r, a.size),
                    _ => return BlockOutcome::Unsupported("aluflags kind"),
                };
                nzcv_live = true;
            }
            Op::AluFlagsPreserveCarry(a) => {
                let (l, r) = (get(&vals, a.lhs), get(&vals, a.rhs));
                let carry = regs.cf & 1 == 1;
                flags = match a.op {
                    BinOpKind::Sub => sub_flags(l, r, a.size),
                    BinOpKind::Add => add_flags(l, r, a.size),
                    _ => return BlockOutcome::Unsupported("aluflags-preserve-carry kind"),
                };
                flags.c = carry;
                nzcv_live = true;
            }
            Op::LoadCarry(_) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, regs.cf & 1);
                }
            }
            Op::ReadCarryOut(read) => {
                if let Some(d) = stmt.result {
                    let carry = if read.from_sub { !flags.c } else { flags.c };
                    vals.insert(d, u64::from(carry));
                }
            }
            Op::StoreCarry(s) => {
                store_carry(regs, get(&vals, s.value));
                nzcv_live = false;
            }
            Op::LoadRflags(_) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, regs.rflags | 2);
                }
            }
            Op::StoreRflags(s) => {
                store_rflags(regs, get(&vals, s.value));
                nzcv_live = false;
            }
            Op::StoreRflagsFromNzcv(s) => store_rflags_from_flags(
                regs,
                flags,
                s.carry,
                s.pf.map(|r| get(&vals, r)),
                s.af.map(|r| get(&vals, r)),
            ),
            Op::StoreRflagsFromBits(s) => {
                store_rflags_from_bits(
                    regs,
                    s.pf.map(|r| get(&vals, r)),
                    s.af.map(|r| get(&vals, r)),
                    get(&vals, s.zf),
                    get(&vals, s.sf),
                    get(&vals, s.of),
                );
                nzcv_live = false;
            }
            // Fused blocks publish the precise x86 PC before every decoded
            // instruction for exception recovery. The marker has no guest
            // architectural effect in the reference interpreter.
            Op::GuestPc(_) => {}
            Op::Select(s) => {
                if !nzcv_live {
                    flags = flags_from_rflags(regs.rflags);
                }
                if let Some(d) = stmt.result {
                    let selected = if eval_cc(s.cc, flags) {
                        get(&vals, s.true_value)
                    } else {
                        get(&vals, s.false_value)
                    };
                    vals.insert(d, mask(selected, s.size));
                }
            }
            Op::CondJumpRel(j) => {
                if !nzcv_live {
                    flags = flags_from_rflags(regs.rflags);
                }
                return if eval_cc(j.cc, flags) {
                    BlockOutcome::Branch(j.target_guest_pc)
                } else {
                    BlockOutcome::Branch(j.fallthrough_guest_pc)
                };
            }
            Op::JumpRel(j) => return BlockOutcome::Branch(j.target_guest_pc),
            Op::Syscall(_) => return BlockOutcome::Syscall,
            Op::Trap(t) => return BlockOutcome::Trap(t.kind),
            Op::TrapIf(t) => {
                if get(&vals, t.condition) != 0 {
                    return BlockOutcome::Trap(t.kind);
                }
            }
            Op::Return(_) | Op::JumpReg(_) | Op::CallReg(_) => {
                return BlockOutcome::DynamicTransfer;
            }
            _ => return BlockOutcome::Unsupported("op"),
        }
    }
    BlockOutcome::Fallthrough
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_decoder::decode::decode_one;
    use prisma_ir::{
        CmpFlags, Constant, GuestPc, LoadCarry, LoadReg, LoadRflags, Stmt, StoreCarry, StoreReg,
        StoreRflags,
    };

    fn vec_bytes(bytes: &[u8]) -> u128 {
        let mut out = [0u8; 16];
        out[..bytes.len()].copy_from_slice(bytes);
        u128::from_le_bytes(out)
    }

    #[test]
    fn guest_pc_marker_has_no_architectural_effect() {
        let marker = Stmt::new(None, Op::GuestPc(GuestPc { pc: 0x1_4000_1000 }));
        let mut regs = GuestRegs {
            gpr: [0x55aa; 16],
            cf: 1,
            rflags: 0x8c3,
            ..GuestRegs::default()
        };
        let before = regs.clone();

        assert_eq!(
            interpret_block(&[marker], &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr, before.gpr);
        assert_eq!(regs.cf, before.cf);
        assert_eq!(regs.rflags, before.rflags);
    }

    #[test]
    fn decoded_bts_register_sets_the_bit_and_preserves_old_bit_in_carry() {
        let decoded = decode_one(b"\x48\x0F\xAB\xD1", 0).expect("decode bts rcx, rdx");
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rcx as usize] = 0;
        regs.gpr[Gpr::Rdx as usize] = 3;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 8);
        assert_eq!(regs.cf, 0);

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 8);
        assert_eq!(regs.cf, 1);
    }

    #[test]
    fn decoded_punpcklbw_interleaves_the_low_bytes() {
        let decoded = decode_one(b"\x66\x0F\x60\xC0", 0).expect("decode punpcklbw xmm0, xmm0");
        let mut regs = GuestRegs::default();
        regs.xmm[0] = vec_bytes(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(
            regs.xmm[0],
            vec_bytes(&[0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7])
        );
    }

    #[test]
    fn decoded_pshuflw_broadcasts_low_halfword_and_preserves_high_half() {
        let decoded = decode_one(b"\xF2\x0F\x70\xC0\x00", 0).expect("decode pshuflw xmm0, xmm0, 0");
        let mut regs = GuestRegs::default();
        regs.xmm[0] = vec_bytes(&[
            0x00, 0x01, 0x10, 0x11, 0x20, 0x21, 0x30, 0x31, 0x40, 0x41, 0x50, 0x51, 0x60,
            0x61, 0x70, 0x71,
        ]);
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(
            regs.xmm[0],
            vec_bytes(&[
                0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x40, 0x41, 0x50, 0x51,
                0x60, 0x61, 0x70, 0x71,
            ])
        );
    }

    #[test]
    fn decoded_pcmpeqb_then_pmovmskb_produces_the_expected_mask() {
        let compare = decode_one(b"\x66\x0F\x74\xC1", 0).expect("decode pcmpeqb xmm0, xmm1");
        let mask = decode_one(b"\x66\x0F\xD7\xF0", 0).expect("decode pmovmskb esi, xmm0");
        let mut regs = GuestRegs::default();
        regs.xmm[0] = vec_bytes(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        regs.xmm[1] = vec_bytes(&[
            0, 0xff, 2, 0xff, 4, 0xff, 6, 0xff, 8, 0xff, 10, 0xff, 12, 0xff, 14, 0xff,
        ]);
        assert_eq!(
            interpret_block(&compare.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(
            interpret_block(&mask.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rsi as usize], 0x5555);
    }

    #[test]
    fn cmp_then_branch_takes_when_nonzero() {
        // ecx loaded, cmp ecx,0, jnz: with ecx=4 -> taken.
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rcx,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::CmpFlags(CmpFlags {
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::CondJumpRel(prisma_ir::CondJumpRel {
                    cc: CondCode::Ne,
                    target_guest_pc: 0x100,
                    fallthrough_guest_pc: 0x200,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rcx as usize] = 4;
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x100)
        );
        regs.gpr[Gpr::Rcx as usize] = 0;
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x200)
        );
    }

    #[test]
    fn wide_unsigned_div_uses_rdx_rax_dividend() {
        let decoded = decode_one(b"\x48\xF7\xF1", 0).unwrap();
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 5;
        regs.gpr[Gpr::Rdx as usize] = 1;
        regs.gpr[Gpr::Rcx as usize] = 3;

        let out = interpret_block(&decoded.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.gpr[Gpr::Rax as usize], 6_148_914_691_236_517_207);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0);
    }

    #[test]
    fn wide_signed_div_uses_signed_rdx_rax_dividend() {
        let decoded = decode_one(b"\x48\xF7\xF9", 0).unwrap();
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = (-100i64) as u64;
        regs.gpr[Gpr::Rdx as usize] = (-1i64) as u64;
        regs.gpr[Gpr::Rcx as usize] = 7;

        let out = interpret_block(&decoded.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.gpr[Gpr::Rax as usize], (-14i64) as u64);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], (-2i64) as u64);
    }

    #[test]
    fn decoded_pcmpistri_equal_any_bytes_writes_ecx_index() {
        let decoded = decode_one(b"\x66\x0F\x3A\x63\xC1\x00", 0).unwrap();
        let mut regs = GuestRegs::default();
        regs.xmm[0] = vec_bytes(b"zbc");
        regs.xmm[1] = vec_bytes(b"abc");

        let out = interpret_block(&decoded.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 1);
        assert_eq!(regs.cf, 1);
        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x004, 0);
        assert_eq!(regs.rflags & 0x010, 0);
        assert_eq!(regs.rflags & 0x040, 0x040);
        assert_eq!(regs.rflags & 0x080, 0x080);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_pcmpestrm_equal_each_bytes_writes_xmm0_mask() {
        let decoded = decode_one(b"\x66\x0F\x3A\x60\xC1\x08", 0).unwrap();
        let mut regs = GuestRegs::default();
        regs.xmm[0] = vec_bytes(b"abc");
        regs.xmm[1] = vec_bytes(b"abd");
        regs.gpr[Gpr::Rax as usize] = 3;
        regs.gpr[Gpr::Rdx as usize] = 3;

        let out = interpret_block(&decoded.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.xmm[0], 0b0011);
        assert_eq!(regs.cf, 1);
        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x004, 0);
        assert_eq!(regs.rflags & 0x010, 0);
        assert_eq!(regs.rflags & 0x040, 0x040);
        assert_eq!(regs.rflags & 0x080, 0x080);
        assert_eq!(regs.rflags & 0x800, 0x800);
    }

    #[test]
    fn decoded_pclmulqdq_multiplies_selected_qword_lanes() {
        let decoded = decode_one(b"\x66\x0F\x3A\x44\xC1\x11", 0).unwrap();
        let mut regs = GuestRegs::default();
        regs.xmm[0] = (u128::from(0b10u64) << 64) | 0xAA;
        regs.xmm[1] = (u128::from(0b11u64) << 64) | 0x55;

        let out = interpret_block(&decoded.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.xmm[0], 0b110);
    }

    #[test]
    fn decoded_f16c_ph2ps_and_ps2ph_round_trip_exact_lanes() {
        let ph2ps = decode_one(b"\xC4\xE2\x79\x13\xC1", 0).unwrap();
        let mut regs = GuestRegs::default();
        let packed_halves = 0x7c00_0000_c000_3c00u64;
        regs.xmm[1] = u128::from(packed_halves);

        let out = interpret_block(&ph2ps.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        let expected_ps =
            (u128::from(0x7f80_0000_0000_0000u64) << 64) | u128::from(0xc000_0000_3f80_0000u64);
        assert_eq!(regs.xmm[0], expected_ps);

        let ps2ph = decode_one(b"\xC4\xE3\x79\x1D\xC1\x00", 0).unwrap();
        regs.xmm[0] = expected_ps;
        regs.xmm[1] = 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffffu128;

        let out = interpret_block(&ps2ph.stmts, &mut regs);

        assert_eq!(out, BlockOutcome::Fallthrough);
        assert_eq!(regs.xmm[1], u128::from(packed_halves));
    }

    #[test]
    fn branch_without_local_flags_reads_persistent_rflags() {
        let stmts = vec![Stmt::new(
            None,
            Op::CondJumpRel(prisma_ir::CondJumpRel {
                cc: CondCode::Eq,
                target_guest_pc: 0x100,
                fallthrough_guest_pc: 0x200,
            }),
        )];
        let mut regs = GuestRegs {
            rflags: 2 | (1 << 6),
            ..GuestRegs::default()
        };
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x100)
        );

        regs.rflags = 2;
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x200)
        );
    }

    #[test]
    fn store_reg_i32_zero_extends() {
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rcx,
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rcx as usize] = 0xFFFF_FFFF_0000_0000;
        let _ = interpret_block(&stmts, &mut regs);
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 5, "32-bit write zero-extends");
    }

    #[test]
    fn rflags_defaults_with_reserved_bit_set() {
        let stmts = vec![
            Stmt::new(Some(0), Op::LoadRflags(LoadRflags)),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        let _ = interpret_block(&stmts, &mut regs);
        assert_eq!(regs.gpr[Gpr::Rax as usize], 2);
    }

    #[test]
    fn store_rflags_forces_reserved_bit_and_syncs_carry() {
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreRflags(StoreRflags { value: 0 })),
            Stmt::new(Some(1), Op::LoadCarry(LoadCarry)),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreRflags(StoreRflags { value: 2 })),
            Stmt::new(Some(3), Op::LoadCarry(LoadCarry)),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rcx,
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        let _ = interpret_block(&stmts, &mut regs);
        assert_eq!(regs.rflags, 3);
        assert_eq!(regs.cf, 1);
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0);
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 1);
    }

    #[test]
    fn store_carry_masks_value_and_updates_rflags_bit_zero() {
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0xAA,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreRflags(StoreRflags { value: 0 })),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreCarry(StoreCarry { value: 1 })),
            Stmt::new(Some(2), Op::LoadRflags(LoadRflags)),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rdx,
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        let _ = interpret_block(&stmts, &mut regs);
        assert_eq!(regs.cf, 1);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0xAB);
    }

    #[test]
    fn store_rflags_from_nzcv_preserves_carry_and_updates_status_bits() {
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 0x8C3,
            ..GuestRegs::default()
        };

        store_rflags_from_flags(
            &mut regs,
            Flags {
                n: false,
                z: true,
                c: false,
                v: false,
            },
            RflagsCarryMode::Preserve,
            None,
            None,
        );

        assert_eq!(regs.cf, 1);
        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x002, 0x002);
        assert_eq!(regs.rflags & 0x040, 0x040);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn store_rflags_from_bits_preserves_carry_and_updates_status_bits() {
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 0x8C3,
            ..GuestRegs::default()
        };

        store_rflags_from_bits(&mut regs, Some(1), Some(1), 1, 0, 1);

        assert_eq!(regs.cf, 1);
        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x002, 0x002);
        assert_eq!(regs.rflags & 0x004, 0x004);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0x040);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0x800);
    }

    #[test]
    fn decoded_adc_publishes_parity_and_aux_flags() {
        let decoded = decode_one(&[0x48, 0x83, 0xD0, 0x01], 0).unwrap(); // adc rax, 1
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 3,
            ..GuestRegs::default()
        };
        regs.gpr[Gpr::Rax as usize] = 0x0f;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x11);
        assert_eq!(regs.rflags & 0x001, 0);
        assert_eq!(regs.rflags & 0x004, 0x004);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_sbb_publishes_parity_aux_sign_and_borrow_flags() {
        let decoded = decode_one(&[0x48, 0x83, 0xD8, 0x00], 0).unwrap(); // sbb rax, 0
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 3,
            ..GuestRegs::default()
        };

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.gpr[Gpr::Rax as usize], u64::MAX);
        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x004, 0x004);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0x080);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_add_publishes_parity_and_aux_flags() {
        let decoded = decode_one(&[0x48, 0x83, 0xC0, 0x01], 0).unwrap(); // add rax, 1
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x0f;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x10);
        assert_eq!(regs.rflags & 0x001, 0);
        assert_eq!(regs.rflags & 0x004, 0);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_sub_publishes_parity_and_aux_flags() {
        let decoded = decode_one(&[0x48, 0x83, 0xE8, 0x01], 0).unwrap(); // sub rax, 1
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x10;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x0f);
        assert_eq!(regs.rflags & 0x001, 0);
        assert_eq!(regs.rflags & 0x004, 0x004);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_test_publishes_parity_and_preserves_undefined_aux_flag() {
        let decoded = decode_one(&[0xA8, 0x03], 0).unwrap(); // test al, 3
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 0x13,
            ..GuestRegs::default()
        };
        regs.gpr[Gpr::Rax as usize] = 0x03;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.rflags & 0x001, 0);
        assert_eq!(regs.rflags & 0x004, 0x004);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0);
        assert_eq!(regs.rflags & 0x800, 0);
    }

    #[test]
    fn decoded_lahf_loads_flags_into_ah() {
        let decoded = decode_one(&[0x9F], 0).unwrap(); // lahf
        let mut regs = GuestRegs {
            rflags: 2 | 1 | 4 | 0x10 | 0x40 | 0x80 | 0x800,
            ..GuestRegs::default()
        };
        regs.gpr[Gpr::Rax as usize] = 0x1122_3344_5566_7788;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x1122_3344_5566_D788);
        assert_eq!(regs.rflags & 0x800, 0x800);
    }

    #[test]
    fn decoded_sahf_stores_ah_into_flags_and_preserves_overflow() {
        let decoded = decode_one(&[0x9E], 0).unwrap(); // sahf
        let mut regs = GuestRegs {
            rflags: 2 | 4 | 0x40 | 0x800,
            ..GuestRegs::default()
        };
        regs.gpr[Gpr::Rax as usize] = 0x9100;

        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );

        assert_eq!(regs.rflags & 0x001, 0x001);
        assert_eq!(regs.rflags & 0x002, 0x002);
        assert_eq!(regs.rflags & 0x004, 0);
        assert_eq!(regs.rflags & 0x010, 0x010);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0x080);
        assert_eq!(regs.rflags & 0x800, 0x800);
        assert_eq!(regs.cf, 1);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn decoded_arithmetic_flag_edge_cases_publish_expected_rflags() {
        const CF: u64 = 1 << 0;
        const PF: u64 = 1 << 2;
        const AF: u64 = 1 << 4;
        const ZF: u64 = 1 << 6;
        const SF: u64 = 1 << 7;
        const OF: u64 = 1 << 11;

        for (name, bytes, rax, cf, initial_rflags, expected_rax, set, clear) in [
            (
                "add_wraps_to_zero",
                &[0x48, 0x83, 0xC0, 0x01][..],
                u64::MAX,
                0,
                2,
                0,
                CF | PF | AF | ZF,
                SF | OF,
            ),
            (
                "add_signed_overflow",
                &[0x48, 0x83, 0xC0, 0x01][..],
                i64::MAX as u64,
                0,
                2,
                i64::MIN as u64,
                PF | AF | SF | OF,
                CF | ZF,
            ),
            (
                "sub_borrows_to_negative",
                &[0x48, 0x83, 0xE8, 0x01][..],
                0,
                0,
                2,
                u64::MAX,
                CF | PF | AF | SF,
                ZF | OF,
            ),
            (
                "sub_signed_overflow",
                &[0x48, 0x83, 0xE8, 0x01][..],
                i64::MIN as u64,
                0,
                2,
                i64::MAX as u64,
                PF | AF | OF,
                CF | ZF | SF,
            ),
            (
                "adc_uses_carry_in",
                &[0x48, 0x83, 0xD0, 0x00][..],
                u64::MAX,
                1,
                3,
                0,
                CF | PF | AF | ZF,
                SF | OF,
            ),
            (
                "adc_signed_overflow_with_carry_in",
                &[0x48, 0x83, 0xD0, 0x00][..],
                i64::MAX as u64,
                1,
                2 | CF | ZF,
                i64::MIN as u64,
                PF | AF | SF | OF,
                CF | ZF,
            ),
            (
                "adc_unsigned_carry_without_signed_overflow",
                &[0x48, 0x83, 0xD0, 0x01][..],
                u64::MAX,
                0,
                2 | SF | OF,
                0,
                CF | PF | AF | ZF,
                SF | OF,
            ),
            (
                "sbb_uses_borrow_in",
                &[0x48, 0x83, 0xD8, 0x00][..],
                0,
                1,
                3,
                u64::MAX,
                CF | PF | AF | SF,
                ZF | OF,
            ),
            (
                "sbb_signed_overflow_with_borrow_in",
                &[0x48, 0x83, 0xD8, 0x00][..],
                i64::MIN as u64,
                1,
                2 | CF | ZF | SF,
                i64::MAX as u64,
                PF | AF | OF,
                CF | ZF | SF,
            ),
            (
                "sbb_no_borrow_clears_stale_flags",
                &[0x48, 0x83, 0xD8, 0x00][..],
                0x11,
                1,
                2 | CF | PF | AF | ZF | SF | OF,
                0x10,
                0,
                CF | PF | AF | ZF | SF | OF,
            ),
            (
                "sbb_equal_with_borrow_to_zero",
                &[0x48, 0x83, 0xD8, 0x00][..],
                1,
                1,
                2 | CF | AF | SF | OF,
                0,
                PF | ZF,
                CF | AF | SF | OF,
            ),
            (
                "test_zero_clears_carry_overflow_and_preserves_af",
                &[0xA8, 0x00][..],
                0,
                1,
                2 | CF | AF | OF,
                0,
                PF | AF | ZF,
                CF | SF | OF,
            ),
        ] {
            let decoded = decode_one(bytes, 0).unwrap();
            let mut regs = GuestRegs {
                cf,
                rflags: initial_rflags,
                ..GuestRegs::default()
            };
            regs.gpr[Gpr::Rax as usize] = rax;

            assert_eq!(
                interpret_block(&decoded.stmts, &mut regs),
                BlockOutcome::Fallthrough,
                "{name}"
            );

            assert_eq!(regs.gpr[Gpr::Rax as usize], expected_rax, "{name}: rax");
            assert_eq!(regs.rflags & set, set, "{name}: expected flags not set");
            assert_eq!(regs.rflags & clear, 0, "{name}: expected flags not clear");
            assert_eq!(regs.rflags & 2, 2, "{name}: reserved bit 1");
            assert_eq!(regs.cf, regs.rflags & CF, "{name}: carry mirror");
        }
    }

    #[test]
    fn decoded_group2_shift_rotate_ops_execute() {
        for (name, bytes, initial_rax, expected_rax) in [
            (
                "rol_rax_4",
                &[0x48, 0xC1, 0xC0, 0x04][..],
                0x8000_0000_0000_0001,
                0x18,
            ),
            (
                "ror_al_1",
                &[0xD0, 0xC8][..],
                0x1122_3344_5566_7701,
                0x1122_3344_5566_7780,
            ),
            (
                "sar_eax_4",
                &[0xC1, 0xF8, 0x04][..],
                0x1234_5678_8000_0000,
                0x0000_0000_F800_0000,
            ),
        ] {
            let decoded = decode_one(bytes, 0).unwrap();
            let mut regs = GuestRegs::default();
            regs.gpr[Gpr::Rax as usize] = initial_rax;

            assert_eq!(
                interpret_block(&decoded.stmts, &mut regs),
                BlockOutcome::Fallthrough,
                "{name}"
            );
            assert_eq!(regs.gpr[Gpr::Rax as usize], expected_rax, "{name}");
        }
    }

    #[test]
    fn decoded_group3_narrow_mul_imul_ops_execute() {
        let decoded = decode_one(&[0xF6, 0xE1], 0).unwrap(); // mul cl
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x1122_3344_5566_00ff;
        regs.gpr[Gpr::Rcx as usize] = 2;
        regs.gpr[Gpr::Rdx as usize] = 0x8877_6655_4433_2211;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x1122_3344_5566_01fe);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0x8877_6655_4433_2211);

        let decoded = decode_one(&[0xF7, 0xE9], 0).unwrap(); // imul ecx
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0xffff_ffff;
        regs.gpr[Gpr::Rcx as usize] = 2;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0xffff_fffe);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0xffff_ffff);
    }

    #[test]
    fn decoded_group3_narrow_div_idiv_ops_execute() {
        let decoded = decode_one(&[0xF6, 0xF1], 0).unwrap(); // div cl
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x1122_3344_5566_0123;
        regs.gpr[Gpr::Rcx as usize] = 0x12;
        regs.gpr[Gpr::Rdx as usize] = 0x8877_6655_4433_2211;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x1122_3344_5566_0310);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0x8877_6655_4433_2211);

        let decoded = decode_one(&[0xF6, 0xF9], 0).unwrap(); // idiv cl
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x1122_3344_5566_ffe2;
        regs.gpr[Gpr::Rcx as usize] = 7;
        regs.gpr[Gpr::Rdx as usize] = 0x8877_6655_4433_2211;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x1122_3344_5566_fefc);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0x8877_6655_4433_2211);

        let decoded = decode_one(&[0x66, 0xF7, 0xF1], 0).unwrap(); // div cx
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0x1122_3344_5566_0000;
        regs.gpr[Gpr::Rcx as usize] = 0x100;
        regs.gpr[Gpr::Rdx as usize] = 0x8877_6655_4433_0001;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0x1122_3344_5566_0100);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0x8877_6655_4433_0000);

        let decoded = decode_one(&[0xF7, 0xF9], 0).unwrap(); // idiv ecx
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rax as usize] = 0xffff_ff9c;
        regs.gpr[Gpr::Rcx as usize] = 7;
        regs.gpr[Gpr::Rdx as usize] = 0xffff_ffff;
        assert_eq!(
            interpret_block(&decoded.stmts, &mut regs),
            BlockOutcome::Fallthrough
        );
        assert_eq!(regs.gpr[Gpr::Rax as usize], 0xffff_fff2);
        assert_eq!(regs.gpr[Gpr::Rdx as usize], 0xffff_fffe);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn decoded_vex_bmi1_ops_execute_and_publish_core_flags() {
        const CF: u64 = 1 << 0;
        const PF: u64 = 1 << 2;
        const AF: u64 = 1 << 4;
        const ZF: u64 = 1 << 6;
        const SF: u64 = 1 << 7;
        const OF: u64 = 1 << 11;

        for (name, bytes, rcx, rdx, expected_rax, expected_cf, expected_set, expected_clear) in [
            (
                "andn_rax_rcx_rdx",
                &[0xC4, 0xE2, 0xF0, 0xF2, 0xC2][..],
                0xff00,
                0x0ff0,
                0x00f0,
                0,
                PF | AF,
                CF | ZF | SF | OF,
            ),
            (
                "blsr_rax_rdx",
                &[0xC4, 0xE2, 0xF8, 0xF3, 0xCA][..],
                0,
                0b101_1000,
                0b101_0000,
                0,
                PF | AF,
                CF | ZF | SF | OF,
            ),
            (
                "blsmsk_rax_rdx",
                &[0xC4, 0xE2, 0xF8, 0xF3, 0xD2][..],
                0,
                0b101_1000,
                0b000_1111,
                0,
                PF | AF,
                CF | ZF | SF | OF,
            ),
            (
                "blsi_rax_rdx",
                &[0xC4, 0xE2, 0xF8, 0xF3, 0xDA][..],
                0,
                0b101_1000,
                0b000_1000,
                1,
                CF | PF | AF,
                ZF | SF | OF,
            ),
            (
                "blsr_zero_sets_cf_and_zf",
                &[0xC4, 0xE2, 0xF8, 0xF3, 0xCA][..],
                0,
                0,
                0,
                1,
                CF | PF | AF | ZF,
                SF | OF,
            ),
            (
                "bextr_rax_rdx_rcx",
                &[0xC4, 0xE2, 0xF0, 0xF7, 0xC2][..],
                0x0808,
                0xFEDC_BA98_7654_3210,
                0x32,
                0,
                PF | AF | SF,
                CF | ZF | OF,
            ),
            (
                "bextr_start_past_width_sets_zero",
                &[0xC4, 0xE2, 0xF0, 0xF7, 0xC2][..],
                64,
                0xFEDC_BA98_7654_3210,
                0,
                0,
                PF | AF | ZF | SF,
                CF | OF,
            ),
        ] {
            let decoded = decode_one(bytes, 0).unwrap();
            let mut regs = GuestRegs {
                cf: 1,
                rflags: 2 | CF | PF | AF | ZF | SF | OF,
                ..GuestRegs::default()
            };
            regs.gpr[Gpr::Rcx as usize] = rcx;
            regs.gpr[Gpr::Rdx as usize] = rdx;

            assert_eq!(
                interpret_block(&decoded.stmts, &mut regs),
                BlockOutcome::Fallthrough,
                "{name}"
            );
            assert_eq!(regs.gpr[Gpr::Rax as usize], expected_rax, "{name}: rax");
            assert_eq!(regs.cf, expected_cf, "{name}: cf mirror");
            assert_eq!(regs.rflags & CF, expected_cf, "{name}: rflags cf");
            assert_eq!(
                regs.rflags & expected_set,
                expected_set,
                "{name}: expected flags not set"
            );
            assert_eq!(
                regs.rflags & expected_clear,
                0,
                "{name}: expected flags not clear"
            );
            assert_eq!(regs.rflags & 2, 2, "{name}: reserved bit 1");
        }
    }

    #[test]
    fn decoded_rcl_rcr_cl_execute_through_carry_and_preserve_count_zero() {
        for (name, bytes, rax, rcx, cf, expected_rax, expected_cf) in [
            (
                "rcl_rax_cl_count_one",
                &[0x48, 0xD3, 0xD0][..],
                0x8000_0000_0000_0001,
                1,
                1,
                0x0000_0000_0000_0003,
                1,
            ),
            (
                "rcr_rax_cl_count_one",
                &[0x48, 0xD3, 0xD8][..],
                0x8000_0000_0000_0001,
                1,
                1,
                0xC000_0000_0000_0000,
                1,
            ),
            (
                "rcl_rax_cl_count_two",
                &[0x48, 0xD3, 0xD0][..],
                0x8000_0000_0000_0000,
                2,
                1,
                0x0000_0000_0000_0003,
                0,
            ),
            (
                "rcl_rax_cl_count_zero_preserves_destination_and_cf",
                &[0x48, 0xD3, 0xD0][..],
                0x1234_5678_9ABC_DEF0,
                0,
                1,
                0x1234_5678_9ABC_DEF0,
                1,
            ),
        ] {
            let decoded = decode_one(bytes, 0).unwrap();
            let mut regs = GuestRegs {
                cf,
                rflags: 2 | cf,
                ..GuestRegs::default()
            };
            regs.gpr[Gpr::Rax as usize] = rax;
            regs.gpr[Gpr::Rcx as usize] = rcx;

            assert_eq!(
                interpret_block(&decoded.stmts, &mut regs),
                BlockOutcome::Fallthrough,
                "{name}"
            );
            assert_eq!(regs.gpr[Gpr::Rax as usize], expected_rax, "{name}: rax");
            assert_eq!(regs.cf, expected_cf, "{name}: cf");
            assert_eq!(regs.rflags & 1, expected_cf, "{name}: rflags cf");
            assert_eq!(regs.rflags & 2, 2, "{name}: reserved bit 1");
        }
    }

    #[test]
    fn store_rflags_from_nzcv_inverts_arm_carry_for_sub_borrow() {
        let mut regs = GuestRegs {
            cf: 1,
            rflags: 0x8C3,
            ..GuestRegs::default()
        };

        store_rflags_from_flags(
            &mut regs,
            Flags {
                n: true,
                z: false,
                c: true,
                v: true,
            },
            RflagsCarryMode::InvertArmCarry,
            None,
            None,
        );

        assert_eq!(regs.cf, 0);
        assert_eq!(regs.rflags & 0x001, 0);
        assert_eq!(regs.rflags & 0x002, 0x002);
        assert_eq!(regs.rflags & 0x040, 0);
        assert_eq!(regs.rflags & 0x080, 0x080);
        assert_eq!(regs.rflags & 0x800, 0x800);
    }
}
