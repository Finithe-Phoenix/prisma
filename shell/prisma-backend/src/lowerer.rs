//! Instruction lowering facade for the Rust backend.

use std::collections::{HashMap, HashSet};

use crate::{
    abi,
    assembler::{Arm64Assembler, Label},
};
use prisma_ir::{BinOpKind, Function, Gpr, Op, OpSize, Ref, RflagsCarryMode, Stmt};
use thiserror::Error;

/// First temporary register used by the migration lowerer.
const FIRST_VALUE_REG: u8 = 9;

/// Number of temporary integer registers managed by this migration slice.
const VALUE_REG_COUNT: u8 = 8;

/// Volatile scratch used to publish exact x64 instruction boundaries. It is
/// outside the x9..x16 value pool and never aliases the ARM64EC TEB in x18.
const GUEST_PC_SCRATCH_REG: u8 = 17;

/// Scratch registers used for transient flag-alignment lowering.
const FLAG_ALIGN_LHS_REG: u8 = 17;
// Windows ARM64 and ARM64EC reserve x18 for the current TEB. JIT blocks must
// preserve it across their entire lifetime, including helper calls and return
// dispatch, so use the otherwise available callee-saved x28 instead.
const FLAG_ALIGN_RHS_REG: u8 = 28;
const FLAG_ALIGN_SHIFT_REG: u8 = 19;

/// Scratch register used for quotient materialization in modulo lowering.
const MOD_QUOTIENT_REG: u8 = 20;
/// Scratch register used for dynamic RSP reads/writes during stack adjustments.
const RSP_ADJUST_TMP_REG: u8 = 21;
/// Scratch register used for large RSP immediate materialization.
const RSP_ADJUST_IMM_REG: u8 = 22;
/// Scratch register used for flag-writing ALU side-effect operations.
const ALU_FLAGS_TMP_REG: u8 = 23;
/// Scratch aliases used while rewriting NZCV after flag-setting operations.
const NZCV_TMP_REG: u8 = MOD_QUOTIENT_REG;
const NZCV_MASK_REG: u8 = RSP_ADJUST_TMP_REG;
const NZCV_CARRY_REG: u8 = RSP_ADJUST_IMM_REG;
const WIDE_REM_REG: u8 = FLAG_ALIGN_LHS_REG;
const WIDE_DIVISOR_REG: u8 = FLAG_ALIGN_RHS_REG;
const WIDE_LOW_REG: u8 = FLAG_ALIGN_SHIFT_REG;
const WIDE_BIT_REG: u8 = MOD_QUOTIENT_REG;
const WIDE_ONE_REG: u8 = RSP_ADJUST_TMP_REG;
const WIDE_MASK_REG: u8 = RSP_ADJUST_IMM_REG;
const WIDE_TMP_REG: u8 = ALU_FLAGS_TMP_REG;
const WIDE_QUOT_SIGN_REG: u8 = MEM_ADDR_SCRATCH;
const WIDE_REM_SIGN_REG: u8 = CAS_STATUS_REG;
const WIDE_SHIFT_REG: u8 = 26;
// Atomic read-modify-write ops keep their input live across the exclusive loop.
// Preserve it outside the cyclic x9..x16 SSA pool so LDAXR cannot overwrite it
// when result/input slots alias.
const ATOMIC_RMW_SOURCE_REG: u8 = WIDE_SHIFT_REG;
const ATOMIC_CMPXCHG_EXPECTED_REG: u8 = WIDE_SHIFT_REG;
const ATOMIC_CMPXCHG_NEW_REG: u8 = ALU_FLAGS_TMP_REG;
const PCMP_HELPER_TARGET_REG: u8 = 16;

/// Scalar register pairs used for the current XMM lowering frontier. The Rust
/// backend does not have NEON/vector-register emission yet, so 128-bit vector
/// values are carried as two callee-saved host `u64` registers while `PCMPxSTRx`
/// lowering calls semantic helpers.
const VEC_REG_PAIRS: [(u8, u8); 3] = [
    (FLAG_ALIGN_SHIFT_REG, MOD_QUOTIENT_REG),
    (RSP_ADJUST_TMP_REG, RSP_ADJUST_IMM_REG),
    (ALU_FLAGS_TMP_REG, WIDE_SHIFT_REG),
];

const PCMP_LEN_LHS_EXPLICIT: u64 = 1;
const PCMP_LEN_RHS_EXPLICIT: u64 = 2;

type PcmpStrHelper = extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64;
type VecUnpackHelper = extern "C" fn(u64, u64, u64, u64, u64, u64, u64) -> u64;
type VecShuffleH4Helper = extern "C" fn(u64, u64, u64, u64, u64) -> u64;
type VecCmpHelper = extern "C" fn(u64, u64, u64, u64, u64, u64, u64) -> u64;
type VecMaskMsbHelper = extern "C" fn(u64, u64) -> u64;

extern "C" fn vec_mask_msb_helper(src_lo: u64, src_hi: u64) -> u64 {
    let src = (u128::from(src_hi) << 64) | u128::from(src_lo);
    let mut result = 0u64;
    for byte in 0..16 {
        result |= u64::try_from((src >> (byte * 8 + 7)) & 1).expect("single mask bit") << byte;
    }
    result
}

extern "C" fn vec_cmp_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lane: u64,
    kind: u64,
    output_high: u64,
) -> u64 {
    let lhs = (u128::from(lhs_hi) << 64) | u128::from(lhs_lo);
    let rhs = (u128::from(rhs_hi) << 64) | u128::from(rhs_lo);
    let lane_bits = match lane {
        x if x == u64::from(prisma_ir::VecLane::B16 as u8) => 8,
        x if x == u64::from(prisma_ir::VecLane::H8 as u8) => 16,
        x if x == u64::from(prisma_ir::VecLane::S4 as u8) => 32,
        x if x == u64::from(prisma_ir::VecLane::D2 as u8) => 64,
        _ => return 0,
    };
    let lane_mask = (1u128 << lane_bits) - 1;
    let sign_bit = 1u128 << (lane_bits - 1);
    let mut result = 0u128;
    for lane_index in 0..(128 / lane_bits) {
        let left = (lhs >> (lane_index * lane_bits)) & lane_mask;
        let right = (rhs >> (lane_index * lane_bits)) & lane_mask;
        let matches = if kind == u64::from(prisma_ir::VecCmpKind::Eq as u8) {
            left == right
        } else {
            (left ^ sign_bit) > (right ^ sign_bit)
        };
        if matches {
            result |= lane_mask << (lane_index * lane_bits);
        }
    }
    if output_high != 0 {
        u64::try_from(result >> 64).expect("upper compare half is 64 bits")
    } else {
        u64::try_from(result & u128::from(u64::MAX)).expect("lower compare half is 64 bits")
    }
}

extern "C" fn vec_shuffle_h4_helper(
    src_lo: u64,
    src_hi: u64,
    control: u64,
    is_high: u64,
    output_high: u64,
) -> u64 {
    let src = (u128::from(src_hi) << 64) | u128::from(src_lo);
    let first = if is_high != 0 { 4 } else { 0 };
    let mut result = src;
    for output_lane in 0..4 {
        let source_lane = first
            + usize::try_from((control >> (output_lane * 2)) & 0x03)
                .expect("shuffle selector is two bits");
        let target_lane = first + output_lane;
        let value = (src >> (source_lane * 16)) & 0xffff;
        result &= !(0xffffu128 << (target_lane * 16));
        result |= value << (target_lane * 16);
    }
    if output_high != 0 {
        u64::try_from(result >> 64).expect("upper shuffle half is 64 bits")
    } else {
        u64::try_from(result & u128::from(u64::MAX)).expect("lower shuffle half is 64 bits")
    }
}

extern "C" fn vec_unpack_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lane: u64,
    is_high: u64,
    output_high: u64,
) -> u64 {
    let lhs = (u128::from(lhs_hi) << 64) | u128::from(lhs_lo);
    let rhs = (u128::from(rhs_hi) << 64) | u128::from(rhs_lo);
    let lane_bits = match lane {
        x if x == u64::from(prisma_ir::VecLane::B16 as u8) => 8,
        x if x == u64::from(prisma_ir::VecLane::H8 as u8) => 16,
        x if x == u64::from(prisma_ir::VecLane::S4 as u8) => 32,
        x if x == u64::from(prisma_ir::VecLane::D2 as u8) => 64,
        _ => return 0,
    };
    let lanes = 128 / lane_bits;
    let first = if is_high != 0 { lanes / 2 } else { 0 };
    let mask = (1u128 << lane_bits) - 1;
    let mut result = 0u128;
    for output_pair in 0..(lanes / 2) {
        let source_lane = first + output_pair;
        result |= ((lhs >> (source_lane * lane_bits)) & mask) << ((output_pair * 2) * lane_bits);
        result |=
            ((rhs >> (source_lane * lane_bits)) & mask) << ((output_pair * 2 + 1) * lane_bits);
    }
    if output_high != 0 {
        u64::try_from(result >> 64).expect("upper unpack half is 64 bits")
    } else {
        u64::try_from(result & u128::from(u64::MAX)).expect("lower unpack half is 64 bits")
    }
}

extern "C" fn fp32_rflags_helper(lhs: u64, rhs: u64) -> u64 {
    fp_rflags(
        f64::from(f32::from_bits(low32(lhs))),
        f64::from(f32::from_bits(low32(rhs))),
    )
}

#[cfg(test)]
extern "C" fn fp64_rflags_helper(lhs: u64, rhs: u64) -> u64 {
    fp_rflags(f64::from_bits(lhs), f64::from_bits(rhs))
}

#[cfg(test)]
extern "C" fn i64_to_f64_helper(value: u64) -> u64 {
    (value as i64 as f64).to_bits()
}

#[cfg(test)]
extern "C" fn i32_to_f64_helper(value: u64) -> u64 {
    (i64::from(value as u32 as i32) as f64).to_bits()
}

#[cfg(test)]
extern "C" fn f64_add_helper(lhs: u64, rhs: u64) -> u64 {
    (f64::from_bits(lhs) + f64::from_bits(rhs)).to_bits()
}

#[cfg(test)]
extern "C" fn f64_sub_helper(lhs: u64, rhs: u64) -> u64 {
    (f64::from_bits(lhs) - f64::from_bits(rhs)).to_bits()
}

#[cfg(test)]
extern "C" fn f64_mul_helper(lhs: u64, rhs: u64) -> u64 {
    (f64::from_bits(lhs) * f64::from_bits(rhs)).to_bits()
}

#[cfg(test)]
extern "C" fn f64_div_helper(lhs: u64, rhs: u64) -> u64 {
    (f64::from_bits(lhs) / f64::from_bits(rhs)).to_bits()
}

#[cfg(test)]
extern "C" fn f64_to_i32_trunc_helper(value: u64) -> u64 {
    let value = f64::from_bits(value);
    if !value.is_finite() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        u64::from(0x8000_0000_u32)
    } else {
        u64::from(value.trunc() as i32 as u32)
    }
}

#[cfg(test)]
extern "C" fn f64_to_i64_trunc_helper(value: u64) -> u64 {
    let value = f64::from_bits(value);
    if !value.is_finite()
        || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
    {
        i64::MIN as u64
    } else {
        value.trunc() as i64 as u64
    }
}

#[allow(clippy::float_cmp)] // IEEE equality is the exact UCOMI architectural rule.
fn fp_rflags(lhs: f64, rhs: f64) -> u64 {
    if lhs.is_nan() || rhs.is_nan() {
        RFLAGS_ZF_BIT | RFLAGS_PF_BIT | RFLAGS_CF_BIT
    } else if lhs < rhs {
        RFLAGS_CF_BIT
    } else if lhs == rhs {
        RFLAGS_ZF_BIT
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy)]
struct BackendPcmpStrEval {
    intres: u16,
    max_lanes: usize,
    lane_bytes: usize,
    lhs_len: usize,
    rhs_len: usize,
}

// Intentional narrowing for the extern "C" helper ABI and packed-lane
// extraction — the truncation is the semantic.
#[allow(clippy::cast_possible_truncation)]
const fn low8(x: u64) -> u8 {
    x as u8
}

#[allow(clippy::cast_possible_truncation)]
const fn low32(x: u64) -> u32 {
    x as u32
}

#[allow(clippy::cast_possible_truncation)]
const fn low64(x: u128) -> u64 {
    x as u64
}

fn pcmp_lane_unsigned(bytes: &[u8; 16], lane: usize, lane_bytes: usize) -> u16 {
    if lane_bytes == 1 {
        u16::from(bytes[lane])
    } else {
        u16::from_le_bytes([bytes[lane * 2], bytes[lane * 2 + 1]])
    }
}

fn pcmp_lane_signed(bytes: &[u8; 16], lane: usize, lane_bytes: usize) -> i16 {
    if lane_bytes == 1 {
        i16::from(bytes[lane].cast_signed())
    } else {
        i16::from_le_bytes([bytes[lane * 2], bytes[lane * 2 + 1]])
    }
}

fn pcmp_effective_len(bytes: &[u8; 16], lane_bytes: usize, explicit: Option<u64>) -> usize {
    let max_lanes = 16 / lane_bytes;
    if let Some(raw) = explicit {
        let lanes = usize::try_from(low32(raw).cast_signed().unsigned_abs())
            .expect("u32 lane count fits usize");
        return lanes.min(max_lanes);
    }
    (0..max_lanes)
        .position(|i| pcmp_lane_unsigned(bytes, i, lane_bytes) == 0)
        .unwrap_or(max_lanes)
}

// Mirrors the 8-slot extern "C" PCMP helper ABI.
#[allow(clippy::too_many_arguments)]
fn eval_backend_pcmp_str(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lhs_len: u64,
    rhs_len: u64,
    len_mode: u64,
    imm8: u8,
) -> BackendPcmpStrEval {
    let lhs = (u128::from(lhs_hi) << 64) | u128::from(lhs_lo);
    let rhs = (u128::from(rhs_hi) << 64) | u128::from(rhs_lo);
    let lhs_bytes = lhs.to_le_bytes();
    let rhs_bytes = rhs.to_le_bytes();
    let lane_bytes = if imm8 & 1 == 0 { 1 } else { 2 };
    let max_lanes = 16 / lane_bytes;
    let signed = imm8 & 0x02 != 0;
    let aggregation = (imm8 >> 2) & 0x03;
    let polarity = (imm8 >> 4) & 0x03;
    let lhs_len = pcmp_effective_len(
        &lhs_bytes,
        lane_bytes,
        (len_mode & PCMP_LEN_LHS_EXPLICIT != 0).then_some(lhs_len),
    );
    let rhs_len = pcmp_effective_len(
        &rhs_bytes,
        lane_bytes,
        (len_mode & PCMP_LEN_RHS_EXPLICIT != 0).then_some(rhs_len),
    );

    let mut bits = 0u16;
    for i in 0..max_lanes {
        let lhs_valid = i < lhs_len;
        let mut matched = lhs_valid
            && match aggregation {
                0 => (0..rhs_len).any(|j| {
                    if signed {
                        pcmp_lane_signed(&lhs_bytes, i, lane_bytes)
                            == pcmp_lane_signed(&rhs_bytes, j, lane_bytes)
                    } else {
                        pcmp_lane_unsigned(&lhs_bytes, i, lane_bytes)
                            == pcmp_lane_unsigned(&rhs_bytes, j, lane_bytes)
                    }
                }),
                1 => (0..rhs_len).step_by(2).any(|j| {
                    if j + 1 >= rhs_len {
                        return false;
                    }
                    if signed {
                        let value = pcmp_lane_signed(&lhs_bytes, i, lane_bytes);
                        let low = pcmp_lane_signed(&rhs_bytes, j, lane_bytes);
                        let high = pcmp_lane_signed(&rhs_bytes, j + 1, lane_bytes);
                        low <= value && value <= high
                    } else {
                        let value = pcmp_lane_unsigned(&lhs_bytes, i, lane_bytes);
                        let low = pcmp_lane_unsigned(&rhs_bytes, j, lane_bytes);
                        let high = pcmp_lane_unsigned(&rhs_bytes, j + 1, lane_bytes);
                        low <= value && value <= high
                    }
                }),
                2 => {
                    i < rhs_len
                        && if signed {
                            pcmp_lane_signed(&lhs_bytes, i, lane_bytes)
                                == pcmp_lane_signed(&rhs_bytes, i, lane_bytes)
                        } else {
                            pcmp_lane_unsigned(&lhs_bytes, i, lane_bytes)
                                == pcmp_lane_unsigned(&rhs_bytes, i, lane_bytes)
                        }
                }
                _ => {
                    rhs_len > 0
                        && i + rhs_len <= lhs_len
                        && (0..rhs_len).all(|j| {
                            if signed {
                                pcmp_lane_signed(&lhs_bytes, i + j, lane_bytes)
                                    == pcmp_lane_signed(&rhs_bytes, j, lane_bytes)
                            } else {
                                pcmp_lane_unsigned(&lhs_bytes, i + j, lane_bytes)
                                    == pcmp_lane_unsigned(&rhs_bytes, j, lane_bytes)
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

    BackendPcmpStrEval {
        intres: bits,
        max_lanes,
        lane_bytes,
        lhs_len,
        rhs_len,
    }
}

fn backend_pcmp_str_index(eval: BackendPcmpStrEval, imm8: u8) -> u64 {
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

fn backend_pcmp_str_mask(eval: BackendPcmpStrEval, imm8: u8) -> u128 {
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

fn backend_pcmp_str_flags(eval: BackendPcmpStrEval) -> u64 {
    let cf = u64::from(eval.intres != 0);
    let zf = u64::from(eval.rhs_len < eval.max_lanes);
    let sf = u64::from(eval.lhs_len < eval.max_lanes);
    let of = u64::from(eval.intres & 1 != 0);
    cf | (zf << 1) | (sf << 2) | (of << 3)
}

#[inline(never)]
extern "C" fn pcmpstr_index_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lhs_len: u64,
    rhs_len: u64,
    len_mode: u64,
    imm8: u64,
) -> u64 {
    let eval = eval_backend_pcmp_str(
        lhs_lo,
        lhs_hi,
        rhs_lo,
        rhs_hi,
        lhs_len,
        rhs_len,
        len_mode,
        low8(imm8),
    );
    backend_pcmp_str_index(eval, low8(imm8))
}

#[inline(never)]
extern "C" fn pcmpstr_mask_lo_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lhs_len: u64,
    rhs_len: u64,
    len_mode: u64,
    imm8: u64,
) -> u64 {
    let eval = eval_backend_pcmp_str(
        lhs_lo,
        lhs_hi,
        rhs_lo,
        rhs_hi,
        lhs_len,
        rhs_len,
        len_mode,
        low8(imm8),
    );
    low64(backend_pcmp_str_mask(eval, low8(imm8)))
}

#[inline(never)]
extern "C" fn pcmpstr_mask_hi_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lhs_len: u64,
    rhs_len: u64,
    len_mode: u64,
    imm8: u64,
) -> u64 {
    let eval = eval_backend_pcmp_str(
        lhs_lo,
        lhs_hi,
        rhs_lo,
        rhs_hi,
        lhs_len,
        rhs_len,
        len_mode,
        low8(imm8),
    );
    low64(backend_pcmp_str_mask(eval, low8(imm8)) >> 64)
}

#[inline(never)]
extern "C" fn pcmpstr_flags_helper(
    lhs_lo: u64,
    lhs_hi: u64,
    rhs_lo: u64,
    rhs_hi: u64,
    lhs_len: u64,
    rhs_len: u64,
    len_mode: u64,
    imm8: u64,
) -> u64 {
    backend_pcmp_str_flags(eval_backend_pcmp_str(
        lhs_lo,
        lhs_hi,
        rhs_lo,
        rhs_hi,
        lhs_len,
        rhs_len,
        len_mode,
        low8(imm8),
    ))
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
        1 => sign != 0 && remainder != 0,
        2 => sign == 0 && remainder != 0,
        3 => false,
        _ => remainder > halfway || (remainder == halfway && lsb != 0),
    }
}

fn f32_bits_to_f16(bits: u32, imm8: u8) -> u16 {
    let mode = if imm8 & 0x04 != 0 { 0 } else { imm8 & 0x03 };
    let sign = u16::try_from((bits >> 16) & 0x8000).expect("f16 sign");
    let exp = ((bits >> 23) & 0xff).cast_signed();
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

fn backend_f16c_ph_to_ps(src_lo: u64) -> u128 {
    let mut out = 0u128;
    for lane in 0..4 {
        let h = u16::try_from((src_lo >> (lane * 16)) & 0xffff).expect("masked half lane");
        out |= u128::from(f16_to_f32_bits(h)) << (lane * 32);
    }
    out
}

fn backend_f16c_ps_to_ph(src_lo: u64, src_hi: u64, imm8: u8) -> u64 {
    let src = (u128::from(src_hi) << 64) | u128::from(src_lo);
    let mut out = 0u64;
    for lane in 0..4 {
        let bits = u32::try_from((src >> (lane * 32)) & 0xffff_ffff).expect("masked f32 lane");
        out |= u64::from(f32_bits_to_f16(bits, imm8)) << (lane * 16);
    }
    out
}

#[inline(never)]
extern "C" fn f16c_ph2ps_lo_helper(src_lo: u64) -> u64 {
    low64(backend_f16c_ph_to_ps(src_lo))
}

#[inline(never)]
extern "C" fn f16c_ph2ps_hi_helper(src_lo: u64) -> u64 {
    low64(backend_f16c_ph_to_ps(src_lo) >> 64)
}

#[inline(never)]
extern "C" fn f16c_ps2ph_helper(src_lo: u64, src_hi: u64, imm8: u64) -> u64 {
    backend_f16c_ps_to_ph(src_lo, src_hi, low8(imm8))
}

/// Lowering failures surfaced by the Rust backend.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LowerError {
    #[error("entry block {0} not found")]
    MissingEntryBlock(u32),
    #[error("target block {0} not found")]
    MissingTargetBlock(u32),
    #[error("statement result is required for {0}")]
    MissingResult(&'static str),
    #[error("SSA ref {0} has no assigned host register")]
    MissingValue(Ref),
    #[error("constant 0x{0:x} is not encodable by the current constant-emission slice")]
    ConstantOutOfRange(u64),
    #[error("immediate 0x{0:x} is not encodable by the current ADD/SUB immediate slice")]
    ImmediateOutOfRange(u64),
    #[error("unsupported op in Rust backend migration slice: {0}")]
    UnsupportedOp(&'static str),
}

/// Lowering strategy used by callers in later phases.
#[derive(Debug, Clone)]
pub struct Lowerer {
    /// Tracks an optional lowering budget for future tuning points.
    budget: usize,
    /// When set, `Op::Return` lowers to the full AAPCS64 block epilogue
    /// (restore callee-saved + `ret`) instead of a bare `ret`. The executor
    /// needs this so a region containing returns balances the prologue's stack
    /// pushes; the default (bare `ret`) preserves the historical lowering used
    /// by the differential and the per-instruction lowering tests.
    return_via_epilogue: bool,
    /// When set, a relative branch (`JumpRel`/`CondJumpRel`) lowers to a
    /// block-exit: it stores the taken guest PC in `CpuStateFrame::next_pc`, marks
    /// `EXIT_BRANCH`, and returns to the run loop — instead of branching to a
    /// sibling block label. Used by the single-block translator path, where there
    /// is no sibling block; an intra-region multi-block lowering leaves this off
    /// so branches stay direct `B`/`B.cond` within the region.
    branch_via_frame: bool,
}

impl Default for Lowerer {
    fn default() -> Self {
        Self::new()
    }
}

impl Lowerer {
    /// Constructs a default lowerer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            budget: 1024,
            return_via_epilogue: false,
            branch_via_frame: false,
        }
    }

    /// Returns a lowerer whose `Op::Return` emits the full block epilogue
    /// (restore callee-saved + `ret`), so a wrapped region with returns leaves
    /// the host stack and callee-saved registers correct on exit.
    #[must_use]
    pub const fn with_returns_via_epilogue(mut self) -> Self {
        self.return_via_epilogue = true;
        self
    }

    /// Returns a lowerer whose relative branches (`JumpRel`/`CondJumpRel`) exit
    /// the block through `CpuStateFrame::next_pc` + `EXIT_BRANCH` instead of
    /// branching to a sibling block. The single-block run-loop translator uses
    /// this; it implies the epilogue return path, so it also sets that.
    #[must_use]
    pub const fn with_branch_exits(mut self) -> Self {
        self.branch_via_frame = true;
        self.return_via_epilogue = true;
        self
    }

    /// Lowers an input instruction buffer into backend words.
    ///
    /// Byte-stream lowering is still owned by the decoder/backend integration
    /// work. Keep this compatibility shim empty until the Rust decoder feeds
    /// `Function` values directly into this crate.
    #[must_use]
    pub const fn lower_ir(&self, _bytes: &[u8]) -> Vec<u32> {
        let _ = self.budget;
        Vec::new()
    }

    /// Lowers a minimal `prisma-ir` function into `AArch64` instruction words.
    ///
    /// Current migration coverage:
    /// - `Constant` values via `MOVZ` + `MOVK`
    /// - `LoadReg`/`StoreReg` over `CpuStateFrame::gpr[]`
    /// - `BinOp Add/Sub` with a register rhs or 12-bit constant rhs
    /// - `BinOp And/Or/Xor/Shl/Shr/Sar/Ror/Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`
    ///   with previously-lowered register operands
    /// - `Compare` values via `CMP` + `CSET` with sized-operand alignment for
    ///   non-I64 compares
    /// - `CmpFlags`/`CondJumpFlags` through ARM64 NZCV + `B.cond`
    /// - `CondJumpRel` through ARM64 `B.cond`
    /// - `Select` via flag-dependent branch sequencing (`B.cond` + `MOV`)
    /// - `LoadMem`/`StoreMem` for `I8`/`I16`/`I32`/`I64` with address/value
    ///   already in registers
    /// - `VecConstant`, `LoadVecReg`/`StoreVecReg`, and `LoadVec`/`StoreVec`
    ///   as scalar low/high 64-bit pairs
    /// - direct `Jump`, `JumpRel`, `CallRel` and `CallReg` between/through
    ///   registers
    /// - `RspAdjust` and `RetAdjusted` stack adjustments over `Rsp` state
    /// - `Return`
    ///
    /// This is deliberately small, but it is a real IR-to-backend path with
    /// exact instruction tests.
    ///
    /// # Errors
    ///
    /// Returns `LowerError` when the function uses IR outside the current
    /// migration slice or references values that have not been lowered.
    pub fn lower_function(&self, func: &Function) -> Result<Vec<u32>, LowerError> {
        let mut asm = Arm64Assembler::new();
        let mut labels = HashMap::<u32, Label>::new();
        for block in &func.blocks {
            labels.insert(block.id, asm.create_label());
        }

        if !labels.contains_key(&func.entry) {
            return Err(LowerError::MissingEntryBlock(func.entry));
        }

        let mut values = HashMap::<Ref, u8>::new();
        let mut vec_values = HashMap::<Ref, (u8, u8)>::new();
        let mut constants = HashMap::<Ref, u64>::new();
        let mut flags = HashSet::<Ref>::new();

        let mut block_order = Vec::with_capacity(func.blocks.len());
        let entry_block = func
            .blocks
            .iter()
            .find(|block| block.id == func.entry)
            .ok_or(LowerError::MissingEntryBlock(func.entry))?;
        block_order.push(entry_block);
        block_order.extend(func.blocks.iter().filter(|block| block.id != func.entry));

        for block in block_order {
            let mut nzcv_live = false;
            let label = labels
                .get(&block.id)
                .copied()
                .ok_or(LowerError::MissingTargetBlock(block.id))?;
            asm.bind_label(label);

            for stmt in &block.stmts {
                lower_stmt(
                    stmt,
                    &mut asm,
                    &labels,
                    &mut values,
                    &mut vec_values,
                    &mut constants,
                    &mut flags,
                    &mut nzcv_live,
                    ExitAbi {
                        return_via_epilogue: self.return_via_epilogue,
                        branch_via_frame: self.branch_via_frame,
                    },
                )?;
            }
        }

        Ok(asm.finish())
    }
}

/// How a block's terminators exit. Bundled so the lowering dispatch stays within
/// the argument budget; both default off (bare `ret`, sibling-block branches).
#[derive(Clone, Copy)]
struct ExitAbi {
    /// `Op::Return` lowers to the full AAPCS64 epilogue rather than a bare `ret`.
    return_via_epilogue: bool,
    /// Relative branches store the taken PC in the frame and exit to the run loop.
    branch_via_frame: bool,
}

// One match arm per IR op; the dispatch is inherently long and splitting it
// would only scatter the op->lowering mapping across helpers. The argument
// list mirrors the per-block lowering state threaded through every op.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn lower_stmt(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    values: &mut HashMap<Ref, u8>,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    constants: &mut HashMap<Ref, u64>,
    flags: &mut HashSet<Ref>,
    nzcv_live: &mut bool,
    exit: ExitAbi,
) -> Result<(), LowerError> {
    match &stmt.op {
        Op::Constant(c) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Constant"))?;
            let reg = value_reg(result);
            emit_u64_constant(asm, reg, c.value);
            values.insert(result, reg);
            constants.insert(result, c.value);
        }
        Op::LoadReg(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadReg"))?;
            let dst = value_reg(result);
            emit_load_reg(asm, load.size, dst, load.reg);
            values.insert(result, dst);
        }
        Op::StoreReg(store) => {
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            emit_store_reg(asm, store.size, value, store.reg);
        }
        Op::VecConstant(c) => {
            let result = stmt
                .result
                .ok_or(LowerError::MissingResult("VecConstant"))?;
            let (lo, hi) = alloc_vec_pair(vec_values, result)?;
            emit_u64_constant(asm, lo, c.lo);
            emit_u64_constant(asm, hi, c.hi);
        }
        Op::VecBinOp(bin) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("VecBinOp"))?;
            let (lhs_lo, lhs_hi) = vec_pair(vec_values, bin.lhs)?;
            let (rhs_lo, rhs_hi) = vec_pair(vec_values, bin.rhs)?;
            let (dst_lo, dst_hi) = alloc_vec_pair(vec_values, result)?;
            match bin.op {
                prisma_ir::VecBinOpKind::Add if bin.lane == prisma_ir::VecLane::S4 => {
                    emit_vec_add_u32x2(asm, dst_lo, lhs_lo, rhs_lo);
                    emit_vec_add_u32x2(asm, dst_hi, lhs_hi, rhs_hi);
                }
                prisma_ir::VecBinOpKind::And => {
                    asm.and_x(dst_lo, lhs_lo, rhs_lo);
                    asm.and_x(dst_hi, lhs_hi, rhs_hi);
                }
                prisma_ir::VecBinOpKind::Or => {
                    asm.orr_x(dst_lo, lhs_lo, rhs_lo);
                    asm.orr_x(dst_hi, lhs_hi, rhs_hi);
                }
                prisma_ir::VecBinOpKind::Xor => {
                    asm.eor_x(dst_lo, lhs_lo, rhs_lo);
                    asm.eor_x(dst_hi, lhs_hi, rhs_hi);
                }
                _ => return Err(LowerError::UnsupportedOp("VecBinOp")),
            }
        }
        Op::VecShuffle32x4(shuffle) => {
            lower_vec_shuffle32x4(stmt, asm, vec_values, shuffle)?;
        }
        Op::VecUnpack(unpack) => {
            lower_vec_unpack(stmt, asm, vec_values, unpack)?;
        }
        Op::VecShuffleH4(shuffle) => {
            lower_vec_shuffle_h4(stmt, asm, vec_values, shuffle)?;
        }
        Op::VecCmp(compare) => {
            lower_vec_cmp(stmt, asm, vec_values, compare)?;
        }
        Op::VecShiftImm(shift) => {
            lower_vec_shift_imm(stmt, asm, vec_values, shift)?;
        }
        Op::LoadVecReg(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadVecReg"))?;
            let (lo, hi) = alloc_vec_pair(vec_values, result)?;
            emit_load_vec_state(asm, lo, hi, load.xmm_index)?;
        }
        Op::StoreVecReg(store) => {
            let (lo, hi) = vec_pair(vec_values, store.value)?;
            emit_store_vec_state(asm, lo, hi, store.xmm_index)?;
        }
        Op::LoadVec(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadVec"))?;
            let addr = *values
                .get(&load.addr)
                .ok_or(LowerError::MissingValue(load.addr))?;
            let (lo, hi) = alloc_vec_pair(vec_values, result)?;
            emit_load_vec_mem(asm, lo, hi, addr);
        }
        Op::StoreVec(store) => {
            let addr = *values
                .get(&store.addr)
                .ok_or(LowerError::MissingValue(store.addr))?;
            let (lo, hi) = vec_pair(vec_values, store.value)?;
            emit_store_vec_mem(asm, lo, hi, addr);
        }
        Op::VecMaskMsb(mask) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("VecMaskMsb"))?;
            let src = vec_pair(vec_values, mask.src_xmm)?;
            let dst = value_reg(result);
            emit_save_for_helper_call(asm);
            asm.mov_x(0, src.0);
            asm.mov_x(1, src.1);
            emit_u64_constant(
                asm,
                PCMP_HELPER_TARGET_REG,
                vec_mask_msb_helper as VecMaskMsbHelper as usize as u64,
            );
            asm.blr_x(PCMP_HELPER_TARGET_REG);
            emit_restore_after_helper_call(asm);
            asm.mov_x(dst, 0);
            values.insert(result, dst);
        }
        Op::WriteFlagsFp(write) => {
            let (lhs_lo, _) = vec_pair(vec_values, write.lhs)?;
            let (rhs_lo, _) = vec_pair(vec_values, write.rhs)?;
            if write.size == prisma_ir::FpSize::F64 {
                lower_write_flags_fp64(asm, lhs_lo, rhs_lo);
            } else {
                emit_save_for_helper_call(asm);
                asm.mov_x(0, lhs_lo);
                asm.mov_x(1, rhs_lo);
                emit_u64_constant(
                    asm,
                    PCMP_HELPER_TARGET_REG,
                    fp32_rflags_helper as *const () as usize as u64,
                );
                asm.blr_x(PCMP_HELPER_TARGET_REG);
                emit_restore_after_helper_call(asm);
                lower_store_rflags(asm, 0);
            }
            *nzcv_live = false;
        }
        Op::IntToFpScalar(convert) => {
            lower_int_to_fp_scalar(stmt, asm, values, vec_values, convert)?;
        }
        Op::VecFpScalarBinOp(bin) => {
            lower_vec_fp_scalar_bin(stmt, asm, vec_values, bin)?;
        }
        Op::FpToIntScalar(convert) => {
            lower_fp_to_int_scalar(stmt, asm, values, vec_values, convert)?;
        }
        Op::XmmFromGpr(x) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("XmmFromGpr"))?;
            let src = *values
                .get(&x.value)
                .ok_or(LowerError::MissingValue(x.value))?;
            let (lo, hi) = alloc_vec_pair(vec_values, result)?;
            match x.size {
                OpSize::I8 | OpSize::I16 => {
                    emit_u64_constant(asm, FLAG_ALIGN_LHS_REG, x.size.mask());
                    asm.and_x(lo, src, FLAG_ALIGN_LHS_REG);
                }
                OpSize::I32 => asm.uxtw_x(lo, src),
                OpSize::I64 => asm.mov_x(lo, src),
            }
            emit_u64_constant(asm, hi, 0);
        }
        Op::GprFromXmm(x) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("GprFromXmm"))?;
            let (lo, _) = vec_pair(vec_values, x.value)?;
            let dst = value_reg(result);
            match x.size {
                OpSize::I8 | OpSize::I16 => {
                    emit_u64_constant(asm, FLAG_ALIGN_LHS_REG, x.size.mask());
                    asm.and_x(dst, lo, FLAG_ALIGN_LHS_REG);
                }
                OpSize::I32 => asm.uxtw_x(dst, lo),
                OpSize::I64 => asm.mov_x(dst, lo),
            }
            values.insert(result, dst);
        }
        Op::BinOp(bin) => {
            lower_binop(stmt, asm, values, constants, bin, exit.return_via_epilogue)?;
        }
        Op::WideDiv(div) => {
            lower_wide_div(stmt, asm, values, div, exit.return_via_epilogue)?;
        }
        Op::LoadMem(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadMem"))?;
            let addr = *values
                .get(&load.addr)
                .ok_or(LowerError::MissingValue(load.addr))?;
            let dst = value_reg(result);
            emit_load_mem(asm, load.size, dst, addr);
            values.insert(result, dst);
        }
        Op::Compare(compare) => {
            lower_compare(stmt, asm, values, compare)?;
            *nzcv_live = true;
        }
        Op::CmpFlags(cmp) => {
            lower_cmp_flags(stmt, asm, values, flags, cmp)?;
            *nzcv_live = true;
        }
        Op::AluFlags(alu) => {
            lower_alu_flags(asm, values, alu)?;
            *nzcv_live = true;
        }
        Op::AluFlagsPreserveCarry(alu) => {
            lower_alu_flags_preserve_carry(asm, values, alu)?;
            *nzcv_live = true;
        }
        Op::StoreMem(store) => {
            let addr = *values
                .get(&store.addr)
                .ok_or(LowerError::MissingValue(store.addr))?;
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            emit_store_mem(asm, store.size, value, addr);
        }
        Op::LoadMemTSO(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadMemTSO"))?;
            let addr = *values
                .get(&load.addr)
                .ok_or(LowerError::MissingValue(load.addr))?;
            let dst = value_reg(result);
            emit_load_mem(asm, load.size, dst, addr);
            asm.fence(prisma_ir::FenceKind::Mfence);
            values.insert(result, dst);
        }
        Op::StoreMemTSO(store) => {
            let addr = *values
                .get(&store.addr)
                .ok_or(LowerError::MissingValue(store.addr))?;
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            asm.fence(prisma_ir::FenceKind::Mfence);
            emit_store_mem(asm, store.size, value, addr);
            asm.fence(prisma_ir::FenceKind::Mfence);
        }
        Op::AtomicCmpxchg(cas) => {
            lower_atomic_cmpxchg(stmt, asm, values, cas)?;
            *nzcv_live = false;
        }
        Op::AtomicXchg(xchg) => {
            lower_atomic_xchg(stmt, asm, values, xchg)?;
            *nzcv_live = false;
        }
        Op::AtomicXadd(xadd) => {
            lower_atomic_xadd(stmt, asm, values, xadd)?;
            *nzcv_live = false;
        }
        Op::AtomicCmpxchgPair(cas) => {
            lower_atomic_cmpxchg_pair(stmt, asm, values, cas)?;
            *nzcv_live = false;
        }
        Op::LoadSegBase(seg) => {
            lower_load_seg_base(stmt, asm, values, seg)?;
        }
        Op::Cpuid(_) => lower_cpuid(asm),
        Op::Xgetbv(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Xgetbv"))?;
            let dst = value_reg(result);
            lower_xgetbv(asm, dst);
            values.insert(result, dst);
        }
        Op::Rdtsc(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Rdtsc"))?;
            let dst = value_reg(result);
            asm.mrs_cntvct(dst);
            values.insert(result, dst);
        }
        Op::Syscall(_) => {
            lower_syscall(asm, exit.return_via_epilogue);
        }
        Op::Trap(_) => {
            asm.movz_x(0, 0, 0);
            if exit.return_via_epilogue {
                abi::emit_block_epilogue_and_ret(asm);
            } else {
                asm.ret();
            }
        }
        Op::TrapIf(trap) => {
            let condition = *values
                .get(&trap.condition)
                .ok_or(LowerError::MissingValue(trap.condition))?;
            let ok = asm.create_label();
            asm.cbz_x_label(condition, ok);
            emit_sigfpe_placeholder_return(asm, exit.return_via_epilogue);
            asm.bind_label(ok);
        }
        Op::Extend(extend) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Extend"))?;
            let dst = value_reg(result);
            let src = *values
                .get(&extend.value)
                .ok_or(LowerError::MissingValue(extend.value))?;
            lower_extend(
                asm,
                dst,
                src,
                extend.from_size,
                extend.to_size,
                extend.is_signed,
            );
            values.insert(result, dst);
        }
        Op::Truncate(trunc) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Truncate"))?;
            let dst = value_reg(result);
            let src = *values
                .get(&trunc.value)
                .ok_or(LowerError::MissingValue(trunc.value))?;
            lower_truncate(asm, dst, src, trunc.to_size);
            values.insert(result, dst);
        }
        Op::Fence(fence) => match fence.kind {
            prisma_ir::FenceKind::Mfence
            | prisma_ir::FenceKind::Lfence
            | prisma_ir::FenceKind::Sfence => {
                asm.fence(fence.kind);
            }
        },
        Op::GuestPc(guest_pc) => {
            emit_u64_constant(asm, GUEST_PC_SCRATCH_REG, guest_pc.pc);
            asm.str_x_unsigned(GUEST_PC_SCRATCH_REG, abi::K_STATE_PTR_REG, NEXT_PC_OFFSET);
        }
        Op::WriteFlags(write_flags) => {
            lower_write_flags(asm, values, flags, write_flags, stmt)?;
            *nzcv_live = true;
        }
        Op::WriteFlagsPopcnt(popcnt) => {
            lower_write_flags_popcnt(asm, values, popcnt)?;
            *nzcv_live = true;
        }
        Op::WriteFlagsCountZero(count_zero) => {
            lower_write_flags_count_zero(asm, values, count_zero)?;
            *nzcv_live = true;
        }
        Op::ReadFlag(flag_read) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("ReadFlag"))?;
            let dst = value_reg(result);
            lower_read_flag(asm, flags, flag_read, dst)?;
            values.insert(result, dst);
        }
        Op::LoadCarry(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadCarry"))?;
            let dst = value_reg(result);
            asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, CF_OFFSET);
            values.insert(result, dst);
        }
        Op::ReadCarryOut(read) => {
            let result = stmt
                .result
                .ok_or(LowerError::MissingResult("ReadCarryOut"))?;
            if !flags.contains(&read.flags) {
                return Err(LowerError::MissingValue(read.flags));
            }
            let dst = value_reg(result);
            // ARM64 C is set on add, inverted (C = NOT borrow) on sub, so x86 CF
            // is `cset <carry-set>` after an add and `cset <carry-clear>` after a
            // sub. In this codebase `Nc` encodes carry-set (0x2) and `Cc` encodes
            // carry-clear (0x3) — see the assembler cond table and jcc_condition.
            let cc = if read.from_sub {
                prisma_ir::CondCode::Cc
            } else {
                prisma_ir::CondCode::Nc
            };
            asm.cset_x(dst, cc);
            values.insert(result, dst);
        }
        Op::StoreCarry(store) => {
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            lower_store_carry(asm, value);
            *nzcv_live = false;
        }
        Op::LoadRflags(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadRflags"))?;
            let dst = value_reg(result);
            asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
            values.insert(result, dst);
        }
        Op::StoreRflags(store) => {
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            lower_store_rflags(asm, value);
            *nzcv_live = false;
        }
        Op::StoreRflagsFromNzcv(store) => {
            let pf = if let Some(r) = store.pf {
                Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
            } else {
                None
            };
            let af = if let Some(r) = store.af {
                Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
            } else {
                None
            };
            lower_store_rflags_from_nzcv(asm, store.carry, pf, af);
        }
        Op::StoreRflagsFromBits(store) => {
            let pf = if let Some(r) = store.pf {
                Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
            } else {
                None
            };
            let af = if let Some(r) = store.af {
                Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
            } else {
                None
            };
            let zf = *values
                .get(&store.zf)
                .ok_or(LowerError::MissingValue(store.zf))?;
            let sf = *values
                .get(&store.sf)
                .ok_or(LowerError::MissingValue(store.sf))?;
            let of = *values
                .get(&store.of)
                .ok_or(LowerError::MissingValue(store.of))?;
            lower_store_rflags_from_bits(asm, pf, af, zf, sf, of);
            *nzcv_live = false;
        }
        Op::Lzcnt(lzcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Lzcnt"))?;
            let src = *values
                .get(&lzcnt.value)
                .ok_or(LowerError::MissingValue(lzcnt.value))?;
            let dst = value_reg(result);
            lower_lzcnt(asm, dst, src, lzcnt.size);
            values.insert(result, dst);
        }
        Op::Tzcnt(tzcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Tzcnt"))?;
            let src = *values
                .get(&tzcnt.value)
                .ok_or(LowerError::MissingValue(tzcnt.value))?;
            let dst = value_reg(result);
            lower_tzcnt(asm, dst, src, tzcnt.size);
            values.insert(result, dst);
        }
        Op::Popcnt(popcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Popcnt"))?;
            let src = *values
                .get(&popcnt.value)
                .ok_or(LowerError::MissingValue(popcnt.value))?;
            let dst = value_reg(result);
            lower_popcnt(asm, dst, src, popcnt.size);
            values.insert(result, dst);
        }
        Op::Bswap(bswap) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Bswap"))?;
            let src = *values
                .get(&bswap.value)
                .ok_or(LowerError::MissingValue(bswap.value))?;
            let dst = value_reg(result);
            lower_bswap(asm, dst, src, bswap.size);
            values.insert(result, dst);
        }
        Op::Crc32c(crc) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Crc32c"))?;
            let crc_reg = *values
                .get(&crc.crc)
                .ok_or(LowerError::MissingValue(crc.crc))?;
            let data_reg = *values
                .get(&crc.data)
                .ok_or(LowerError::MissingValue(crc.data))?;
            let dst = value_reg(result);
            lower_crc32c(asm, dst, crc_reg, data_reg, crc.data_size);
            values.insert(result, dst);
        }
        Op::Jump(jump) => {
            let target = labels
                .get(&jump.target_block)
                .copied()
                .ok_or(LowerError::MissingTargetBlock(jump.target_block))?;
            asm.b_label(target);
        }
        Op::CondJump(jump) => {
            lower_cond_jump(asm, labels, values, jump)?;
        }
        Op::CondJumpFlags(jump) => {
            lower_cond_jump_flags(asm, labels, flags, jump)?;
        }
        Op::CondJumpRel(jump) => {
            ensure_nzcv_live(asm, nzcv_live);
            if exit.branch_via_frame {
                // Host-wrapped single block: no sibling block to branch to.
                // Compute the taken guest PC and exit to the run loop, which
                // translates and runs the next block.
                lower_cond_jump_rel_exit(asm, jump);
            } else {
                lower_cond_jump_rel(asm, labels, jump)?;
            }
        }
        Op::Select(select) => {
            ensure_nzcv_live(asm, nzcv_live);
            lower_select(asm, values, select, stmt)?;
        }
        Op::JumpReg(jump) => {
            let target = *values
                .get(&jump.target)
                .ok_or(LowerError::MissingValue(jump.target))?;
            if exit.branch_via_frame {
                lower_jump_reg_exit(asm, target);
            } else {
                asm.br_x(target);
            }
        }
        Op::JumpRel(jump) => {
            if exit.branch_via_frame {
                lower_jump_rel_exit(asm, jump.target_guest_pc);
            } else {
                let target = block_label(labels, jump.target_guest_pc)?;
                asm.b_label(target);
            }
        }
        Op::CallRel(call) => {
            if exit.branch_via_frame {
                lower_call_rel_exit(asm, call.target_guest_pc, call.return_guest_pc)?;
            } else {
                let target = block_label(labels, call.target_guest_pc)?;
                asm.b_label(target);
                let _ = call.return_guest_pc;
            }
        }
        Op::CallReg(call) => {
            let target = *values
                .get(&call.target)
                .ok_or(LowerError::MissingValue(call.target))?;
            if exit.branch_via_frame {
                lower_call_reg_exit(asm, target, call.return_guest_pc)?;
            } else {
                asm.blr_x(target);
                let _ = call.return_guest_pc;
            }
        }
        Op::RspAdjust(rsp) => {
            lower_rsp_adjust(asm, rsp)?;
        }
        Op::Return(_) => {
            if exit.branch_via_frame {
                // The guest stack holds the return address (RFC 0020): pop it and
                // chain through the run loop instead of returning to the host.
                lower_return_exit(asm, 8)?;
            } else if exit.return_via_epilogue {
                abi::emit_block_epilogue_and_ret(asm);
            } else {
                asm.ret();
            }
        }
        Op::RetAdjusted(ret) => {
            if exit.branch_via_frame {
                lower_return_exit(asm, ret.pop_bytes)?;
            } else {
                lower_ret_adjusted(asm, ret.pop_bytes)?;
            }
        }
        Op::VecClMul(clmul) => {
            lower_vec_clmul(stmt, asm, vec_values, clmul)?;
        }
        Op::VecF16Cvt(cvt) => {
            lower_vec_f16cvt(stmt, asm, vec_values, cvt)?;
        }
        Op::PcmpStrIndex(pcmp) => {
            lower_pcmpstr_index(stmt, asm, values, vec_values, pcmp)?;
        }
        Op::PcmpStrMask(pcmp) => {
            lower_pcmpstr_mask(stmt, asm, values, vec_values, pcmp)?;
        }
        Op::PcmpStrFlags(pcmp) => {
            lower_pcmpstr_flags(stmt, asm, values, vec_values, pcmp)?;
        }
        Op::RepMovs(rep) => {
            lower_rep_string(
                asm,
                rep.size,
                rep.reverse,
                rep.pc_of_rep,
                rep.pc_after_rep,
                true,
                exit,
            );
            *nzcv_live = false;
        }
        Op::RepStos(rep) => {
            lower_rep_string(
                asm,
                rep.size,
                rep.reverse,
                rep.pc_of_rep,
                rep.pc_after_rep,
                false,
                exit,
            );
            *nzcv_live = false;
        }
        _ => return Err(LowerError::UnsupportedOp("unsupported")),
    }

    Ok(())
}

const FS_BASE_OFFSET: u16 = 792;
const GS_BASE_OFFSET: u16 = 800;
/// Byte offset of XMM0 in `CpuStateFrame`. The Rust frame keeps this range in
/// its reserved vector-state span, matching the C++ frame layout.
const XMM_BASE_OFFSET: u16 = 144;
const XMM_SLOT_BYTES: u16 = 16;
/// Byte offset of the persistent x86 carry flag in `CpuStateFrame` (follows
/// `gs_base`). Matches `prisma_runtime::executor::CpuStateFrame::cf`.
const CF_OFFSET: u16 = 808;
/// Byte offset of the persistent x86 RFLAGS subset in `CpuStateFrame`.
const RFLAGS_OFFSET: u16 = 816;
const RFLAGS_CF_BIT: u64 = 1 << 0;
const RFLAGS_PF_BIT: u64 = 1 << 2;
const RFLAGS_AF_BIT: u64 = 1 << 4;
const RFLAGS_ZF_BIT: u64 = 1 << 6;
const RFLAGS_SF_BIT: u64 = 1 << 7;
const RFLAGS_OF_BIT: u64 = 1 << 11;
/// Byte offset of the block exit-reason word in `CpuStateFrame` (follows
/// `rflags`).
/// Matches `prisma_runtime::executor::CpuStateFrame::exit_reason`; a `SYSCALL`
/// block stores `EXIT_SYSCALL` here before returning to the host.
const EXIT_REASON_OFFSET: u16 = 824;
/// Exit-reason value a `SYSCALL` block writes (`EXIT_SYSCALL` in the runtime).
const EXIT_SYSCALL_MARK: u16 = 1;
/// Byte offset of the resume-PC word in `CpuStateFrame` (follows `exit_reason`).
/// Matches `prisma_runtime::executor::CpuStateFrame::next_pc`; a relative-branch
/// block stores its taken target here before returning to the host run loop.
const NEXT_PC_OFFSET: u16 = 832;
/// Exit-reason value a relative-branch block writes (`EXIT_BRANCH` in runtime).
const EXIT_BRANCH_MARK: u16 = 2;
/// Byte offset of the guest-memory base in `CpuStateFrame` (follows `next_pc`).
/// Matches `prisma_runtime::executor::CpuStateFrame::mem_base`. Every guest
/// memory access is rebased to `host = mem_base + guest_va` so the JIT can reach
/// a contiguous host arena that is not identity-mapped to the guest VAs (RFC
/// 0020). A `mem_base` of 0 reproduces the legacy `host == guest` behaviour.
const MEM_BASE_OFFSET: u16 = 840;
/// Scratch register holding the rebased host address inside a memory op. Outside
/// the value-register pool (x9..x16) so it never aliases the `addr`/`value`/`dst`
/// operands, and inside the prologue's callee-saved set so the body may clobber
/// it. The block body otherwise touches x9..x17, x19..x26, x27 (state) and
/// x28; Windows' platform register x18 is never allocated.
const MEM_ADDR_SCRATCH: u8 = 24;
/// Scratch register receiving `STLXR*` status in atomic compare-exchange loops.
const CAS_STATUS_REG: u8 = 25;
const KSTATE_CPUID_MAX_LEAF: u64 = 7;
const KSTATE_CPUID_VENDOR_EBX: u64 = 0x756E_6547;
const KSTATE_CPUID_VENDOR_EDX: u64 = 0x4965_6E69;
const KSTATE_CPUID_VENDOR_ECX: u64 = 0x6C65_746E;
const KSTATE_CPUID_LEAF1_EAX: u64 = 0x0002_06A7;
const KSTATE_CPUID_LEAF1_EBX: u64 = 0x0000_0800;
const KSTATE_CPUID_LEAF1_ECX: u64 = (1u64 << 0)
    | (1u64 << 1)
    | (1u64 << 9)
    | (1u64 << 12)
    | (1u64 << 13)
    | (1u64 << 19)
    | (1u64 << 20)
    | (1u64 << 22)
    | (1u64 << 23)
    | (1u64 << 27)
    | (1u64 << 28)
    | (1u64 << 29);
const KSTATE_CPUID_LEAF1_EDX: u64 =
    (1u64 << 0) | (1u64 << 4) | (1u64 << 8) | (1u64 << 15) | (1u64 << 25) | (1u64 << 26);
const KSTATE_CPUID_LEAF7_EBX: u64 = (1u64 << 3) | (1u64 << 8);
const KSTATE_XCR0_EAX: u64 = 0x7;

fn lower_select(
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    select: &prisma_ir::Select,
    stmt: &Stmt,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("Select"))?;
    let true_value = *values
        .get(&select.true_value)
        .ok_or(LowerError::MissingValue(select.true_value))?;
    let false_value = *values
        .get(&select.false_value)
        .ok_or(LowerError::MissingValue(select.false_value))?;

    let result_reg = value_reg(result);
    let true_label = asm.create_label();
    let end_label = asm.create_label();

    asm.b_cond_label(select.cc, true_label);
    asm.mov_x(result_reg, false_value);
    asm.b_label(end_label);
    asm.bind_label(true_label);
    asm.mov_x(result_reg, true_value);
    asm.bind_label(end_label);

    values.insert(result, result_reg);
    Ok(())
}

fn lower_bswap(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => asm.rev_x(dst, src),
        OpSize::I32 => asm.rev_w(dst, src),
        OpSize::I16 => {
            asm.rev_w(dst, src);
            emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 16);
            asm.lsr_x(dst, dst, FLAG_ALIGN_SHIFT_REG);
        }
        OpSize::I8 => lower_truncate(asm, dst, src, OpSize::I8),
    }
}

fn lower_crc32c(asm: &mut Arm64Assembler, dst: u8, crc: u8, data: u8, data_size: OpSize) {
    match data_size {
        OpSize::I8 => asm.crc32cb(dst, crc, data),
        OpSize::I16 => asm.crc32ch(dst, crc, data),
        OpSize::I32 => asm.crc32cw(dst, crc, data),
        OpSize::I64 => asm.crc32cx(dst, crc, data),
    }
}

fn lower_write_flags_count_zero(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    count_zero: &prisma_ir::WriteFlagsCountZero,
) -> Result<(), LowerError> {
    let src = *values
        .get(&count_zero.src)
        .ok_or(LowerError::MissingValue(count_zero.src))?;
    let result = *values
        .get(&count_zero.result)
        .ok_or(LowerError::MissingValue(count_zero.result))?;

    let z_bit = FLAG_ALIGN_LHS_REG;
    let c_bit = FLAG_ALIGN_RHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;

    if count_zero.size == OpSize::I64 {
        asm.cmp_x(result, 31);
    } else {
        lower_truncate(asm, z_bit, result, count_zero.size);
        asm.cmp_x(z_bit, 31);
    }
    asm.cset_x(z_bit, prisma_ir::CondCode::Eq);
    if count_zero.size == OpSize::I64 {
        asm.cmp_x(src, 31);
    } else {
        lower_truncate(asm, c_bit, src, count_zero.size);
        asm.cmp_x(c_bit, 31);
    }
    asm.cset_x(c_bit, prisma_ir::CondCode::Eq);
    emit_u64_constant(asm, shift, 30);
    asm.lsl_x(z_bit, z_bit, shift);
    emit_u64_constant(asm, shift, 29);
    asm.lsl_x(c_bit, c_bit, shift);
    asm.orr_x(z_bit, z_bit, c_bit);
    asm.msr_nzcv(z_bit);

    Ok(())
}

fn lower_write_flags_popcnt(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    popcnt: &prisma_ir::WriteFlagsPopcnt,
) -> Result<(), LowerError> {
    let src = *values
        .get(&popcnt.src)
        .ok_or(LowerError::MissingValue(popcnt.src))?;

    let z_bit = FLAG_ALIGN_LHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;

    if popcnt.size == OpSize::I64 {
        asm.cmp_x(src, 31);
    } else {
        lower_truncate(asm, z_bit, src, popcnt.size);
        asm.cmp_x(z_bit, 31);
    }
    asm.cset_x(z_bit, prisma_ir::CondCode::Eq);
    emit_u64_constant(asm, shift, 30);
    asm.lsl_x(z_bit, z_bit, shift);
    asm.msr_nzcv(z_bit);

    Ok(())
}

fn lower_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    constants: &HashMap<Ref, u64>,
    bin: &prisma_ir::BinOp,
    return_via_epilogue: bool,
) -> Result<(), LowerError> {
    match bin.op {
        BinOpKind::Add | BinOpKind::Sub => lower_add_sub(stmt, asm, values, constants, bin),
        BinOpKind::And
        | BinOpKind::Or
        | BinOpKind::Xor
        | BinOpKind::Shl
        | BinOpKind::Shr
        | BinOpKind::Sar
        | BinOpKind::Ror
        | BinOpKind::Rol
        | BinOpKind::Mul
        | BinOpKind::UMulHi
        | BinOpKind::SMulHi
        | BinOpKind::UDiv
        | BinOpKind::SDiv => lower_reg_binop(stmt, asm, values, bin, return_via_epilogue),
        BinOpKind::UMod | BinOpKind::SMod => {
            lower_mod_binop(stmt, asm, values, bin, return_via_epilogue)
        }
        _ => Err(LowerError::UnsupportedOp("BinOp")),
    }
}

fn lower_cpuid(asm: &mut Arm64Assembler) {
    let leaf0 = asm.create_label();
    let leaf1 = asm.create_label();
    let leaf7 = asm.create_label();
    let other = asm.create_label();
    let done = asm.create_label();

    let rax = FLAG_ALIGN_SHIFT_REG;
    let rcx = FLAG_ALIGN_RHS_REG;
    let tmp = FLAG_ALIGN_LHS_REG;
    let shift = RSP_ADJUST_TMP_REG;

    emit_load_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_load_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    asm.uxtw_x(rax, rax);
    asm.uxtw_x(rcx, rcx);

    asm.cbz_x_label(rax, leaf0);
    emit_u64_constant(asm, tmp, 1);
    asm.eor_x(tmp, rax, tmp);
    asm.cbz_x_label(tmp, leaf1);
    emit_u64_constant(asm, tmp, 7);
    asm.eor_x(tmp, rax, tmp);
    asm.cbz_x_label(tmp, leaf7);

    emit_u64_constant(asm, tmp, 31);
    asm.lsr_x(shift, rax, tmp);
    asm.cbnz_x_label(shift, other);

    emit_u64_constant(asm, tmp, 3);
    asm.lsr_x(shift, rax, tmp);
    asm.cbnz_x_label(shift, leaf7);

    asm.b_label(other);

    asm.bind_label(leaf7);
    asm.cbnz_x_label(rcx, other);
    emit_u64_constant(asm, rax, 0);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_LEAF7_EBX);
    emit_u64_constant(asm, rcx, 0);
    emit_u64_constant(asm, shift, 0);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(leaf1);
    emit_u64_constant(asm, rax, KSTATE_CPUID_LEAF1_EAX);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_LEAF1_EBX);
    emit_u64_constant(asm, rcx, KSTATE_CPUID_LEAF1_ECX);
    emit_u64_constant(asm, shift, KSTATE_CPUID_LEAF1_EDX);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(leaf0);
    emit_u64_constant(asm, rax, KSTATE_CPUID_MAX_LEAF);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_VENDOR_EBX);
    emit_u64_constant(asm, rcx, KSTATE_CPUID_VENDOR_ECX);
    emit_u64_constant(asm, shift, KSTATE_CPUID_VENDOR_EDX);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(other);
    emit_u64_constant(asm, rax, 0);
    emit_u64_constant(asm, tmp, 0);
    emit_u64_constant(asm, rcx, 0);
    emit_u64_constant(asm, shift, 0);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);

    asm.bind_label(done);
}

fn lower_xgetbv(asm: &mut Arm64Assembler, dst: u8) {
    let rcx = RSP_ADJUST_TMP_REG;
    let other = asm.create_label();
    let done = asm.create_label();

    emit_load_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    asm.uxtw_x(rcx, rcx);
    asm.cbnz_x_label(rcx, other);
    emit_u64_constant(asm, dst, KSTATE_XCR0_EAX);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rax);
    emit_u64_constant(asm, rcx, 0);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(other);
    emit_u64_constant(asm, dst, 0);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rdx);

    asm.bind_label(done);
}

fn lower_add_sub(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    constants: &HashMap<Ref, u64>,
    bin: &prisma_ir::BinOp,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let dst = value_reg(result);
    if let Some(rhs) = constants.get(&bin.rhs) {
        if let Ok(imm12) = u16::try_from(*rhs) {
            if imm12 < 4096 {
                match bin.op {
                    BinOpKind::Add => asm.add_x_imm(dst, lhs, imm12),
                    BinOpKind::Sub => asm.sub_x_imm(dst, lhs, imm12),
                    _ => unreachable!("called only for Add/Sub"),
                }
            } else {
                let rhs = *values
                    .get(&bin.rhs)
                    .ok_or(LowerError::MissingValue(bin.rhs))?;
                match bin.op {
                    BinOpKind::Add => asm.add_x(dst, lhs, rhs),
                    BinOpKind::Sub => asm.sub_x(dst, lhs, rhs),
                    _ => unreachable!("called only for Add/Sub"),
                }
            }
        } else {
            let rhs = *values
                .get(&bin.rhs)
                .ok_or(LowerError::MissingValue(bin.rhs))?;
            match bin.op {
                BinOpKind::Add => asm.add_x(dst, lhs, rhs),
                BinOpKind::Sub => asm.sub_x(dst, lhs, rhs),
                _ => unreachable!("called only for Add/Sub"),
            }
        }
    } else {
        let rhs = *values
            .get(&bin.rhs)
            .ok_or(LowerError::MissingValue(bin.rhs))?;
        match bin.op {
            BinOpKind::Add => asm.add_x(dst, lhs, rhs),
            BinOpKind::Sub => asm.sub_x(dst, lhs, rhs),
            _ => unreachable!("called only for Add/Sub"),
        }
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_reg_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    bin: &prisma_ir::BinOp,
    return_via_epilogue: bool,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let rhs = *values
        .get(&bin.rhs)
        .ok_or(LowerError::MissingValue(bin.rhs))?;
    let dst = value_reg(result);
    match bin.op {
        BinOpKind::And => asm.and_x(dst, lhs, rhs),
        BinOpKind::Or => asm.orr_x(dst, lhs, rhs),
        BinOpKind::Xor => asm.eor_x(dst, lhs, rhs),
        BinOpKind::Shl => asm.lsl_x(dst, lhs, rhs),
        BinOpKind::Shr => asm.lsr_x(dst, lhs, rhs),
        BinOpKind::Sar => asm.asr_x(dst, lhs, rhs),
        BinOpKind::Ror => asm.ror_x(dst, lhs, rhs),
        BinOpKind::Rol => {
            asm.sub_x(FLAG_ALIGN_SHIFT_REG, 31, rhs);
            asm.ror_x(dst, lhs, FLAG_ALIGN_SHIFT_REG);
        }
        BinOpKind::Mul => asm.mul_x(dst, lhs, rhs),
        BinOpKind::UMulHi => asm.umulh_x(dst, lhs, rhs),
        BinOpKind::SMulHi => asm.smulh_x(dst, lhs, rhs),
        BinOpKind::UDiv => {
            emit_divisor_zero_sigfpe_guard(asm, rhs, return_via_epilogue);
            asm.udiv_x(dst, lhs, rhs);
        }
        BinOpKind::SDiv => {
            emit_divisor_zero_sigfpe_guard(asm, rhs, return_via_epilogue);
            asm.sdiv_x(dst, lhs, rhs);
        }
        _ => unreachable!("called only for register-register binops"),
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_mod_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    bin: &prisma_ir::BinOp,
    return_via_epilogue: bool,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let rhs = *values
        .get(&bin.rhs)
        .ok_or(LowerError::MissingValue(bin.rhs))?;
    let dst = value_reg(result);
    emit_divisor_zero_sigfpe_guard(asm, rhs, return_via_epilogue);
    match bin.op {
        BinOpKind::UMod => asm.udiv_x(MOD_QUOTIENT_REG, lhs, rhs),
        BinOpKind::SMod => asm.sdiv_x(MOD_QUOTIENT_REG, lhs, rhs),
        _ => unreachable!("called only for modulo binops"),
    }
    asm.msub_x(dst, MOD_QUOTIENT_REG, rhs, lhs);
    values.insert(result, dst);
    Ok(())
}

fn emit_divisor_zero_sigfpe_guard(
    asm: &mut Arm64Assembler,
    divisor: u8,
    return_via_epilogue: bool,
) {
    let ok = asm.create_label();
    asm.cbnz_x_label(divisor, ok);
    emit_sigfpe_placeholder_return(asm, return_via_epilogue);
    asm.bind_label(ok);
}

fn emit_sigfpe_placeholder_return(asm: &mut Arm64Assembler, return_via_epilogue: bool) {
    asm.movz_x(0, 0, 0);
    if return_via_epilogue {
        abi::emit_block_epilogue_and_ret(asm);
    } else {
        asm.ret();
    }
}

fn lower_wide_div(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    div: &prisma_ir::WideDiv,
    return_via_epilogue: bool,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("WideDiv"))?;
    let high = *values
        .get(&div.high)
        .ok_or(LowerError::MissingValue(div.high))?;
    let low = *values
        .get(&div.low)
        .ok_or(LowerError::MissingValue(div.low))?;
    let divisor = *values
        .get(&div.divisor)
        .ok_or(LowerError::MissingValue(div.divisor))?;
    let dst = value_reg(result);

    emit_divisor_zero_sigfpe_guard(asm, divisor, return_via_epilogue);
    asm.mov_x(WIDE_REM_REG, high);
    asm.mov_x(WIDE_LOW_REG, low);
    asm.mov_x(WIDE_DIVISOR_REG, divisor);

    if div.signed {
        lower_wide_div_abs_signed_inputs(asm);
        emit_unsigned_wide_div_overflow_guard(
            asm,
            WIDE_REM_REG,
            WIDE_DIVISOR_REG,
            return_via_epilogue,
        );
    } else {
        emit_unsigned_wide_div_overflow_guard(asm, high, divisor, return_via_epilogue);
        emit_u64_constant(asm, WIDE_QUOT_SIGN_REG, 0);
        emit_u64_constant(asm, WIDE_REM_SIGN_REG, 0);
    }

    lower_wide_udiv_core(asm, dst);
    if div.signed {
        emit_signed_wide_div_quotient_overflow_guard(asm, dst, return_via_epilogue);
    }

    if div.result == prisma_ir::WideDivResult::Remainder {
        asm.mov_x(dst, WIDE_REM_REG);
    }

    if div.signed {
        match div.result {
            prisma_ir::WideDivResult::Quotient => {
                let done = asm.create_label();
                asm.cbz_x_label(WIDE_QUOT_SIGN_REG, done);
                asm.sub_x(dst, 31, dst);
                asm.bind_label(done);
            }
            prisma_ir::WideDivResult::Remainder => {
                let done = asm.create_label();
                asm.cbz_x_label(WIDE_REM_SIGN_REG, done);
                asm.sub_x(dst, 31, dst);
                asm.bind_label(done);
            }
        }
    }

    values.insert(result, dst);
    Ok(())
}

fn emit_unsigned_wide_div_overflow_guard(
    asm: &mut Arm64Assembler,
    high: u8,
    divisor: u8,
    return_via_epilogue: bool,
) {
    let ok = asm.create_label();
    asm.cmp_x(high, divisor);
    asm.b_cond_label(prisma_ir::CondCode::Ult, ok);
    emit_sigfpe_placeholder_return(asm, return_via_epilogue);
    asm.bind_label(ok);
}

fn emit_signed_wide_div_quotient_overflow_guard(
    asm: &mut Arm64Assembler,
    quotient: u8,
    return_via_epilogue: bool,
) {
    let negative = asm.create_label();
    let done = asm.create_label();

    emit_u64_constant(asm, WIDE_SHIFT_REG, 63);
    asm.cbnz_x_label(WIDE_QUOT_SIGN_REG, negative);
    asm.lsr_x(WIDE_TMP_REG, quotient, WIDE_SHIFT_REG);
    asm.cbz_x_label(WIDE_TMP_REG, done);
    emit_sigfpe_placeholder_return(asm, return_via_epilogue);

    asm.bind_label(negative);
    emit_u64_constant(asm, WIDE_MASK_REG, 1_u64 << 63);
    asm.cmp_x(quotient, WIDE_MASK_REG);
    asm.b_cond_label(prisma_ir::CondCode::Ule, done);
    emit_sigfpe_placeholder_return(asm, return_via_epilogue);

    asm.bind_label(done);
}

fn lower_wide_div_abs_signed_inputs(asm: &mut Arm64Assembler) {
    emit_u64_constant(asm, WIDE_SHIFT_REG, 63);
    asm.lsr_x(WIDE_REM_SIGN_REG, WIDE_REM_REG, WIDE_SHIFT_REG);
    asm.lsr_x(WIDE_TMP_REG, WIDE_DIVISOR_REG, WIDE_SHIFT_REG);
    asm.eor_x(WIDE_QUOT_SIGN_REG, WIDE_REM_SIGN_REG, WIDE_TMP_REG);

    let dividend_low_zero = asm.create_label();
    let dividend_done = asm.create_label();
    asm.cbz_x_label(WIDE_REM_SIGN_REG, dividend_done);
    emit_u64_constant(asm, WIDE_MASK_REG, u64::MAX);
    asm.cbz_x_label(WIDE_LOW_REG, dividend_low_zero);
    asm.sub_x(WIDE_LOW_REG, 31, WIDE_LOW_REG);
    asm.eor_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_MASK_REG);
    asm.b_label(dividend_done);
    asm.bind_label(dividend_low_zero);
    asm.sub_x(WIDE_REM_REG, 31, WIDE_REM_REG);
    asm.bind_label(dividend_done);

    let divisor_positive = asm.create_label();
    asm.cbz_x_label(WIDE_TMP_REG, divisor_positive);
    asm.sub_x(WIDE_DIVISOR_REG, 31, WIDE_DIVISOR_REG);
    asm.bind_label(divisor_positive);
}

fn lower_wide_udiv_core(asm: &mut Arm64Assembler, quotient_dst: u8) {
    emit_u64_constant(asm, WIDE_ONE_REG, 1);
    emit_u64_constant(asm, WIDE_SHIFT_REG, 63);
    emit_u64_constant(asm, WIDE_MASK_REG, 1_u64 << 63);
    emit_u64_constant(asm, quotient_dst, 0);

    for _ in 0..64 {
        let subtract = asm.create_label();
        let done = asm.create_label();

        asm.lsr_x(WIDE_BIT_REG, WIDE_LOW_REG, WIDE_SHIFT_REG);
        asm.lsl_x(WIDE_LOW_REG, WIDE_LOW_REG, WIDE_ONE_REG);
        asm.lsr_x(WIDE_TMP_REG, WIDE_REM_REG, WIDE_SHIFT_REG);
        asm.lsl_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_ONE_REG);
        asm.orr_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_BIT_REG);

        asm.cbnz_x_label(WIDE_TMP_REG, subtract);
        asm.cmp_x(WIDE_REM_REG, WIDE_DIVISOR_REG);
        asm.b_cond_label(prisma_ir::CondCode::Uge, subtract);
        asm.b_label(done);

        asm.bind_label(subtract);
        asm.sub_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_DIVISOR_REG);
        asm.orr_x(quotient_dst, quotient_dst, WIDE_MASK_REG);

        asm.bind_label(done);
        asm.lsr_x(WIDE_MASK_REG, WIDE_MASK_REG, WIDE_ONE_REG);
    }
}

fn lower_compare(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    compare: &prisma_ir::Compare,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("Compare"))?;
    let lhs = *values
        .get(&compare.lhs)
        .ok_or(LowerError::MissingValue(compare.lhs))?;
    let rhs = *values
        .get(&compare.rhs)
        .ok_or(LowerError::MissingValue(compare.rhs))?;
    let dst = value_reg(result);
    let (lhs, rhs) = align_flag_operands(asm, compare.size, lhs, rhs);
    asm.cmp_x(lhs, rhs);
    asm.cset_x(dst, compare.cc);
    values.insert(result, dst);
    Ok(())
}

fn lower_cmp_flags(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    flags: &mut HashSet<Ref>,
    cmp: &prisma_ir::CmpFlags,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("CmpFlags"))?;
    let lhs = *values
        .get(&cmp.lhs)
        .ok_or(LowerError::MissingValue(cmp.lhs))?;
    let rhs = *values
        .get(&cmp.rhs)
        .ok_or(LowerError::MissingValue(cmp.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, cmp.size, lhs, rhs);
    asm.cmp_x(lhs, rhs);
    flags.insert(result);
    Ok(())
}

fn lower_alu_flags(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    alu: &prisma_ir::AluFlags,
) -> Result<(), LowerError> {
    let lhs = *values
        .get(&alu.lhs)
        .ok_or(LowerError::MissingValue(alu.lhs))?;
    let rhs = *values
        .get(&alu.rhs)
        .ok_or(LowerError::MissingValue(alu.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, alu.size, lhs, rhs);
    match alu.op {
        BinOpKind::Sub => asm.cmp_x(lhs, rhs),
        BinOpKind::Add => asm.adds_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        BinOpKind::And => emit_logical_flags_and(asm, lhs, rhs),
        // x86 logical ops clear CF/OF. Direct unsigned Jcc lowering interprets
        // ARM C as !x86-CF, so comparing the result with zero preserves N/Z,
        // forces ARM C=1 (x86 CF=0), and clears V.
        BinOpKind::Or => emit_logical_flags_or_xor(asm, lhs, rhs, false),
        BinOpKind::Xor => emit_logical_flags_or_xor(asm, lhs, rhs, true),
        _ => {
            return Err(LowerError::UnsupportedOp(
                "AluFlags only supports Sub/Add/And/Or/Xor today",
            ));
        }
    }
    Ok(())
}

fn lower_alu_flags_preserve_carry(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    alu: &prisma_ir::AluFlagsPreserveCarry,
) -> Result<(), LowerError> {
    let lhs = *values
        .get(&alu.lhs)
        .ok_or(LowerError::MissingValue(alu.lhs))?;
    let rhs = *values
        .get(&alu.rhs)
        .ok_or(LowerError::MissingValue(alu.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, alu.size, lhs, rhs);
    match alu.op {
        BinOpKind::Sub => asm.cmp_x(lhs, rhs),
        BinOpKind::Add => asm.adds_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        _ => {
            return Err(LowerError::UnsupportedOp(
                "AluFlagsPreserveCarry only supports Sub/Add today",
            ));
        }
    }

    asm.mrs_nzcv(NZCV_TMP_REG);
    emit_u64_constant(asm, NZCV_MASK_REG, !(1_u64 << 29));
    asm.and_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_MASK_REG);
    asm.ldr_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET);
    emit_u64_constant(asm, NZCV_MASK_REG, 1);
    asm.and_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 29);
    asm.lsl_x(NZCV_CARRY_REG, NZCV_CARRY_REG, FLAG_ALIGN_SHIFT_REG);
    asm.orr_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_CARRY_REG);
    asm.msr_nzcv(NZCV_TMP_REG);

    Ok(())
}

fn ensure_nzcv_live(asm: &mut Arm64Assembler, nzcv_live: &mut bool) {
    if !*nzcv_live {
        lower_restore_nzcv_from_rflags(asm);
        *nzcv_live = true;
    }
}

fn lower_restore_nzcv_from_rflags(asm: &mut Arm64Assembler) {
    asm.ldr_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
    // ARM subtraction records C as "no borrow", while x86 records CF as
    // "borrow". Seed ARM C and toggle it with the persisted x86 CF below.
    emit_u64_constant(asm, NZCV_CARRY_REG, 1_u64 << 29);
    emit_or_rflags_bit_into_nzcv(asm, 7, 31); // SF -> N
    emit_or_rflags_bit_into_nzcv(asm, 6, 30); // ZF -> Z
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 0);
    asm.lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 1);
    asm.and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 29);
    asm.lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
    asm.eor_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
    emit_or_rflags_bit_into_nzcv(asm, 11, 28); // OF -> V
    asm.msr_nzcv(NZCV_CARRY_REG);
}

fn emit_or_rflags_bit_into_nzcv(asm: &mut Arm64Assembler, rflags_shift: u64, nzcv_shift: u64) {
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, rflags_shift);
    asm.lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 1);
    asm.and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, nzcv_shift);
    asm.lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
    asm.orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
}

fn lower_store_carry(asm: &mut Arm64Assembler, value: u8) {
    emit_u64_constant(asm, NZCV_MASK_REG, 1);
    asm.and_x(NZCV_CARRY_REG, value, NZCV_MASK_REG);
    asm.str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET);

    asm.ldr_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
    emit_u64_constant(asm, NZCV_MASK_REG, !1_u64);
    asm.and_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_MASK_REG);
    asm.orr_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_CARRY_REG);
    asm.str_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
}

fn lower_store_rflags(asm: &mut Arm64Assembler, value: u8) {
    emit_u64_constant(asm, NZCV_TMP_REG, 2);
    asm.orr_x(NZCV_TMP_REG, value, NZCV_TMP_REG);
    asm.str_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);

    emit_u64_constant(asm, NZCV_MASK_REG, 1);
    asm.and_x(NZCV_CARRY_REG, NZCV_TMP_REG, NZCV_MASK_REG);
    asm.str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET);
}

fn lower_write_flags_fp64(asm: &mut Arm64Assembler, lhs: u8, rhs: u8) {
    asm.fmov_d_x(0, lhs);
    asm.fmov_d_x(1, rhs);
    asm.fcmp_d(0, 1);
    asm.mrs_nzcv(FLAG_ALIGN_LHS_REG);
    emit_u64_constant(asm, FLAG_ALIGN_RHS_REG, 0);
    emit_u64_constant(asm, MEM_ADDR_SCRATCH, 1);

    // ARM FCMP: less=N, equal=Z, unordered=V. x86 UCOMI requires
    // CF=N|V, ZF=Z|V, PF=V; greater clears all three.
    emit_nzcv_bit_into_rflags(asm, 31, 0);
    emit_nzcv_bit_into_rflags(asm, 28, 0);
    emit_nzcv_bit_into_rflags(asm, 30, 6);
    emit_nzcv_bit_into_rflags(asm, 28, 6);
    emit_nzcv_bit_into_rflags(asm, 28, 2);
    lower_store_rflags(asm, FLAG_ALIGN_RHS_REG);
}

fn emit_nzcv_bit_into_rflags(asm: &mut Arm64Assembler, nzcv_shift: u8, rflags_shift: u8) {
    asm.lsr_x_imm(CAS_STATUS_REG, FLAG_ALIGN_LHS_REG, nzcv_shift);
    asm.and_x(CAS_STATUS_REG, CAS_STATUS_REG, MEM_ADDR_SCRATCH);
    if rflags_shift != 0 {
        asm.lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, rflags_shift);
    }
    asm.orr_x(FLAG_ALIGN_RHS_REG, FLAG_ALIGN_RHS_REG, CAS_STATUS_REG);
}

fn lower_store_rflags_from_nzcv(
    asm: &mut Arm64Assembler,
    carry: RflagsCarryMode,
    pf: Option<u8>,
    af: Option<u8>,
) {
    asm.mrs_nzcv(NZCV_TMP_REG);
    asm.ldr_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);

    let mut clear_bits = RFLAGS_ZF_BIT | RFLAGS_SF_BIT | RFLAGS_OF_BIT;
    if !matches!(carry, RflagsCarryMode::Preserve) {
        clear_bits |= RFLAGS_CF_BIT;
    }
    if pf.is_some() {
        clear_bits |= RFLAGS_PF_BIT;
    }
    if af.is_some() {
        clear_bits |= RFLAGS_AF_BIT;
    }
    let clear_mask = match carry {
        RflagsCarryMode::Preserve
        | RflagsCarryMode::ArmCarry
        | RflagsCarryMode::InvertArmCarry
        | RflagsCarryMode::Clear => !clear_bits,
    };
    emit_u64_constant(asm, NZCV_MASK_REG, clear_mask);
    asm.and_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
    emit_u64_constant(asm, NZCV_MASK_REG, 2);
    asm.orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);

    emit_or_nzcv_bit_into_rflags(asm, 30, 6); // Z -> ZF
    emit_or_nzcv_bit_into_rflags(asm, 31, 7); // N -> SF
    emit_or_nzcv_bit_into_rflags(asm, 28, 11); // V -> OF
    if let Some(pf) = pf {
        emit_or_ref_bit_into_rflags(asm, pf, 2);
    }
    if let Some(af) = af {
        emit_or_ref_bit_into_rflags(asm, af, 4);
    }

    match carry {
        RflagsCarryMode::ArmCarry => emit_or_nzcv_bit_into_rflags(asm, 29, 0),
        RflagsCarryMode::InvertArmCarry => {
            emit_extract_nzcv_bit(asm, 29);
            emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 1);
            asm.eor_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
            emit_or_extracted_bit_into_rflags(asm, 0);
        }
        RflagsCarryMode::Clear | RflagsCarryMode::Preserve => {}
    }

    asm.str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
    emit_u64_constant(asm, NZCV_MASK_REG, 1);
    asm.and_x(NZCV_MASK_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
    asm.str_x_unsigned(NZCV_MASK_REG, abi::K_STATE_PTR_REG, CF_OFFSET);
}

fn emit_or_nzcv_bit_into_rflags(asm: &mut Arm64Assembler, nzcv_shift: u64, rflags_shift: u64) {
    emit_extract_nzcv_bit(asm, nzcv_shift);
    emit_or_extracted_bit_into_rflags(asm, rflags_shift);
}

fn emit_extract_nzcv_bit(asm: &mut Arm64Assembler, nzcv_shift: u64) {
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, nzcv_shift);
    asm.lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG);
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 1);
    asm.and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
}

fn emit_or_extracted_bit_into_rflags(asm: &mut Arm64Assembler, rflags_shift: u64) {
    if rflags_shift != 0 {
        emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, rflags_shift);
        asm.lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG);
    }
    asm.orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
}

fn lower_store_rflags_from_bits(
    asm: &mut Arm64Assembler,
    pf: Option<u8>,
    af: Option<u8>,
    zf: u8,
    sf: u8,
    of: u8,
) {
    asm.ldr_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
    let mut clear_mask = RFLAGS_ZF_BIT | RFLAGS_SF_BIT | RFLAGS_OF_BIT;
    if pf.is_some() {
        clear_mask |= RFLAGS_PF_BIT;
    }
    if af.is_some() {
        clear_mask |= RFLAGS_AF_BIT;
    }
    emit_u64_constant(asm, NZCV_MASK_REG, !clear_mask);
    asm.and_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);
    emit_u64_constant(asm, NZCV_MASK_REG, 2);
    asm.orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG);

    if let Some(pf) = pf {
        emit_or_ref_bit_into_rflags(asm, pf, 2);
    }
    if let Some(af) = af {
        emit_or_ref_bit_into_rflags(asm, af, 4);
    }
    emit_or_ref_bit_into_rflags(asm, zf, 6);
    emit_or_ref_bit_into_rflags(asm, sf, 7);
    emit_or_ref_bit_into_rflags(asm, of, 11);

    asm.str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET);
}

fn emit_or_ref_bit_into_rflags(asm: &mut Arm64Assembler, value: u8, rflags_shift: u64) {
    emit_u64_constant(asm, NZCV_MASK_REG, 1);
    asm.and_x(NZCV_MASK_REG, value, NZCV_MASK_REG);
    emit_or_extracted_bit_into_rflags(asm, rflags_shift);
}

/// Emit a flag-setting OR (`is_xor == false`) or XOR over the flag-aligned
/// operands. ARM64 has no `orrs`/`eors`, so we compute into the ALU flags
/// scratch register and compare with zero to publish x86-compatible N/Z/C/V.
fn emit_logical_flags_and(asm: &mut Arm64Assembler, lhs: u8, rhs: u8) {
    asm.and_x(ALU_FLAGS_TMP_REG, lhs, rhs);
    asm.cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG);
}

fn emit_logical_flags_or_xor(asm: &mut Arm64Assembler, lhs: u8, rhs: u8, is_xor: bool) {
    if is_xor {
        asm.eor_x(ALU_FLAGS_TMP_REG, lhs, rhs);
    } else {
        asm.orr_x(ALU_FLAGS_TMP_REG, lhs, rhs);
    }
    asm.cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG);
}

fn lower_load_seg_base(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    seg: &prisma_ir::LoadSegBase,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("LoadSegBase"))?;
    let dst = value_reg(result);
    match seg.seg {
        prisma_ir::SegmentReg::Fs => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, FS_BASE_OFFSET),
        prisma_ir::SegmentReg::Gs => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, GS_BASE_OFFSET),
        prisma_ir::SegmentReg::Es
        | prisma_ir::SegmentReg::Cs
        | prisma_ir::SegmentReg::Ss
        | prisma_ir::SegmentReg::Ds => emit_u64_constant(asm, dst, 0),
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_extend(
    asm: &mut Arm64Assembler,
    dst: u8,
    src: u8,
    from_size: OpSize,
    to_size: OpSize,
    is_signed: bool,
) {
    if is_signed {
        match from_size {
            OpSize::I8 => asm.sxtb_x(dst, src),
            OpSize::I16 => asm.sxth_x(dst, src),
            OpSize::I32 => asm.sxtw_x(dst, src),
            OpSize::I64 => asm.mov_x(dst, src),
        }
        if to_size != OpSize::I64 {
            lower_truncate(asm, dst, dst, to_size);
        }
    } else {
        lower_truncate(asm, dst, src, from_size);
    }
}

fn lower_truncate(asm: &mut Arm64Assembler, dst: u8, src: u8, to_size: OpSize) {
    match to_size {
        OpSize::I8 => asm.uxtb_x(dst, src),
        OpSize::I16 => asm.uxth_x(dst, src),
        OpSize::I32 => asm.uxtw_x(dst, src),
        OpSize::I64 => asm.mov_x(dst, src),
    }
}

fn lower_write_flags(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    flags: &mut HashSet<Ref>,
    write: &prisma_ir::WriteFlags,
    stmt: &Stmt,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("WriteFlags"))?;
    let lhs = *values
        .get(&write.lhs)
        .ok_or(LowerError::MissingValue(write.lhs))?;
    let rhs = *values
        .get(&write.rhs)
        .ok_or(LowerError::MissingValue(write.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, write.size, lhs, rhs);
    match write.op {
        BinOpKind::Sub => asm.cmp_x(lhs, rhs),
        BinOpKind::Add => asm.adds_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        BinOpKind::And => emit_logical_flags_and(asm, lhs, rhs),
        BinOpKind::Or => emit_logical_flags_or_xor(asm, lhs, rhs, false),
        BinOpKind::Xor => emit_logical_flags_or_xor(asm, lhs, rhs, true),
        _ => {
            return Err(LowerError::UnsupportedOp(
                "WriteFlags only supports Sub/Add/And/Or/Xor today",
            ));
        }
    }
    flags.insert(result);
    Ok(())
}

fn lower_read_flag(
    asm: &mut Arm64Assembler,
    flags: &HashSet<Ref>,
    read: &prisma_ir::ReadFlag,
    dst: u8,
) -> Result<(), LowerError> {
    if !flags.contains(&read.flags) {
        return Err(LowerError::MissingValue(read.flags));
    }
    let cc = match read.which {
        prisma_ir::FlagBit::Carry => prisma_ir::CondCode::Cc,
        prisma_ir::FlagBit::Zero => prisma_ir::CondCode::Eq,
        prisma_ir::FlagBit::Sign => prisma_ir::CondCode::Mi,
        prisma_ir::FlagBit::Overflow => prisma_ir::CondCode::Ov,
        prisma_ir::FlagBit::Parity | prisma_ir::FlagBit::Aux => {
            return Err(LowerError::UnsupportedOp(
                "ReadFlag(Parity/Aux) needs software emulation",
            ));
        }
    };
    asm.cset_x(dst, cc);
    Ok(())
}

/// A guest `SYSCALL` exits the block back to the host so it can be serviced.
/// The syscall number + args are already in the guest GPRs (rax/rdi/...); this
/// records the exit reason in the state frame and returns. The host services the
/// syscall, writes the result to rax, advances the guest PC past the 2-byte
/// `SYSCALL`, and re-enters at the next block.
///
/// `x9` is a caller-saved scratch — the block is exiting, so clobbering it is
/// safe (the epilogue restores only the callee-saved registers).
fn lower_syscall(asm: &mut Arm64Assembler, return_via_epilogue: bool) {
    const SCRATCH: u8 = 9;
    asm.movz_x(SCRATCH, EXIT_SYSCALL_MARK, 0);
    asm.str_x_unsigned(SCRATCH, abi::K_STATE_PTR_REG, EXIT_REASON_OFFSET);
    if return_via_epilogue {
        abi::emit_block_epilogue_and_ret(asm);
    } else {
        asm.ret();
    }
}

fn lower_lzcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => asm.clz_x(dst, src),
        OpSize::I32 => asm.clz_w(dst, src),
        OpSize::I16 | OpSize::I8 => {
            let shift = u16::try_from(64 - size.bit_width()).expect("small size shift fits");
            lower_truncate(asm, dst, src, size);
            asm.movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0);
            asm.lsl_x(dst, dst, FLAG_ALIGN_SHIFT_REG);
            asm.clz_x(dst, dst);
            clamp_count_to_width(asm, dst, size);
        }
    }
}

fn lower_tzcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => {
            asm.rbit_x(dst, src);
            asm.clz_x(dst, dst);
        }
        OpSize::I32 => {
            asm.rbit_w(dst, src);
            asm.clz_w(dst, dst);
        }
        OpSize::I16 | OpSize::I8 => {
            lower_truncate(asm, dst, src, size);
            asm.rbit_w(dst, dst);
            asm.clz_w(dst, dst);
            clamp_count_to_width(asm, dst, size);
        }
    }
}

fn lower_popcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    let tmp = FLAG_ALIGN_LHS_REG;
    let mask = FLAG_ALIGN_RHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;
    let mul = RSP_ADJUST_TMP_REG;

    lower_truncate(asm, dst, src, size);

    emit_u64_constant(asm, shift, 1);
    asm.lsr_x(tmp, dst, shift);
    emit_u64_constant(asm, mask, 0x5555_5555_5555_5555);
    asm.and_x(tmp, tmp, mask);
    asm.sub_x(dst, dst, tmp);

    emit_u64_constant(asm, shift, 2);
    asm.lsr_x(tmp, dst, shift);
    emit_u64_constant(asm, mask, 0x3333_3333_3333_3333);
    asm.and_x(dst, dst, mask);
    asm.and_x(tmp, tmp, mask);
    asm.add_x(dst, dst, tmp);

    emit_u64_constant(asm, shift, 4);
    asm.lsr_x(tmp, dst, shift);
    asm.add_x(dst, dst, tmp);
    emit_u64_constant(asm, mask, 0x0f0f_0f0f_0f0f_0f0f);
    asm.and_x(dst, dst, mask);

    emit_u64_constant(asm, mul, 0x0101_0101_0101_0101);
    asm.mul_x(dst, dst, mul);
    emit_u64_constant(asm, shift, 56);
    asm.lsr_x(dst, dst, shift);
}

fn clamp_count_to_width(asm: &mut Arm64Assembler, dst: u8, size: OpSize) {
    let done = asm.create_label();
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, u64::from(size.bit_width()));
    asm.cmp_x(dst, FLAG_ALIGN_SHIFT_REG);
    asm.b_cond_label(prisma_ir::CondCode::Ule, done);
    asm.mov_x(dst, FLAG_ALIGN_SHIFT_REG);
    asm.bind_label(done);
}

fn align_flag_operands(asm: &mut Arm64Assembler, size: OpSize, lhs: u8, rhs: u8) -> (u8, u8) {
    if size == OpSize::I64 {
        return (lhs, rhs);
    }

    let shift = u16::try_from(64 - size.bit_width()).expect("OpSize bit width is <= 64");
    asm.movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0);
    asm.lsl_x(FLAG_ALIGN_LHS_REG, lhs, FLAG_ALIGN_SHIFT_REG);
    asm.lsl_x(FLAG_ALIGN_RHS_REG, rhs, FLAG_ALIGN_SHIFT_REG);
    (FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG)
}

fn lower_cond_jump(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    values: &HashMap<Ref, u8>,
    jump: &prisma_ir::CondJump,
) -> Result<(), LowerError> {
    let cond = *values
        .get(&jump.cond)
        .ok_or(LowerError::MissingValue(jump.cond))?;
    let if_true = labels
        .get(&jump.if_true)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_true))?;
    let if_false = labels
        .get(&jump.if_false)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_false))?;
    asm.cbnz_x_label(cond, if_true);
    asm.b_label(if_false);
    Ok(())
}

fn lower_cond_jump_flags(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    flags: &HashSet<Ref>,
    jump: &prisma_ir::CondJumpFlags,
) -> Result<(), LowerError> {
    if !flags.contains(&jump.flags) {
        return Err(LowerError::MissingValue(jump.flags));
    }
    let if_true = labels
        .get(&jump.if_true)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_true))?;
    let if_false = labels
        .get(&jump.if_false)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_false))?;
    asm.b_cond_label(jump.cc, if_true);
    asm.b_label(if_false);
    Ok(())
}

fn lower_cond_jump_rel(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    jump: &prisma_ir::CondJumpRel,
) -> Result<(), LowerError> {
    let if_true = block_label(labels, jump.target_guest_pc)?;
    let if_false = block_label(labels, jump.fallthrough_guest_pc)?;
    asm.b_cond_label(jump.cc, if_true);
    asm.b_label(if_false);
    Ok(())
}

/// Scratch registers for the branch block-exit sequence. Both are value-register
/// slots, dead at a terminator (all guest state is already stored to the frame),
/// so clobbering them is safe — the same reasoning `lower_syscall` uses for x9.
const BRANCH_EXIT_TARGET_REG: u8 = 9;
const BRANCH_EXIT_FALL_REG: u8 = 10;
// A return target is loaded from guest memory after every ordinary SSA value
// is dead. Keep it out of the cyclic x9..x16 value pool all the way through the
// frame write: long fused blocks can otherwise leave x9 participating in the
// final guest-memory sequence, which is needlessly fragile on ARM64EC.
const RETURN_EXIT_TARGET_REG: u8 = WIDE_SHIFT_REG;

// REP string operations are terminators, so all transient SSA values are dead
// when this bounded native loop starts. These callee-saved scratch registers
// are preserved by the block wrapper and deliberately avoid x18 (Windows TEB),
// x24 (memory rebasing), and x27 (the CpuStateFrame pointer).
const REP_RCX_REG: u8 = 19;
const REP_RDI_REG: u8 = 20;
const REP_RSI_REG: u8 = 21;
const REP_MAX_REG: u8 = 22;
const REP_ITER_REG: u8 = 23;
const REP_VALUE_REG: u8 = 25;

#[allow(clippy::too_many_arguments)]
fn lower_rep_string(
    asm: &mut Arm64Assembler,
    size: OpSize,
    reverse: bool,
    pc_of_rep: u64,
    pc_after_rep: u64,
    copy_source: bool,
    exit: ExitAbi,
) {
    asm.ldr_x_unsigned(
        REP_RCX_REG,
        abi::K_STATE_PTR_REG,
        gpr_offset_bytes(Gpr::Rcx),
    );
    asm.ldr_x_unsigned(
        REP_RDI_REG,
        abi::K_STATE_PTR_REG,
        gpr_offset_bytes(Gpr::Rdi),
    );
    if copy_source {
        asm.ldr_x_unsigned(
            REP_RSI_REG,
            abi::K_STATE_PTR_REG,
            gpr_offset_bytes(Gpr::Rsi),
        );
    } else {
        asm.ldr_x_unsigned(
            REP_VALUE_REG,
            abi::K_STATE_PTR_REG,
            gpr_offset_bytes(Gpr::Rax),
        );
    }

    let done = asm.create_label();
    asm.cbz_x_label(REP_RCX_REG, done);
    let step = match size {
        OpSize::I8 => 1_u16,
        OpSize::I16 => 2,
        OpSize::I32 => 4,
        OpSize::I64 => 8,
    };
    let iteration_cap = prisma_ir::REP_MAX_BYTES_PER_CALL / u64::from(step);
    emit_u64_constant(asm, REP_MAX_REG, iteration_cap);
    asm.cmp_x(REP_RCX_REG, REP_MAX_REG);
    asm.csel_x(
        REP_ITER_REG,
        REP_RCX_REG,
        REP_MAX_REG,
        prisma_ir::CondCode::Ult,
    );
    asm.sub_x(REP_RCX_REG, REP_RCX_REG, REP_ITER_REG);

    let loop_body = asm.create_label();
    asm.bind_label(loop_body);
    if copy_source {
        emit_load_mem(asm, size, REP_VALUE_REG, REP_RSI_REG);
    }
    emit_store_mem(asm, size, REP_VALUE_REG, REP_RDI_REG);
    if reverse {
        asm.sub_x_imm(REP_RDI_REG, REP_RDI_REG, step);
        if copy_source {
            asm.sub_x_imm(REP_RSI_REG, REP_RSI_REG, step);
        }
    } else {
        asm.add_x_imm(REP_RDI_REG, REP_RDI_REG, step);
        if copy_source {
            asm.add_x_imm(REP_RSI_REG, REP_RSI_REG, step);
        }
    }
    asm.sub_x_imm(REP_ITER_REG, REP_ITER_REG, 1);
    asm.cbnz_x_label(REP_ITER_REG, loop_body);
    asm.bind_label(done);

    asm.str_x_unsigned(
        REP_RCX_REG,
        abi::K_STATE_PTR_REG,
        gpr_offset_bytes(Gpr::Rcx),
    );
    asm.str_x_unsigned(
        REP_RDI_REG,
        abi::K_STATE_PTR_REG,
        gpr_offset_bytes(Gpr::Rdi),
    );
    if copy_source {
        asm.str_x_unsigned(
            REP_RSI_REG,
            abi::K_STATE_PTR_REG,
            gpr_offset_bytes(Gpr::Rsi),
        );
    }

    emit_u64_constant(asm, BRANCH_EXIT_TARGET_REG, pc_after_rep);
    let target_ready = asm.create_label();
    asm.cbz_x_label(REP_RCX_REG, target_ready);
    emit_u64_constant(asm, BRANCH_EXIT_TARGET_REG, pc_of_rep);
    asm.bind_label(target_ready);
    if exit.branch_via_frame {
        emit_branch_exit(asm);
    } else {
        asm.mov_x(0, BRANCH_EXIT_TARGET_REG);
        if exit.return_via_epilogue {
            abi::emit_block_epilogue_and_ret(asm);
        } else {
            asm.ret();
        }
    }
}

/// Record `next_pc` + `EXIT_BRANCH` in the frame and return to the host run loop.
/// `MOVZ`/`MOVK` (constant emission) and `STR` do not touch NZCV, so a preceding
/// condition stays live for [`lower_cond_jump_rel_exit`]'s `CSEL`.
fn emit_branch_exit(asm: &mut Arm64Assembler) {
    emit_branch_exit_from(asm, BRANCH_EXIT_TARGET_REG);
}

fn emit_branch_exit_from(asm: &mut Arm64Assembler, target_reg: u8) {
    asm.str_x_unsigned(target_reg, abi::K_STATE_PTR_REG, NEXT_PC_OFFSET);
    asm.movz_x(BRANCH_EXIT_TARGET_REG, EXIT_BRANCH_MARK, 0);
    asm.str_x_unsigned(
        BRANCH_EXIT_TARGET_REG,
        abi::K_STATE_PTR_REG,
        EXIT_REASON_OFFSET,
    );
    abi::emit_block_epilogue_and_ret(asm);
}

/// Block-exit lowering for an unconditional `JumpRel`: store the target guest PC
/// and exit to the run loop.
fn lower_jump_rel_exit(asm: &mut Arm64Assembler, target_guest_pc: u64) {
    emit_u64_constant(asm, BRANCH_EXIT_TARGET_REG, target_guest_pc);
    emit_branch_exit(asm);
}

/// Block-exit lowering for a `CondJumpRel`: select the taken target (`CSEL` on
/// the live NZCV the block's compare set), store it, and exit to the run loop.
fn lower_cond_jump_rel_exit(asm: &mut Arm64Assembler, jump: &prisma_ir::CondJumpRel) {
    emit_u64_constant(asm, BRANCH_EXIT_TARGET_REG, jump.target_guest_pc);
    emit_u64_constant(asm, BRANCH_EXIT_FALL_REG, jump.fallthrough_guest_pc);
    asm.csel_x(
        BRANCH_EXIT_TARGET_REG,
        BRANCH_EXIT_TARGET_REG,
        BRANCH_EXIT_FALL_REG,
        jump.cc,
    );
    emit_branch_exit(asm);
}

/// Block-exit lowering for a `CallRel`: push the return address onto the guest
/// stack (real memory since RFC 0020) and exit to the run loop at the call
/// target. The guest stack *is* the return-address stack — a later `Return`
/// pops what this pushes — so cross-block calls chain through the run loop with
/// no host-side call frame.
fn lower_call_rel_exit(
    asm: &mut Arm64Assembler,
    target_guest_pc: u64,
    return_guest_pc: u64,
) -> Result<(), LowerError> {
    // rsp -= 8
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, -8)?;
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    // [rsp] = return address (rebased to host through mem_base by emit_store_mem)
    emit_u64_constant(asm, BRANCH_EXIT_FALL_REG, return_guest_pc);
    emit_store_mem(asm, OpSize::I64, BRANCH_EXIT_FALL_REG, RSP_ADJUST_TMP_REG);
    // Exit to the call target.
    emit_u64_constant(asm, BRANCH_EXIT_TARGET_REG, target_guest_pc);
    emit_branch_exit(asm);
    Ok(())
}

/// Block-exit lowering for a `Return` (and `RetAdjusted`): pop the return address
/// off the guest stack and exit to the run loop there. `rsp_delta` is the total
/// bytes to add back to RSP — 8 for a plain `ret`, `pop_bytes` (which already
/// includes the 8) for `ret imm16`.
fn lower_return_exit(asm: &mut Arm64Assembler, rsp_delta: u64) -> Result<(), LowerError> {
    // target = [rsp] (rebased to host through mem_base by emit_load_mem)
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_load_mem(asm, OpSize::I64, RETURN_EXIT_TARGET_REG, RSP_ADJUST_TMP_REG);
    // rsp += rsp_delta
    let delta = i64::try_from(rsp_delta).map_err(|_| LowerError::ImmediateOutOfRange(rsp_delta))?;
    emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, delta)?;
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_branch_exit_from(asm, RETURN_EXIT_TARGET_REG);
    Ok(())
}

/// Block-exit lowering for an indirect `JumpReg`: exit to the run loop at the
/// (dynamic) guest PC held in `target_reg`, instead of a host `br` to that value
/// as a host address.
fn lower_jump_reg_exit(asm: &mut Arm64Assembler, target_reg: u8) {
    if target_reg != BRANCH_EXIT_TARGET_REG {
        asm.mov_x(BRANCH_EXIT_TARGET_REG, target_reg);
    }
    emit_branch_exit(asm);
}

/// Block-exit lowering for an indirect `CallReg`: push the return address onto
/// the guest stack and exit to the run loop at the dynamic target in
/// `target_reg`. The target is captured into the exit register first, before the
/// push clobbers caller-saved scratch.
fn lower_call_reg_exit(
    asm: &mut Arm64Assembler,
    target_reg: u8,
    return_guest_pc: u64,
) -> Result<(), LowerError> {
    if target_reg != BRANCH_EXIT_TARGET_REG {
        asm.mov_x(BRANCH_EXIT_TARGET_REG, target_reg);
    }
    // rsp -= 8; [rsp] = return address (rebased to host through mem_base)
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, -8)?;
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_u64_constant(asm, BRANCH_EXIT_FALL_REG, return_guest_pc);
    emit_store_mem(asm, OpSize::I64, BRANCH_EXIT_FALL_REG, RSP_ADJUST_TMP_REG);
    emit_branch_exit(asm);
    Ok(())
}

fn lower_rsp_adjust(
    asm: &mut Arm64Assembler,
    adjust: &prisma_ir::RspAdjust,
) -> Result<(), LowerError> {
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, adjust.delta_bytes)?;
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    Ok(())
}

fn lower_ret_adjusted(asm: &mut Arm64Assembler, pop_bytes: u64) -> Result<(), LowerError> {
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    if pop_bytes != 0 {
        let pop =
            i64::try_from(pop_bytes).map_err(|_| LowerError::ImmediateOutOfRange(pop_bytes))?;
        emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, pop)?;
    }
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    asm.ret();
    Ok(())
}

fn emit_rsp_imm_add(
    asm: &mut Arm64Assembler,
    register: u8,
    delta_bytes: i64,
) -> Result<(), LowerError> {
    match delta_bytes {
        0 => Ok(()),
        1..=4095 => {
            let imm = u16::try_from(delta_bytes).expect("small positive immediate fits");
            asm.add_x_imm(register, register, imm);
            Ok(())
        }
        -4095..=-1 => {
            let imm = delta_bytes
                .checked_neg()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| LowerError::ImmediateOutOfRange(delta_bytes.cast_unsigned()))?;
            asm.sub_x_imm(register, register, imm);
            Ok(())
        }
        _ => {
            if delta_bytes == i64::MIN {
                return Err(LowerError::ImmediateOutOfRange(delta_bytes.cast_unsigned()));
            }
            let abs = if delta_bytes.is_negative() {
                delta_bytes.unsigned_abs()
            } else {
                u64::try_from(delta_bytes).expect("non-negative delta in this branch")
            };
            emit_u64_constant(asm, RSP_ADJUST_IMM_REG, abs);
            if delta_bytes.is_negative() {
                asm.sub_x(register, register, RSP_ADJUST_IMM_REG);
            } else {
                asm.add_x(register, register, RSP_ADJUST_IMM_REG);
            }
            Ok(())
        }
    }
}

fn block_label(labels: &HashMap<u32, Label>, guest_pc: u64) -> Result<Label, LowerError> {
    let block_id = u32::try_from(guest_pc).map_err(|_| LowerError::UnsupportedOp("CondJumpRel"))?;
    labels
        .get(&block_id)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(block_id))
}

fn value_reg(reference: Ref) -> u8 {
    let slot = reference % u32::from(VALUE_REG_COUNT);
    FIRST_VALUE_REG + u8::try_from(slot).expect("slot is bounded by VALUE_REG_COUNT")
}

fn alloc_vec_pair(
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    reference: Ref,
) -> Result<(u8, u8), LowerError> {
    if let Some(pair) = vec_values.get(&reference) {
        return Ok(*pair);
    }

    let pair = VEC_REG_PAIRS
        .get(vec_values.len())
        .copied()
        .ok_or(LowerError::UnsupportedOp("too many vector temporaries"))?;
    vec_values.insert(reference, pair);
    Ok(pair)
}

fn vec_pair(vec_values: &HashMap<Ref, (u8, u8)>, reference: Ref) -> Result<(u8, u8), LowerError> {
    vec_values
        .get(&reference)
        .copied()
        .ok_or(LowerError::MissingValue(reference))
}

fn lower_vec_shuffle32x4(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    shuffle: &prisma_ir::VecShuffle32x4,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("VecShuffle32x4"))?;
    let src = vec_pair(vec_values, shuffle.src)?;
    let dst = alloc_vec_pair(vec_values, result)?;
    debug_assert!(dst.0 != src.0 && dst.0 != src.1 && dst.1 != src.0 && dst.1 != src.1);

    emit_vec_u32_lane(asm, dst.0, src, shuffle.control & 0x03);
    emit_vec_u32_lane(asm, CAS_STATUS_REG, src, (shuffle.control >> 2) & 0x03);
    asm.lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32);
    asm.orr_x(dst.0, dst.0, CAS_STATUS_REG);

    emit_vec_u32_lane(asm, dst.1, src, (shuffle.control >> 4) & 0x03);
    emit_vec_u32_lane(asm, CAS_STATUS_REG, src, (shuffle.control >> 6) & 0x03);
    asm.lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32);
    asm.orr_x(dst.1, dst.1, CAS_STATUS_REG);
    Ok(())
}

fn lower_vec_unpack(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    unpack: &prisma_ir::VecUnpack,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("VecUnpack"))?;
    let lhs = vec_pair(vec_values, unpack.lhs)?;
    let rhs = vec_pair(vec_values, unpack.rhs)?;
    let dst = alloc_vec_pair(vec_values, result)?;
    let helper = vec_unpack_helper as VecUnpackHelper as usize as u64;

    for (output_high, output_reg) in [(0u64, dst.0), (1u64, dst.1)] {
        emit_save_for_helper_call(asm);
        asm.mov_x(0, lhs.0);
        asm.mov_x(1, lhs.1);
        asm.mov_x(2, rhs.0);
        asm.mov_x(3, rhs.1);
        emit_u64_constant(asm, 4, u64::from(unpack.lane as u8));
        emit_u64_constant(asm, 5, u64::from(unpack.is_high));
        emit_u64_constant(asm, 6, output_high);
        emit_u64_constant(asm, PCMP_HELPER_TARGET_REG, helper);
        asm.blr_x(PCMP_HELPER_TARGET_REG);
        emit_restore_after_helper_call(asm);
        asm.mov_x(output_reg, 0);
    }
    Ok(())
}

fn lower_vec_shuffle_h4(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    shuffle: &prisma_ir::VecShuffleH4,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("VecShuffleH4"))?;
    let src = vec_pair(vec_values, shuffle.src)?;
    let dst = alloc_vec_pair(vec_values, result)?;
    let helper = vec_shuffle_h4_helper as VecShuffleH4Helper as usize as u64;

    for (output_high, output_reg) in [(0u64, dst.0), (1u64, dst.1)] {
        emit_save_for_helper_call(asm);
        asm.mov_x(0, src.0);
        asm.mov_x(1, src.1);
        emit_u64_constant(asm, 2, u64::from(shuffle.control));
        emit_u64_constant(asm, 3, u64::from(shuffle.is_high));
        emit_u64_constant(asm, 4, output_high);
        emit_u64_constant(asm, PCMP_HELPER_TARGET_REG, helper);
        asm.blr_x(PCMP_HELPER_TARGET_REG);
        emit_restore_after_helper_call(asm);
        asm.mov_x(output_reg, 0);
    }
    Ok(())
}

fn lower_vec_cmp(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    compare: &prisma_ir::VecCmp,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("VecCmp"))?;
    let lhs = vec_pair(vec_values, compare.lhs)?;
    let rhs = vec_pair(vec_values, compare.rhs)?;
    let dst = alloc_vec_pair(vec_values, result)?;
    let helper = vec_cmp_helper as VecCmpHelper as usize as u64;
    for (output_high, output_reg) in [(0u64, dst.0), (1u64, dst.1)] {
        emit_save_for_helper_call(asm);
        asm.mov_x(0, lhs.0);
        asm.mov_x(1, lhs.1);
        asm.mov_x(2, rhs.0);
        asm.mov_x(3, rhs.1);
        emit_u64_constant(asm, 4, u64::from(compare.lane as u8));
        emit_u64_constant(asm, 5, u64::from(compare.kind as u8));
        emit_u64_constant(asm, 6, output_high);
        emit_u64_constant(asm, PCMP_HELPER_TARGET_REG, helper);
        asm.blr_x(PCMP_HELPER_TARGET_REG);
        emit_restore_after_helper_call(asm);
        asm.mov_x(output_reg, 0);
    }
    Ok(())
}

fn emit_vec_u32_lane(asm: &mut Arm64Assembler, dst: u8, src: (u8, u8), lane: u8) {
    let word = if lane < 2 { src.0 } else { src.1 };
    if lane & 1 == 0 {
        asm.uxtw_x(dst, word);
    } else {
        asm.lsr_x_imm(dst, word, 32);
        asm.uxtw_x(dst, dst);
    }
}

fn emit_vec_add_u32x2(asm: &mut Arm64Assembler, dst: u8, lhs: u8, rhs: u8) {
    asm.uxtw_x(dst, lhs);
    asm.uxtw_x(CAS_STATUS_REG, rhs);
    asm.add_x(dst, dst, CAS_STATUS_REG);
    asm.uxtw_x(dst, dst);

    asm.lsr_x_imm(FLAG_ALIGN_LHS_REG, lhs, 32);
    asm.lsr_x_imm(CAS_STATUS_REG, rhs, 32);
    asm.add_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, CAS_STATUS_REG);
    asm.uxtw_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG);
    asm.lsl_x_imm(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, 32);
    asm.orr_x(dst, dst, FLAG_ALIGN_LHS_REG);
}

fn lower_vec_shift_imm(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    shift: &prisma_ir::VecShiftImm,
) -> Result<(), LowerError> {
    if shift.lane != prisma_ir::VecLane::S4
        || !matches!(
            shift.kind,
            prisma_ir::VecShiftKind::ShiftL | prisma_ir::VecShiftKind::LogicalShr
        )
    {
        return Err(LowerError::UnsupportedOp("VecShiftImm"));
    }
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("VecShiftImm"))?;
    let src = vec_pair(vec_values, shift.src)?;
    let dst = alloc_vec_pair(vec_values, result)?;
    emit_vec_shift_u32x2(asm, dst.0, src.0, shift.kind, shift.count);
    emit_vec_shift_u32x2(asm, dst.1, src.1, shift.kind, shift.count);
    Ok(())
}

fn lower_int_to_fp_scalar(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    convert: &prisma_ir::IntToFpScalar,
) -> Result<(), LowerError> {
    if convert.fp_size != prisma_ir::FpSize::F64 {
        return Err(LowerError::UnsupportedOp("IntToFpScalar"));
    }
    let src = *values
        .get(&convert.value)
        .ok_or(LowerError::MissingValue(convert.value))?;
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("IntToFpScalar"))?;
    let (lo, hi) = alloc_vec_pair(vec_values, result)?;
    let signed = match convert.int_size {
        OpSize::I32 => {
            asm.sxtw_x(FLAG_ALIGN_LHS_REG, src);
            FLAG_ALIGN_LHS_REG
        }
        OpSize::I64 => src,
        _ => return Err(LowerError::UnsupportedOp("IntToFpScalar")),
    };
    asm.scvtf_d_x(0, signed);
    asm.fmov_x_d(lo, 0);
    emit_u64_constant(asm, hi, 0);
    Ok(())
}

fn lower_vec_fp_scalar_bin(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    bin: &prisma_ir::VecFpScalarBinOp,
) -> Result<(), LowerError> {
    if bin.size != prisma_ir::FpSize::F64 {
        return Err(LowerError::UnsupportedOp("VecFpScalarBinOp"));
    }
    let lhs = vec_pair(vec_values, bin.lhs)?;
    let rhs = vec_pair(vec_values, bin.rhs)?;
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("VecFpScalarBinOp"))?;
    let dst = alloc_vec_pair(vec_values, result)?;
    asm.fmov_d_x(0, lhs.0);
    asm.fmov_d_x(1, rhs.0);
    match bin.op {
        prisma_ir::VecFpBinOpKind::Add => asm.fadd_d(0, 0, 1),
        prisma_ir::VecFpBinOpKind::Sub => asm.fsub_d(0, 0, 1),
        prisma_ir::VecFpBinOpKind::Mul => asm.fmul_d(0, 0, 1),
        prisma_ir::VecFpBinOpKind::Div => asm.fdiv_d(0, 0, 1),
        _ => return Err(LowerError::UnsupportedOp("VecFpScalarBinOp")),
    }
    asm.fmov_x_d(dst.0, 0);
    asm.mov_x(dst.1, lhs.1);
    Ok(())
}

fn lower_fp_to_int_scalar(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    vec_values: &HashMap<Ref, (u8, u8)>,
    convert: &prisma_ir::FpToIntScalar,
) -> Result<(), LowerError> {
    if convert.fp_size != prisma_ir::FpSize::F64 {
        return Err(LowerError::UnsupportedOp("FpToIntScalar"));
    }
    let src = vec_pair(vec_values, convert.value)?.0;
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("FpToIntScalar"))?;
    let dst = value_reg(result);
    let limit = match convert.int_size {
        OpSize::I32 => 0x41e0_0000_0000_0000,
        OpSize::I64 => 0x43e0_0000_0000_0000,
        _ => return Err(LowerError::UnsupportedOp("FpToIntScalar")),
    };
    emit_u64_constant(asm, FLAG_ALIGN_RHS_REG, 0x7fff_ffff_ffff_ffff);
    asm.and_x(FLAG_ALIGN_LHS_REG, src, FLAG_ALIGN_RHS_REG);
    emit_u64_constant(asm, CAS_STATUS_REG, limit);
    asm.lsr_x_imm(FLAG_ALIGN_RHS_REG, src, 63);

    let negative = asm.create_label();
    let convert_value = asm.create_label();
    let invalid = asm.create_label();
    let done = asm.create_label();
    asm.cbnz_x_label(FLAG_ALIGN_RHS_REG, negative);
    asm.cmp_x(FLAG_ALIGN_LHS_REG, CAS_STATUS_REG);
    asm.b_cond_label(prisma_ir::CondCode::Uge, invalid);
    asm.b_label(convert_value);
    asm.bind_label(negative);
    asm.cmp_x(FLAG_ALIGN_LHS_REG, CAS_STATUS_REG);
    asm.b_cond_label(prisma_ir::CondCode::Ugt, invalid);
    asm.bind_label(convert_value);
    asm.fmov_d_x(0, src);
    match convert.int_size {
        OpSize::I32 => asm.fcvtzs_w_d(dst, 0),
        OpSize::I64 => asm.fcvtzs_x_d(dst, 0),
        _ => unreachable!("validated above"),
    }
    asm.b_label(done);
    asm.bind_label(invalid);
    emit_u64_constant(
        asm,
        dst,
        if convert.int_size == OpSize::I32 {
            u64::from(0x8000_0000_u32)
        } else {
            i64::MIN as u64
        },
    );
    asm.bind_label(done);
    values.insert(result, dst);
    Ok(())
}

fn emit_vec_shift_u32x2(
    asm: &mut Arm64Assembler,
    dst: u8,
    src: u8,
    kind: prisma_ir::VecShiftKind,
    count: u8,
) {
    if count >= 32 {
        emit_u64_constant(asm, dst, 0);
        return;
    }
    asm.uxtw_x(dst, src);
    asm.lsr_x_imm(CAS_STATUS_REG, src, 32);
    match kind {
        prisma_ir::VecShiftKind::ShiftL => {
            asm.lsl_x_imm(dst, dst, count);
            asm.uxtw_x(dst, dst);
            asm.lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, count);
            asm.uxtw_x(CAS_STATUS_REG, CAS_STATUS_REG);
        }
        prisma_ir::VecShiftKind::LogicalShr => {
            asm.lsr_x_imm(dst, dst, count);
            asm.lsr_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, count);
        }
        prisma_ir::VecShiftKind::ArithShr => unreachable!("validated by caller"),
    }
    asm.lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32);
    asm.orr_x(dst, dst, CAS_STATUS_REG);
}

fn lower_vec_clmul(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    op: &prisma_ir::VecClMul,
) -> Result<(), LowerError> {
    const CLMUL_WORK_LO: u8 = FLAG_ALIGN_LHS_REG; // X17
    const CLMUL_WORK_HI: u8 = FLAG_ALIGN_RHS_REG; // X28
    const CLMUL_RHS: u8 = MEM_ADDR_SCRATCH; // X24
    const CLMUL_TMP: u8 = CAS_STATUS_REG; // X25
    const CLMUL_COUNT: u8 = 30;

    let result = stmt.result.ok_or(LowerError::MissingResult("VecClMul"))?;
    let lhs = vec_pair(vec_values, op.lhs)?;
    let rhs = vec_pair(vec_values, op.rhs)?;
    let lhs_reg = if op.lhs_high { lhs.1 } else { lhs.0 };
    let rhs_reg = if op.rhs_high { rhs.1 } else { rhs.0 };
    let (out_lo, out_hi) = alloc_vec_pair(vec_values, result)?;

    asm.mov_x(CLMUL_WORK_LO, lhs_reg);
    emit_u64_constant(asm, CLMUL_WORK_HI, 0);
    asm.mov_x(CLMUL_RHS, rhs_reg);
    emit_u64_constant(asm, out_lo, 0);
    emit_u64_constant(asm, out_hi, 0);
    emit_u64_constant(asm, CLMUL_COUNT, 64);

    let loop_label = asm.create_label();
    let skip_xor = asm.create_label();
    asm.bind_label(loop_label);
    asm.lsl_x_imm(CLMUL_TMP, CLMUL_RHS, 63);
    asm.cbz_x_label(CLMUL_TMP, skip_xor);
    asm.eor_x(out_lo, out_lo, CLMUL_WORK_LO);
    asm.eor_x(out_hi, out_hi, CLMUL_WORK_HI);
    asm.bind_label(skip_xor);
    asm.lsr_x_imm(CLMUL_TMP, CLMUL_WORK_LO, 63);
    asm.lsl_x_imm(CLMUL_WORK_LO, CLMUL_WORK_LO, 1);
    asm.lsl_x_imm(CLMUL_WORK_HI, CLMUL_WORK_HI, 1);
    asm.orr_x(CLMUL_WORK_HI, CLMUL_WORK_HI, CLMUL_TMP);
    asm.lsr_x_imm(CLMUL_RHS, CLMUL_RHS, 1);
    asm.sub_x_imm(CLMUL_COUNT, CLMUL_COUNT, 1);
    asm.cbnz_x_label(CLMUL_COUNT, loop_label);

    Ok(())
}

fn lower_vec_f16cvt(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    op: &prisma_ir::VecF16Cvt,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("VecF16Cvt"))?;
    let (src_lo, src_hi) = vec_pair(vec_values, op.src)?;
    let (out_lo, out_hi) = alloc_vec_pair(vec_values, result)?;

    match op.kind {
        prisma_ir::VecF16CvtKind::PhToPs => {
            emit_save_for_helper_call(asm);
            asm.mov_x(0, src_lo);
            emit_u64_constant(
                asm,
                PCMP_HELPER_TARGET_REG,
                f16c_ph2ps_lo_helper as *const () as usize as u64,
            );
            asm.blr_x(PCMP_HELPER_TARGET_REG);
            emit_restore_after_helper_call(asm);
            asm.mov_x(out_lo, 0);

            emit_save_for_helper_call(asm);
            asm.mov_x(0, src_lo);
            emit_u64_constant(
                asm,
                PCMP_HELPER_TARGET_REG,
                f16c_ph2ps_hi_helper as *const () as usize as u64,
            );
            asm.blr_x(PCMP_HELPER_TARGET_REG);
            emit_restore_after_helper_call(asm);
            asm.mov_x(out_hi, 0);
        }
        prisma_ir::VecF16CvtKind::PsToPh => {
            emit_save_for_helper_call(asm);
            asm.mov_x(0, src_lo);
            asm.mov_x(1, src_hi);
            emit_u64_constant(asm, 2, u64::from(op.rounding));
            emit_u64_constant(
                asm,
                PCMP_HELPER_TARGET_REG,
                f16c_ps2ph_helper as *const () as usize as u64,
            );
            asm.blr_x(PCMP_HELPER_TARGET_REG);
            emit_restore_after_helper_call(asm);
            asm.mov_x(out_lo, 0);
            emit_u64_constant(asm, out_hi, 0);
        }
    }

    Ok(())
}

fn lower_pcmpstr_index(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    vec_values: &HashMap<Ref, (u8, u8)>,
    pcmp: &prisma_ir::PcmpStrIndex,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("PcmpStrIndex"))?;
    let dst = value_reg(result);
    emit_pcmpstr_helper_call(
        asm,
        values,
        vec_values,
        pcmp_parts_index(pcmp),
        pcmpstr_index_helper,
    )?;
    asm.mov_x(dst, 0);
    values.insert(result, dst);
    Ok(())
}

fn lower_pcmpstr_mask(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    vec_values: &mut HashMap<Ref, (u8, u8)>,
    pcmp: &prisma_ir::PcmpStrMask,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("PcmpStrMask"))?;
    let (lo, hi) = alloc_vec_pair(vec_values, result)?;
    let parts = pcmp_parts_mask(pcmp);
    emit_pcmpstr_helper_call(asm, values, vec_values, parts, pcmpstr_mask_lo_helper)?;
    asm.mov_x(lo, 0);
    emit_pcmpstr_helper_call(asm, values, vec_values, parts, pcmpstr_mask_hi_helper)?;
    asm.mov_x(hi, 0);
    Ok(())
}

fn lower_pcmpstr_flags(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    vec_values: &HashMap<Ref, (u8, u8)>,
    pcmp: &prisma_ir::PcmpStrFlags,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("PcmpStrFlags"))?;
    let dst = value_reg(result);
    emit_pcmpstr_helper_call(
        asm,
        values,
        vec_values,
        pcmp_parts_flags(pcmp),
        pcmpstr_flags_helper,
    )?;
    asm.mov_x(dst, 0);
    values.insert(result, dst);
    Ok(())
}

#[derive(Clone, Copy)]
struct PcmpStrParts {
    lhs: Ref,
    rhs: Ref,
    lhs_len: Option<Ref>,
    rhs_len: Option<Ref>,
    imm8: u8,
}

fn pcmp_parts_index(pcmp: &prisma_ir::PcmpStrIndex) -> PcmpStrParts {
    PcmpStrParts {
        lhs: pcmp.lhs,
        rhs: pcmp.rhs,
        lhs_len: pcmp.lhs_len,
        rhs_len: pcmp.rhs_len,
        imm8: pcmp.imm8,
    }
}

fn pcmp_parts_mask(pcmp: &prisma_ir::PcmpStrMask) -> PcmpStrParts {
    PcmpStrParts {
        lhs: pcmp.lhs,
        rhs: pcmp.rhs,
        lhs_len: pcmp.lhs_len,
        rhs_len: pcmp.rhs_len,
        imm8: pcmp.imm8,
    }
}

fn pcmp_parts_flags(pcmp: &prisma_ir::PcmpStrFlags) -> PcmpStrParts {
    PcmpStrParts {
        lhs: pcmp.lhs,
        rhs: pcmp.rhs,
        lhs_len: pcmp.lhs_len,
        rhs_len: pcmp.rhs_len,
        imm8: pcmp.imm8,
    }
}

fn emit_pcmpstr_helper_call(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    vec_values: &HashMap<Ref, (u8, u8)>,
    parts: PcmpStrParts,
    helper: PcmpStrHelper,
) -> Result<(), LowerError> {
    let (lhs_lo, lhs_hi) = vec_pair(vec_values, parts.lhs)?;
    let (rhs_lo, rhs_hi) = vec_pair(vec_values, parts.rhs)?;
    let mut len_mode = 0u64;
    let lhs_len = if let Some(r) = parts.lhs_len {
        len_mode |= PCMP_LEN_LHS_EXPLICIT;
        Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
    } else {
        None
    };
    let rhs_len = if let Some(r) = parts.rhs_len {
        len_mode |= PCMP_LEN_RHS_EXPLICIT;
        Some(*values.get(&r).ok_or(LowerError::MissingValue(r))?)
    } else {
        None
    };

    emit_save_for_helper_call(asm);
    asm.mov_x(0, lhs_lo);
    asm.mov_x(1, lhs_hi);
    asm.mov_x(2, rhs_lo);
    asm.mov_x(3, rhs_hi);
    if let Some(reg) = lhs_len {
        asm.mov_x(4, reg);
    } else {
        emit_u64_constant(asm, 4, 0);
    }
    if let Some(reg) = rhs_len {
        asm.mov_x(5, reg);
    } else {
        emit_u64_constant(asm, 5, 0);
    }
    emit_u64_constant(asm, 6, len_mode);
    emit_u64_constant(asm, 7, u64::from(parts.imm8));
    emit_u64_constant(asm, PCMP_HELPER_TARGET_REG, helper as usize as u64);
    asm.blr_x(PCMP_HELPER_TARGET_REG);
    emit_restore_after_helper_call(asm);
    Ok(())
}

fn emit_save_for_helper_call(asm: &mut Arm64Assembler) {
    for (left, right) in [(9, 10), (11, 12), (13, 14), (15, 16), (17, 18), (29, 30)] {
        asm.stp_x_pre_sp(left, right, -16);
    }
}

fn emit_restore_after_helper_call(asm: &mut Arm64Assembler) {
    for (left, right) in [(29, 30), (17, 18), (15, 16), (13, 14), (11, 12), (9, 10)] {
        asm.ldp_x_post_sp(left, right, 16);
    }
}

fn emit_u64_constant(asm: &mut Arm64Assembler, dst: u8, value: u64) {
    let low = u16::try_from(value & 0xffff).expect("masked to 16 bits");
    asm.movz_x(dst, low, 0);
    for chunk in 1..4 {
        let part = u16::try_from((value >> (chunk * 16)) & 0xffff).expect("masked to 16 bits");
        if part != 0 {
            asm.movk_x(
                dst,
                part,
                u8::try_from(chunk * 16).expect("valid MOVK shift"),
            );
        }
    }
}

/// Rebase a guest virtual address to its host address in `MEM_ADDR_SCRATCH`:
/// `host = mem_base + guest_va`. `mem_base` is reloaded per access (cheap, and
/// keeps the change localized to the memory ops — no prologue/ABI change). The
/// scratch is disjoint from the value-register pool, so `addr` (and a store's
/// `value`) stay live for any later use.
fn emit_rebase_addr(asm: &mut Arm64Assembler, addr: u8) {
    asm.ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET);
    asm.add_x(MEM_ADDR_SCRATCH, addr, MEM_ADDR_SCRATCH);
}

fn emit_load_mem(asm: &mut Arm64Assembler, size: OpSize, dst: u8, addr: u8) {
    emit_rebase_addr(asm, addr);
    let base = MEM_ADDR_SCRATCH;
    match size {
        OpSize::I8 => asm.ldrb_unsigned(dst, base, 0),
        OpSize::I16 => asm.ldrh_unsigned(dst, base, 0),
        OpSize::I32 => asm.ldr_w_unsigned(dst, base, 0),
        OpSize::I64 => asm.ldr_x_unsigned(dst, base, 0),
    }
}

fn emit_load_vec_mem(asm: &mut Arm64Assembler, lo: u8, hi: u8, addr: u8) {
    emit_rebase_addr(asm, addr);
    asm.ldr_x_unsigned(lo, MEM_ADDR_SCRATCH, 0);
    asm.ldr_x_unsigned(hi, MEM_ADDR_SCRATCH, 8);
}

fn lower_atomic_cmpxchg(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    cas: &prisma_ir::AtomicCmpxchg,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("AtomicCmpxchg"))?;
    let addr = *values
        .get(&cas.addr)
        .ok_or(LowerError::MissingValue(cas.addr))?;
    let expected = *values
        .get(&cas.expected)
        .ok_or(LowerError::MissingValue(cas.expected))?;
    let new_value = *values
        .get(&cas.new_value)
        .ok_or(LowerError::MissingValue(cas.new_value))?;
    let loaded = value_reg(result);

    asm.mov_x(ATOMIC_CMPXCHG_EXPECTED_REG, expected);
    asm.mov_x(ATOMIC_CMPXCHG_NEW_REG, new_value);
    values.insert(cas.expected, ATOMIC_CMPXCHG_EXPECTED_REG);
    values.insert(cas.new_value, ATOMIC_CMPXCHG_NEW_REG);
    emit_rebase_addr(asm, addr);
    let retry = asm.create_label();
    let fail = asm.create_label();
    let done = asm.create_label();

    asm.bind_label(retry);
    emit_atomic_load_acquire(asm, cas.size, loaded, MEM_ADDR_SCRATCH);
    let (lhs, rhs) = align_flag_operands(asm, cas.size, loaded, ATOMIC_CMPXCHG_EXPECTED_REG);
    asm.cmp_x(lhs, rhs);
    asm.b_cond_label(prisma_ir::CondCode::Ne, fail);
    emit_atomic_store_release(
        asm,
        cas.size,
        CAS_STATUS_REG,
        ATOMIC_CMPXCHG_NEW_REG,
        MEM_ADDR_SCRATCH,
    );
    asm.cbnz_x_label(CAS_STATUS_REG, retry);
    asm.b_label(done);
    asm.bind_label(fail);
    asm.clrex();
    asm.bind_label(done);

    values.insert(result, loaded);
    Ok(())
}

fn lower_atomic_xadd(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    xadd: &prisma_ir::AtomicXadd,
) -> Result<(), LowerError> {
    const NEW_VALUE: u8 = FLAG_ALIGN_LHS_REG;
    let result = stmt.result.ok_or(LowerError::MissingResult("AtomicXadd"))?;
    let addr = *values
        .get(&xadd.addr)
        .ok_or(LowerError::MissingValue(xadd.addr))?;
    let value = *values
        .get(&xadd.value)
        .ok_or(LowerError::MissingValue(xadd.value))?;
    let old = value_reg(result);

    emit_rebase_addr(asm, addr);
    asm.mov_x(ATOMIC_RMW_SOURCE_REG, value);
    values.insert(xadd.value, ATOMIC_RMW_SOURCE_REG);
    let retry = asm.create_label();
    asm.bind_label(retry);
    emit_atomic_load_acquire(asm, xadd.size, old, MEM_ADDR_SCRATCH);
    asm.add_x(NEW_VALUE, old, ATOMIC_RMW_SOURCE_REG);
    emit_atomic_store_release(asm, xadd.size, CAS_STATUS_REG, NEW_VALUE, MEM_ADDR_SCRATCH);
    asm.cbnz_x_label(CAS_STATUS_REG, retry);
    values.insert(result, old);
    Ok(())
}

fn lower_atomic_xchg(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    xchg: &prisma_ir::AtomicXchg,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("AtomicXchg"))?;
    let addr = *values
        .get(&xchg.addr)
        .ok_or(LowerError::MissingValue(xchg.addr))?;
    let value = *values
        .get(&xchg.value)
        .ok_or(LowerError::MissingValue(xchg.value))?;
    let old = value_reg(result);

    asm.mov_x(ATOMIC_RMW_SOURCE_REG, value);
    values.insert(xchg.value, ATOMIC_RMW_SOURCE_REG);
    emit_rebase_addr(asm, addr);
    let retry = asm.create_label();
    asm.bind_label(retry);
    emit_atomic_load_acquire(asm, xchg.size, old, MEM_ADDR_SCRATCH);
    emit_atomic_store_release(
        asm,
        xchg.size,
        CAS_STATUS_REG,
        ATOMIC_RMW_SOURCE_REG,
        MEM_ADDR_SCRATCH,
    );
    asm.cbnz_x_label(CAS_STATUS_REG, retry);
    values.insert(result, old);
    Ok(())
}

fn lower_atomic_cmpxchg_pair(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    cas: &prisma_ir::AtomicCmpxchgPair,
) -> Result<(), LowerError> {
    let old_low_ref = stmt
        .result
        .ok_or(LowerError::MissingResult("AtomicCmpxchgPair"))?;
    let addr = *values
        .get(&cas.addr)
        .ok_or(LowerError::MissingValue(cas.addr))?;
    let expected_low = *values
        .get(&cas.expected_low)
        .ok_or(LowerError::MissingValue(cas.expected_low))?;
    let expected_high = *values
        .get(&cas.expected_high)
        .ok_or(LowerError::MissingValue(cas.expected_high))?;
    let new_low = *values
        .get(&cas.new_low)
        .ok_or(LowerError::MissingValue(cas.new_low))?;
    let new_high = *values
        .get(&cas.new_high)
        .ok_or(LowerError::MissingValue(cas.new_high))?;
    let old_low = value_reg(old_low_ref);
    let old_high = value_reg(cas.old_high);

    emit_rebase_addr(asm, addr);
    let retry = asm.create_label();
    let fail = asm.create_label();
    let done = asm.create_label();

    asm.bind_label(retry);
    asm.ldaxp_x(old_low, old_high, MEM_ADDR_SCRATCH);
    asm.cmp_x(old_low, expected_low);
    asm.b_cond_label(prisma_ir::CondCode::Ne, fail);
    asm.cmp_x(old_high, expected_high);
    asm.b_cond_label(prisma_ir::CondCode::Ne, fail);
    asm.stlxp_x(CAS_STATUS_REG, new_low, new_high, MEM_ADDR_SCRATCH);
    asm.cbnz_x_label(CAS_STATUS_REG, retry);
    asm.b_label(done);
    asm.bind_label(fail);
    asm.clrex();
    asm.bind_label(done);

    values.insert(old_low_ref, old_low);
    values.insert(cas.old_high, old_high);
    Ok(())
}

fn emit_atomic_load_acquire(asm: &mut Arm64Assembler, size: OpSize, dst: u8, base: u8) {
    match size {
        OpSize::I8 => asm.ldaxrb(dst, base),
        OpSize::I16 => asm.ldaxrh(dst, base),
        OpSize::I32 => asm.ldaxr_w(dst, base),
        OpSize::I64 => asm.ldaxr_x(dst, base),
    }
}

fn emit_atomic_store_release(
    asm: &mut Arm64Assembler,
    size: OpSize,
    status: u8,
    value: u8,
    base: u8,
) {
    match size {
        OpSize::I8 => asm.stlxrb(status, value, base),
        OpSize::I16 => asm.stlxrh(status, value, base),
        OpSize::I32 => asm.stlxr_w(status, value, base),
        OpSize::I64 => asm.stlxr_x(status, value, base),
    }
}

fn emit_load_reg(asm: &mut Arm64Assembler, size: OpSize, dst: u8, reg: Gpr) {
    let offset = gpr_offset_bytes(reg);
    match size {
        OpSize::I8 => asm.ldrb_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I16 => asm.ldrh_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I32 => asm.ldr_w_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I64 => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, offset),
    }
}

fn emit_store_mem(asm: &mut Arm64Assembler, size: OpSize, value: u8, addr: u8) {
    emit_rebase_addr(asm, addr);
    let base = MEM_ADDR_SCRATCH;
    match size {
        OpSize::I8 => asm.strb_unsigned(value, base, 0),
        OpSize::I16 => asm.strh_unsigned(value, base, 0),
        OpSize::I32 => asm.str_w_unsigned(value, base, 0),
        OpSize::I64 => asm.str_x_unsigned(value, base, 0),
    }
}

fn emit_store_vec_mem(asm: &mut Arm64Assembler, lo: u8, hi: u8, addr: u8) {
    emit_rebase_addr(asm, addr);
    asm.str_x_unsigned(lo, MEM_ADDR_SCRATCH, 0);
    asm.str_x_unsigned(hi, MEM_ADDR_SCRATCH, 8);
}

/// ARM64 zero register (`WZR`/`XZR`) in the `Rt` position of a load/store.
const ZERO_REG: u8 = 31;

fn emit_load_vec_state(
    asm: &mut Arm64Assembler,
    lo: u8,
    hi: u8,
    xmm_index: u8,
) -> Result<(), LowerError> {
    let offset = xmm_offset_bytes(xmm_index)?;
    asm.ldr_x_unsigned(lo, abi::K_STATE_PTR_REG, offset);
    asm.ldr_x_unsigned(hi, abi::K_STATE_PTR_REG, offset + 8);
    Ok(())
}

fn emit_store_vec_state(
    asm: &mut Arm64Assembler,
    lo: u8,
    hi: u8,
    xmm_index: u8,
) -> Result<(), LowerError> {
    let offset = xmm_offset_bytes(xmm_index)?;
    asm.str_x_unsigned(lo, abi::K_STATE_PTR_REG, offset);
    asm.str_x_unsigned(hi, abi::K_STATE_PTR_REG, offset + 8);
    Ok(())
}

fn xmm_offset_bytes(xmm_index: u8) -> Result<u16, LowerError> {
    if xmm_index >= 16 {
        return Err(LowerError::UnsupportedOp("xmm index"));
    }
    Ok(XMM_BASE_OFFSET + u16::from(xmm_index) * XMM_SLOT_BYTES)
}

fn emit_store_reg(asm: &mut Arm64Assembler, size: OpSize, value: u8, reg: Gpr) {
    let offset = gpr_offset_bytes(reg);
    match size {
        OpSize::I8 => asm.strb_unsigned(value, abi::K_STATE_PTR_REG, offset),
        OpSize::I16 => asm.strh_unsigned(value, abi::K_STATE_PTR_REG, offset),
        OpSize::I32 => {
            // x86-64: writing a 32-bit GPR zero-extends into the full 64-bit
            // register. The slot is 8 bytes wide and the low store touches only
            // 4, so clear the upper 4 bytes with WZR. (The C++ core gets this
            // for free via register-pinned guest state + `mov w,w`.)
            asm.str_w_unsigned(value, abi::K_STATE_PTR_REG, offset);
            asm.str_w_unsigned(ZERO_REG, abi::K_STATE_PTR_REG, offset + 4);
        }
        OpSize::I64 => asm.str_x_unsigned(value, abi::K_STATE_PTR_REG, offset),
    }
}

fn gpr_offset_bytes(reg: Gpr) -> u16 {
    u16::from(reg as u8) * 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi;
    use crate::assembler::{
        add_x, add_x_imm, adds_x, and_x, b, b_cond, blr_x, clrex, clz_w, clz_x, cmp_x, crc32cb,
        crc32ch, crc32cw, crc32cx, cset_x, eor_x, fadd_d, fcmp_d, fcvtzs_w_d, fence, fmov_d_x,
        fmov_x_d, ldaxp_x, ldaxr_x, ldp_x_post_sp, ldr_w_unsigned, ldr_x_unsigned, ldrb_unsigned,
        ldrh_unsigned, lsl_x, lsl_x_imm, lsr_x, lsr_x_imm, mov_x, movk_x, movz_x, mrs_cntvct,
        mrs_nzcv, msr_nzcv, orr_x, rbit_w, rbit_x, scvtf_d_x, stlxp_x, stlxr_x, stp_x_pre_sp,
        str_w_unsigned, str_x_unsigned, strb_unsigned, strh_unsigned, sub_x, sub_x_imm, sxtb_x,
        uxth_x, uxtw_x,
    };
    use prisma_ir::{
        AluFlags, AluFlagsPreserveCarry, AtomicCmpxchg, AtomicCmpxchgPair, AtomicXadd, AtomicXchg,
        BasicBlock, BinOp, Bswap, CmpFlags, Compare, CondCode, CondJump, CondJumpFlags,
        CondJumpRel, Constant, Crc32c, Fence, FenceKind, FlagBit, FpSize, FpToIntScalar, Gpr,
        GuestPc, IntToFpScalar, Jump, LoadMem, LoadMemTSO, LoadReg, LoadRflags, LoadSegBase,
        LoadVec, LoadVecReg, Lzcnt, PcmpStrFlags, PcmpStrIndex, PcmpStrMask, Popcnt, Rdtsc,
        ReadFlag, RepMovs, RepStos, Return, RflagsCarryMode, RspAdjust, SegmentReg, Select, Stmt,
        StoreCarry, StoreMem, StoreMemTSO, StoreReg, StoreRflags, StoreRflagsFromBits,
        StoreRflagsFromNzcv, StoreVec, StoreVecReg, Truncate, Tzcnt, VecBinOp, VecBinOpKind,
        VecClMul, VecCmp, VecCmpKind, VecConstant, VecF16Cvt, VecF16CvtKind, VecFpBinOpKind,
        VecFpScalarBinOp, VecLane, VecMaskMsb, VecShiftImm, VecShiftKind, VecShuffle32x4,
        VecUnpack, WideDiv, WideDivResult, WriteFlags, WriteFlagsCountZero, WriteFlagsFp,
        WriteFlagsPopcnt,
    };

    fn function(stmts: Vec<Stmt>) -> Function {
        Function {
            blocks: vec![BasicBlock { id: 0, stmts }],
            entry: 0,
        }
    }

    #[test]
    fn windows_platform_register_x18_is_not_allocated() {
        let scalar_pool = FIRST_VALUE_REG..FIRST_VALUE_REG + VALUE_REG_COUNT;
        assert!(!scalar_pool.contains(&18));
        assert_ne!(FLAG_ALIGN_LHS_REG, 18);
        assert_ne!(FLAG_ALIGN_RHS_REG, 18);
        assert_ne!(FLAG_ALIGN_SHIFT_REG, 18);
        assert_ne!(MOD_QUOTIENT_REG, 18);
        assert_ne!(RSP_ADJUST_TMP_REG, 18);
        assert_ne!(RSP_ADJUST_IMM_REG, 18);
        assert_ne!(ALU_FLAGS_TMP_REG, 18);
        assert_ne!(MEM_ADDR_SCRATCH, 18);
        assert_ne!(CAS_STATUS_REG, 18);
        assert_ne!(WIDE_SHIFT_REG, 18);
        assert_ne!(PCMP_HELPER_TARGET_REG, 18);
        assert!(VEC_REG_PAIRS.iter().all(|(lo, hi)| *lo != 18 && *hi != 18));
    }

    #[test]
    fn lowers_guest_pc_marker_into_the_shared_state_frame() {
        let guest_pc = 0x1_4000_1234;
        let func = function(vec![Stmt::new(None, Op::GuestPc(GuestPc { pc: guest_pc }))]);
        let words = Lowerer::new().lower_function(&func).unwrap();

        let expected = [
            movz_x(GUEST_PC_SCRATCH_REG, 0x1234, 0),
            movk_x(GUEST_PC_SCRATCH_REG, 0x4000, 16),
            movk_x(GUEST_PC_SCRATCH_REG, 0x0001, 32),
            str_x_unsigned(GUEST_PC_SCRATCH_REG, abi::K_STATE_PTR_REG, NEXT_PC_OFFSET),
        ];
        assert!(
            words
                .windows(expected.len())
                .any(|window| window == expected),
            "guest PC marker reaches CpuStateFrame::next_pc"
        );
    }

    fn function_with_blocks(blocks: Vec<BasicBlock>, entry: u32) -> Function {
        Function { blocks, entry }
    }

    fn restore_nzcv_from_rflags_words() -> Vec<u32> {
        let mut words = vec![
            ldr_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET),
            movz_x(NZCV_CARRY_REG, 0, 0),
            movk_x(NZCV_CARRY_REG, 1 << 13, 16),
        ];
        for (rflags_shift, nzcv_shift) in [(7, 31), (6, 30)] {
            words.extend([
                movz_x(FLAG_ALIGN_SHIFT_REG, rflags_shift, 0),
                lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 1, 0),
                and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, nzcv_shift, 0),
                lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
                orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG),
            ]);
        }
        words.extend([
            movz_x(FLAG_ALIGN_SHIFT_REG, 0, 0),
            lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG),
            movz_x(FLAG_ALIGN_SHIFT_REG, 1, 0),
            and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
            movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
            lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
            eor_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG),
        ]);
        for (rflags_shift, nzcv_shift) in [(11, 28)] {
            words.extend([
                movz_x(FLAG_ALIGN_SHIFT_REG, rflags_shift, 0),
                lsr_x(NZCV_MASK_REG, NZCV_TMP_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 1, 0),
                and_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, nzcv_shift, 0),
                lsl_x(NZCV_MASK_REG, NZCV_MASK_REG, FLAG_ALIGN_SHIFT_REG),
                orr_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG),
            ]);
        }
        words.push(msr_nzcv(NZCV_CARRY_REG));
        words
    }

    #[test]
    fn lowers_segment_bases_from_state_frame() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Fs,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Gs,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Ds,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(value_reg(0), abi::K_STATE_PTR_REG, FS_BASE_OFFSET),
                ldr_x_unsigned(value_reg(1), abi::K_STATE_PTR_REG, GS_BASE_OFFSET),
                movz_x(value_reg(2), 0, 0),
            ]
        );
    }

    #[test]
    fn lowers_extend_and_truncate_scalars() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0xff,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Extend(prisma_ir::Extend {
                    value: 0,
                    from_size: OpSize::I8,
                    to_size: OpSize::I64,
                    is_signed: true,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Extend(prisma_ir::Extend {
                    value: 0,
                    from_size: OpSize::I16,
                    to_size: OpSize::I64,
                    is_signed: false,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Truncate(Truncate {
                    value: 0,
                    to_size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0xff, 0),
                sxtb_x(value_reg(1), value_reg(0)),
                uxth_x(value_reg(2), value_reg(0)),
                uxtw_x(value_reg(3), value_reg(0)),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_then_read_flag() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::WriteFlags(WriteFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::ReadFlag(ReadFlag {
                    flags: 2,
                    which: FlagBit::Carry,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 7, 0),
                movz_x(value_reg(1), 3, 0),
                adds_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                cset_x(value_reg(3), CondCode::Cc),
            ]
        );
    }

    #[test]
    fn rejects_read_flag_parity_without_emulation() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::WriteFlags(WriteFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::ReadFlag(ReadFlag {
                    flags: 2,
                    which: FlagBit::Parity,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp(
                "ReadFlag(Parity/Aux) needs software emulation"
            ))
        );
    }

    #[test]
    fn lowers_write_flags_count_zero_to_nzcv() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 64,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsCountZero(WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0, 0),
                movz_x(value_reg(1), 64, 0),
                cmp_x(value_reg(1), 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                cmp_x(value_reg(0), 31),
                cset_x(FLAG_ALIGN_RHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
                lsl_x(FLAG_ALIGN_RHS_REG, FLAG_ALIGN_RHS_REG, FLAG_ALIGN_SHIFT_REG),
                crate::assembler::orr_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_count_zero_truncates_narrow_values_before_compare() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsCountZero(WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x100, 0),
                movz_x(value_reg(1), 0x100, 0),
                crate::assembler::uxtb_x(FLAG_ALIGN_LHS_REG, value_reg(1)),
                cmp_x(FLAG_ALIGN_LHS_REG, 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                crate::assembler::uxtb_x(FLAG_ALIGN_RHS_REG, value_reg(0)),
                cmp_x(FLAG_ALIGN_RHS_REG, 31),
                cset_x(FLAG_ALIGN_RHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
                lsl_x(FLAG_ALIGN_RHS_REG, FLAG_ALIGN_RHS_REG, FLAG_ALIGN_SHIFT_REG),
                crate::assembler::orr_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_popcnt_to_zf_only_nzcv() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x8000_0000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsPopcnt(WriteFlagsPopcnt {
                    src: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0, 0),
                crate::assembler::movk_x(value_reg(0), 0x8000, 16),
                crate::assembler::uxtw_x(FLAG_ALIGN_LHS_REG, value_reg(0)),
                cmp_x(FLAG_ALIGN_LHS_REG, 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_rdtsc_and_fence() {
        let func = function(vec![
            Stmt::new(Some(0), Op::Rdtsc(Rdtsc)),
            Stmt::new(
                None,
                Op::Fence(Fence {
                    kind: FenceKind::Mfence,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![mrs_cntvct(value_reg(0)), fence(FenceKind::Mfence)]
        );
    }

    #[test]
    fn lowers_constant_add_and_return() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0149, 0xD280_00EA, 0x9100_1D2B, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_sub_immediate() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0149, 0xD280_006A, 0xD100_0D2B]
        );
    }

    #[test]
    fn lowers_large_add_immediate_via_register_fallback() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 10, 0),
                movz_x(value_reg(1), 0x1234, 0),
                add_x(value_reg(2), value_reg(0), value_reg(1)),
            ]
        );
    }

    #[test]
    fn lowers_logical_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0xf0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x0f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::Or,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Xor,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1E09,
                0xD280_01EA,
                0x8A0A_012B,
                0xAA0A_012C,
                0xCA0A_012D,
            ]
        );
    }

    #[test]
    fn lowers_shift_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x80,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shl,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shr,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sar,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::BinOp(BinOp {
                    op: BinOpKind::Ror,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1009,
                0xD280_006A,
                0x9ACA_212B,
                0x9ACA_252C,
                0x9ACA_292D,
                0x9ACA_2D2E,
            ]
        );
    }

    #[test]
    fn lowers_multiply_divide_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 21,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Mul,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::UMulHi,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::SMulHi,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::BinOp(BinOp {
                    op: BinOpKind::UDiv,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(6),
                Op::BinOp(BinOp {
                    op: BinOpKind::SDiv,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_02A9,
                0xD280_006A,
                0x9B0A_7D2B,
                0x9BCA_7D2C,
                0x9B4A_7D2D,
                0xB500_006A,
                0xD280_0000,
                0xD65F_03C0,
                0x9ACA_092E,
                0xB500_006A,
                0xD280_0000,
                0xD65F_03C0,
                0x9ACA_0D2F,
            ]
        );
    }

    #[test]
    fn lowers_lzcnt_and_tzcnt_i64_i32() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Lzcnt(Lzcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Tzcnt(Tzcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Lzcnt(Lzcnt {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Tzcnt(Tzcnt {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x10, 0),
                clz_x(value_reg(1), value_reg(0)),
                rbit_x(value_reg(2), value_reg(0)),
                clz_x(value_reg(2), value_reg(2)),
                clz_w(value_reg(3), value_reg(0)),
                rbit_w(value_reg(4), value_reg(0)),
                clz_w(value_reg(4), value_reg(4)),
            ]
        );
    }

    #[test]
    fn lowers_popcnt_i64_scalar_sequence() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Popcnt(Popcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x10, 0),
                mov_x(value_reg(1), value_reg(0)),
                movz_x(FLAG_ALIGN_SHIFT_REG, 1, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x5555, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 48),
                crate::assembler::and_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                crate::assembler::sub_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 2, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x3333, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 48),
                crate::assembler::and_x(value_reg(1), value_reg(1), FLAG_ALIGN_RHS_REG),
                crate::assembler::and_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                crate::assembler::add_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 4, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                crate::assembler::add_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 48),
                crate::assembler::and_x(value_reg(1), value_reg(1), FLAG_ALIGN_RHS_REG),
                movz_x(RSP_ADJUST_TMP_REG, 0x0101, 0),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 16),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 32),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 48),
                crate::assembler::mul_x(value_reg(1), value_reg(1), RSP_ADJUST_TMP_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsr_x(value_reg(1), value_reg(1), FLAG_ALIGN_SHIFT_REG),
            ]
        );
    }

    #[test]
    fn lowers_modulo_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 21,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::UMod,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::SMod,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_02A9,
                0xD280_00AA,
                0xB500_006A,
                0xD280_0000,
                0xD65F_03C0,
                0x9ACA_0934,
                0x9B0A_A68B,
                0xB500_006A,
                0xD280_0000,
                0xD65F_03C0,
                0x9ACA_0D34,
                0x9B0A_A68C,
            ]
        );
    }

    #[test]
    fn lowers_wide_div_unsigned_quotient_core() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::WideDiv(WideDiv {
                    high: 0,
                    low: 1,
                    divisor: 2,
                    signed: false,
                    result: WideDivResult::Quotient,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(
            code.len() > 64 * 8,
            "wide division should emit the 64-step core"
        );
        assert!(code.contains(&mov_x(WIDE_REM_REG, value_reg(0))));
        assert!(code.contains(&mov_x(WIDE_LOW_REG, value_reg(1))));
        assert!(code.contains(&mov_x(WIDE_DIVISOR_REG, value_reg(2))));
        assert!(code.contains(&lsr_x(WIDE_BIT_REG, WIDE_LOW_REG, WIDE_SHIFT_REG)));
        assert!(code.contains(&lsl_x(WIDE_LOW_REG, WIDE_LOW_REG, WIDE_ONE_REG)));
        assert!(code.contains(&cmp_x(WIDE_REM_REG, WIDE_DIVISOR_REG)));
        assert!(code.contains(&sub_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_DIVISOR_REG)));
        assert!(code.contains(&orr_x(value_reg(3), value_reg(3), WIDE_MASK_REG)));
    }

    #[test]
    fn lowers_wide_div_unsigned_remainder_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::WideDiv(WideDiv {
                    high: 0,
                    low: 1,
                    divisor: 2,
                    signed: false,
                    result: WideDivResult::Remainder,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&sub_x(WIDE_REM_REG, WIDE_REM_REG, WIDE_DIVISOR_REG)));
        assert!(code.contains(&mov_x(value_reg(3), WIDE_REM_REG)));
    }

    #[test]
    fn lowers_wide_div_signed_sign_normalization() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: u64::MAX,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: (-7i64).cast_unsigned(),
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::WideDiv(WideDiv {
                    high: 0,
                    low: 1,
                    divisor: 2,
                    signed: true,
                    result: WideDivResult::Quotient,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&lsr_x(WIDE_REM_SIGN_REG, WIDE_REM_REG, WIDE_SHIFT_REG)));
        assert!(code.contains(&eor_x(WIDE_QUOT_SIGN_REG, WIDE_REM_SIGN_REG, WIDE_TMP_REG)));
        assert!(code.contains(&sub_x(WIDE_DIVISOR_REG, 31, WIDE_DIVISOR_REG)));
        assert!(code.contains(&sub_x(value_reg(3), 31, value_reg(3))));
    }

    #[test]
    fn rejects_unsupported_binop() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::BinOp(BinOp {
                op: BinOpKind::Pdep,
                lhs: 1,
                rhs: 2,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp("BinOp"))
        );
    }

    #[test]
    fn rejects_missing_entry() {
        let func = Function {
            blocks: Vec::new(),
            entry: 42,
        };
        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingEntryBlock(42))
        );
    }

    #[test]
    fn lowers_wide_constant_with_movk() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::Constant(Constant {
                value: 0x1234_5678_9ABC_DEF0,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD29B_DE09, 0xF2B3_5789, 0xF2CA_CF09, 0xF2E2_4689]
        );
    }

    #[test]
    fn lowers_store_reg_from_constant() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x42,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: prisma_ir::Gpr::Rax,
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0849, 0xF900_0369]
        );
    }

    #[test]
    fn store_reg_i32_zero_extends_upper_half() {
        // A 32-bit register write must zero the upper 32 bits (x86-64). The
        // 8-byte slot therefore needs the low `STR Wt` plus a `STR WZR` to
        // [x27, #4]. Constant 0x42 -> x9, then StoreReg Rax I32.
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x42,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: prisma_ir::Gpr::Rax,
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        let words = Lowerer::new().lower_function(&func).unwrap();
        // Last two words: STR W9, [x27] then STR WZR, [x27, #4].
        assert_eq!(
            &words[words.len() - 2..],
            &[0xB900_0369, 0xB900_077F],
            "I32 StoreReg must low-store then zero the upper word"
        );
    }

    #[test]
    fn lowers_load_reg_to_value() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::LoadReg(LoadReg {
                reg: prisma_ir::Gpr::Rcx,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xF940_0769]
        );
    }

    #[test]
    fn lowers_add_registers_and_stores_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: prisma_ir::Gpr::Rcx,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::LoadReg(LoadReg {
                    reg: prisma_ir::Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: prisma_ir::Gpr::Rax,
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xF940_0769, 0xF940_036A, 0x8B09_014B, 0xF900_036B]
        );
    }

    #[test]
    fn lowers_i64_store_and_load_mem() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        // Each memory op rebases the guest VA: ldr x24,[x27,#MEM_BASE_OFFSET];
        // add x24, addr, x24; then the load/store off x24.
        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                movz_x(value_reg(1), 0x2a, 0),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                str_x_unsigned(value_reg(1), MEM_ADDR_SCRATCH, 0),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                ldr_x_unsigned(value_reg(2), MEM_ADDR_SCRATCH, 0),
            ]
        );
    }

    #[test]
    fn lowers_rep_movs_and_stos_to_bounded_stateful_loops() {
        let movs = function(vec![Stmt::new(
            None,
            Op::RepMovs(RepMovs {
                size: OpSize::I64,
                reverse: false,
                pc_of_rep: 0x1_4001_5c3e,
                pc_after_rep: 0x1_4001_5c41,
            }),
        )]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&movs)
            .expect("lower REP MOVSQ");
        assert!(words.contains(&ldr_x_unsigned(
            REP_RCX_REG,
            abi::K_STATE_PTR_REG,
            gpr_offset_bytes(Gpr::Rcx)
        )));
        assert!(words.contains(&ldr_x_unsigned(REP_VALUE_REG, MEM_ADDR_SCRATCH, 0)));
        assert!(words.contains(&str_x_unsigned(REP_VALUE_REG, MEM_ADDR_SCRATCH, 0)));
        assert!(words.contains(&str_x_unsigned(
            REP_RSI_REG,
            abi::K_STATE_PTR_REG,
            gpr_offset_bytes(Gpr::Rsi)
        )));
        assert_eq!(words.last(), Some(&0xD65F_03C0));

        let stos = function(vec![Stmt::new(
            None,
            Op::RepStos(RepStos {
                size: OpSize::I8,
                reverse: true,
                pc_of_rep: 0x2000,
                pc_after_rep: 0x2002,
            }),
        )]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&stos)
            .expect("lower REP STOSB");
        assert!(words.contains(&strb_unsigned(REP_VALUE_REG, MEM_ADDR_SCRATCH, 0)));
        assert!(words.contains(&sub_x_imm(REP_RDI_REG, REP_RDI_REG, 1)));
        assert_eq!(words.last(), Some(&0xD65F_03C0));
    }

    #[test]
    fn lowers_vec_constant_and_xmm_round_trip_as_two_u64_words() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 0x11, hi: 0x22 })),
            Stmt::new(
                None,
                Op::StoreVecReg(StoreVecReg {
                    xmm_index: 3,
                    value: 0,
                }),
            ),
            Stmt::new(Some(1), Op::LoadVecReg(LoadVecReg { xmm_index: 3 })),
            Stmt::new(
                None,
                Op::StoreVecReg(StoreVecReg {
                    xmm_index: 4,
                    value: 1,
                }),
            ),
        ]);

        let xmm3 = XMM_BASE_OFFSET + 3 * XMM_SLOT_BYTES;
        let xmm4 = XMM_BASE_OFFSET + 4 * XMM_SLOT_BYTES;
        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(FLAG_ALIGN_SHIFT_REG, 0x11, 0),
                movz_x(MOD_QUOTIENT_REG, 0x22, 0),
                str_x_unsigned(FLAG_ALIGN_SHIFT_REG, abi::K_STATE_PTR_REG, xmm3),
                str_x_unsigned(MOD_QUOTIENT_REG, abi::K_STATE_PTR_REG, xmm3 + 8),
                ldr_x_unsigned(RSP_ADJUST_TMP_REG, abi::K_STATE_PTR_REG, xmm3),
                ldr_x_unsigned(RSP_ADJUST_IMM_REG, abi::K_STATE_PTR_REG, xmm3 + 8),
                str_x_unsigned(RSP_ADJUST_TMP_REG, abi::K_STATE_PTR_REG, xmm4),
                str_x_unsigned(RSP_ADJUST_IMM_REG, abi::K_STATE_PTR_REG, xmm4 + 8),
            ]
        );
    }

    #[test]
    fn lowers_vec_memory_load_and_store_as_two_rebased_u64_words() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(Some(1), Op::LoadVec(LoadVec { addr: 0 })),
            Stmt::new(None, Op::StoreVec(StoreVec { addr: 0, value: 1 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                ldr_x_unsigned(FLAG_ALIGN_SHIFT_REG, MEM_ADDR_SCRATCH, 0),
                ldr_x_unsigned(MOD_QUOTIENT_REG, MEM_ADDR_SCRATCH, 8),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                str_x_unsigned(FLAG_ALIGN_SHIFT_REG, MEM_ADDR_SCRATCH, 0),
                str_x_unsigned(MOD_QUOTIENT_REG, MEM_ADDR_SCRATCH, 8),
            ]
        );
    }

    #[test]
    fn lowers_vec_clmul_with_scalar_carryless_loop() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 0xaa, hi: 0x02 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 0x55, hi: 0x03 })),
            Stmt::new(
                Some(2),
                Op::VecClMul(VecClMul {
                    lhs: 0,
                    rhs: 1,
                    lhs_high: true,
                    rhs_high: true,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreVecReg(StoreVecReg {
                    xmm_index: 0,
                    value: 2,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&lsl_x_imm(CAS_STATUS_REG, MEM_ADDR_SCRATCH, 63)));
        assert!(code.contains(&eor_x(
            ALU_FLAGS_TMP_REG,
            ALU_FLAGS_TMP_REG,
            FLAG_ALIGN_LHS_REG
        )));
        assert!(code.contains(&eor_x(WIDE_SHIFT_REG, WIDE_SHIFT_REG, FLAG_ALIGN_RHS_REG)));
        assert!(code.contains(&lsr_x_imm(CAS_STATUS_REG, FLAG_ALIGN_LHS_REG, 63)));
        assert!(code.contains(&sub_x_imm(30, 30, 1)));
    }

    #[test]
    fn backend_f16c_helpers_round_trip_exact_lanes() {
        let packed_halves = 0x7c00_0000_c000_3c00u64;
        let expected =
            (u128::from(0x7f80_0000_0000_0000u64) << 64) | u128::from(0xc000_0000_3f80_0000u64);

        assert_eq!(backend_f16c_ph_to_ps(packed_halves), expected);
        assert_eq!(
            backend_f16c_ps_to_ph(low64(expected), low64(expected >> 64), 0),
            packed_halves
        );
    }

    #[test]
    fn lowers_vec_f16cvt_via_semantic_helpers() {
        let ph2ps = function(vec![
            Stmt::new(
                Some(0),
                Op::VecConstant(VecConstant {
                    lo: 0x7c00_0000_c000_3c00,
                    hi: 0,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::VecF16Cvt(VecF16Cvt {
                    kind: VecF16CvtKind::PhToPs,
                    src: 0,
                    rounding: 0,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreVecReg(StoreVecReg {
                    xmm_index: 0,
                    value: 1,
                }),
            ),
        ]);
        let ph2ps_code = Lowerer::new().lower_function(&ph2ps).unwrap();
        assert_eq!(
            ph2ps_code
                .iter()
                .filter(|word| **word == blr_x(PCMP_HELPER_TARGET_REG))
                .count(),
            2
        );

        let ps2ph = function(vec![
            Stmt::new(
                Some(0),
                Op::VecConstant(VecConstant {
                    lo: 0xc000_0000_3f80_0000,
                    hi: 0x7f80_0000_0000_0000,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::VecF16Cvt(VecF16Cvt {
                    kind: VecF16CvtKind::PsToPh,
                    src: 0,
                    rounding: 0,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreVecReg(StoreVecReg {
                    xmm_index: 1,
                    value: 1,
                }),
            ),
        ]);
        let ps2ph_code = Lowerer::new().lower_function(&ps2ph).unwrap();
        assert_eq!(
            ps2ph_code
                .iter()
                .filter(|word| **word == blr_x(PCMP_HELPER_TARGET_REG))
                .count(),
            1
        );
    }

    #[test]
    fn backend_pcmp_string_helpers_cover_index_mask_and_flags() {
        fn pack(bytes: &[u8]) -> (u64, u64) {
            let mut padded = [0u8; 16];
            padded[..bytes.len()].copy_from_slice(bytes);
            (
                u64::from_le_bytes(padded[..8].try_into().unwrap()),
                u64::from_le_bytes(padded[8..].try_into().unwrap()),
            )
        }

        let (lhs_lo, lhs_hi) = pack(b"zab");
        let (rhs_lo, rhs_hi) = pack(b"abc");
        assert_eq!(
            pcmpstr_index_helper(lhs_lo, lhs_hi, rhs_lo, rhs_hi, 0, 0, 0, 0),
            1
        );

        let (lhs_lo, lhs_hi) = pack(&[1, 2, 3]);
        let (rhs_lo, rhs_hi) = pack(&[1, 9, 3]);
        let mode = PCMP_LEN_LHS_EXPLICIT | PCMP_LEN_RHS_EXPLICIT;
        assert_eq!(
            pcmpstr_mask_lo_helper(lhs_lo, lhs_hi, rhs_lo, rhs_hi, 3, 3, mode, 0x08),
            0b101
        );
        assert_eq!(
            pcmpstr_flags_helper(lhs_lo, lhs_hi, rhs_lo, rhs_hi, 3, 3, mode, 0x08),
            0b1111
        );
    }

    #[test]
    fn lowers_pcmp_string_ops_via_semantic_helpers() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::VecConstant(VecConstant {
                    lo: 0x0062_6100,
                    hi: 0,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::VecConstant(VecConstant {
                    lo: 0x0063_6261,
                    hi: 0,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::PcmpStrIndex(PcmpStrIndex {
                    lhs: 0,
                    rhs: 1,
                    lhs_len: None,
                    rhs_len: None,
                    imm8: 0,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::PcmpStrMask(PcmpStrMask {
                    lhs: 0,
                    rhs: 1,
                    lhs_len: None,
                    rhs_len: None,
                    imm8: 0,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::PcmpStrFlags(PcmpStrFlags {
                    lhs: 0,
                    rhs: 1,
                    lhs_len: None,
                    rhs_len: None,
                    imm8: 0,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert_eq!(
            code.iter()
                .filter(|word| **word == blr_x(PCMP_HELPER_TARGET_REG))
                .count(),
            4
        );
        assert!(code.contains(&stp_x_pre_sp(9, 10, -16)));
        assert!(code.contains(&stp_x_pre_sp(29, 30, -16)));
        assert!(code.contains(&ldp_x_post_sp(29, 30, 16)));
        assert!(code.contains(&ldp_x_post_sp(9, 10, 16)));
        assert!(code.contains(&mov_x(value_reg(2), 0)));
        assert!(code.contains(&mov_x(ALU_FLAGS_TMP_REG, 0)));
        assert!(code.contains(&mov_x(WIDE_SHIFT_REG, 0)));
        assert!(code.contains(&mov_x(value_reg(4), 0)));
    }

    #[test]
    fn lowers_vec_xor_as_two_scalar_eors() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 3, hi: 4 })),
            Stmt::new(
                Some(2),
                Op::VecBinOp(VecBinOp {
                    op: VecBinOpKind::Xor,
                    lhs: 0,
                    rhs: 1,
                    lane: VecLane::B16,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&eor_x(
            VEC_REG_PAIRS[2].0,
            VEC_REG_PAIRS[0].0,
            VEC_REG_PAIRS[1].0
        )));
        assert!(code.contains(&eor_x(
            VEC_REG_PAIRS[2].1,
            VEC_REG_PAIRS[0].1,
            VEC_REG_PAIRS[1].1
        )));
    }

    #[test]
    fn vec_unpack_helper_matches_punpck_low_and_high_families() {
        let lhs = 0x0f0e_0d0c_0b0a_0908_0706_0504_0302_0100u128;
        let rhs = 0x1f1e_1d1c_1b1a_1918_1716_1514_1312_1110u128;
        for (lane, high, expected) in [
            (
                VecLane::B16,
                false,
                0x1707_1606_1505_1404_1303_1202_1101_1000u128,
            ),
            (
                VecLane::H8,
                true,
                0x1f1e_0f0e_1d1c_0d0c_1b1a_0b0a_1918_0908u128,
            ),
            (
                VecLane::S4,
                false,
                0x1716_1514_0706_0504_1312_1110_0302_0100u128,
            ),
            (
                VecLane::D2,
                true,
                0x1f1e_1d1c_1b1a_1918_0f0e_0d0c_0b0a_0908u128,
            ),
        ] {
            let lo = vec_unpack_helper(
                lhs as u64,
                (lhs >> 64) as u64,
                rhs as u64,
                (rhs >> 64) as u64,
                u64::from(lane as u8),
                u64::from(high),
                0,
            );
            let hi = vec_unpack_helper(
                lhs as u64,
                (lhs >> 64) as u64,
                rhs as u64,
                (rhs >> 64) as u64,
                u64::from(lane as u8),
                u64::from(high),
                1,
            );
            assert_eq!((u128::from(hi) << 64) | u128::from(lo), expected);
        }
    }

    #[test]
    fn lowers_vec_unpack_through_the_arm64ec_safe_helper() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 3, hi: 4 })),
            Stmt::new(
                Some(2),
                Op::VecUnpack(VecUnpack {
                    is_high: false,
                    lhs: 0,
                    rhs: 1,
                    lane: VecLane::B16,
                }),
            ),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert_eq!(
            code.iter()
                .filter(|word| **word == blr_x(PCMP_HELPER_TARGET_REG))
                .count(),
            2
        );
        assert!(code.contains(&mov_x(VEC_REG_PAIRS[2].0, 0)));
        assert!(code.contains(&mov_x(VEC_REG_PAIRS[2].1, 0)));
    }

    #[test]
    fn vec_shuffle_h4_helper_matches_pshuflw_and_pshufhw() {
        let src = 0x7777_6666_5555_4444_3333_2222_1111_0000u128;
        for (high, control, expected) in [
            (false, 0x00, 0x7777_6666_5555_4444_0000_0000_0000_0000u128),
            (true, 0x1b, 0x4444_5555_6666_7777_3333_2222_1111_0000u128),
        ] {
            let lo =
                vec_shuffle_h4_helper(src as u64, (src >> 64) as u64, control, u64::from(high), 0);
            let hi =
                vec_shuffle_h4_helper(src as u64, (src >> 64) as u64, control, u64::from(high), 1);
            assert_eq!((u128::from(hi) << 64) | u128::from(lo), expected);
        }
    }

    #[test]
    fn vector_compare_and_mask_helpers_match_pcmpeqb_pipeline() {
        let lhs = 0x0f0e_0d0c_0b0a_0908_0706_0504_0302_0100u128;
        let rhs = 0xff0e_ff0c_ff0a_ff08_ff06_ff04_ff02_ff00u128;
        let lo = vec_cmp_helper(
            lhs as u64,
            (lhs >> 64) as u64,
            rhs as u64,
            (rhs >> 64) as u64,
            u64::from(VecLane::B16 as u8),
            u64::from(VecCmpKind::Eq as u8),
            0,
        );
        let hi = vec_cmp_helper(
            lhs as u64,
            (lhs >> 64) as u64,
            rhs as u64,
            (rhs >> 64) as u64,
            u64::from(VecLane::B16 as u8),
            u64::from(VecCmpKind::Eq as u8),
            1,
        );
        assert_eq!(vec_mask_msb_helper(lo, hi), 0x5555);
    }

    #[test]
    fn lowers_vec_compare_and_mask_with_abi_safe_helpers() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 3, hi: 4 })),
            Stmt::new(
                Some(2),
                Op::VecCmp(VecCmp {
                    kind: VecCmpKind::Eq,
                    lhs: 0,
                    rhs: 1,
                    lane: VecLane::B16,
                }),
            ),
            Stmt::new(Some(3), Op::VecMaskMsb(VecMaskMsb { src_xmm: 2 })),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert_eq!(
            code.iter()
                .filter(|word| **word == blr_x(PCMP_HELPER_TARGET_REG))
                .count(),
            3
        );
        assert!(code.contains(&mov_x(value_reg(3), 0)));
    }

    #[test]
    fn lowers_pshufd_lane_zero_broadcast_to_four_u32_lanes() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::VecConstant(VecConstant {
                    lo: 0x1122_3344_5566_7788,
                    hi: 0x99aa_bbcc_ddee_ff00,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::VecShuffle32x4(VecShuffle32x4 { src: 0, control: 0 }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert_eq!(
            &code[code.len() - 8..],
            &[
                uxtw_x(VEC_REG_PAIRS[1].0, VEC_REG_PAIRS[0].0),
                uxtw_x(CAS_STATUS_REG, VEC_REG_PAIRS[0].0),
                lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32),
                orr_x(VEC_REG_PAIRS[1].0, VEC_REG_PAIRS[1].0, CAS_STATUS_REG),
                uxtw_x(VEC_REG_PAIRS[1].1, VEC_REG_PAIRS[0].0),
                uxtw_x(CAS_STATUS_REG, VEC_REG_PAIRS[0].0),
                lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32),
                orr_x(VEC_REG_PAIRS[1].1, VEC_REG_PAIRS[1].1, CAS_STATUS_REG),
            ]
        );
    }

    #[test]
    fn lowers_paddd_without_carry_between_u32_lanes() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 3, hi: 4 })),
            Stmt::new(
                Some(2),
                Op::VecBinOp(VecBinOp {
                    op: VecBinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    lane: VecLane::S4,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        for (dst, lhs, rhs) in [
            (VEC_REG_PAIRS[2].0, VEC_REG_PAIRS[0].0, VEC_REG_PAIRS[1].0),
            (VEC_REG_PAIRS[2].1, VEC_REG_PAIRS[0].1, VEC_REG_PAIRS[1].1),
        ] {
            assert!(code.contains(&uxtw_x(dst, lhs)));
            assert!(code.contains(&add_x(dst, dst, CAS_STATUS_REG)));
            assert!(code.contains(&uxtw_x(dst, dst)));
            assert!(code.contains(&lsr_x_imm(FLAG_ALIGN_LHS_REG, lhs, 32)));
            assert!(code.contains(&lsr_x_imm(CAS_STATUS_REG, rhs, 32)));
        }
    }

    #[test]
    fn lowers_pslld_and_psrld_per_u32_lane() {
        for kind in [VecShiftKind::ShiftL, VecShiftKind::LogicalShr] {
            let func = function(vec![
                Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
                Stmt::new(
                    Some(1),
                    Op::VecShiftImm(VecShiftImm {
                        kind,
                        src: 0,
                        count: 16,
                        lane: VecLane::S4,
                    }),
                ),
            ]);
            let code = Lowerer::new().lower_function(&func).unwrap();
            assert!(code.contains(&uxtw_x(VEC_REG_PAIRS[1].0, VEC_REG_PAIRS[0].0)));
            assert!(code.contains(&lsr_x_imm(CAS_STATUS_REG, VEC_REG_PAIRS[0].0, 32)));
            assert!(code.contains(&lsl_x_imm(CAS_STATUS_REG, CAS_STATUS_REG, 32)));
            assert!(code.contains(&orr_x(
                VEC_REG_PAIRS[1].0,
                VEC_REG_PAIRS[1].0,
                CAS_STATUS_REG
            )));
        }
    }

    #[test]
    fn fp_compare_helpers_match_x86_ucomi_flags() {
        assert_eq!(
            fp64_rflags_helper(1.0f64.to_bits(), 2.0f64.to_bits()),
            RFLAGS_CF_BIT
        );
        assert_eq!(fp64_rflags_helper(2.0f64.to_bits(), 1.0f64.to_bits()), 0);
        assert_eq!(
            fp64_rflags_helper((-0.0f64).to_bits(), 0.0f64.to_bits()),
            RFLAGS_ZF_BIT
        );
        assert_eq!(
            fp64_rflags_helper(f64::NAN.to_bits(), 0.0f64.to_bits()),
            RFLAGS_ZF_BIT | RFLAGS_PF_BIT | RFLAGS_CF_BIT
        );
    }

    #[test]
    fn lowers_signed_integer_to_f64_with_native_fp() {
        assert_eq!(i64_to_f64_helper(u64::MAX), (-1.0f64).to_bits());
        assert_eq!(i32_to_f64_helper(u64::from(u32::MAX)), (-1.0f64).to_bits());
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 42,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::IntToFpScalar(IntToFpScalar {
                    value: 0,
                    int_size: OpSize::I64,
                    fp_size: FpSize::F64,
                }),
            ),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&scvtf_d_x(0, value_reg(0))));
        assert!(code.contains(&fmov_x_d(VEC_REG_PAIRS[0].0, 0)));
        assert!(code.contains(&movz_x(VEC_REG_PAIRS[0].1, 0, 0)));
    }

    #[test]
    fn lowers_scalar_f64_arithmetic_and_preserves_upper_xmm() {
        assert_eq!(
            f64_add_helper(1.5f64.to_bits(), 2.0f64.to_bits()),
            3.5f64.to_bits()
        );
        assert_eq!(
            f64_sub_helper(1.5f64.to_bits(), 2.0f64.to_bits()),
            (-0.5f64).to_bits()
        );
        assert_eq!(
            f64_mul_helper(1.5f64.to_bits(), 2.0f64.to_bits()),
            3.0f64.to_bits()
        );
        assert_eq!(
            f64_div_helper(1.5f64.to_bits(), 2.0f64.to_bits()),
            0.75f64.to_bits()
        );
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 2 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 3, hi: 4 })),
            Stmt::new(
                Some(2),
                Op::VecFpScalarBinOp(VecFpScalarBinOp {
                    op: VecFpBinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: FpSize::F64,
                }),
            ),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&fmov_d_x(0, VEC_REG_PAIRS[0].0)));
        assert!(code.contains(&fmov_d_x(1, VEC_REG_PAIRS[1].0)));
        assert!(code.contains(&fadd_d(0, 0, 1)));
        assert!(code.contains(&fmov_x_d(VEC_REG_PAIRS[2].0, 0)));
        assert!(code.contains(&mov_x(VEC_REG_PAIRS[2].1, VEC_REG_PAIRS[0].1)));
    }

    #[test]
    fn lowers_cvttsd2si_with_x86_indefinite_results() {
        assert_eq!(f64_to_i32_trunc_helper(42.9f64.to_bits()), 42);
        assert_eq!(
            f64_to_i32_trunc_helper((-42.9f64).to_bits()),
            u64::from((-42_i32) as u32)
        );
        assert_eq!(
            f64_to_i32_trunc_helper(f64::NAN.to_bits()),
            u64::from(0x8000_0000_u32)
        );
        assert_eq!(
            f64_to_i64_trunc_helper(f64::INFINITY.to_bits()),
            i64::MIN as u64
        );
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 0 })),
            Stmt::new(
                Some(1),
                Op::FpToIntScalar(FpToIntScalar {
                    value: 0,
                    fp_size: FpSize::F64,
                    int_size: OpSize::I32,
                }),
            ),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&fmov_d_x(0, VEC_REG_PAIRS[0].0)));
        assert!(code.contains(&fcvtzs_w_d(value_reg(1), 0)));
        assert!(!code.contains(&blr_x(PCMP_HELPER_TARGET_REG)));
    }

    #[test]
    fn lowers_fp64_flag_compare_without_arm64ec_helper_transition() {
        let func = function(vec![
            Stmt::new(Some(0), Op::VecConstant(VecConstant { lo: 1, hi: 0 })),
            Stmt::new(Some(1), Op::VecConstant(VecConstant { lo: 2, hi: 0 })),
            Stmt::new(
                None,
                Op::WriteFlagsFp(WriteFlagsFp {
                    lhs: 0,
                    rhs: 1,
                    size: prisma_ir::FpSize::F64,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code.contains(&fmov_d_x(0, VEC_REG_PAIRS[0].0)));
        assert!(code.contains(&fmov_d_x(1, VEC_REG_PAIRS[1].0)));
        assert!(code.contains(&fcmp_d(0, 1)));
        assert!(!code.contains(&blr_x(PCMP_HELPER_TARGET_REG)));
        assert!(code.contains(&str_x_unsigned(
            NZCV_TMP_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
    }

    #[test]
    fn lowers_tso_memory_ops_with_full_barriers() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMemTSO(StoreMemTSO {
                    addr: 0,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMemTSO(LoadMemTSO {
                    addr: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                movz_x(value_reg(1), 0x2a, 0),
                fence(FenceKind::Mfence),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                str_x_unsigned(value_reg(1), MEM_ADDR_SCRATCH, 0),
                fence(FenceKind::Mfence),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                ldr_x_unsigned(value_reg(2), MEM_ADDR_SCRATCH, 0),
                fence(FenceKind::Mfence),
            ]
        );
    }

    #[test]
    fn lowers_atomic_cmpxchg_i64_to_exclusive_loop() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 0x63,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::AtomicCmpxchg(AtomicCmpxchg {
                    addr: 0,
                    expected: 1,
                    new_value: 2,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                movz_x(value_reg(1), 0x2a, 0),
                movz_x(value_reg(2), 0x63, 0),
                mov_x(ATOMIC_CMPXCHG_EXPECTED_REG, value_reg(1)),
                mov_x(ATOMIC_CMPXCHG_NEW_REG, value_reg(2)),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                ldaxr_x(value_reg(3), MEM_ADDR_SCRATCH),
                cmp_x(value_reg(3), ATOMIC_CMPXCHG_EXPECTED_REG),
                b_cond(CondCode::Ne, 16),
                stlxr_x(CAS_STATUS_REG, ATOMIC_CMPXCHG_NEW_REG, MEM_ADDR_SCRATCH),
                crate::assembler::cbnz_x(CAS_STATUS_REG, -16),
                b(8),
                clrex(),
            ]
        );
    }

    #[test]
    fn atomic_cmpxchg_preserves_new_value_when_result_register_wraps_to_same_slot() {
        let mut stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1806_09ee_e000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ];
        for reference in 3..8 {
            stmts.push(Stmt::new(
                Some(reference),
                Op::Constant(Constant {
                    value: u64::from(reference),
                    size: OpSize::I64,
                }),
            ));
        }
        stmts.push(Stmt::new(
            Some(8),
            Op::AtomicCmpxchg(AtomicCmpxchg {
                addr: 1,
                expected: 2,
                new_value: 0,
                size: OpSize::I64,
            }),
        ));

        assert_eq!(value_reg(0), value_reg(8), "test requires allocator wrap");
        let code = Lowerer::new().lower_function(&function(stmts)).unwrap();
        let preserve = mov_x(ATOMIC_CMPXCHG_NEW_REG, value_reg(0));
        let load_old = ldaxr_x(value_reg(8), MEM_ADDR_SCRATCH);
        let preserve_index = code.iter().position(|word| *word == preserve).unwrap();
        let load_index = code.iter().position(|word| *word == load_old).unwrap();
        assert!(preserve_index < load_index);
        assert!(code.iter().any(|word| {
            *word == stlxr_x(CAS_STATUS_REG, ATOMIC_CMPXCHG_NEW_REG, MEM_ADDR_SCRATCH)
        }));
    }

    #[test]
    fn lowers_atomic_xadd_i64_to_exclusive_retry_loop() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::AtomicXadd(AtomicXadd {
                    addr: 0,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        assert!(code
            .iter()
            .any(|word| *word == mov_x(ATOMIC_RMW_SOURCE_REG, value_reg(1))));
        assert!(code
            .iter()
            .any(|word| *word == ldaxr_x(value_reg(2), MEM_ADDR_SCRATCH)));
        assert!(code.iter().any(|word| {
            *word == add_x(FLAG_ALIGN_LHS_REG, value_reg(2), ATOMIC_RMW_SOURCE_REG)
        }));
        assert!(code.iter().any(|word| {
            *word == stlxr_x(CAS_STATUS_REG, FLAG_ALIGN_LHS_REG, MEM_ADDR_SCRATCH)
        }));
    }

    #[test]
    fn atomic_xadd_preserves_source_when_result_register_wraps_to_same_slot() {
        let mut stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
        ];
        for reference in 2..8 {
            stmts.push(Stmt::new(
                Some(reference),
                Op::Constant(Constant {
                    value: u64::from(reference),
                    size: OpSize::I64,
                }),
            ));
        }
        stmts.extend([
            Stmt::new(
                Some(8),
                Op::AtomicXadd(AtomicXadd {
                    addr: 1,
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rbx,
                    value: 8,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 8,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(value_reg(0), value_reg(8), "test requires allocator wrap");
        let code = Lowerer::new().lower_function(&function(stmts)).unwrap();
        let preserve = mov_x(ATOMIC_RMW_SOURCE_REG, value_reg(0));
        let load_old = ldaxr_x(value_reg(8), MEM_ADDR_SCRATCH);
        let add_new = add_x(FLAG_ALIGN_LHS_REG, value_reg(8), ATOMIC_RMW_SOURCE_REG);
        let flags = adds_x(ALU_FLAGS_TMP_REG, value_reg(8), ATOMIC_RMW_SOURCE_REG);
        let preserve_index = code.iter().position(|word| *word == preserve).unwrap();
        let load_index = code.iter().position(|word| *word == load_old).unwrap();
        assert!(
            preserve_index < load_index,
            "source must be saved before LDAXR"
        );
        assert!(
            code.contains(&add_new),
            "atomic sum must use preserved input"
        );
        assert!(code.contains(&flags), "flags must use preserved input");
    }

    #[test]
    fn atomic_xchg_i8_preserves_source_before_exclusive_load_when_registers_alias() {
        let mut stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
        ];
        for reference in 2..8 {
            stmts.push(Stmt::new(
                Some(reference),
                Op::Constant(Constant {
                    value: u64::from(reference),
                    size: OpSize::I64,
                }),
            ));
        }
        stmts.push(Stmt::new(
            Some(8),
            Op::AtomicXchg(AtomicXchg {
                addr: 1,
                value: 0,
                size: OpSize::I8,
            }),
        ));

        assert_eq!(value_reg(0), value_reg(8), "test requires allocator wrap");
        let code = Lowerer::new().lower_function(&function(stmts)).unwrap();
        let preserve = mov_x(ATOMIC_RMW_SOURCE_REG, value_reg(0));
        let load_old = crate::assembler::ldaxrb(value_reg(8), MEM_ADDR_SCRATCH);
        let store_new =
            crate::assembler::stlxrb(CAS_STATUS_REG, ATOMIC_RMW_SOURCE_REG, MEM_ADDR_SCRATCH);
        let retry = crate::assembler::cbnz_x(CAS_STATUS_REG, -8);
        let preserve_index = code.iter().position(|word| *word == preserve).unwrap();
        let load_index = code.iter().position(|word| *word == load_old).unwrap();
        assert!(preserve_index < load_index, "source must survive LDAXRB");
        assert!(code.contains(&store_new), "exchange must use STLXRB");
        assert!(code.contains(&retry), "failed exclusive store must retry");
    }

    #[test]
    fn lowers_atomic_cmpxchg_pair_to_exclusive_pair_loop() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x11,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 0x22,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Constant(Constant {
                    value: 0x33,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Constant(Constant {
                    value: 0x44,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::AtomicCmpxchgPair(AtomicCmpxchgPair {
                    addr: 0,
                    expected_low: 1,
                    expected_high: 2,
                    new_low: 3,
                    new_high: 4,
                    old_high: 6,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rdx,
                    value: 6,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                movz_x(value_reg(1), 0x11, 0),
                movz_x(value_reg(2), 0x22, 0),
                movz_x(value_reg(3), 0x33, 0),
                movz_x(value_reg(4), 0x44, 0),
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, value_reg(0), MEM_ADDR_SCRATCH),
                ldaxp_x(value_reg(5), value_reg(6), MEM_ADDR_SCRATCH),
                cmp_x(value_reg(5), value_reg(1)),
                b_cond(CondCode::Ne, 24),
                cmp_x(value_reg(6), value_reg(2)),
                b_cond(CondCode::Ne, 16),
                stlxp_x(CAS_STATUS_REG, value_reg(3), value_reg(4), MEM_ADDR_SCRATCH),
                crate::assembler::cbnz_x(CAS_STATUS_REG, -24),
                b(8),
                clrex(),
                str_x_unsigned(
                    value_reg(6),
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rdx)
                ),
            ]
        );
    }

    #[test]
    fn lowers_i8_i16_i32_memory_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        // Every access rebases via x24 first (ldr base; add x24, addr, base).
        let rebase = |addr: u8| {
            [
                ldr_x_unsigned(MEM_ADDR_SCRATCH, abi::K_STATE_PTR_REG, MEM_BASE_OFFSET),
                add_x(MEM_ADDR_SCRATCH, addr, MEM_ADDR_SCRATCH),
            ]
        };
        let a = value_reg(0);
        let mut expected = vec![
            movz_x(value_reg(0), 0x1000, 0),
            movz_x(value_reg(1), 0x2a, 0),
        ];
        expected.extend(rebase(a));
        expected.push(strb_unsigned(value_reg(1), MEM_ADDR_SCRATCH, 0));
        expected.extend(rebase(a));
        expected.push(ldrb_unsigned(value_reg(2), MEM_ADDR_SCRATCH, 0));
        expected.extend(rebase(a));
        expected.push(strh_unsigned(value_reg(1), MEM_ADDR_SCRATCH, 0));
        expected.extend(rebase(a));
        expected.push(ldrh_unsigned(value_reg(3), MEM_ADDR_SCRATCH, 0));
        expected.extend(rebase(a));
        expected.push(str_w_unsigned(value_reg(1), MEM_ADDR_SCRATCH, 0));
        expected.extend(rebase(a));
        expected.push(ldr_w_unsigned(value_reg(4), MEM_ADDR_SCRATCH, 0));
        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_compare_eq() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_00E9, 0xD280_00EA, 0xEB0A_013F, 0x9A9F_17EB]
        );
    }

    #[test]
    fn lowers_i8_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        let lhs_reg = value_reg(0);
        let rhs_reg = value_reg(1);
        let result_reg = value_reg(2);
        let shift = u16::try_from(64 - OpSize::I8.bit_width()).expect("I8 fits in u16 shift");
        let expected = vec![
            movz_x(lhs_reg, 0x7f, 0),
            movz_x(rhs_reg, 0x7f, 0),
            movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
            lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
            lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
            cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            cset_x(result_reg, CondCode::Eq),
        ];

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_i16_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I16,
                }),
            ),
        ]);

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), {
            let lhs_reg = value_reg(0);
            let rhs_reg = value_reg(1);
            let result_reg = value_reg(2);
            let shift = u16::try_from(64 - OpSize::I16.bit_width()).expect("I16 fits in u16 shift");
            vec![
                movz_x(lhs_reg, 5, 0),
                movz_x(rhs_reg, 5, 0),
                movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                cset_x(result_reg, CondCode::Eq),
            ]
        });
    }

    #[test]
    fn lowers_i32_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
        ]);

        let lhs_reg = value_reg(0);
        let rhs_reg = value_reg(1);
        let result_reg = value_reg(2);
        let shift = u16::try_from(64 - OpSize::I32.bit_width()).expect("I32 fits in u16 shift");
        let expected = vec![
            movz_x(lhs_reg, 10, 0),
            movz_x(rhs_reg, 10, 0),
            movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
            lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
            lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
            cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            cset_x(result_reg, CondCode::Eq),
        ];

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_forward_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(None, Op::Jump(Jump { target_block: 1 })),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0029, 0x1400_0001, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_backward_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 1 }))],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 0 }))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001, 0x17FF_FFFF]
        );
    }

    #[test]
    fn rejects_jump_to_missing_block() {
        let func = function(vec![Stmt::new(None, Op::Jump(Jump { target_block: 42 }))]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingTargetBlock(42))
        );
    }

    #[test]
    fn lowers_cond_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJump(CondJump {
                                cond: 0,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0029,
                0xB500_0049,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn returns_via_epilogue_flag_emits_full_epilogue() {
        let func = function_with_blocks(
            vec![BasicBlock {
                id: 0,
                stmts: vec![Stmt::new(None, Op::Return(Return))],
            }],
            0,
        );
        // Default: a bare `ret`.
        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD65F_03C0]
        );
        // With the flag: the full epilogue (6 callee-saved ldp pairs + ret).
        let with_epi = Lowerer::new()
            .with_returns_via_epilogue()
            .lower_function(&func)
            .unwrap();
        assert_eq!(*with_epi.last().unwrap(), 0xD65F_03C0, "ends in ret");
        assert_eq!(with_epi.len(), 7, "6 ldp restores + ret");
    }

    #[test]
    fn lowers_cond_jump_rel_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJumpRel(CondJumpRel {
                            cc: CondCode::Eq,
                            target_guest_pc: 1,
                            fallthrough_guest_pc: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        let mut expected = restore_nzcv_from_rflags_words();
        expected.extend([0x5400_0040, 0x1400_0002, 0xD65F_03C0, 0xD65F_03C0]);
        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_cond_jump_rel_without_restore_after_local_flags() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpRel(CondJumpRel {
                                cc: CondCode::Eq,
                                target_guest_pc: 1,
                                fallthrough_guest_pc: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        let code = Lowerer::new().lower_function(&func).unwrap();
        let restore = restore_nzcv_from_rflags_words();
        assert!(!code
            .windows(restore.len())
            .any(|window| window == restore.as_slice()));
        assert!(code.contains(&cmp_x(value_reg(0), value_reg(1))));
    }

    #[test]
    fn lowers_select_false_path_from_compare() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 1, 0),
                movz_x(value_reg(1), 2, 0),
                cmp_x(value_reg(0), value_reg(1)),
                cset_x(value_reg(2), CondCode::Eq),
                b_cond(CondCode::Eq, 12),
                mov_x(value_reg(3), value_reg(1)),
                b(8),
                mov_x(value_reg(3), value_reg(0)),
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_select_true_path_from_compare() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 1, 0),
                movz_x(value_reg(1), 2, 0),
                cmp_x(value_reg(0), value_reg(0)),
                cset_x(value_reg(2), CondCode::Eq),
                b_cond(CondCode::Eq, 12),
                mov_x(value_reg(3), value_reg(1)),
                b(8),
                mov_x(value_reg(3), value_reg(0)),
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn rejects_select_missing_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingResult("Select"))
        );
    }

    #[test]
    fn rejects_select_missing_true_value() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 99,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
        );
    }

    #[test]
    fn lowers_compare_then_cond_jump() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 7,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 3,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::Compare(Compare {
                                cc: CondCode::Ne,
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJump(CondJump {
                                cond: 2,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                0x9A9F_07EB,
                0xB500_004B,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_cmp_flags_then_cond_jump_flags() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 7,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 3,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Ne,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                0x5400_0041,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_alu_flags_sub_add_and() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                adds_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                and_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG),
            ]
        );
    }

    #[test]
    fn lowers_alu_flags_preserve_carry_side_effect() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlagsPreserveCarry(AluFlagsPreserveCarry {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                adds_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                mrs_nzcv(NZCV_TMP_REG),
                movz_x(NZCV_MASK_REG, 0xffff, 0),
                movk_x(NZCV_MASK_REG, 0xdfff, 16),
                movk_x(NZCV_MASK_REG, 0xffff, 32),
                movk_x(NZCV_MASK_REG, 0xffff, 48),
                and_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_MASK_REG),
                ldr_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET),
                movz_x(NZCV_MASK_REG, 1, 0),
                and_x(NZCV_CARRY_REG, NZCV_CARRY_REG, NZCV_MASK_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
                lsl_x(NZCV_CARRY_REG, NZCV_CARRY_REG, FLAG_ALIGN_SHIFT_REG),
                orr_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_CARRY_REG),
                msr_nzcv(NZCV_TMP_REG),
            ]
        );
    }

    #[test]
    fn lowers_load_rflags_from_state_frame() {
        let func = function(vec![Stmt::new(Some(0), Op::LoadRflags(LoadRflags))]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![ldr_x_unsigned(
                value_reg(0),
                abi::K_STATE_PTR_REG,
                RFLAGS_OFFSET
            )]
        );
    }

    #[test]
    fn lowers_store_rflags_and_syncs_carry_slot() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreRflags(StoreRflags { value: 0 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x10, 0),
                movz_x(NZCV_TMP_REG, 2, 0),
                orr_x(NZCV_TMP_REG, value_reg(0), NZCV_TMP_REG),
                str_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET),
                movz_x(NZCV_MASK_REG, 1, 0),
                and_x(NZCV_CARRY_REG, NZCV_TMP_REG, NZCV_MASK_REG),
                str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET),
            ]
        );
    }

    #[test]
    fn lowers_lahf_style_rflags_to_ah_sequence() {
        let func = function(vec![
            Stmt::new(Some(0), Op::LoadRflags(LoadRflags)),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0xD7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Constant(Constant {
                    value: 8,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shl,
                    lhs: 2,
                    rhs: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(6),
                Op::Constant(Constant {
                    value: !0xFF00_u64,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(7),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 5,
                    rhs: 6,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(8),
                Op::BinOp(BinOp {
                    op: BinOpKind::Or,
                    lhs: 7,
                    rhs: 4,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 8,
                    size: OpSize::I64,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();

        assert_eq!(
            code.first(),
            Some(&ldr_x_unsigned(
                value_reg(0),
                abi::K_STATE_PTR_REG,
                RFLAGS_OFFSET
            ))
        );
        assert!(code.contains(&and_x(value_reg(2), value_reg(0), value_reg(1))));
        assert!(code.contains(&lsl_x(value_reg(4), value_reg(2), value_reg(3))));
        assert!(code.contains(&ldr_x_unsigned(value_reg(5), abi::K_STATE_PTR_REG, 0)));
        assert!(code.contains(&and_x(value_reg(7), value_reg(5), value_reg(6))));
        assert!(code.contains(&orr_x(value_reg(8), value_reg(7), value_reg(4))));
        assert_eq!(
            code.last(),
            Some(&str_x_unsigned(value_reg(8), abi::K_STATE_PTR_REG, 0))
        );
    }

    #[test]
    fn lowers_sahf_style_ah_to_rflags_sequence() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 8,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shr,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Constant(Constant {
                    value: 0xD5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 2,
                    rhs: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(Some(5), Op::LoadRflags(LoadRflags)),
            Stmt::new(
                Some(6),
                Op::Constant(Constant {
                    value: !0xD5_u64,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(7),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 5,
                    rhs: 6,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(8),
                Op::BinOp(BinOp {
                    op: BinOpKind::Or,
                    lhs: 7,
                    rhs: 4,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreRflags(StoreRflags { value: 8 })),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();

        assert_eq!(
            code.first(),
            Some(&ldr_x_unsigned(value_reg(0), abi::K_STATE_PTR_REG, 0))
        );
        assert!(code.contains(&lsr_x(value_reg(2), value_reg(0), value_reg(1))));
        assert!(code.contains(&and_x(value_reg(4), value_reg(2), value_reg(3))));
        assert!(code.contains(&ldr_x_unsigned(
            value_reg(5),
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert!(code.contains(&and_x(value_reg(7), value_reg(5), value_reg(6))));
        assert!(code.contains(&orr_x(value_reg(8), value_reg(7), value_reg(4))));
        assert!(code.contains(&str_x_unsigned(
            NZCV_TMP_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert_eq!(
            code.last(),
            Some(&str_x_unsigned(
                NZCV_CARRY_REG,
                abi::K_STATE_PTR_REG,
                CF_OFFSET
            ))
        );
    }

    #[test]
    fn lowers_store_carry_and_syncs_rflags_bit_zero() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::StoreCarry(StoreCarry { value: 0 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 3, 0),
                movz_x(NZCV_MASK_REG, 1, 0),
                and_x(NZCV_CARRY_REG, value_reg(0), NZCV_MASK_REG),
                str_x_unsigned(NZCV_CARRY_REG, abi::K_STATE_PTR_REG, CF_OFFSET),
                ldr_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET),
                movz_x(NZCV_MASK_REG, 0xfffe, 0),
                movk_x(NZCV_MASK_REG, 0xffff, 16),
                movk_x(NZCV_MASK_REG, 0xffff, 32),
                movk_x(NZCV_MASK_REG, 0xffff, 48),
                and_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_MASK_REG),
                orr_x(NZCV_TMP_REG, NZCV_TMP_REG, NZCV_CARRY_REG),
                str_x_unsigned(NZCV_TMP_REG, abi::K_STATE_PTR_REG, RFLAGS_OFFSET),
            ]
        );
    }

    #[test]
    fn lowers_cpuid_leaf7_advertises_bmi1_and_bmi2() {
        let func = function(vec![Stmt::new(None, Op::Cpuid(prisma_ir::Cpuid))]);
        let code = Lowerer::new().lower_function(&func).unwrap();

        assert!(code.contains(&movz_x(
            FLAG_ALIGN_LHS_REG,
            u16::try_from(KSTATE_CPUID_LEAF7_EBX).expect("leaf7 EBX fits u16"),
            0
        )));
        assert_ne!(KSTATE_CPUID_LEAF7_EBX & (1 << 3), 0, "BMI1 bit");
        assert_ne!(KSTATE_CPUID_LEAF7_EBX & (1 << 8), 0, "BMI2 bit");
    }

    #[test]
    fn lowers_cpuid_leaf1_advertises_sse42_and_pclmul() {
        let func = function(vec![Stmt::new(None, Op::Cpuid(prisma_ir::Cpuid))]);
        let code = Lowerer::new().lower_function(&func).unwrap();
        let leaf1_ecx_hi =
            u16::try_from((KSTATE_CPUID_LEAF1_ECX >> 16) & 0xffff).expect("masked to u16");

        assert_ne!(KSTATE_CPUID_LEAF1_ECX & (1 << 20), 0, "SSE4.2 bit");
        assert_ne!(KSTATE_CPUID_LEAF1_ECX & (1 << 1), 0, "PCLMULQDQ bit");
        assert_ne!(KSTATE_CPUID_LEAF1_ECX & (1 << 29), 0, "F16C bit");
        assert!(code.contains(&movk_x(FLAG_ALIGN_RHS_REG, leaf1_ecx_hi, 16)));
    }

    #[test]
    fn lowers_store_rflags_from_nzcv_preserving_carry() {
        let func = function(vec![Stmt::new(
            None,
            Op::StoreRflagsFromNzcv(StoreRflagsFromNzcv {
                carry: RflagsCarryMode::Preserve,
                pf: None,
                af: None,
            }),
        )]);

        let code = Lowerer::new().lower_function(&func).unwrap();

        assert_eq!(code.first(), Some(&mrs_nzcv(NZCV_TMP_REG)));
        assert!(code.contains(&ldr_x_unsigned(
            NZCV_CARRY_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert!(code.contains(&movz_x(NZCV_MASK_REG, 0xf73f, 0)));
        assert!(code.contains(&str_x_unsigned(
            NZCV_CARRY_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert_eq!(
            code.last(),
            Some(&str_x_unsigned(
                NZCV_MASK_REG,
                abi::K_STATE_PTR_REG,
                CF_OFFSET
            ))
        );
    }

    #[test]
    fn lowers_store_rflags_from_explicit_bits_preserving_carry_slot() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0,
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
            Stmt::new(
                Some(3),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreRflagsFromBits(StoreRflagsFromBits {
                    pf: Some(3),
                    af: Some(4),
                    zf: 0,
                    sf: 1,
                    of: 2,
                }),
            ),
        ]);

        let code = Lowerer::new().lower_function(&func).unwrap();

        assert!(code.contains(&ldr_x_unsigned(
            NZCV_CARRY_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert!(code.contains(&str_x_unsigned(
            NZCV_CARRY_REG,
            abi::K_STATE_PTR_REG,
            RFLAGS_OFFSET
        )));
        assert!(!code.contains(&str_x_unsigned(
            NZCV_MASK_REG,
            abi::K_STATE_PTR_REG,
            CF_OFFSET
        )));
    }

    #[test]
    fn rejects_unsupported_alu_flags_op() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Mul,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp(
                "AluFlags only supports Sub/Add/And/Or/Xor today"
            ))
        );
    }

    #[test]
    fn rejects_unsupported_alu_flags_preserve_carry_op() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlagsPreserveCarry(AluFlagsPreserveCarry {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp(
                "AluFlagsPreserveCarry only supports Sub/Add today"
            ))
        );
    }

    #[test]
    fn lowers_alu_flags_or_xor() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Or,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Xor,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                orr_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG),
                eor_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG),
            ]
        );
    }

    #[test]
    fn lowers_bswap_scalar_sizes() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Bswap(Bswap {
                    value: 0,
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
            Stmt::new(
                Some(3),
                Op::Bswap(Bswap {
                    value: 2,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Constant(Constant {
                    value: 0x1122,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::Bswap(Bswap {
                    value: 4,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(6),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(7),
                Op::Bswap(Bswap {
                    value: 6,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                crate::assembler::movz_x(value_reg(0), 1, 0),
                crate::assembler::rev_x(value_reg(1), value_reg(0)),
                crate::assembler::movz_x(value_reg(2), 1, 0),
                crate::assembler::rev_w(value_reg(3), value_reg(2)),
                crate::assembler::movz_x(value_reg(4), 0x1122, 0),
                crate::assembler::rev_w(value_reg(5), value_reg(4)),
                crate::assembler::movz_x(FLAG_ALIGN_SHIFT_REG, 16, 0),
                crate::assembler::lsr_x(value_reg(5), value_reg(5), FLAG_ALIGN_SHIFT_REG),
                crate::assembler::movz_x(value_reg(6), 0x7f, 0),
                crate::assembler::uxtb_x(value_reg(7), value_reg(6)),
            ]
        );
    }

    #[test]
    fn lowers_crc32c_scalar_sizes() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0xabcd,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I8,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                crate::assembler::movz_x(value_reg(0), 0x1234, 0),
                crate::assembler::movz_x(value_reg(1), 0xabcd, 0),
                crc32cb(value_reg(2), value_reg(0), value_reg(1)),
                crc32ch(value_reg(3), value_reg(0), value_reg(1)),
                crc32cw(value_reg(4), value_reg(0), value_reg(1)),
                crc32cx(value_reg(5), value_reg(0), value_reg(1)),
            ]
        );
    }

    #[test]
    fn lowers_i8_alu_flags_are_aligned_for_flags_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                adds_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                and_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_i32_alu_flags_are_aligned_for_flags_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x5678,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1234, 0),
                movz_x(value_reg(1), 0x5678, 0),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                adds_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                and_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                cmp_x(ALU_FLAGS_TMP_REG, ZERO_REG),
            ]
        );
    }

    #[test]
    fn lowers_i8_cmp_flags_with_operand_alignment() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 0xff,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I8,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Ult,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1FE9,
                0xD280_002A,
                0xD280_0713,
                0x9AD3_2131,
                0x9AD3_215C,
                0xEB1C_023F,
                0x5400_0043,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_i32_cmp_flags_with_operand_alignment() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 0x8000_0000,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
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
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Slt,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0009,
                0xF2B0_0009,
                0xD280_002A,
                0xD280_0413,
                0x9AD3_2131,
                0x9AD3_215C,
                0xEB1C_023F,
                0x5400_004B,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn rejects_cmp_flags_missing_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::CmpFlags(CmpFlags {
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingResult("CmpFlags"))
        );
    }

    #[test]
    fn rejects_cond_jump_flags_missing_flags() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJumpFlags(CondJumpFlags {
                            flags: 99,
                            cc: CondCode::Eq,
                            if_true: 1,
                            if_false: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 2,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
        );
    }

    #[test]
    fn rejects_cond_jump_missing_condition() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJump(CondJump {
                            cond: 7,
                            if_true: 1,
                            if_false: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 2,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(7))
        );
    }

    #[test]
    fn lowers_jump_reg() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::JumpReg(prisma_ir::JumpReg { target: 0 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD282_0009, 0xD61F_0120]
        );
    }

    #[test]
    fn rejects_jump_reg_missing_target() {
        let func = function(vec![Stmt::new(
            None,
            Op::JumpReg(prisma_ir::JumpReg { target: 99 }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
        );
    }

    #[test]
    fn lowers_jump_rel() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::JumpRel(prisma_ir::JumpRel {
                            target_guest_pc: 0x1000,
                        }),
                    )],
                },
                BasicBlock {
                    id: 0x1000,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001]
        );
    }

    #[test]
    fn lowers_call_rel_as_tail_jump() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CallRel(prisma_ir::CallRel {
                            target_guest_pc: 0x1000,
                            return_guest_pc: 0x1005,
                        }),
                    )],
                },
                BasicBlock {
                    id: 0x1000,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 0x1005,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_call_rel_branch_exit_pushes_return_and_exits() {
        // In branch-exit mode a CallRel is a block-exit: push the return address
        // onto the guest stack and exit to the run loop at the target — not a
        // tail `b` to a sibling block.
        let func = function(vec![Stmt::new(
            None,
            Op::CallRel(prisma_ir::CallRel {
                target_guest_pc: 0x1000,
                return_guest_pc: 0x1005,
            }),
        )]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&func)
            .unwrap();
        // rsp -= 8 (sub x21,x21,#8), the mem-base rebase for the push, the
        // EXIT_BRANCH mark (movz x9,#2), and the epilogue ret.
        assert!(
            words.contains(&sub_x_imm(21, 21, 8)),
            "rsp decrement: {words:#x?}"
        );
        assert!(
            words.contains(&ldr_x_unsigned(
                MEM_ADDR_SCRATCH,
                abi::K_STATE_PTR_REG,
                MEM_BASE_OFFSET
            )),
            "rebased store of the return address: {words:#x?}"
        );
        assert!(
            words.contains(&movz_x(9, 2, 0)),
            "EXIT_BRANCH mark: {words:#x?}"
        );
        assert_eq!(words.last().copied(), Some(0xD65F_03C0), "ends with ret");
        assert!(!words.contains(&0x1400_0001), "not a tail b");
    }

    #[test]
    fn lowers_return_branch_exit_pops_and_exits() {
        // In branch-exit mode a Return pops the return address off the guest
        // stack and exits to the run loop there — not a host `ret`.
        let func = function(vec![Stmt::new(None, Op::Return(Return))]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&func)
            .unwrap();
        // The rebase load of the target, rsp += 8, the EXIT_BRANCH mark, ret.
        assert!(
            words.contains(&ldr_x_unsigned(
                MEM_ADDR_SCRATCH,
                abi::K_STATE_PTR_REG,
                MEM_BASE_OFFSET
            )),
            "rebased load of the return address: {words:#x?}"
        );
        assert!(
            words.contains(&ldr_x_unsigned(RETURN_EXIT_TARGET_REG, MEM_ADDR_SCRATCH, 0)),
            "return target stays in the dedicated register: {words:#x?}"
        );
        assert!(
            words.contains(&add_x_imm(21, 21, 8)),
            "rsp increment: {words:#x?}"
        );
        assert!(
            words.contains(&movz_x(9, 2, 0)),
            "EXIT_BRANCH mark: {words:#x?}"
        );
        assert!(
            words.contains(&str_x_unsigned(
                RETURN_EXIT_TARGET_REG,
                abi::K_STATE_PTR_REG,
                NEXT_PC_OFFSET
            )),
            "dedicated return target reaches next_pc: {words:#x?}"
        );
        assert_eq!(words.last().copied(), Some(0xD65F_03C0), "ends with ret");
    }

    #[test]
    fn lowers_jump_reg_branch_exit_not_host_br() {
        // Indirect jmp reg in branch-exit mode exits to the run loop at the
        // dynamic target — not a host `br` to that guest PC as a host address.
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::JumpReg(prisma_ir::JumpReg { target: 0 })),
        ]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&func)
            .unwrap();
        assert!(
            words.contains(&movz_x(9, 2, 0)),
            "EXIT_BRANCH mark: {words:#x?}"
        );
        assert_eq!(words.last().copied(), Some(0xD65F_03C0), "ends with ret");
        // br_x(Rt) has top byte 0xD6 with opcode 0xD61F0000|rn<<5; ensure no raw
        // `br x9` (0xD61F0120) slipped through.
        assert!(!words.contains(&0xD61F_0120), "no host br: {words:#x?}");
    }

    #[test]
    fn lowers_call_reg_branch_exit_pushes_and_exits() {
        // Indirect call reg in branch-exit mode pushes the return address and
        // exits to the dynamic target — not a host `blr`.
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::CallReg(prisma_ir::CallReg {
                    target: 0,
                    return_guest_pc: 0x1234,
                }),
            ),
        ]);
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&func)
            .unwrap();
        assert!(
            words.contains(&sub_x_imm(21, 21, 8)),
            "rsp decrement: {words:#x?}"
        );
        assert!(
            words.contains(&movz_x(9, 2, 0)),
            "EXIT_BRANCH mark: {words:#x?}"
        );
        assert_eq!(words.last().copied(), Some(0xD65F_03C0), "ends with ret");
    }

    #[test]
    fn lowers_call_reg() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::CallReg(prisma_ir::CallReg {
                    target: 0,
                    return_guest_pc: 0x1234,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(
                    value_reg(0),
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rax)
                ),
                blr_x(value_reg(0)),
            ]
        );
    }

    #[test]
    fn lowers_rsp_adjust_push_pop() {
        let func = function(vec![
            Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 })),
            Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 16 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                sub_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 8),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                add_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 16),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
            ]
        );
    }

    #[test]
    fn lowers_ret_adjusted_to_return() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::RetAdjusted(prisma_ir::RetAdjusted { pop_bytes: 16 }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0029,
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                add_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 16),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                0xD65F_03C0
            ]
        );
    }
}
