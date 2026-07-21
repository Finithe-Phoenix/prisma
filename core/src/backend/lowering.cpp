// core/src/backend/lowering.cpp — implementation of the Lowerer.
//
// Two-pass algorithm:
//   1. compute_liveness scans the IR once to record each Ref's last-use
//      statement index.
//   2. The main forward scan lowers stmt by stmt; after each stmt we
//      return the host regs of any Ref whose interval has ended to a
//      free-list pool (the "linear-scan" expire step), and free any
//      single-stmt temporaries.
//
// The SSA property (every Ref has a unique def preceding its uses —
// RFC 0001) makes this sound for a single basic block: no phi handling
// required, live intervals are contiguous, and forward-only expiry
// never releases a register that will be read again.

#include "prisma/lowering.hpp"

#include "prisma/abi.hpp"
#include "prisma/cpu_state.hpp"

#include <algorithm>
#include <array>
#include <unordered_set>
#include <utility>
#include <variant>

namespace prisma::backend {

namespace {

constexpr unsigned kScratchPoolSize = 10;  // x0..x9
// F2-BK-006 — wider FP pool. V0..V23 (24 regs) for SSA scratch;
// V24..V31 reserved for emitter helpers (kInternalFpScratchV = V31,
// kAuxV = V30; V24..V29 left as future-proofing for multi-temp
// helpers and potential AVX-256 pair-allocator bookkeeping).
constexpr unsigned kFpScratchPoolSize = 24;

constexpr arm64::Reg scratch_reg(unsigned idx) noexcept {
  // x0 = 0, x1 = 1, ... x9 = 9.
  return static_cast<arm64::Reg>(idx);
}

constexpr Emitter::FpReg fp_scratch_reg(unsigned idx) noexcept {
  return static_cast<Emitter::FpReg>(idx);  // V0..V7
}

// Map an IR BinOpKind to the Emitter method. Expressed as a small switch
// rather than a table so the compiler catches missing cases.
LowerResult emit_binop(Emitter& em, ir::BinOpKind op, arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
  switch (op) {
    case ir::BinOpKind::Add:
      em.add(rd, rn, rm);
      return {};
    case ir::BinOpKind::Sub:
      em.sub(rd, rn, rm);
      return {};
    case ir::BinOpKind::Mul:
      em.mul(rd, rn, rm);
      return {};
    case ir::BinOpKind::And:
      em.and_(rd, rn, rm);
      return {};
    case ir::BinOpKind::Or:
      em.orr(rd, rn, rm);
      return {};
    case ir::BinOpKind::Xor:
      em.eor(rd, rn, rm);
      return {};
    case ir::BinOpKind::Shl:
      em.lsl(rd, rn, rm);
      return {};
    case ir::BinOpKind::Shr:
      em.lsr(rd, rn, rm);
      return {};
    case ir::BinOpKind::Sar:
      em.asr(rd, rn, rm);
      return {};
    case ir::BinOpKind::Ror:
      em.ror(rd, rn, rm);
      return {};
    case ir::BinOpKind::Rcr:
      em.ror(rd, rn, rm);
      return {};
    case ir::BinOpKind::UMulHi:
      em.umulh(rd, rn, rm);
      return {};
    case ir::BinOpKind::SMulHi:
      em.smulh(rd, rn, rm);
      return {};
    case ir::BinOpKind::UDiv:
      em.udiv(rd, rn, rm);
      return {};
    case ir::BinOpKind::SDiv:
      em.sdiv(rd, rn, rm);
      return {};
    case ir::BinOpKind::Rol:
    case ir::BinOpKind::Rcl:
      // Rol/Rcl are lowered via a neg+ror helper register in lower_stmt.
      return {false, LowerError::UnsupportedOp,
              "rotate-left emulation requires a temporary scratch register"};
    case ir::BinOpKind::UMod:
    case ir::BinOpKind::SMod:
      // Mod is q = n / m; r = n - q * m — needs a scratch held in
      // lower_stmt to materialise q before the msub.
      return {false, LowerError::UnsupportedOp, "UMod/SMod requires a temporary scratch register"};
    case ir::BinOpKind::Pdep:
    case ir::BinOpKind::Pext:
      return {false, LowerError::UnsupportedOp, "PDEP/PEXT requires a software loop"};
  }
  return {false, LowerError::UnsupportedOp, "unknown BinOpKind"};
}

void emit_bit_permute(Emitter& em, bool deposit, arm64::Reg rd, arm64::Reg rn, arm64::Reg rm,
                      arm64::Reg rsrc, arm64::Reg rmask, arm64::Reg rbit, arm64::Reg rone,
                      arm64::Reg rlowbit, arm64::Reg rtmp, ir::OpSize size) {
  em.mov_reg_reg(rsrc, rn);
  em.mov_reg_reg(rmask, rm);
  if (size == ir::OpSize::I32) {
    em.uxtw(rsrc, rsrc);
    em.uxtw(rmask, rmask);
  }

  em.mov_imm64(rd, 0);
  em.mov_imm64(rbit, 1);
  em.mov_imm64(rone, 1);

  const auto loop = em.create_label();
  const auto skip = em.create_label();
  const auto done = em.create_label();

  em.bind(loop);
  em.cbz(rmask, done);
  em.neg(rlowbit, rmask);
  em.and_(rlowbit, rlowbit, rmask);
  em.and_(rtmp, rsrc, deposit ? rbit : rlowbit);
  em.cbz(rtmp, skip);
  em.orr(rd, rd, deposit ? rlowbit : rbit);
  em.bind(skip);
  em.sub(rtmp, rmask, rone);
  em.and_(rmask, rmask, rtmp);
  em.add(rbit, rbit, rbit);
  em.branch(loop);
  em.bind(done);

  if (size == ir::OpSize::I32) {
    em.uxtw(rd, rd);
  }
}

struct WideDivWorkRegs {
  arm64::Reg rem;
  arm64::Reg low;
  arm64::Reg divisor;
  arm64::Reg mask;
  arm64::Reg bit;
  arm64::Reg overflow;
};

void emit_abs_signed_wide_div_inputs(Emitter& em, const WideDivWorkRegs& r) {
  em.lsr_imm(r.bit, r.rem, 63);

  const auto dividend_low_zero = em.create_label();
  const auto dividend_done = em.create_label();
  em.cbz(r.bit, dividend_done);
  em.mov_imm64(r.mask, ~0ull);
  em.cbz(r.low, dividend_low_zero);
  em.neg(r.low, r.low);
  em.eor(r.rem, r.rem, r.mask);
  em.branch(dividend_done);
  em.bind(dividend_low_zero);
  em.neg(r.rem, r.rem);
  em.bind(dividend_done);

  const auto divisor_done = em.create_label();
  em.lsr_imm(r.bit, r.divisor, 63);
  em.cbz(r.bit, divisor_done);
  em.neg(r.divisor, r.divisor);
  em.bind(divisor_done);
}

void emit_wide_udiv_core(Emitter& em, arm64::Reg quotient, const WideDivWorkRegs& r) {
  em.mov_imm64(r.mask, 1ull << 63);
  em.mov_imm64(quotient, 0);

  for (unsigned i = 0; i < 64; ++i) {
    const auto subtract = em.create_label();
    const auto done = em.create_label();

    em.lsr_imm(r.bit, r.low, 63);
    em.add(r.low, r.low, r.low);
    em.lsr_imm(r.overflow, r.rem, 63);
    em.add(r.rem, r.rem, r.rem);
    em.orr(r.rem, r.rem, r.bit);

    em.cbnz(r.overflow, subtract);
    em.cmp(r.rem, r.divisor);
    em.branch_cc(subtract, ir::CondCode::Uge);
    em.branch(done);

    em.bind(subtract);
    em.sub(r.rem, r.rem, r.divisor);
    em.orr(quotient, quotient, r.mask);

    em.bind(done);
    em.lsr_imm(r.mask, r.mask, 1);
  }
}

void emit_sigfpe_placeholder_return(Emitter& em, bool emit_ret_on_terminator) {
  em.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
  if (emit_ret_on_terminator) {
    em.ret();
  } else {
    abi::emit_block_epilogue_and_ret(em);
  }
}

void emit_divisor_zero_sigfpe_guard(Emitter& em, arm64::Reg divisor, bool emit_ret_on_terminator) {
  const auto ok = em.create_label();
  em.cbnz(divisor, ok);
  emit_sigfpe_placeholder_return(em, emit_ret_on_terminator);
  em.bind(ok);
}

void emit_unsigned_wide_div_overflow_guard(Emitter& em, arm64::Reg high, arm64::Reg divisor,
                                           bool emit_ret_on_terminator) {
  const auto ok = em.create_label();
  em.cmp(high, divisor);
  em.branch_cc(ok, ir::CondCode::Ult);
  emit_sigfpe_placeholder_return(em, emit_ret_on_terminator);
  em.bind(ok);
}

void emit_signed_wide_div_quotient_overflow_guard(Emitter& em, arm64::Reg quotient,
                                                  bool emit_ret_on_terminator,
                                                  const WideDivWorkRegs& r) {
  const auto negative = em.create_label();
  const auto done = em.create_label();

  em.mov_imm64(r.mask, 63);
  em.cbnz(r.bit, negative);
  em.lsr(r.overflow, quotient, r.mask);
  em.cbz(r.overflow, done);
  emit_sigfpe_placeholder_return(em, emit_ret_on_terminator);

  em.bind(negative);
  em.mov_imm64(r.mask, 1ull << 63);
  em.cmp(quotient, r.mask);
  em.branch_cc(done, ir::CondCode::Ule);
  emit_sigfpe_placeholder_return(em, emit_ret_on_terminator);

  em.bind(done);
}

void emit_apply_signed_wide_div_result(Emitter& em, arm64::Reg dst, arm64::Reg high,
                                       arm64::Reg divisor, ir::WideDivResult result,
                                       const WideDivWorkRegs& r) {
  const auto done = em.create_label();
  em.lsr_imm(r.bit, high, 63);
  if (result == ir::WideDivResult::Quotient) {
    em.lsr_imm(r.overflow, divisor, 63);
    em.eor(r.bit, r.bit, r.overflow);
  }
  em.cbz(r.bit, done);
  em.neg(dst, dst);
  em.bind(done);
}

constexpr std::uint64_t kPcmpLenLhsExplicit = 1;
constexpr std::uint64_t kPcmpLenRhsExplicit = 2;

struct PcmpStrEval {
  std::uint16_t intres{0};
  std::size_t max_lanes{0};
  std::size_t lane_bytes{0};
  std::size_t lhs_len{0};
  std::size_t rhs_len{0};
};

struct PcmpMaskResult {
  std::uint64_t lo;
  std::uint64_t hi;
};

std::array<std::uint8_t, 16> pcmp_bytes(std::uint64_t lo, std::uint64_t hi) {
  std::array<std::uint8_t, 16> out{};
  for (unsigned i = 0; i < 8; ++i) {
    out[i] = static_cast<std::uint8_t>((lo >> (i * 8u)) & 0xffu);
    out[i + 8] = static_cast<std::uint8_t>((hi >> (i * 8u)) & 0xffu);
  }
  return out;
}

std::uint16_t pcmp_lane_unsigned(const std::array<std::uint8_t, 16>& bytes, std::size_t lane,
                                 std::size_t lane_bytes) {
  if (lane_bytes == 1)
    return bytes[lane];
  return static_cast<std::uint16_t>(bytes[lane * 2] |
                                    (static_cast<std::uint16_t>(bytes[lane * 2 + 1]) << 8));
}

std::int16_t pcmp_lane_signed(const std::array<std::uint8_t, 16>& bytes, std::size_t lane,
                              std::size_t lane_bytes) {
  if (lane_bytes == 1) {
    return static_cast<std::int16_t>(static_cast<std::int8_t>(bytes[lane]));
  }
  return static_cast<std::int16_t>(pcmp_lane_unsigned(bytes, lane, lane_bytes));
}

std::size_t pcmp_effective_len(const std::array<std::uint8_t, 16>& bytes, std::size_t lane_bytes,
                               std::uint64_t raw_len, bool explicit_len) {
  const std::size_t max_lanes = 16 / lane_bytes;
  if (explicit_len) {
    const auto signed_len = static_cast<std::int32_t>(static_cast<std::uint32_t>(raw_len));
    const std::uint32_t abs_len =
        signed_len < 0 ? static_cast<std::uint32_t>(-static_cast<std::int64_t>(signed_len))
                       : static_cast<std::uint32_t>(signed_len);
    return std::min<std::size_t>(abs_len, max_lanes);
  }
  for (std::size_t i = 0; i < max_lanes; ++i) {
    if (pcmp_lane_unsigned(bytes, i, lane_bytes) == 0)
      return i;
  }
  return max_lanes;
}

PcmpStrEval eval_pcmpstr(std::uint64_t lhs_lo, std::uint64_t lhs_hi, std::uint64_t rhs_lo,
                         std::uint64_t rhs_hi, std::uint64_t lhs_len_raw, std::uint64_t rhs_len_raw,
                         std::uint64_t len_mode, std::uint8_t imm8) {
  const auto lhs = pcmp_bytes(lhs_lo, lhs_hi);
  const auto rhs = pcmp_bytes(rhs_lo, rhs_hi);
  const std::size_t lane_bytes = (imm8 & 1u) == 0 ? 1u : 2u;
  const std::size_t max_lanes = 16 / lane_bytes;
  const bool is_signed = (imm8 & 0x02u) != 0;
  const std::uint8_t aggregation = (imm8 >> 2) & 0x03u;
  const std::uint8_t polarity = (imm8 >> 4) & 0x03u;
  const std::size_t lhs_len =
      pcmp_effective_len(lhs, lane_bytes, lhs_len_raw, (len_mode & kPcmpLenLhsExplicit) != 0);
  const std::size_t rhs_len =
      pcmp_effective_len(rhs, lane_bytes, rhs_len_raw, (len_mode & kPcmpLenRhsExplicit) != 0);

  std::uint16_t bits = 0;
  for (std::size_t i = 0; i < max_lanes; ++i) {
    const bool lhs_valid = i < lhs_len;
    bool matched = false;
    if (lhs_valid) {
      switch (aggregation) {
        case 0:
          for (std::size_t j = 0; j < rhs_len && !matched; ++j) {
            matched = is_signed ? pcmp_lane_signed(lhs, i, lane_bytes) ==
                                      pcmp_lane_signed(rhs, j, lane_bytes)
                                : pcmp_lane_unsigned(lhs, i, lane_bytes) ==
                                      pcmp_lane_unsigned(rhs, j, lane_bytes);
          }
          break;
        case 1:
          for (std::size_t j = 0; j + 1 < rhs_len && !matched; j += 2) {
            if (is_signed) {
              const auto value = pcmp_lane_signed(lhs, i, lane_bytes);
              const auto low = pcmp_lane_signed(rhs, j, lane_bytes);
              const auto high = pcmp_lane_signed(rhs, j + 1, lane_bytes);
              matched = low <= value && value <= high;
            } else {
              const auto value = pcmp_lane_unsigned(lhs, i, lane_bytes);
              const auto low = pcmp_lane_unsigned(rhs, j, lane_bytes);
              const auto high = pcmp_lane_unsigned(rhs, j + 1, lane_bytes);
              matched = low <= value && value <= high;
            }
          }
          break;
        case 2:
          matched = i < rhs_len && (is_signed ? pcmp_lane_signed(lhs, i, lane_bytes) ==
                                                    pcmp_lane_signed(rhs, i, lane_bytes)
                                              : pcmp_lane_unsigned(lhs, i, lane_bytes) ==
                                                    pcmp_lane_unsigned(rhs, i, lane_bytes));
          break;
        default:
          matched = rhs_len > 0 && i + rhs_len <= lhs_len;
          for (std::size_t j = 0; matched && j < rhs_len; ++j) {
            matched = is_signed ? pcmp_lane_signed(lhs, i + j, lane_bytes) ==
                                      pcmp_lane_signed(rhs, j, lane_bytes)
                                : pcmp_lane_unsigned(lhs, i + j, lane_bytes) ==
                                      pcmp_lane_unsigned(rhs, j, lane_bytes);
          }
          break;
      }
    }

    const bool valid_for_polarity = (polarity & 0x02u) != 0 ? lhs_valid : true;
    if ((polarity & 0x01u) != 0 && valid_for_polarity)
      matched = !matched;
    if (!lhs_valid && (polarity & 0x02u) != 0)
      matched = false;
    if (matched)
      bits |= static_cast<std::uint16_t>(1u << i);
  }
  return PcmpStrEval{bits, max_lanes, lane_bytes, lhs_len, rhs_len};
}

std::uint64_t pcmpstr_index(PcmpStrEval eval, std::uint8_t imm8) {
  if ((imm8 & 0x40u) == 0) {
    for (std::size_t i = 0; i < eval.max_lanes; ++i) {
      if ((eval.intres & (1u << i)) != 0)
        return i;
    }
  } else {
    for (std::size_t i = eval.max_lanes; i-- > 0;) {
      if ((eval.intres & (1u << i)) != 0)
        return i;
    }
  }
  return eval.max_lanes;
}

PcmpMaskResult pcmpstr_mask(PcmpStrEval eval, std::uint8_t imm8) {
  if ((imm8 & 0x40u) == 0)
    return {eval.intres, 0};

  std::uint64_t lo = 0;
  std::uint64_t hi = 0;
  for (std::size_t i = 0; i < eval.max_lanes; ++i) {
    if ((eval.intres & (1u << i)) == 0)
      continue;
    const std::uint64_t lane_mask = eval.lane_bytes == 1 ? 0xffull : 0xffffull;
    const std::size_t shift = i * eval.lane_bytes * 8;
    if (shift < 64) {
      lo |= lane_mask << shift;
    } else {
      hi |= lane_mask << (shift - 64);
    }
  }
  return {lo, hi};
}

std::uint64_t pcmpstr_flags(PcmpStrEval eval) {
  const std::uint64_t cf = eval.intres != 0 ? 1ull : 0ull;
  const std::uint64_t zf = eval.rhs_len < eval.max_lanes ? 1ull : 0ull;
  const std::uint64_t sf = eval.lhs_len < eval.max_lanes ? 1ull : 0ull;
  const std::uint64_t of = (eval.intres & 1u) != 0 ? 1ull : 0ull;
  return cf | (zf << 1) | (sf << 2) | (of << 3);
}

std::uint64_t prisma_backend_pcmpstr_index_helper(std::uint64_t lhs_lo, std::uint64_t lhs_hi,
                                                  std::uint64_t rhs_lo, std::uint64_t rhs_hi,
                                                  std::uint64_t lhs_len, std::uint64_t rhs_len,
                                                  std::uint64_t len_mode, std::uint64_t imm8) {
  const auto eval = eval_pcmpstr(lhs_lo, lhs_hi, rhs_lo, rhs_hi, lhs_len, rhs_len, len_mode,
                                 static_cast<std::uint8_t>(imm8));
  return pcmpstr_index(eval, static_cast<std::uint8_t>(imm8));
}

PcmpMaskResult prisma_backend_pcmpstr_mask_helper(std::uint64_t lhs_lo, std::uint64_t lhs_hi,
                                                  std::uint64_t rhs_lo, std::uint64_t rhs_hi,
                                                  std::uint64_t lhs_len, std::uint64_t rhs_len,
                                                  std::uint64_t len_mode, std::uint64_t imm8) {
  const auto eval = eval_pcmpstr(lhs_lo, lhs_hi, rhs_lo, rhs_hi, lhs_len, rhs_len, len_mode,
                                 static_cast<std::uint8_t>(imm8));
  return pcmpstr_mask(eval, static_cast<std::uint8_t>(imm8));
}

std::uint64_t prisma_backend_pcmpstr_flags_helper(std::uint64_t lhs_lo, std::uint64_t lhs_hi,
                                                  std::uint64_t rhs_lo, std::uint64_t rhs_hi,
                                                  std::uint64_t lhs_len, std::uint64_t rhs_len,
                                                  std::uint64_t len_mode, std::uint64_t imm8) {
  return pcmpstr_flags(eval_pcmpstr(lhs_lo, lhs_hi, rhs_lo, rhs_hi, lhs_len, rhs_len, len_mode,
                                    static_cast<std::uint8_t>(imm8)));
}

// W2-09 — F16C software conversions, bit-exact with the Rust backend
// helpers so differential fixtures agree on every rounding mode and
// NaN payload.

std::uint32_t f16c_half_to_f32_bits(std::uint16_t h) {
  const std::uint32_t sign = static_cast<std::uint32_t>(h & 0x8000u) << 16;
  const std::uint16_t exp = static_cast<std::uint16_t>((h >> 10) & 0x1fu);
  const std::uint32_t frac = h & 0x03ffu;
  if (exp == 0) {
    if (frac == 0)
      return sign;
    std::uint32_t mant = frac;
    int e = -14;
    while ((mant & 0x0400u) == 0) {
      mant <<= 1;
      --e;
    }
    mant &= 0x03ffu;
    return sign | (static_cast<std::uint32_t>(e + 127) << 23) | (mant << 13);
  }
  if (exp == 0x1f)
    return sign | 0x7f800000u | (frac << 13);
  return sign | ((static_cast<std::uint32_t>(exp) + 112u) << 23) | (frac << 13);
}

std::uint16_t f16c_overflow(std::uint16_t sign, std::uint8_t mode) {
  const std::uint16_t inf = sign | 0x7c00u;
  const std::uint16_t max = sign | 0x7bffu;
  switch (mode) {
    case 1:
      return sign != 0 ? inf : max;  // toward -inf
    case 2:
      return sign == 0 ? inf : max;  // toward +inf
    case 3:
      return max;  // truncate
    default:
      return inf;  // nearest-even
  }
}

bool f16c_should_round(std::uint16_t sign, std::uint8_t mode, std::uint64_t remainder,
                       std::uint64_t halfway, std::uint64_t lsb) {
  switch (mode) {
    case 1:
      return sign != 0 && remainder != 0;
    case 2:
      return sign == 0 && remainder != 0;
    case 3:
      return false;
    default:
      return remainder > halfway || (remainder == halfway && lsb != 0);
  }
}

std::uint16_t f16c_f32_bits_to_half(std::uint32_t bits, std::uint8_t imm8) {
  const std::uint8_t mode = (imm8 & 0x04u) != 0 ? 0u : static_cast<std::uint8_t>(imm8 & 0x03u);
  const std::uint16_t sign = static_cast<std::uint16_t>((bits >> 16) & 0x8000u);
  const int exp = static_cast<int>((bits >> 23) & 0xffu);
  const std::uint32_t frac = bits & 0x007fffffu;

  if (exp == 0xff) {
    if (frac == 0)
      return sign | 0x7c00u;
    return static_cast<std::uint16_t>(sign | 0x7e00u |
                                      static_cast<std::uint16_t>((frac >> 13) & 0x01ffu));
  }

  const int half_exp = exp - 127 + 15;
  if (half_exp >= 0x1f)
    return f16c_overflow(sign, mode);

  if (half_exp <= 0) {
    if (half_exp < -10) {
      const bool inc = (mode == 1 && sign != 0) || (mode == 2 && sign == 0);
      return static_cast<std::uint16_t>(sign | (inc ? 1u : 0u));
    }
    const std::uint64_t mant = frac | 0x00800000u;
    const unsigned shift = static_cast<unsigned>(14 - half_exp);
    const std::uint64_t q = mant >> shift;
    const std::uint64_t rem = mant & ((1ull << shift) - 1u);
    const std::uint64_t halfway = 1ull << (shift - 1u);
    const std::uint64_t rounded =
        q + (f16c_should_round(sign, mode, rem, halfway, q & 1u) ? 1u : 0u);
    return static_cast<std::uint16_t>(sign | rounded);
  }

  std::uint16_t half_exp_u = static_cast<std::uint16_t>(half_exp);
  std::uint16_t mant = static_cast<std::uint16_t>(frac >> 13);
  const std::uint64_t rem = frac & 0x1fffu;
  if (f16c_should_round(sign, mode, rem, 0x1000u, mant & 1u)) {
    mant = static_cast<std::uint16_t>(mant + 1u);
    if (mant == 0x0400u) {
      mant = 0;
      ++half_exp_u;
      if (half_exp_u >= 0x1f)
        return f16c_overflow(sign, mode);
    }
  }
  return static_cast<std::uint16_t>(sign | (half_exp_u << 10) | mant);
}

PcmpMaskResult prisma_backend_f16c_ph2ps_helper(std::uint64_t src_lo) {
  std::uint64_t lo = 0;
  std::uint64_t hi = 0;
  for (unsigned lane = 0; lane < 2; ++lane) {
    const auto h = static_cast<std::uint16_t>(src_lo >> (lane * 16));
    lo |= static_cast<std::uint64_t>(f16c_half_to_f32_bits(h)) << (lane * 32);
  }
  for (unsigned lane = 2; lane < 4; ++lane) {
    const auto h = static_cast<std::uint16_t>(src_lo >> (lane * 16));
    hi |= static_cast<std::uint64_t>(f16c_half_to_f32_bits(h)) << ((lane - 2) * 32);
  }
  return {lo, hi};
}

PcmpMaskResult prisma_backend_f16c_ps2ph_helper(std::uint64_t src_lo, std::uint64_t src_hi,
                                                std::uint64_t imm8) {
  const auto rounding = static_cast<std::uint8_t>(imm8);
  std::uint64_t out = 0;
  for (unsigned lane = 0; lane < 2; ++lane) {
    const auto b = static_cast<std::uint32_t>(src_lo >> (lane * 32));
    out |= static_cast<std::uint64_t>(f16c_f32_bits_to_half(b, rounding)) << (lane * 16);
  }
  for (unsigned lane = 0; lane < 2; ++lane) {
    const auto b = static_cast<std::uint32_t>(src_hi >> (lane * 32));
    out |= static_cast<std::uint64_t>(f16c_f32_bits_to_half(b, rounding)) << ((lane + 2) * 16);
  }
  return {out, 0};
}

arm64::Reg dummy_gpr_for_pair(arm64::Reg r) noexcept {
  return r == arm64::Reg::X0 ? arm64::Reg::X1 : arm64::Reg::X0;
}

Emitter::FpReg dummy_fp_for_pair(Emitter::FpReg r) noexcept {
  return r == Emitter::FpReg::V0 ? Emitter::FpReg::V1 : Emitter::FpReg::V0;
}

std::vector<std::pair<arm64::Reg, arm64::Reg>> make_gpr_save_pairs(std::vector<arm64::Reg> regs) {
  std::sort(regs.begin(), regs.end(), [](arm64::Reg a, arm64::Reg b) {
    return static_cast<std::uint8_t>(a) < static_cast<std::uint8_t>(b);
  });
  std::vector<std::pair<arm64::Reg, arm64::Reg>> pairs;
  pairs.reserve((regs.size() + 1) / 2);
  for (std::size_t i = 0; i < regs.size(); i += 2) {
    const arm64::Reg second = i + 1 < regs.size() ? regs[i + 1] : dummy_gpr_for_pair(regs[i]);
    pairs.push_back({regs[i], second});
  }
  return pairs;
}

std::vector<std::pair<Emitter::FpReg, Emitter::FpReg>>
make_fp_save_pairs(std::vector<Emitter::FpReg> regs) {
  std::sort(regs.begin(), regs.end(), [](Emitter::FpReg a, Emitter::FpReg b) {
    return static_cast<std::uint8_t>(a) < static_cast<std::uint8_t>(b);
  });
  std::vector<std::pair<Emitter::FpReg, Emitter::FpReg>> pairs;
  pairs.reserve((regs.size() + 1) / 2);
  for (std::size_t i = 0; i < regs.size(); i += 2) {
    const Emitter::FpReg second = i + 1 < regs.size() ? regs[i + 1] : dummy_fp_for_pair(regs[i]);
    pairs.push_back({regs[i], second});
  }
  return pairs;
}

void save_gpr_pairs(Emitter& emitter, const std::vector<std::pair<arm64::Reg, arm64::Reg>>& pairs) {
  for (const auto& [a, b] : pairs)
    emitter.push_pair(a, b);
}

void restore_gpr_pairs(Emitter& emitter,
                       const std::vector<std::pair<arm64::Reg, arm64::Reg>>& pairs) {
  for (auto it = pairs.rbegin(); it != pairs.rend(); ++it) {
    emitter.pop_pair(it->first, it->second);
  }
}

void save_fp_pairs(Emitter& emitter,
                   const std::vector<std::pair<Emitter::FpReg, Emitter::FpReg>>& pairs) {
  for (const auto& [a, b] : pairs) {
    emitter.push_fp_q(a);
    emitter.push_fp_q(b);
  }
}

void restore_fp_pairs(Emitter& emitter,
                      const std::vector<std::pair<Emitter::FpReg, Emitter::FpReg>>& pairs) {
  for (auto it = pairs.rbegin(); it != pairs.rend(); ++it) {
    emitter.pop_fp_q(it->second);
    emitter.pop_fp_q(it->first);
  }
}

}  // namespace

bool Lowerer::allocate_scratch(ir::Ref ref, arm64::Reg& out) {
  if (free_regs_.empty()) {
    // F1-BK-008: try to evict a currently-held ref to a spill slot.
    if (!spill_one_ref())
      return false;
  }
  out = free_regs_.back();
  free_regs_.pop_back();
  ref_to_scratch_[ref] = out;
  const unsigned live = static_cast<unsigned>(ref_to_scratch_.size() + stmt_temporaries_.size());
  peak_live_ = std::max(peak_live_, live);
  return true;
}

bool Lowerer::allocate_fp_scratch(ir::Ref ref, Emitter::FpReg& out) {
  if (fp_free_.empty())
    return false;
  out = fp_free_.back();
  fp_free_.pop_back();
  ref_to_fp_[ref] = out;
  return true;
}

bool Lowerer::fp_reg_of(ir::Ref ref, Emitter::FpReg& out) {
  auto it = ref_to_fp_.find(ref);
  if (it == ref_to_fp_.end())
    return false;
  out = it->second;
  return true;
}

LowerResult Lowerer::emit_pcmpstr_scalar_helper(ir::Ref lhs, ir::Ref rhs,
                                                std::optional<ir::Ref> lhs_len,
                                                std::optional<ir::Ref> rhs_len, std::uint8_t imm8,
                                                std::uint64_t helper_addr, const char* name,
                                                ir::Ref result) {
  Emitter::FpReg lhs_v, rhs_v;
  if (!fp_reg_of(lhs, lhs_v))
    return {false, LowerError::DanglingRef, "PcmpStr.lhs"};
  if (!fp_reg_of(rhs, rhs_v))
    return {false, LowerError::DanglingRef, "PcmpStr.rhs"};

  std::optional<arm64::Reg> lhs_len_r;
  std::optional<arm64::Reg> rhs_len_r;
  if (lhs_len) {
    arm64::Reg r;
    if (!reg_of(*lhs_len, r))
      return {false, LowerError::DanglingRef, "PcmpStr.lhs_len"};
    lhs_len_r = r;
  }
  if (rhs_len) {
    arm64::Reg r;
    if (!reg_of(*rhs_len, r))
      return {false, LowerError::DanglingRef, "PcmpStr.rhs_len"};
    rhs_len_r = r;
  }

  arm64::Reg rd, dummy;
  if (!allocate_scratch(result, rd) || !allocate_temporary(dummy)) {
    return {false, LowerError::OutOfScratchRegs, name};
  }

  std::vector<arm64::Reg> live_gprs;
  live_gprs.reserve(ref_to_scratch_.size());
  for (const auto& [_, reg] : ref_to_scratch_)
    live_gprs.push_back(reg);
  const auto gpr_pairs = make_gpr_save_pairs(std::move(live_gprs));

  std::vector<Emitter::FpReg> live_fps;
  live_fps.reserve(ref_to_fp_.size());
  for (const auto& [_, reg] : ref_to_fp_)
    live_fps.push_back(reg);
  const auto fp_pairs = make_fp_save_pairs(std::move(live_fps));

  const std::int32_t result_offset =
      static_cast<std::int32_t>(gpr_pairs.size() * 16u + fp_pairs.size() * 32u);
  const std::uint64_t len_mode =
      (lhs_len ? kPcmpLenLhsExplicit : 0) | (rhs_len ? kPcmpLenRhsExplicit : 0);

  emitter_.push_pair(arm64::Reg::X0, arm64::Reg::X0);  // result slot
  save_gpr_pairs(emitter_, gpr_pairs);

  if (lhs_len_r)
    emitter_.mov_reg_reg(arm64::Reg::X4, *lhs_len_r);
  else
    emitter_.mov_imm64(arm64::Reg::X4, 0);
  if (rhs_len_r)
    emitter_.mov_reg_reg(arm64::Reg::X5, *rhs_len_r);
  else
    emitter_.mov_imm64(arm64::Reg::X5, 0);
  emitter_.mov_imm64(arm64::Reg::X6, len_mode);
  emitter_.mov_imm64(arm64::Reg::X7, imm8);
  emitter_.vumov_w_from_lane(arm64::Reg::X0, lhs_v, 0, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X1, lhs_v, 1, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X2, rhs_v, 0, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X3, rhs_v, 1, Emitter::VecLane::D2);

  save_fp_pairs(emitter_, fp_pairs);
  emitter_.mov_imm64(arm64::Reg::X9, helper_addr);
  emitter_.blr(arm64::Reg::X9);
  emitter_.sp_store(arm64::Reg::X0, result_offset);
  restore_fp_pairs(emitter_, fp_pairs);
  restore_gpr_pairs(emitter_, gpr_pairs);
  emitter_.pop_pair(rd, dummy);
  return {};
}

LowerResult Lowerer::emit_pcmpstr_mask_helper(ir::Ref lhs, ir::Ref rhs,
                                              std::optional<ir::Ref> lhs_len,
                                              std::optional<ir::Ref> rhs_len, std::uint8_t imm8,
                                              ir::Ref result) {
  Emitter::FpReg lhs_v, rhs_v;
  if (!fp_reg_of(lhs, lhs_v))
    return {false, LowerError::DanglingRef, "PcmpStrMask.lhs"};
  if (!fp_reg_of(rhs, rhs_v))
    return {false, LowerError::DanglingRef, "PcmpStrMask.rhs"};

  std::optional<arm64::Reg> lhs_len_r;
  std::optional<arm64::Reg> rhs_len_r;
  if (lhs_len) {
    arm64::Reg r;
    if (!reg_of(*lhs_len, r))
      return {false, LowerError::DanglingRef, "PcmpStrMask.lhs_len"};
    lhs_len_r = r;
  }
  if (rhs_len) {
    arm64::Reg r;
    if (!reg_of(*rhs_len, r))
      return {false, LowerError::DanglingRef, "PcmpStrMask.rhs_len"};
    rhs_len_r = r;
  }

  Emitter::FpReg rd;
  arm64::Reg lo, hi;
  if (!allocate_fp_scratch(result, rd) || !allocate_temporary(lo) || !allocate_temporary(hi)) {
    return {false, LowerError::OutOfScratchRegs, "PcmpStrMask"};
  }

  std::vector<arm64::Reg> live_gprs;
  live_gprs.reserve(ref_to_scratch_.size());
  for (const auto& [_, reg] : ref_to_scratch_)
    live_gprs.push_back(reg);
  const auto gpr_pairs = make_gpr_save_pairs(std::move(live_gprs));

  std::vector<Emitter::FpReg> live_fps;
  live_fps.reserve(ref_to_fp_.size());
  for (const auto& [_, reg] : ref_to_fp_)
    live_fps.push_back(reg);
  const auto fp_pairs = make_fp_save_pairs(std::move(live_fps));

  const std::int32_t result_offset =
      static_cast<std::int32_t>(gpr_pairs.size() * 16u + fp_pairs.size() * 32u);
  const std::uint64_t len_mode =
      (lhs_len ? kPcmpLenLhsExplicit : 0) | (rhs_len ? kPcmpLenRhsExplicit : 0);

  emitter_.push_pair(arm64::Reg::X0, arm64::Reg::X0);  // lo/hi result slot
  save_gpr_pairs(emitter_, gpr_pairs);

  if (lhs_len_r)
    emitter_.mov_reg_reg(arm64::Reg::X4, *lhs_len_r);
  else
    emitter_.mov_imm64(arm64::Reg::X4, 0);
  if (rhs_len_r)
    emitter_.mov_reg_reg(arm64::Reg::X5, *rhs_len_r);
  else
    emitter_.mov_imm64(arm64::Reg::X5, 0);
  emitter_.mov_imm64(arm64::Reg::X6, len_mode);
  emitter_.mov_imm64(arm64::Reg::X7, imm8);
  emitter_.vumov_w_from_lane(arm64::Reg::X0, lhs_v, 0, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X1, lhs_v, 1, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X2, rhs_v, 0, Emitter::VecLane::D2);
  emitter_.vumov_w_from_lane(arm64::Reg::X3, rhs_v, 1, Emitter::VecLane::D2);

  save_fp_pairs(emitter_, fp_pairs);
  emitter_.mov_imm64(arm64::Reg::X9, static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(
                                         &prisma_backend_pcmpstr_mask_helper)));
  emitter_.blr(arm64::Reg::X9);
  emitter_.sp_store(arm64::Reg::X0, result_offset);
  emitter_.sp_store(arm64::Reg::X1, result_offset + 8);
  restore_fp_pairs(emitter_, fp_pairs);
  restore_gpr_pairs(emitter_, gpr_pairs);
  emitter_.pop_pair(lo, hi);
  emitter_.fmov_v_from_x(rd, lo, ir::FpSize::F64);
  emitter_.vins_lane_from_w(rd, rd, 1, hi, Emitter::VecLane::D2);
  return {};
}

LowerResult Lowerer::lower_pcmpstr_index(const ir::PcmpStrIndex& op, ir::Ref result) {
  return emit_pcmpstr_scalar_helper(op.lhs, op.rhs, op.lhs_len, op.rhs_len, op.imm8,
                                    static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(
                                        &prisma_backend_pcmpstr_index_helper)),
                                    "PcmpStrIndex", result);
}

LowerResult Lowerer::lower_pcmpstr_mask(const ir::PcmpStrMask& op, ir::Ref result) {
  return emit_pcmpstr_mask_helper(op.lhs, op.rhs, op.lhs_len, op.rhs_len, op.imm8, result);
}

LowerResult Lowerer::lower_pcmpstr_flags(const ir::PcmpStrFlags& op, ir::Ref result) {
  return emit_pcmpstr_scalar_helper(op.lhs, op.rhs, op.lhs_len, op.rhs_len, op.imm8,
                                    static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(
                                        &prisma_backend_pcmpstr_flags_helper)),
                                    "PcmpStrFlags", result);
}

LowerResult Lowerer::lower_vec_f16cvt(const ir::VecF16Cvt& op, ir::Ref result) {
  Emitter::FpReg src_v;
  if (!fp_reg_of(op.src, src_v)) {
    return {false, LowerError::DanglingRef, "VecF16Cvt.src"};
  }

  Emitter::FpReg rd;
  arm64::Reg lo, hi;
  if (!allocate_fp_scratch(result, rd) || !allocate_temporary(lo) || !allocate_temporary(hi)) {
    return {false, LowerError::OutOfScratchRegs, "VecF16Cvt"};
  }

  std::vector<arm64::Reg> live_gprs;
  live_gprs.reserve(ref_to_scratch_.size());
  for (const auto& [_, reg] : ref_to_scratch_)
    live_gprs.push_back(reg);
  const auto gpr_pairs = make_gpr_save_pairs(std::move(live_gprs));

  std::vector<Emitter::FpReg> live_fps;
  live_fps.reserve(ref_to_fp_.size());
  for (const auto& [_, reg] : ref_to_fp_)
    live_fps.push_back(reg);
  const auto fp_pairs = make_fp_save_pairs(std::move(live_fps));

  const std::int32_t result_offset =
      static_cast<std::int32_t>(gpr_pairs.size() * 16u + fp_pairs.size() * 32u);

  emitter_.push_pair(arm64::Reg::X0, arm64::Reg::X0);  // lo/hi result slot
  save_gpr_pairs(emitter_, gpr_pairs);

  if (op.kind == ir::VecF16CvtKind::PsToPh) {
    emitter_.mov_imm64(arm64::Reg::X2, op.rounding);
    emitter_.vumov_w_from_lane(arm64::Reg::X1, src_v, 1, Emitter::VecLane::D2);
  }
  emitter_.vumov_w_from_lane(arm64::Reg::X0, src_v, 0, Emitter::VecLane::D2);

  save_fp_pairs(emitter_, fp_pairs);
  const std::uint64_t helper_addr = static_cast<std::uint64_t>(
      op.kind == ir::VecF16CvtKind::PhToPs
          ? reinterpret_cast<std::uintptr_t>(&prisma_backend_f16c_ph2ps_helper)
          : reinterpret_cast<std::uintptr_t>(&prisma_backend_f16c_ps2ph_helper));
  emitter_.mov_imm64(arm64::Reg::X9, helper_addr);
  emitter_.blr(arm64::Reg::X9);
  emitter_.sp_store(arm64::Reg::X0, result_offset);
  emitter_.sp_store(arm64::Reg::X1, result_offset + 8);
  restore_fp_pairs(emitter_, fp_pairs);
  restore_gpr_pairs(emitter_, gpr_pairs);
  emitter_.pop_pair(lo, hi);
  emitter_.fmov_v_from_x(rd, lo, ir::FpSize::F64);
  emitter_.vins_lane_from_w(rd, rd, 1, hi, Emitter::VecLane::D2);
  return {};
}

bool Lowerer::allocate_temporary(arm64::Reg& out) {
  if (free_regs_.empty()) {
    if (!spill_one_ref())
      return false;
  }
  out = free_regs_.back();
  free_regs_.pop_back();
  stmt_temporaries_.push_back(out);
  const unsigned live = static_cast<unsigned>(ref_to_scratch_.size() + stmt_temporaries_.size());
  peak_live_ = std::max(peak_live_, live);
  return true;
}

bool Lowerer::spill_one_ref() {
  if (options_.spill_slots == 0)
    return false;
  if (free_slots_.empty())
    return false;
  if (ref_to_scratch_.empty())
    return false;

  // Belady: evict the ref whose next use is furthest in the future.
  // `last_use_[r]` is the last use index; anything > current stmt
  // index is still in the future, so the max such value is the
  // farthest-used ref. Refs whose last_use <= stmt_index_ are about
  // to be expired anyway — skipping them avoids spilling a zombie.
  ir::Ref victim{};
  std::size_t best = 0;
  bool found = false;
  for (const auto& [r, _reg] : ref_to_scratch_) {
    auto it = last_use_.find(r);
    const std::size_t lu = (it == last_use_.end()) ? stmt_index_ : it->second;
    if (lu <= stmt_index_)
      continue;  // dies this stmt or earlier
    if (!found || lu > best) {
      best = lu;
      victim = r;
      found = true;
    }
  }
  if (!found)
    return false;

  const arm64::Reg vreg = ref_to_scratch_[victim];
  const std::uint32_t slot = free_slots_.back();
  free_slots_.pop_back();

  const std::int32_t offset = options_.spill_slot_base_offset + static_cast<std::int32_t>(slot) * 8;
  emitter_.sp_store(vreg, offset);
  spilled_to_slot_[victim] = slot;
  ref_to_scratch_.erase(victim);
  free_regs_.push_back(vreg);

  const unsigned in_flight = static_cast<unsigned>(spilled_to_slot_.size());
  peak_spills_ = std::max(peak_spills_, in_flight);
  return true;
}

bool Lowerer::reg_of(ir::Ref ref, arm64::Reg& out) {
  auto it = ref_to_scratch_.find(ref);
  if (it != ref_to_scratch_.end()) {
    out = it->second;
    return true;
  }
  // Maybe it's spilled. Reload into a fresh scratch (spilling again if
  // necessary — a spilled ref's reload can itself cause eviction).
  auto sp_it = spilled_to_slot_.find(ref);
  if (sp_it == spilled_to_slot_.end())
    return false;

  const std::uint32_t slot = sp_it->second;
  arm64::Reg rd;
  if (!allocate_scratch(ref, rd))
    return false;
  const std::int32_t offset = options_.spill_slot_base_offset + static_cast<std::int32_t>(slot) * 8;
  emitter_.sp_load(rd, offset);
  spilled_to_slot_.erase(sp_it);
  free_slots_.push_back(slot);
  out = rd;
  return true;
}

LowerResult Lowerer::align_flag_operands(arm64::Reg lhs, arm64::Reg rhs, ir::OpSize size,
                                         arm64::Reg& out_lhs, arm64::Reg& out_rhs) {
  if (size == ir::OpSize::I64) {
    out_lhs = lhs;
    out_rhs = rhs;
    return {};
  }

  arm64::Reg shift;
  if (!allocate_temporary(out_lhs) || !allocate_temporary(out_rhs) || !allocate_temporary(shift)) {
    return {false, LowerError::OutOfScratchRegs, "align_flag_operands needs three temps"};
  }

  emitter_.mov_imm64(shift, 64u - ir::bit_width(size));
  emitter_.lsl(out_lhs, lhs, shift);
  emitter_.lsl(out_rhs, rhs, shift);
  return {};
}

void Lowerer::compute_liveness(std::span<const ir::Stmt> stmts) {
  last_use_.clear();
  auto bump = [&](ir::Ref r, std::size_t i) {
    auto it = last_use_.find(r);
    if (it == last_use_.end() || it->second < i)
      last_use_[r] = i;
  };
  for (std::size_t i = 0; i < stmts.size(); ++i) {
    const auto& s = stmts[i];
    if (s.result) {
      // Seed last_use with the def index; reads below may extend it.
      // This also ensures a never-used def is freed after its own
      // statement instead of being pinned forever.
      auto& entry = last_use_[*s.result];
      if (entry < i)
        entry = i;
    }
    if (const auto* cas = std::get_if<ir::AtomicCmpxchgPair>(&s.op)) {
      auto& entry = last_use_[cas->old_high];
      if (entry < i)
        entry = i;
    }
    std::visit(
        [&](const auto& op) {
          using T = std::decay_t<decltype(op)>;
          if constexpr (std::is_same_v<T, ir::BinOp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::WideDiv>) {
            bump(op.high, i);
            bump(op.low, i);
            bump(op.divisor, i);
          } else if constexpr (std::is_same_v<T, ir::Compare>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::Select>) {
            bump(op.true_value, i);
            bump(op.false_value, i);
          } else if constexpr (std::is_same_v<T, ir::LoadMem>) {
            bump(op.addr, i);
          } else if constexpr (std::is_same_v<T, ir::StoreMem>) {
            bump(op.addr, i);
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::LoadMemTSO>) {
            bump(op.addr, i);
          } else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) {
            bump(op.addr, i);
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchg>) {
            bump(op.addr, i);
            bump(op.expected, i);
            bump(op.new_value, i);
          } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchgPair>) {
            bump(op.addr, i);
            bump(op.expected_low, i);
            bump(op.expected_high, i);
            bump(op.new_low, i);
            bump(op.new_high, i);
          } else if constexpr (std::is_same_v<T, ir::StoreCarry>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::StoreRflags>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromNzcv>) {
            if (op.pf)
              bump(*op.pf, i);
            if (op.af)
              bump(*op.af, i);
          } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromBits>) {
            if (op.pf)
              bump(*op.pf, i);
            if (op.af)
              bump(*op.af, i);
            bump(op.zf, i);
            bump(op.sf, i);
            bump(op.of, i);
          } else if constexpr (std::is_same_v<T, ir::StoreReg>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::CmpFlags>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::AluFlags>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::CondJump>) {
            bump(op.cond, i);
          } else if constexpr (std::is_same_v<T, ir::JumpReg>) {
            bump(op.target, i);
          } else if constexpr (std::is_same_v<T, ir::CallReg>) {
            bump(op.target, i);
          } else if constexpr (std::is_same_v<T, ir::TrapIf>) {
            bump(op.condition, i);
          } else if constexpr (std::is_same_v<T, ir::Extend>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::Truncate>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::FpBinOp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::WriteFlags>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::ReadFlag>) {
            bump(op.flags, i);
          } else if constexpr (std::is_same_v<T, ir::CondJumpFlags>) {
            bump(op.flags, i);
          } else if constexpr (std::is_same_v<T, ir::VecBinOp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecClMul>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecF16Cvt>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::StoreVecReg>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::StoreVecRegHi>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::VecFpBinOp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecFpFma>) {
            bump(op.a, i);
            bump(op.b, i);
            bump(op.c, i);
          } else if constexpr (std::is_same_v<T, ir::VecFpScalarFma>) {
            bump(op.a, i);
            bump(op.b, i);
            bump(op.c, i);
            bump(op.scalar_upper, i);
          } else if constexpr (std::is_same_v<T, ir::RepStos>) {
            (void)op;
          }  // pinned-reg side effects only
          else if constexpr (std::is_same_v<T, ir::RepMovs>) {
            (void)op;
          } else if constexpr (std::is_same_v<T, ir::VecFpScalarBinOp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::LoadVec>) {
            bump(op.addr, i);
          } else if constexpr (std::is_same_v<T, ir::StoreVec>) {
            bump(op.addr, i);
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::XmmFromGpr>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::GprFromXmm>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::PcmpStrIndex>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
            if (op.lhs_len)
              bump(*op.lhs_len, i);
            if (op.rhs_len)
              bump(*op.rhs_len, i);
          } else if constexpr (std::is_same_v<T, ir::PcmpStrMask>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
            if (op.lhs_len)
              bump(*op.lhs_len, i);
            if (op.rhs_len)
              bump(*op.rhs_len, i);
          } else if constexpr (std::is_same_v<T, ir::PcmpStrFlags>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
            if (op.lhs_len)
              bump(*op.lhs_len, i);
            if (op.rhs_len)
              bump(*op.rhs_len, i);
          } else if constexpr (std::is_same_v<T, ir::VecCmp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecShuffle32x4>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecUnpack>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecShiftImm>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecShiftBytes>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::IntToFpScalar>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::FpToIntScalar>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::FpCvtScalar>) {
            bump(op.lhs, i);
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecShuffle2Src>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecInsertLane>) {
            bump(op.lhs_xmm, i);
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::VecExtractLaneU>) {
            bump(op.src_xmm, i);
          } else if constexpr (std::is_same_v<T, ir::VecMaskMsb>) {
            bump(op.src_xmm, i);
          } else if constexpr (std::is_same_v<T, ir::WriteFlagsFp>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecShuffleH4>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecMaskFp>) {
            bump(op.src_xmm, i);
          } else if constexpr (std::is_same_v<T, ir::VecFpCompare>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecPshufb>) {
            bump(op.src, i);
            bump(op.mask, i);
          } else if constexpr (std::is_same_v<T, ir::VecAbs>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecAlignr>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecExtend>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecFpRound>) {
            bump(op.lhs, i);
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::Popcnt>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::Lzcnt>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::Tzcnt>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::WriteFlagsCountZero>) {
            bump(op.src, i);
            bump(op.result, i);
          } else if constexpr (std::is_same_v<T, ir::VecBlend>) {
            bump(op.dst, i);
            bump(op.src, i);
            bump(op.mask, i);
          } else if constexpr (std::is_same_v<T, ir::WriteFlagsPtest>) {
            bump(op.lhs, i);
            bump(op.rhs, i);
          } else if constexpr (std::is_same_v<T, ir::WriteFlagsPtestYmm>) {
            bump(op.lo_lhs, i);
            bump(op.lo_rhs, i);
            bump(op.hi_lhs, i);
            bump(op.hi_rhs, i);
          } else if constexpr (std::is_same_v<T, ir::VecTbl2>) {
            bump(op.src_lo, i);
            bump(op.src_hi, i);
            bump(op.idx, i);
          } else if constexpr (std::is_same_v<T, ir::VecAes>) {
            bump(op.src, i);
            bump(op.key, i);
          } else if constexpr (std::is_same_v<T, ir::VecAesKeygenAssist>) {
            bump(op.src, i);
          } else if constexpr (std::is_same_v<T, ir::VecSha>) {
            bump(op.a, i);
            bump(op.b, i);
            bump(op.wk, i);
          } else if constexpr (std::is_same_v<T, ir::Bswap>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::Crc32c>) {
            bump(op.crc, i);
            bump(op.data, i);
          } else if constexpr (std::is_same_v<T, ir::VecGather>) {
            bump(op.base, i);
            bump(op.index, i);
            bump(op.mask, i);
            bump(op.prev, i);
          } else if constexpr (std::is_same_v<T, ir::X87Load>) {
            (void)op;
          } else if constexpr (std::is_same_v<T, ir::X87Store>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::X87Push>) {
            bump(op.value, i);
          } else if constexpr (std::is_same_v<T, ir::X87Pop>) {
            (void)op;
          }
          // Constant, LoadReg, LoadSegBase, Jump, JumpRel, CondJumpRel,
          // Return, CallRel, RetAdjusted, Cpuid, Syscall, Trap, Fence
          // have no operand refs — nothing to bump.
        },
        s.op);
  }
}

void Lowerer::expire_intervals() {
  // Return any single-stmt temporaries to the pool.
  for (arm64::Reg r : stmt_temporaries_)
    free_regs_.push_back(r);
  stmt_temporaries_.clear();

  // Expire SSA refs whose last_use is behind us.
  for (auto it = ref_to_scratch_.begin(); it != ref_to_scratch_.end();) {
    auto lu_it = last_use_.find(it->first);
    // If for some reason the ref is missing from last_use_, keep it
    // alive — better to OOM later than to free a live reg.
    const bool expired = lu_it != last_use_.end() && lu_it->second <= stmt_index_;
    if (expired) {
      free_regs_.push_back(it->second);
      it = ref_to_scratch_.erase(it);
    } else {
      ++it;
    }
  }

  // F2-BK-006 — same liveness-based expiry for the FP pool. Without
  // this, every Vec*/Fp* SSA ref sticks to its V-reg until end-of-
  // block, and AVX-256 chains exhaust the pool deterministically.
  for (auto it = ref_to_fp_.begin(); it != ref_to_fp_.end();) {
    auto lu_it = last_use_.find(it->first);
    const bool expired = lu_it != last_use_.end() && lu_it->second <= stmt_index_;
    if (expired) {
      fp_free_.push_back(it->second);
      it = ref_to_fp_.erase(it);
    } else {
      ++it;
    }
  }
}

LowerResult Lowerer::lower(std::span<const ir::Stmt> stmts) {
  // Reset per-call state. `last_use_` and friends may carry stale data
  // from a prior lower() invocation on the same Lowerer instance.
  ref_to_scratch_.clear();
  stmt_temporaries_.clear();
  free_regs_.clear();
  spilled_to_slot_.clear();
  free_slots_.clear();
  block_labels_.clear();
  ref_to_fp_.clear();
  fp_free_.clear();
  fp_free_.reserve(kFpScratchPoolSize);
  for (unsigned i = kFpScratchPoolSize; i-- > 0;) {
    fp_free_.push_back(fp_scratch_reg(i));
  }
  flag_refs_.clear();
  fp_flag_refs_.clear();
  peak_live_ = 0;
  peak_spills_ = 0;

  // Seed the spill slot free-list if spilling is enabled. Slots are
  // handed out in order 0..N-1 so stack layout is predictable.
  if (options_.spill_slots > 0) {
    free_slots_.reserve(options_.spill_slots);
    for (unsigned i = options_.spill_slots; i-- > 0;) {
      free_slots_.push_back(i);
    }
  }

  // Seed the free-list so that allocate_scratch hands out x0 first, x1
  // second, etc. — matches the old bump-pointer ordering, which keeps
  // existing emitter tests happy.
  free_regs_.reserve(kScratchPoolSize);
  for (unsigned i = kScratchPoolSize; i-- > 0;) {
    free_regs_.push_back(scratch_reg(i));
  }

  compute_liveness(stmts);

  for (stmt_index_ = 0; stmt_index_ < stmts.size(); ++stmt_index_) {
    LowerResult r = lower_stmt(stmts[stmt_index_]);
    if (!r.success)
      return r;
    expire_intervals();
  }
  return {};
}

LowerResult Lowerer::lower(const ir::Function& fn) {
  // Per-call reset, identical to the flat overload.
  ref_to_scratch_.clear();
  stmt_temporaries_.clear();
  free_regs_.clear();
  spilled_to_slot_.clear();
  free_slots_.clear();
  block_labels_.clear();
  ref_to_fp_.clear();
  fp_free_.clear();
  fp_free_.reserve(kFpScratchPoolSize);
  for (unsigned i = kFpScratchPoolSize; i-- > 0;) {
    fp_free_.push_back(fp_scratch_reg(i));
  }
  flag_refs_.clear();
  fp_flag_refs_.clear();
  peak_live_ = 0;
  peak_spills_ = 0;
  if (options_.spill_slots > 0) {
    free_slots_.reserve(options_.spill_slots);
    for (unsigned i = options_.spill_slots; i-- > 0;)
      free_slots_.push_back(i);
  }

  // Pre-create one Label per block so forward branches resolve.
  block_labels_.reserve(fn.blocks.size());
  std::unordered_set<std::uint32_t> seen_blocks;
  bool has_entry = false;
  for (const auto& b : fn.blocks) {
    if (!seen_blocks.insert(b.id).second) {
      return {false, LowerError::InvalidBlock, "duplicate block id"};
    }
    has_entry = has_entry || (b.id == fn.entry);
    block_labels_[b.id] = emitter_.create_label();
  }
  if (!has_entry) {
    return {false, LowerError::InvalidBlock, "entry block missing"};
  }

  // Per-block: rebuild liveness, refill the scratch pool, bind the
  // label, then lower the block's stmts. SSA refs are block-local in
  // the current MVP (no cross-block phi support yet — F1-IR-021/022
  // will introduce that), so register state is reset between blocks.
  for (const auto& b : fn.blocks) {
    ref_to_scratch_.clear();
    stmt_temporaries_.clear();
    free_regs_.clear();
    free_regs_.reserve(kScratchPoolSize);
    for (unsigned i = kScratchPoolSize; i-- > 0;) {
      free_regs_.push_back(scratch_reg(i));
    }
    compute_liveness(b.stmts);

    emitter_.bind(block_labels_.at(b.id));

    for (stmt_index_ = 0; stmt_index_ < b.stmts.size(); ++stmt_index_) {
      LowerResult r = lower_stmt(b.stmts[stmt_index_]);
      if (!r.success)
        return r;
      expire_intervals();
    }
  }
  return {};
}

LowerResult Lowerer::lower_stmt(const ir::Stmt& s) {
  return std::visit(
      [&](auto const& op) -> LowerResult {
        using T = std::decay_t<decltype(op)>;

        if constexpr (std::is_same_v<T, ir::Constant>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "Constant without result ref"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Constant"};
          }
          emitter_.mov_imm64(rd, ir::mask_to_size(op.value, op.size));
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadReg>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "LoadReg without result ref"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadReg"};
          }
          // Copy the pinned host reg into a scratch so subsequent StoreReg
          // writes cannot clobber this value.
          emitter_.mov_reg_reg(rd, arm64::host_reg_for(op.reg), op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreReg>) {
          arm64::Reg src;
          if (!reg_of(op.value, src)) {
            return {false, LowerError::DanglingRef, "StoreReg.value"};
          }
          emitter_.store_reg_reg(arm64::host_reg_for(op.reg), src, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::BinOp>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "BinOp without result ref"};
          arm64::Reg rn, rm;
          if (!reg_of(op.lhs, rn))
            return {false, LowerError::DanglingRef, "BinOp.lhs"};
          if (!reg_of(op.rhs, rm))
            return {false, LowerError::DanglingRef, "BinOp.rhs"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "BinOp"};
          }
          // Rotate-left and rotate-left-with-carry lower via the
          // dedicated emitter op that encapsulates `neg tmp, rm; ror
          // rd, rn, tmp`. The temporary comes from the scratch pool
          // and is released at end-of-stmt by the linear-scan
          // allocator.
          if (op.op == ir::BinOpKind::Rol || op.op == ir::BinOpKind::Rcl) {
            arm64::Reg tmp;
            if (!allocate_temporary(tmp)) {
              return {false, LowerError::OutOfScratchRegs, "BinOp temporary"};
            }
            emitter_.rol(rd, rn, rm, tmp);
            return {};
          }
          if (op.op == ir::BinOpKind::Pdep || op.op == ir::BinOpKind::Pext) {
            arm64::Reg rsrc, rmask, rbit, rone, rlowbit, rtmp;
            if (!allocate_temporary(rsrc) || !allocate_temporary(rmask) ||
                !allocate_temporary(rbit) || !allocate_temporary(rone) ||
                !allocate_temporary(rlowbit) || !allocate_temporary(rtmp)) {
              return {false, LowerError::OutOfScratchRegs, "PDEP/PEXT temporaries"};
            }
            emit_bit_permute(emitter_, op.op == ir::BinOpKind::Pdep, rd, rn, rm, rsrc, rmask, rbit,
                             rone, rlowbit, rtmp, op.size);
            return {};
          }
          // F2-BK-007 — UMod / SMod = n - (n / m) * m.
          if (op.op == ir::BinOpKind::UMod || op.op == ir::BinOpKind::SMod) {
            arm64::Reg q;
            if (!allocate_temporary(q)) {
              return {false, LowerError::OutOfScratchRegs, "Mod temporary"};
            }
            emit_divisor_zero_sigfpe_guard(emitter_, rm, options_.emit_ret_on_terminator);
            if (op.op == ir::BinOpKind::UMod) {
              emitter_.udiv(q, rn, rm);
            } else {
              emitter_.sdiv(q, rn, rm);
            }
            emitter_.msub(rd, q, rm, rn);  // rd = rn - q*rm
            return {};
          }
          if (op.op == ir::BinOpKind::UDiv || op.op == ir::BinOpKind::SDiv) {
            emit_divisor_zero_sigfpe_guard(emitter_, rm, options_.emit_ret_on_terminator);
          }
          return emit_binop(emitter_, op.op, rd, rn, rm);
        } else if constexpr (std::is_same_v<T, ir::WideDiv>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "WideDiv without result ref"};
          arm64::Reg high, low, divisor;
          if (!reg_of(op.high, high))
            return {false, LowerError::DanglingRef, "WideDiv.high"};
          if (!reg_of(op.low, low))
            return {false, LowerError::DanglingRef, "WideDiv.low"};
          if (!reg_of(op.divisor, divisor))
            return {false, LowerError::DanglingRef, "WideDiv.divisor"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "WideDiv"};
          }
          WideDivWorkRegs work{};
          if (!allocate_temporary(work.rem) || !allocate_temporary(work.low) ||
              !allocate_temporary(work.divisor) || !allocate_temporary(work.mask) ||
              !allocate_temporary(work.bit) || !allocate_temporary(work.overflow)) {
            return {false, LowerError::OutOfScratchRegs, "WideDiv temporaries"};
          }

          emit_divisor_zero_sigfpe_guard(emitter_, divisor, options_.emit_ret_on_terminator);
          emitter_.mov_reg_reg(work.rem, high);
          emitter_.mov_reg_reg(work.low, low);
          emitter_.mov_reg_reg(work.divisor, divisor);
          if (op.is_signed) {
            emit_abs_signed_wide_div_inputs(emitter_, work);
            emit_unsigned_wide_div_overflow_guard(emitter_, work.rem, work.divisor,
                                                  options_.emit_ret_on_terminator);
          } else {
            emit_unsigned_wide_div_overflow_guard(emitter_, high, divisor,
                                                  options_.emit_ret_on_terminator);
          }

          emit_wide_udiv_core(emitter_, rd, work);
          if (op.result == ir::WideDivResult::Remainder) {
            emitter_.mov_reg_reg(rd, work.rem);
          }
          if (op.is_signed) {
            emitter_.lsr_imm(work.bit, high, 63);
            emitter_.lsr_imm(work.overflow, divisor, 63);
            emitter_.eor(work.bit, work.bit, work.overflow);
            emit_signed_wide_div_quotient_overflow_guard(emitter_, rd,
                                                         options_.emit_ret_on_terminator, work);
            emit_apply_signed_wide_div_result(emitter_, rd, high, divisor, op.result, work);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::Compare>) {
          // Compare produces a 0/1 value in an SSA ref. Lower to
          // cmp + cset. Size-specific comparison is a future concern
          // (we currently do 64-bit comparison regardless of op.size).
          if (!s.result)
            return {false, LowerError::DanglingRef, "Compare without result ref"};
          arm64::Reg rn, rm;
          if (!reg_of(op.lhs, rn))
            return {false, LowerError::DanglingRef, "Compare.lhs"};
          if (!reg_of(op.rhs, rm))
            return {false, LowerError::DanglingRef, "Compare.rhs"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Compare"};
          }
          emitter_.cmp(rn, rm);
          emitter_.cset(rd, op.cc);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Select>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "Select without result ref"};
          arm64::Reg r_true;
          if (!reg_of(op.true_value, r_true)) {
            return {false, LowerError::DanglingRef, "Select.true_value"};
          }
          arm64::Reg r_false;
          if (!reg_of(op.false_value, r_false)) {
            return {false, LowerError::DanglingRef, "Select.false_value"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Select"};
          }
          // Select maps directly to ARM64 conditional select using current NZCV
          // (from a prior CmpFlags in MVP flow).
          emitter_.csel(rd, r_true, r_false, op.cc);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadMem>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "LoadMem without result ref"};
          arm64::Reg raddr;
          if (!reg_of(op.addr, raddr))
            return {false, LowerError::DanglingRef, "LoadMem.addr"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadMem"};
          }
          emitter_.load(rd, raddr, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreMem>) {
          arm64::Reg raddr, rv;
          if (!reg_of(op.addr, raddr))
            return {false, LowerError::DanglingRef, "StoreMem.addr"};
          if (!reg_of(op.value, rv))
            return {false, LowerError::DanglingRef, "StoreMem.value"};
          emitter_.store(rv, raddr, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadMemTSO>) {
          if (!s.result)
            return {false, LowerError::DanglingRef, "LoadMemTSO without result ref"};
          arm64::Reg raddr;
          if (!reg_of(op.addr, raddr))
            return {false, LowerError::DanglingRef, "LoadMemTSO.addr"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadMemTSO"};
          }
          emitter_.load_acquire(rd, raddr, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) {
          arm64::Reg raddr, rv;
          if (!reg_of(op.addr, raddr))
            return {false, LowerError::DanglingRef, "StoreMemTSO.addr"};
          if (!reg_of(op.value, rv))
            return {false, LowerError::DanglingRef, "StoreMemTSO.value"};
          emitter_.store_release(rv, raddr, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchg>) {
          if (!s.result) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchg without result ref"};
          }
          arm64::Reg raddr, expected, new_value;
          if (!reg_of(op.addr, raddr)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchg.addr"};
          }
          if (!reg_of(op.expected, expected)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchg.expected"};
          }
          if (!reg_of(op.new_value, new_value)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchg.new_value"};
          }

          arm64::Reg old_value, status;
          if (!allocate_scratch(*s.result, old_value)) {
            return {false, LowerError::OutOfScratchRegs, "AtomicCmpxchg.old_value"};
          }
          if (!allocate_temporary(status)) {
            return {false, LowerError::OutOfScratchRegs, "AtomicCmpxchg.status"};
          }

          Emitter::Label retry = emitter_.create_label();
          Emitter::Label fail = emitter_.create_label();
          Emitter::Label done = emitter_.create_label();

          emitter_.bind(retry);
          emitter_.ldaxr(old_value, raddr, op.size);
          arm64::Reg cmp_lhs = old_value;
          arm64::Reg cmp_rhs = expected;
          auto aligned = align_flag_operands(old_value, expected, op.size, cmp_lhs, cmp_rhs);
          if (!aligned.success)
            return aligned;
          emitter_.cmp(cmp_lhs, cmp_rhs);
          emitter_.branch_cc(fail, ir::CondCode::Ne);
          emitter_.stlxr(status, new_value, raddr, op.size);
          emitter_.cbnz(status, retry);
          emitter_.branch(done);
          emitter_.bind(fail);
          emitter_.clrex();
          emitter_.bind(done);
          return {};
        } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchgPair>) {
          if (!s.result) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair without low result ref"};
          }
          arm64::Reg raddr, expected_low, expected_high, new_low, new_high;
          if (!reg_of(op.addr, raddr)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair.addr"};
          }
          if (!reg_of(op.expected_low, expected_low)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair.expected_low"};
          }
          if (!reg_of(op.expected_high, expected_high)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair.expected_high"};
          }
          if (!reg_of(op.new_low, new_low)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair.new_low"};
          }
          if (!reg_of(op.new_high, new_high)) {
            return {false, LowerError::DanglingRef, "AtomicCmpxchgPair.new_high"};
          }
          arm64::Reg old_low, old_high, status;
          if (!allocate_scratch(*s.result, old_low)) {
            return {false, LowerError::OutOfScratchRegs, "AtomicCmpxchgPair.old_low"};
          }
          if (!allocate_scratch(op.old_high, old_high)) {
            return {false, LowerError::OutOfScratchRegs, "AtomicCmpxchgPair.old_high"};
          }
          if (!allocate_temporary(status)) {
            return {false, LowerError::OutOfScratchRegs, "AtomicCmpxchgPair.status"};
          }

          Emitter::Label retry = emitter_.create_label();
          Emitter::Label fail = emitter_.create_label();
          Emitter::Label done = emitter_.create_label();

          emitter_.bind(retry);
          emitter_.ldaxp(old_low, old_high, raddr);
          emitter_.cmp(old_low, expected_low);
          emitter_.branch_cc(fail, ir::CondCode::Ne);
          emitter_.cmp(old_high, expected_high);
          emitter_.branch_cc(fail, ir::CondCode::Ne);
          emitter_.stlxp(status, new_low, new_high, raddr);
          emitter_.cbnz(status, retry);
          emitter_.branch(done);
          emitter_.bind(fail);
          emitter_.clrex();
          emitter_.bind(done);
          return {};
        } else if constexpr (std::is_same_v<T, ir::CmpFlags>) {
          // Side-effecting: emits ARM64 `cmp`, leaves NZCV set for the
          // NEXT CondJumpRel / SetCC. No result ref; no scratch
          // allocated.
          arm64::Reg rl, rr;
          if (!reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "CmpFlags.lhs"};
          if (!reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "CmpFlags.rhs"};
          arm64::Reg fl, fr;
          auto aligned = align_flag_operands(rl, rr, op.size, fl, fr);
          if (!aligned.success)
            return aligned;
          emitter_.cmp(fl, fr);
          return {};
        } else if constexpr (std::is_same_v<T, ir::AluFlags>) {
          // Side-effecting ALU flags for instructions such as XADD:
          // leave NZCV set for a following CondJumpRel without creating
          // a flag-typed SSA ref.
          arm64::Reg rl, rr;
          if (!reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "AluFlags.lhs"};
          if (!reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "AluFlags.rhs"};
          arm64::Reg fl, fr;
          auto aligned = align_flag_operands(rl, rr, op.size, fl, fr);
          if (!aligned.success)
            return aligned;
          switch (op.op) {
            case ir::BinOpKind::Sub:
              emitter_.cmp(fl, fr);
              break;
            case ir::BinOpKind::Add:
            case ir::BinOpKind::And:
            case ir::BinOpKind::Or:
            case ir::BinOpKind::Xor: {
              arm64::Reg rd_tmp;
              if (!allocate_temporary(rd_tmp)) {
                return {false, LowerError::OutOfScratchRegs,
                        "AluFlags(Add/And/Or/Xor) needs a temp"};
              }
              if (op.op == ir::BinOpKind::Add) {
                emitter_.adds(rd_tmp, fl, fr);
              } else if (op.op == ir::BinOpKind::And) {
                emitter_.ands(rd_tmp, fl, fr);
              } else {
                if (op.op == ir::BinOpKind::Or) {
                  emitter_.orr(rd_tmp, fl, fr);
                } else {
                  emitter_.eor(rd_tmp, fl, fr);
                }
                emitter_.ands(rd_tmp, rd_tmp, rd_tmp);
              }
              break;
            }
            default:
              return {false, LowerError::UnsupportedOp,
                      "AluFlags only supports Sub/Add/And/Or/Xor today"};
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::JumpRel>) {
          // Load the absolute guest target into x0. The dispatcher
          // reads it as the next guest PC. Translator-wrapped mode
          // elides the ret so it can emit a state-save epilogue first.
          emitter_.mov_imm64(arm64::Reg::X0, op.target_guest_pc);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::JumpReg>) {
          arm64::Reg target;
          if (!reg_of(op.target, target)) {
            return {false, LowerError::DanglingRef, "JumpReg.target"};
          }
          emitter_.mov_reg_reg(arm64::Reg::X0, target);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::Trap>) {
          // Placeholder until the runtime grows first-class guest
          // signal delivery. The Translator/Dispatcher surfaces the
          // trap via block metadata; the machine code just returns.
          emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::TrapIf>) {
          arm64::Reg condition;
          if (!reg_of(op.condition, condition)) {
            return {false, LowerError::DanglingRef, "TrapIf.condition"};
          }
          const auto ok = emitter_.create_label();
          emitter_.cbz(condition, ok);
          emit_sigfpe_placeholder_return(emitter_, options_.emit_ret_on_terminator);
          emitter_.bind(ok);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Cpuid>) {
          // Guest CPUID with translation-time-baked values (the
          // Translator derives them from runtime::host_features()).
          // Modelled leaves:
          //   EAX=0                -> max basic leaf + vendor string;
          //   EAX=1                -> signature + feature bits;
          //   EAX=7 or EAX>7 basic -> leaf-7 view: EBX =
          //                           cpuid_leaf7_ebx when ECX=0,
          //                           zeros otherwise. The >max
          //                           clamp matches the SDM ("data
          //                           for the highest basic
          //                           information leaf").
          //   EAX=2..6             -> all zeros (placeholder until
          //                           those leaves are modelled);
          //   EAX bit 31 set       -> all zeros (extended range
          //                           unmodelled).
          // CPUID must not affect guest flags (SDM: "Flags Affected:
          // None"), so the leaf dispatch uses orr/eor/lsr + cbz/cbnz
          // instead of cmp — NZCV set by an earlier CmpFlags
          // survives. The W-forms also give the architectural
          // EAX/ECX (not RAX/RCX) comparison: upper bits ignored.
          const arm64::Reg rax = arm64::host_reg_for(ir::Gpr::Rax);
          const arm64::Reg rbx = arm64::host_reg_for(ir::Gpr::Rbx);
          const arm64::Reg rcx = arm64::host_reg_for(ir::Gpr::Rcx);
          const arm64::Reg rdx = arm64::host_reg_for(ir::Gpr::Rdx);
          arm64::Reg t0, t1;
          if (!allocate_temporary(t0) || !allocate_temporary(t1)) {
            return {false, LowerError::OutOfScratchRegs, "Cpuid temporaries"};
          }
          Emitter::Label leaf0 = emitter_.create_label();
          Emitter::Label leaf1 = emitter_.create_label();
          Emitter::Label leaf7 = emitter_.create_label();
          Emitter::Label other = emitter_.create_label();
          Emitter::Label done = emitter_.create_label();
          emitter_.orr_w(t0, rax, rax);  // t0 = EAX, zero-extended
          emitter_.cbz(t0, leaf0);
          emitter_.mov_imm64(t1, 1);
          emitter_.eor_w(t1, t0, t1);  // t1 = EAX ^ 1
          emitter_.cbz(t1, leaf1);
          emitter_.mov_imm64(t1, 7);
          emitter_.eor_w(t1, t0, t1);  // t1 = EAX ^ 7
          emitter_.cbz(t1, leaf7);
          emitter_.lsr_imm(t1, t0, 31);  // extended range (bit 31)?
          emitter_.cbnz(t1, other);
          emitter_.lsr_imm(t1, t0, 3);  // EAX >= 8: clamp to max
          emitter_.cbnz(t1, leaf7);     // basic leaf (7) per SDM
          emitter_.branch(other);       // EAX in 2..6: unmodelled
          emitter_.bind(leaf7);
          emitter_.orr_w(t0, rcx, rcx);  // t0 = ECX (subleaf)
          emitter_.cbnz(t0, other);
          // CPUID.(EAX=7, ECX=0): EAX = max subleaf (0), EBX = the
          // baked feature bits (BMI1 / BMI2 / SHA, host-gated where the
          // lowering needs host crypto).
          emitter_.mov_imm64(rax, 0);
          emitter_.mov_imm64(rbx, options_.cpuid_leaf7_ebx);
          emitter_.mov_imm64(rcx, 0);
          emitter_.mov_imm64(rdx, 0);
          emitter_.branch(done);
          emitter_.bind(leaf1);
          emitter_.mov_imm64(rax, options_.cpuid_leaf1_eax);
          emitter_.mov_imm64(rbx, options_.cpuid_leaf1_ebx);
          emitter_.mov_imm64(rcx, options_.cpuid_leaf1_ecx);
          emitter_.mov_imm64(rdx, options_.cpuid_leaf1_edx);
          emitter_.branch(done);
          emitter_.bind(leaf0);
          emitter_.mov_imm64(rax, options_.cpuid_max_leaf);
          emitter_.mov_imm64(rbx, options_.cpuid_vendor_ebx);
          emitter_.mov_imm64(rcx, options_.cpuid_vendor_ecx);
          emitter_.mov_imm64(rdx, options_.cpuid_vendor_edx);
          emitter_.branch(done);
          emitter_.bind(other);
          emitter_.mov_imm64(rax, 0);
          emitter_.mov_imm64(rbx, 0);
          emitter_.mov_imm64(rcx, 0);
          emitter_.mov_imm64(rdx, 0);
          emitter_.bind(done);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Rdtsc>) {
          // Guest RDTSC time source: the ARM virtual counter.
          // Monotonic; frequency is CNTFRQ, not core clock — guests
          // calibrate ratios, so any monotonic source is sound.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Rdtsc without result"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Rdtsc"};
          }
          emitter_.mrs_cntvct(rd);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Xgetbv>) {
          // XGETBV with a translation-time-baked XCR0. ECX selects
          // the XCR; only XCR0 (ECX=0) is modelled — other indices
          // raise #GP on hardware, placeholder returns zeros. Writes
          // EDX:EAX only (EBX/ECX untouched, like hardware). Flag-
          // free for the same reason as Cpuid.
          const arm64::Reg rax = arm64::host_reg_for(ir::Gpr::Rax);
          const arm64::Reg rcx = arm64::host_reg_for(ir::Gpr::Rcx);
          const arm64::Reg rdx = arm64::host_reg_for(ir::Gpr::Rdx);
          arm64::Reg t0;
          if (!allocate_temporary(t0)) {
            return {false, LowerError::OutOfScratchRegs, "Xgetbv temporary"};
          }
          Emitter::Label other = emitter_.create_label();
          Emitter::Label done = emitter_.create_label();
          emitter_.orr_w(t0, rcx, rcx);  // t0 = ECX, zero-extended
          emitter_.cbnz(t0, other);
          emitter_.mov_imm64(rax, options_.xgetbv_xcr0 & 0xFFFFFFFFu);
          emitter_.mov_imm64(rdx, options_.xgetbv_xcr0 >> 32);
          emitter_.branch(done);
          emitter_.bind(other);
          emitter_.mov_imm64(rax, 0);
          emitter_.mov_imm64(rdx, 0);
          emitter_.bind(done);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Syscall>) {
          // Save caller-saved pinned guest regs (x10-x17 = guest
          // rax..rdi) back to the state frame so the C++ handler sees
          // the current guest register values (args in RDI/RSI/RDX).
          // The callee-saved regs (x19-x26 = guest r8..r15) are
          // preserved across the AAPCS64 blr call automatically.
          // x27 is the state pointer (abi::kStatePtrReg).
          for (std::size_t i = 0; i < 8; ++i) {
            const auto g = static_cast<ir::Gpr>(i);
            const auto host = arm64::host_reg_for(g);
            const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
            emitter_.store_offset(host, arm64::Reg::X27, off);
          }

          // Advance guest_pc to the next instruction before the handler is called.
          // The handler can overwrite it if it needs to (e.g. rt_sigreturn).
          emitter_.mov_imm64(arm64::Reg::X9, op.next_pc);
          emitter_.store_offset(arm64::Reg::X9, arm64::Reg::X27, runtime::CpuStateFrame::guest_pc_offset());

          // If a syscall handler has been configured, emit blr to it.
          // The handler reads guest regs from the state frame, performs
          // the host operation, and writes results (including RAX / CF)
          // back to the frame.
          if (options_.syscall_handler) {
            const std::uint64_t fn_addr = reinterpret_cast<std::uint64_t>(options_.syscall_handler);
            // Use x9 (first scratch reg) as the call target.
            // All x0-x9 are caller-saved and will be clobbered by
            // the called function anyway.
            emitter_.mov_imm64(arm64::Reg::X9, fn_addr);
            emitter_.blr(arm64::Reg::X9);
          }

          // Reload caller-saved pinned guest regs from the state frame
          // so the host registers reflect any changes the handler made
          // (most importantly RAX = return value).
          for (std::size_t i = 0; i < 8; ++i) {
            const auto g = static_cast<ir::Gpr>(i);
            const auto host = arm64::host_reg_for(g);
            const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
            emitter_.load_offset(host, arm64::Reg::X27, off);
          }
          // The block ends here. The next block will start at whatever guest_pc is.
          // Syscall handler may have changed it (e.g., rt_sigreturn).
          emitter_.load_offset(arm64::Reg::X0, arm64::Reg::X27, runtime::CpuStateFrame::guest_pc_offset());
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::CondJumpRel>) {
          // Invariant: some earlier op (CmpFlags in MVP) set NZCV and
          // nothing has clobbered it since. movz / movk used by
          // mov_imm64 don't touch flags, so we can use csel safely.
          //
          //   mov x0, fallthrough
          //   mov x1, target
          //   csel x0, x1, x0, <cc>    ; x0 = cc ? target : fallthrough
          //   ret                      ; (or state-save epilogue + ret)
          emitter_.mov_imm64(arm64::Reg::X0, op.fallthrough_guest_pc);
          emitter_.mov_imm64(arm64::Reg::X1, op.target_guest_pc);
          emitter_.csel(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X0, op.cc);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::Return>) {
          // Guest RET in MVP: set x0 = 0 (the dispatcher halt sentinel,
          // `CpuStateFrame::kHaltSentinel`). The Translator-wrapped
          // path appends a state-save epilogue before the ret, so the
          // Lowerer only emits ret when running in standalone mode.
          emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::Jump>) {
          // F1-BK-003. Only valid inside a Function lowering; the flat
          // overload has an empty block_labels_ and falls through.
          auto it = block_labels_.find(op.target_block);
          if (it == block_labels_.end()) {
            const bool in_function = !block_labels_.empty();
            return {false, in_function ? LowerError::InvalidBlock : LowerError::UnsupportedOp,
                    in_function ? "Jump target block missing"
                                : "Jump outside Function lowering (no block label)"};
          }
          emitter_.branch(it->second);
          return {};
        } else if constexpr (std::is_same_v<T, ir::CondJump>) {
          // F1-BK-004. Cond is an SSA Ref holding 0 or 1 (typically the
          // result of a Compare). Lower as `cbnz xcond, label_true; b
          // label_false`. cbnz / b have the same range envelope so a
          // veneer fires for either or neither, never asymmetrically.
          arm64::Reg rc;
          if (!reg_of(op.cond, rc)) {
            return {false, LowerError::DanglingRef, "CondJump.cond"};
          }
          auto it_t = block_labels_.find(op.if_true);
          auto it_f = block_labels_.find(op.if_false);
          if (it_t == block_labels_.end() || it_f == block_labels_.end()) {
            const bool in_function = !block_labels_.empty();
            return {false, in_function ? LowerError::InvalidBlock : LowerError::UnsupportedOp,
                    in_function ? "CondJump target block missing"
                                : "CondJump outside Function lowering (no block label)"};
          }
          emitter_.cbnz(rc, it_t->second);
          emitter_.branch(it_f->second);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadSegBase>) {
          // F2-SY-029: read segment base from CpuStateFrame.fs_base / gs_base.
          // The frame pointer (kStatePtrReg = X27) is live throughout every
          // translated block; the offset is a compile-time constant so this
          // compiles to a single `ldr xd, [x27, #off]`.
          arm64::Reg rd;
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadSegBase requires a result ref"};
          }
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadSegBase"};
          }
          const std::int32_t seg_off = (op.seg == ir::SegmentReg::Fs)
                                           ? runtime::CpuStateFrame::fs_base_offset()
                                           : runtime::CpuStateFrame::gs_base_offset();
          emitter_.load_offset(rd, abi::kStatePtrReg, seg_off);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadCarry>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadCarry requires a result ref"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadCarry"};
          }
          emitter_.load_offset(rd, abi::kStatePtrReg, runtime::CpuStateFrame::cf_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreCarry>) {
          arm64::Reg value;
          if (!reg_of(op.value, value)) {
            return {false, LowerError::DanglingRef, "StoreCarry.value"};
          }
          arm64::Reg masked, rflags, clear_mask;
          if (!allocate_temporary(masked) || !allocate_temporary(rflags) ||
              !allocate_temporary(clear_mask)) {
            return {false, LowerError::OutOfScratchRegs, "StoreCarry"};
          }
          emitter_.mov_imm64(clear_mask, 1);
          emitter_.and_(masked, value, clear_mask);
          emitter_.store_offset(masked, abi::kStatePtrReg, runtime::CpuStateFrame::cf_offset());

          emitter_.load_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          emitter_.mov_imm64(clear_mask, 0xFFFFFFFFFFFFFFFEULL);
          emitter_.and_(rflags, rflags, clear_mask);
          emitter_.orr(rflags, rflags, masked);
          emitter_.store_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadRflags>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadRflags requires a result ref"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadRflags"};
          }
          emitter_.load_offset(rd, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreRflags>) {
          arm64::Reg value;
          if (!reg_of(op.value, value)) {
            return {false, LowerError::DanglingRef, "StoreRflags.value"};
          }
          arm64::Reg forced, mask, carry;
          if (!allocate_temporary(forced) || !allocate_temporary(mask) ||
              !allocate_temporary(carry)) {
            return {false, LowerError::OutOfScratchRegs, "StoreRflags"};
          }
          emitter_.mov_imm64(mask, 2);
          emitter_.orr(forced, value, mask);
          emitter_.store_offset(forced, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());

          emitter_.mov_imm64(mask, 1);
          emitter_.and_(carry, forced, mask);
          emitter_.store_offset(carry, abi::kStatePtrReg, runtime::CpuStateFrame::cf_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromNzcv>) {
          constexpr std::uint64_t kCf = 1ull << 0;
          constexpr std::uint64_t kPf = 1ull << 2;
          constexpr std::uint64_t kAf = 1ull << 4;
          constexpr std::uint64_t kZf = 1ull << 6;
          constexpr std::uint64_t kSf = 1ull << 7;
          constexpr std::uint64_t kOf = 1ull << 11;

          arm64::Reg rflags, mask, bit, shift, carry;
          if (!allocate_temporary(rflags) || !allocate_temporary(mask) ||
              !allocate_temporary(bit) || !allocate_temporary(shift) ||
              !allocate_temporary(carry)) {
            return {false, LowerError::OutOfScratchRegs, "StoreRflagsFromNzcv"};
          }

          std::uint64_t clear_bits = kZf | kSf | kOf;
          if (op.carry != ir::RflagsCarryMode::Preserve)
            clear_bits |= kCf;
          if (op.pf.has_value())
            clear_bits |= kPf;
          if (op.af.has_value())
            clear_bits |= kAf;

          emitter_.load_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          emitter_.mov_imm64(mask, ~clear_bits);
          emitter_.and_(rflags, rflags, mask);
          emitter_.mov_imm64(mask, 2);
          emitter_.orr(rflags, rflags, mask);

          auto or_bit_reg = [&](arm64::Reg value, unsigned rflags_shift) {
            emitter_.mov_imm64(mask, 1);
            emitter_.and_(bit, value, mask);
            if (rflags_shift != 0) {
              emitter_.mov_imm64(shift, rflags_shift);
              emitter_.lsl(bit, bit, shift);
            }
            emitter_.orr(rflags, rflags, bit);
          };

          auto or_bit_ref = [&](std::optional<ir::Ref> ref, unsigned rflags_shift,
                                const char* name) -> LowerResult {
            if (!ref.has_value())
              return {};
            arm64::Reg value;
            if (!reg_of(*ref, value)) {
              return {false, LowerError::DanglingRef, name};
            }
            or_bit_reg(value, rflags_shift);
            return {};
          };

          auto or_condition = [&](ir::CondCode cc, unsigned rflags_shift) {
            emitter_.cset(bit, cc);
            if (rflags_shift != 0) {
              emitter_.mov_imm64(shift, rflags_shift);
              emitter_.lsl(bit, bit, shift);
            }
            emitter_.orr(rflags, rflags, bit);
          };

          or_condition(ir::CondCode::Eq, 6);
          or_condition(ir::CondCode::Mi, 7);
          or_condition(ir::CondCode::Ov, 11);
          auto pf_res = or_bit_ref(op.pf, 2, "StoreRflagsFromNzcv.pf");
          if (!pf_res.success)
            return pf_res;
          auto af_res = or_bit_ref(op.af, 4, "StoreRflagsFromNzcv.af");
          if (!af_res.success)
            return af_res;

          switch (op.carry) {
            case ir::RflagsCarryMode::ArmCarry:
              or_condition(ir::CondCode::Nc, 0);
              break;
            case ir::RflagsCarryMode::InvertArmCarry:
              or_condition(ir::CondCode::Cc, 0);
              break;
            case ir::RflagsCarryMode::Clear:
            case ir::RflagsCarryMode::Preserve:
              break;
          }

          emitter_.store_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          emitter_.mov_imm64(mask, 1);
          emitter_.and_(carry, rflags, mask);
          emitter_.store_offset(carry, abi::kStatePtrReg, runtime::CpuStateFrame::cf_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromBits>) {
          constexpr std::uint64_t kPf = 1ull << 2;
          constexpr std::uint64_t kAf = 1ull << 4;
          constexpr std::uint64_t kZf = 1ull << 6;
          constexpr std::uint64_t kSf = 1ull << 7;
          constexpr std::uint64_t kOf = 1ull << 11;

          arm64::Reg rflags, mask, bit, shift;
          if (!allocate_temporary(rflags) || !allocate_temporary(mask) ||
              !allocate_temporary(bit) || !allocate_temporary(shift)) {
            return {false, LowerError::OutOfScratchRegs, "StoreRflagsFromBits"};
          }

          std::uint64_t clear_bits = kZf | kSf | kOf;
          if (op.pf.has_value())
            clear_bits |= kPf;
          if (op.af.has_value())
            clear_bits |= kAf;

          emitter_.load_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          emitter_.mov_imm64(mask, ~clear_bits);
          emitter_.and_(rflags, rflags, mask);
          emitter_.mov_imm64(mask, 2);
          emitter_.orr(rflags, rflags, mask);

          auto or_bit_ref = [&](ir::Ref ref, unsigned rflags_shift,
                                const char* name) -> LowerResult {
            arm64::Reg value;
            if (!reg_of(ref, value)) {
              return {false, LowerError::DanglingRef, name};
            }
            emitter_.mov_imm64(mask, 1);
            emitter_.and_(bit, value, mask);
            if (rflags_shift != 0) {
              emitter_.mov_imm64(shift, rflags_shift);
              emitter_.lsl(bit, bit, shift);
            }
            emitter_.orr(rflags, rflags, bit);
            return {};
          };

          if (op.pf) {
            auto res = or_bit_ref(*op.pf, 2, "StoreRflagsFromBits.pf");
            if (!res.success)
              return res;
          }
          if (op.af) {
            auto res = or_bit_ref(*op.af, 4, "StoreRflagsFromBits.af");
            if (!res.success)
              return res;
          }
          auto zf_res = or_bit_ref(op.zf, 6, "StoreRflagsFromBits.zf");
          if (!zf_res.success)
            return zf_res;
          auto sf_res = or_bit_ref(op.sf, 7, "StoreRflagsFromBits.sf");
          if (!sf_res.success)
            return sf_res;
          auto of_res = or_bit_ref(op.of, 11, "StoreRflagsFromBits.of");
          if (!of_res.success)
            return of_res;

          emitter_.store_offset(rflags, abi::kStatePtrReg, runtime::CpuStateFrame::rflags_offset());
          return {};
        } else if constexpr (std::is_same_v<T, ir::Extend>) {
          // F1-BK-022. Sign / zero-extend a narrower view of the
          // source ref into a fresh 64-bit destination scratch. The
          // ARM64 sxt*/uxt* instructions all read Wn and write Xd.
          arm64::Reg rs;
          if (!reg_of(op.value, rs)) {
            return {false, LowerError::DanglingRef, "Extend.value"};
          }
          arm64::Reg rd;
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Extend requires a result ref"};
          }
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Extend"};
          }
          if (op.is_signed) {
            switch (op.from_size) {
              case ir::OpSize::I8:
                emitter_.sxtb(rd, rs);
                break;
              case ir::OpSize::I16:
                emitter_.sxth(rd, rs);
                break;
              case ir::OpSize::I32:
                emitter_.sxtw(rd, rs);
                break;
              case ir::OpSize::I64:
                emitter_.mov_reg_reg(rd, rs);  // identity
                break;
            }
            if (op.to_size != ir::OpSize::I64) {
              emitter_.truncate(rd, rd, op.to_size);
            }
          } else {
            switch (op.from_size) {
              case ir::OpSize::I8:
                emitter_.uxtb(rd, rs);
                break;
              case ir::OpSize::I16:
                emitter_.uxth(rd, rs);
                break;
              case ir::OpSize::I32:
                emitter_.uxtw(rd, rs);
                break;
              case ir::OpSize::I64:
                emitter_.mov_reg_reg(rd, rs);  // identity
                break;
            }
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::Truncate>) {
          // F1-BK-022. Narrow `value` to `to_size` in a fresh 64-bit
          // scratch. For I32 the cheap idiom is `mov wd, wn` (which
          // zeroes the upper bits). For I8/I16 we materialise the
          // mask and AND.
          arm64::Reg rs;
          if (!reg_of(op.value, rs)) {
            return {false, LowerError::DanglingRef, "Truncate.value"};
          }
          arm64::Reg rd;
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Truncate requires a result ref"};
          }
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Truncate"};
          }
          switch (op.to_size) {
            case ir::OpSize::I8:
              emitter_.uxtb(rd, rs);
              break;
            case ir::OpSize::I16:
              emitter_.uxth(rd, rs);
              break;
            case ir::OpSize::I32:
              emitter_.uxtw(rd, rs);
              break;
            case ir::OpSize::I64:
              emitter_.mov_reg_reg(rd, rs);  // identity
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::GuestPc>) {
          // Pseudo-op: no machine code. Tracked in ir::Stmt for cache
          // keying / debugging only. The presence of this op never
          // affects emitted bytes.
          return {};
        } else if constexpr (std::is_same_v<T, ir::InlineAsm>) {
          // Placeholder until the dispatcher learns how to call into
          // a software interpreter for the raw guest bytes (planned
          // alongside F1-RT-011 guest signal delivery). For now we
          // just halt the block, returning the sentinel so the
          // dispatcher knows we couldn't handle this region.
          emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::RepStos>) {
          // F2-BK-008 + Blocker A. Bounded native ARM64 loop.
          //
          //   if RCX == 0: x0 = pc_after_rep; goto block exit.
          //   iter = min(RCX, kRepMaxBytesPerCall / step)
          //   RCX -= iter                       (remaining count for next hop)
          //   loop: store rax→[rdi]; rdi += step; iter -= 1; loop while iter != 0.
          //   x0 = (RCX == 0) ? pc_after_rep : pc_of_rep
          //
          // The clamp turns a guest-controlled RCX into at most
          // `kRepMaxBytesPerCall` bytes of host work per dispatch hop.
          // If the loop did not consume all of RCX, the block exits
          // with `x0 = pc_of_rep` so the dispatcher re-enters the
          // same REP STOS on the next hop with the remaining count —
          // matching x86 REP-is-interruptible semantics exactly.
          const arm64::Reg rcx = arm64::host_reg_for(ir::Gpr::Rcx);
          const arm64::Reg rdi = arm64::host_reg_for(ir::Gpr::Rdi);
          const arm64::Reg rax = arm64::host_reg_for(ir::Gpr::Rax);
          Emitter::Label done_label = emitter_.create_label();
          Emitter::Label tail_label = emitter_.create_label();
          emitter_.cbz(rcx, done_label);
          arm64::Reg max_reg, iter_reg, step_reg, one_reg;
          if (!allocate_temporary(max_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepStos max"};
          }
          if (!allocate_temporary(iter_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepStos iter"};
          }
          if (!allocate_temporary(step_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepStos step"};
          }
          if (!allocate_temporary(one_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepStos one"};
          }
          const std::uint64_t step = static_cast<std::uint64_t>(ir::bit_width(op.size) / 8u);
          const std::uint64_t iter_cap = ir::kRepMaxBytesPerCall / step;
          emitter_.mov_imm64(max_reg, iter_cap);
          emitter_.mov_imm64(step_reg, step);
          emitter_.mov_imm64(one_reg, 1ULL);
          // iter_reg = (RCX < iter_cap) ? RCX : iter_cap
          emitter_.cmp(rcx, max_reg);
          emitter_.csel(iter_reg, rcx, max_reg, ir::CondCode::Ult);
          // RCX = RCX - iter_reg  (remaining count for next hop)
          emitter_.sub(rcx, rcx, iter_reg);
          Emitter::Label loop_label = emitter_.create_label();
          emitter_.bind(loop_label);
          emitter_.store(rax, rdi, op.size);
          if (op.reverse) {
            emitter_.sub(rdi, rdi, step_reg);
          } else {
            emitter_.add(rdi, rdi, step_reg);
          }
          emitter_.sub(iter_reg, iter_reg, one_reg);
          emitter_.cbnz(iter_reg, loop_label);
          emitter_.bind(done_label);
          // Block exit: pick pc_after_rep when RCX hit 0; otherwise
          // pc_of_rep so the dispatcher re-enters the same REP.
          emitter_.mov_imm64(arm64::Reg::X0, op.pc_of_rep);
          emitter_.cbnz(rcx, tail_label);
          emitter_.mov_imm64(arm64::Reg::X0, op.pc_after_rep);
          emitter_.bind(tail_label);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::RepMovs>) {
          // F2-BK-009 + Blocker A. Bounded native ARM64 loop. Same
          // shape as RepStos but reads from [RSI] into a scratch and
          // writes to [RDI]; both pointers advance per iteration.
          // See RepStos for the clamp + re-entry contract.
          const arm64::Reg rcx = arm64::host_reg_for(ir::Gpr::Rcx);
          const arm64::Reg rdi = arm64::host_reg_for(ir::Gpr::Rdi);
          const arm64::Reg rsi = arm64::host_reg_for(ir::Gpr::Rsi);
          Emitter::Label done_label = emitter_.create_label();
          Emitter::Label tail_label = emitter_.create_label();
          emitter_.cbz(rcx, done_label);
          arm64::Reg max_reg, iter_reg, step_reg, one_reg, byte_reg;
          if (!allocate_temporary(max_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepMovs max"};
          }
          if (!allocate_temporary(iter_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepMovs iter"};
          }
          if (!allocate_temporary(step_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepMovs step"};
          }
          if (!allocate_temporary(one_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepMovs one"};
          }
          if (!allocate_temporary(byte_reg)) {
            return {false, LowerError::OutOfScratchRegs, "RepMovs byte"};
          }
          const std::uint64_t step = static_cast<std::uint64_t>(ir::bit_width(op.size) / 8u);
          const std::uint64_t iter_cap = ir::kRepMaxBytesPerCall / step;
          emitter_.mov_imm64(max_reg, iter_cap);
          emitter_.mov_imm64(step_reg, step);
          emitter_.mov_imm64(one_reg, 1ULL);
          emitter_.cmp(rcx, max_reg);
          emitter_.csel(iter_reg, rcx, max_reg, ir::CondCode::Ult);
          emitter_.sub(rcx, rcx, iter_reg);
          Emitter::Label loop_label = emitter_.create_label();
          emitter_.bind(loop_label);
          emitter_.load(byte_reg, rsi, op.size);
          emitter_.store(byte_reg, rdi, op.size);
          if (op.reverse) {
            emitter_.sub(rsi, rsi, step_reg);
            emitter_.sub(rdi, rdi, step_reg);
          } else {
            emitter_.add(rsi, rsi, step_reg);
            emitter_.add(rdi, rdi, step_reg);
          }
          emitter_.sub(iter_reg, iter_reg, one_reg);
          emitter_.cbnz(iter_reg, loop_label);
          emitter_.bind(done_label);
          emitter_.mov_imm64(arm64::Reg::X0, op.pc_of_rep);
          emitter_.cbnz(rcx, tail_label);
          emitter_.mov_imm64(arm64::Reg::X0, op.pc_after_rep);
          emitter_.bind(tail_label);
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else if constexpr (std::is_same_v<T, ir::RspAdjust>) {
          // F1-RT-013. Add or subtract the literal delta from the
          // pinned host register backing guest RSP. Materialise the
          // (possibly negative) delta via mov_imm64 + add (vixl
          // selects the cheap immediate-add encoding when it fits).
          const arm64::Reg rsp_host = arm64::host_reg_for(ir::Gpr::Rsp);
          arm64::Reg tmp;
          if (!allocate_temporary(tmp)) {
            return {false, LowerError::OutOfScratchRegs, "RspAdjust"};
          }
          if (op.delta_bytes >= 0) {
            emitter_.mov_imm64(tmp, static_cast<std::uint64_t>(op.delta_bytes));
            emitter_.add(rsp_host, rsp_host, tmp);
          } else {
            emitter_.mov_imm64(tmp, static_cast<std::uint64_t>(-op.delta_bytes));
            emitter_.sub(rsp_host, rsp_host, tmp);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsFp>) {
          // F2-IR-026. fcmp on the low FP lanes of the two xmm regs.
          // Result Ref lives in NZCV — same model as WriteFlags but
          // we record it in fp_flag_refs_ so ReadFlag dispatches the
          // FP-specific cond-code mapping.
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "WriteFlagsFp.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "WriteFlagsFp.rhs"};
          emitter_.fcmp_scalar(rl, rr, op.size);
          if (s.result.has_value()) {
            flag_refs_.insert(*s.result);
            fp_flag_refs_.insert(*s.result);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::WriteFlags>) {
          // F1-IR-004 / F1-BK lowering. Emit the flag-setting variant
          // of the integer op so NZCV reflects the result, then bind
          // the result Ref to "NZCV is current" (no machine register
          // backing; the consumer reads NZCV directly).
          arm64::Reg rl, rr;
          if (!reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "WriteFlags.lhs"};
          if (!reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "WriteFlags.rhs"};
          arm64::Reg fl, fr;
          auto aligned = align_flag_operands(rl, rr, op.size, fl, fr);
          if (!aligned.success)
            return aligned;
          switch (op.op) {
            case ir::BinOpKind::Sub:
              // `cmp` is `subs xzr, lhs, rhs` — sets NZCV without
              // writing a destination.
              emitter_.cmp(fl, fr);
              break;
            case ir::BinOpKind::Add:
            case ir::BinOpKind::And:
            case ir::BinOpKind::Or:
            case ir::BinOpKind::Xor: {
              // Flag-setting ALU forms need a destination register.
              // Allocate a single-stmt temporary so the result value is
              // discarded but NZCV is set as a side effect.
              arm64::Reg rd_tmp;
              if (!allocate_temporary(rd_tmp)) {
                return {false, LowerError::OutOfScratchRegs,
                        "WriteFlags(Add/And/Or/Xor) needs a temp"};
              }
              if (op.op == ir::BinOpKind::Add) {
                emitter_.adds(rd_tmp, fl, fr);
              } else if (op.op == ir::BinOpKind::And) {
                emitter_.ands(rd_tmp, fl, fr);
              } else {
                if (op.op == ir::BinOpKind::Or) {
                  emitter_.orr(rd_tmp, fl, fr);
                } else {
                  emitter_.eor(rd_tmp, fl, fr);
                }
                emitter_.ands(rd_tmp, rd_tmp, rd_tmp);
              }
              break;
            }
            default:
              return {false, LowerError::UnsupportedOp,
                      "WriteFlags only supports Sub/Add/And/Or/Xor today"};
          }
          // Result Ref lives in NZCV; track it in flag_refs_ so
          // consumers verify their operand was a real WriteFlags
          // result (no host register backing).
          if (s.result.has_value()) {
            flag_refs_.insert(*s.result);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::ReadFlag>) {
          // F1-IR-005. Emit `cset rd, <armcond>` for the requested
          // FlagBit. The producer (WriteFlags) must have set NZCV
          // and nothing in between may have clobbered it.
          if (flag_refs_.find(op.flags) == flag_refs_.end()) {
            return {false, LowerError::DanglingRef, "ReadFlag.flags must be a WriteFlags result"};
          }
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "ReadFlag requires a result ref"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "ReadFlag"};
          }
          const bool is_fp = fp_flag_refs_.count(op.flags) > 0;
          ir::CondCode cc = ir::CondCode::Eq;
          if (is_fp) {
            // FP source (UCOMISS/UCOMISD): NZCV from fcmp.
            //   x86 ZF → ARM "eq" (Z=1)
            //   x86 CF → ARM "lt" (N!=V) — true when lhs<rhs or unordered
            //   x86 PF → ARM "vs" (V=1) — true on unordered
            //   SF/OF/AF: x86 clears them after UCOMI*; we'd need
            //   to materialise constant 0 — return error for now.
            switch (op.which) {
              case ir::FlagBit::Zero:
                cc = ir::CondCode::Eq;
                break;
              case ir::FlagBit::Carry:
                cc = ir::CondCode::Slt;
                break;
              case ir::FlagBit::Parity:
                cc = ir::CondCode::Ov;
                break;  // V=1 → vs
              case ir::FlagBit::Sign:
              case ir::FlagBit::Overflow:
              case ir::FlagBit::Aux:
                return {false, LowerError::UnsupportedOp, "ReadFlag(SF/OF/AF) on FP-source flags"};
            }
          } else {
            switch (op.which) {
              case ir::FlagBit::Carry:
                cc = ir::CondCode::Cc;
                break;
              case ir::FlagBit::Zero:
                cc = ir::CondCode::Eq;
                break;
              case ir::FlagBit::Sign:
                cc = ir::CondCode::Mi;
                break;
              case ir::FlagBit::Overflow:
                cc = ir::CondCode::Ov;
                break;
              case ir::FlagBit::Parity:
              case ir::FlagBit::Aux:
                return {false, LowerError::UnsupportedOp,
                        "ReadFlag(Parity/Aux) needs SW emulation"};
            }
          }
          emitter_.cset(rd, cc);
          return {};
        } else if constexpr (std::is_same_v<T, ir::CondJumpFlags>) {
          // F1-IR-007. Branch on NZCV using the supplied CondCode.
          auto it_t = block_labels_.find(op.if_true);
          auto it_f = block_labels_.find(op.if_false);
          if (it_t == block_labels_.end() || it_f == block_labels_.end()) {
            const bool in_function = !block_labels_.empty();
            return {false, in_function ? LowerError::InvalidBlock : LowerError::UnsupportedOp,
                    in_function ? "CondJumpFlags target block missing"
                                : "CondJumpFlags outside Function lowering (no block label)"};
          }
          if (flag_refs_.find(op.flags) == flag_refs_.end()) {
            return {false, LowerError::DanglingRef,
                    "CondJumpFlags.flags must be a WriteFlags result"};
          }
          // F2-IR-026. For FP-source flags, remap x86 CF-based codes
          // onto the ARM "lt/ge/gt/le" family so the branch matches
          // x86 UCOMISD semantics (where CF=1 means lhs<rhs ∨ unordered,
          // i.e., ARM N!=V).
          ir::CondCode cc = op.cc;
          if (fp_flag_refs_.count(op.flags) > 0) {
            switch (cc) {
              case ir::CondCode::Cc:
              case ir::CondCode::Uge:
                cc = ir::CondCode::Sge;
                break;
              case ir::CondCode::Nc:
              case ir::CondCode::Ult:
                cc = ir::CondCode::Slt;
                break;
              case ir::CondCode::Ugt:
                cc = ir::CondCode::Sgt;
                break;
              case ir::CondCode::Ule:
                cc = ir::CondCode::Sle;
                break;
              default:
                break;  // Eq/Ne/etc. unchanged.
            }
          }
          emitter_.branch_cc(it_t->second, cc);
          emitter_.branch(it_f->second);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadVecReg>) {
          // Read CpuStateFrame::xmm[idx] into a fresh V scratch. The
          // base register is the pinned state pointer
          // (`backend::abi::kStatePtrReg` = x27); the offset comes
          // from the layout-stable `xmm_offset_bytes(idx)`.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadVecReg requires a result ref"};
          }
          if (op.xmm_index >= ir::kXmmCount) {
            return {false, LowerError::UnsupportedOp, "LoadVecReg: xmm_index out of range"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadVecReg"};
          }
          const std::int32_t off =
              static_cast<std::int32_t>(runtime::CpuStateFrame::xmm_offset_bytes(op.xmm_index));
          emitter_.vld1_q_offset(rd, arm64::Reg::X27, off);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreVecReg>) {
          if (op.xmm_index >= ir::kXmmCount) {
            return {false, LowerError::UnsupportedOp, "StoreVecReg: xmm_index out of range"};
          }
          Emitter::FpReg rv;
          if (!fp_reg_of(op.value, rv)) {
            return {false, LowerError::DanglingRef, "StoreVecReg.value"};
          }
          const std::int32_t off =
              static_cast<std::int32_t>(runtime::CpuStateFrame::xmm_offset_bytes(op.xmm_index));
          emitter_.vst1_q_offset(rv, arm64::Reg::X27, off);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadVecRegHi>) {
          // F2-IR-005 — read CpuStateFrame::ymm_hi[idx] (high 128 bits
          // of YMM) into a fresh V scratch.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadVecRegHi requires a result ref"};
          }
          if (op.ymm_index >= ir::kXmmCount) {
            return {false, LowerError::UnsupportedOp, "LoadVecRegHi: ymm_index out of range"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadVecRegHi"};
          }
          const std::int32_t off =
              static_cast<std::int32_t>(runtime::CpuStateFrame::ymm_hi_offset_bytes(op.ymm_index));
          emitter_.vld1_q_offset(rd, arm64::Reg::X27, off);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreVecRegHi>) {
          if (op.ymm_index >= ir::kXmmCount) {
            return {false, LowerError::UnsupportedOp, "StoreVecRegHi: ymm_index out of range"};
          }
          Emitter::FpReg rv;
          if (!fp_reg_of(op.value, rv)) {
            return {false, LowerError::DanglingRef, "StoreVecRegHi.value"};
          }
          const std::int32_t off =
              static_cast<std::int32_t>(runtime::CpuStateFrame::ymm_hi_offset_bytes(op.ymm_index));
          emitter_.vst1_q_offset(rv, arm64::Reg::X27, off);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecConstant>) {
          // F2-IR-001 lowering. Materialise a 128-bit immediate.
          // ARM64 has no single-instruction 128-bit immediate; we
          // load the two halves separately via fmov_imm (which
          // routes through vixl's literal pool) and INS the high
          // half into lane 1 of rd.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecConstant requires a result ref"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecConstant"};
          }
          emitter_.vec_const_128(rd, op.lo, op.hi);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecBinOp>) {
          // F2-IR-002/003 lowering. 128-bit SIMD integer ops.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecBinOp requires a result ref"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "VecBinOp.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "VecBinOp.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecBinOp"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          switch (op.op) {
            case ir::VecBinOpKind::Add:
              emitter_.vadd_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::Sub:
              emitter_.vsub_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::And:
              emitter_.vand_q(rd, rl, rr);
              break;
            case ir::VecBinOpKind::Or:
              emitter_.vorr_q(rd, rl, rr);
              break;
            case ir::VecBinOpKind::Xor:
              emitter_.veor_q(rd, rl, rr);
              break;
            case ir::VecBinOpKind::Mul:
              emitter_.vmul_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::SqAdd:
              emitter_.vsqadd_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::UqAdd:
              emitter_.vuqadd_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::SqSub:
              emitter_.vsqsub_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::UqSub:
              emitter_.vuqsub_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::UMin:
              emitter_.vumin_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::UMax:
              emitter_.vumax_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::SMin:
              emitter_.vsmin_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::SMax:
              emitter_.vsmax_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::SMulHi:
              emitter_.vmulhi_h8(rd, rl, rr, /*signed=*/true);
              break;
            case ir::VecBinOpKind::UMulHi:
              emitter_.vmulhi_h8(rd, rl, rr, /*signed=*/false);
              break;
            case ir::VecBinOpKind::UMul32To64:
              emitter_.vmul_u32_to_64(rd, rl, rr);
              break;
            case ir::VecBinOpKind::SadBw:
              emitter_.vsad_bw(rd, rl, rr);
              break;
            case ir::VecBinOpKind::PairAddInt:
              emitter_.vaddp_q(rd, rl, rr, lane);
              break;
            case ir::VecBinOpKind::PairSubInt:
              emitter_.vsubp_q(rd, rl, rr, lane);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecClMul>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecClMul requires a result ref"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "VecClMul.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "VecClMul.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecClMul"};
          }
          emitter_.vpclmulqdq(rd, rl, rr, op.lhs_high, op.rhs_high);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecF16Cvt>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecF16Cvt requires a result ref"};
          }
          return lower_vec_f16cvt(op, *s.result);
        } else if constexpr (std::is_same_v<T, ir::VecFpBinOp>) {
          // F2-IR-005. Packed-FP arithmetic — ADDPS/SUBPS/MULPS/DIVPS
          // (S4) and ADDPD/SUBPD/MULPD/DIVPD (D2).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpBinOp requires a result ref"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "VecFpBinOp.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "VecFpBinOp.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpBinOp"};
          }
          const Emitter::VecLane lane =
              op.size == ir::VecFpSize::S4 ? Emitter::VecLane::S4 : Emitter::VecLane::D2;
          switch (op.op) {
            case ir::VecFpBinOpKind::Add:
              emitter_.vfadd_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Sub:
              emitter_.vfsub_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Mul:
              emitter_.vfmul_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Div:
              emitter_.vfdiv_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Min:
              emitter_.vfmin_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Max:
              emitter_.vfmax_q(rd, rl, rr, lane);
              break;
            case ir::VecFpBinOpKind::Sqrt:
              emitter_.vfsqrt_q(rd, rr, lane);
              break;
            case ir::VecFpBinOpKind::HAdd:
              emitter_.vfaddp_q(rd, rl, rr, lane);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecUnpack>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecUnpack without result"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "VecUnpack.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "VecUnpack.rhs"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecUnpack"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          if (op.is_high)
            emitter_.vzip2_q(rd, rl, rr, lane);
          else
            emitter_.vzip1_q(rd, rl, rr, lane);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecShiftImm>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecShiftImm without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn))
            return {false, LowerError::DanglingRef, "VecShiftImm.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecShiftImm"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          switch (op.kind) {
            case ir::VecShiftKind::ShiftL:
              emitter_.vshl_imm_q(rd, rn, op.count, lane);
              break;
            case ir::VecShiftKind::LogicalShr:
              emitter_.vushr_imm_q(rd, rn, op.count, lane);
              break;
            case ir::VecShiftKind::ArithShr:
              emitter_.vsshr_imm_q(rd, rn, op.count, lane);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecShuffle2Src>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecShuffle2Src without result"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "VecShuffle2Src.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "VecShuffle2Src.rhs"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecShuffle2Src"};
          }
          if (op.is_pd)
            emitter_.vshuffle_2src_d2(rd, rl, rr, op.control);
          else
            emitter_.vshuffle_2src_s4(rd, rl, rr, op.control);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecInsertLane>) {
          // F2-IR-022. Lane insert from a GPR.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecInsertLane without result"};
          }
          Emitter::FpReg rl;
          if (!fp_reg_of(op.lhs_xmm, rl))
            return {false, LowerError::DanglingRef, "VecInsertLane.lhs"};
          arm64::Reg rv;
          if (!reg_of(op.value, rv))
            return {false, LowerError::DanglingRef, "VecInsertLane.value"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecInsertLane"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          emitter_.vins_lane_from_w(rd, rl, op.lane_idx, rv, lane);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecPshufb>) {
          // F2-IR-036 SSSE3 PSHUFB.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecPshufb without result"};
          }
          Emitter::FpReg rn, rm;
          if (!fp_reg_of(op.src, rn))
            return {false, LowerError::DanglingRef, "VecPshufb.src"};
          if (!fp_reg_of(op.mask, rm))
            return {false, LowerError::DanglingRef, "VecPshufb.mask"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecPshufb"};
          }
          emitter_.vpshufb(rd, rn, rm);
          return {};
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsPtest>) {
          // F2-IR-047 PTEST.
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "WriteFlagsPtest.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "WriteFlagsPtest.rhs"};
          arm64::Reg w_tmp;
          if (!allocate_temporary(w_tmp)) {
            return {false, LowerError::OutOfScratchRegs, "PTEST tmp"};
          }
          emitter_.vptest(rl, rr, w_tmp);
          if (s.result.has_value()) {
            flag_refs_.insert(*s.result);
            // Integer-source mapping: ReadFlag(Carry) → cset cc → returns 1 when ARM C=0.
            // Our PTEST builds NZCV with ARM C = NOT is_zero_b, so cset cc → is_zero_b = x86 CF. ✓
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsPtestYmm>) {
          // F2-IR-049 VPTEST ymm. Same flag semantics as PTEST xmm
          // applied to the lo+hi 128-bit pair (see ir.hpp).
          Emitter::FpReg r_ll, r_lr, r_hl, r_hr;
          if (!fp_reg_of(op.lo_lhs, r_ll))
            return {false, LowerError::DanglingRef, "WriteFlagsPtestYmm.lo_lhs"};
          if (!fp_reg_of(op.lo_rhs, r_lr))
            return {false, LowerError::DanglingRef, "WriteFlagsPtestYmm.lo_rhs"};
          if (!fp_reg_of(op.hi_lhs, r_hl))
            return {false, LowerError::DanglingRef, "WriteFlagsPtestYmm.hi_lhs"};
          if (!fp_reg_of(op.hi_rhs, r_hr))
            return {false, LowerError::DanglingRef, "WriteFlagsPtestYmm.hi_rhs"};
          arm64::Reg w_tmp;
          if (!allocate_temporary(w_tmp)) {
            return {false, LowerError::OutOfScratchRegs, "VPTEST_ymm tmp"};
          }
          emitter_.vptest_ymm(r_ll, r_lr, r_hl, r_hr, w_tmp);
          if (s.result.has_value()) {
            flag_refs_.insert(*s.result);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecTbl2>) {
          // F2-IR-051 lane-crossing byte permute from a 256-bit
          // source pair, controlled by a runtime byte-index vector.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecTbl2 without result"};
          }
          Emitter::FpReg r_lo, r_hi, r_idx;
          if (!fp_reg_of(op.src_lo, r_lo))
            return {false, LowerError::DanglingRef, "VecTbl2.src_lo"};
          if (!fp_reg_of(op.src_hi, r_hi))
            return {false, LowerError::DanglingRef, "VecTbl2.src_hi"};
          if (!fp_reg_of(op.idx, r_idx))
            return {false, LowerError::DanglingRef, "VecTbl2.idx"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecTbl2"};
          }
          emitter_.vtbl2_q(rd, r_lo, r_hi, r_idx);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecGather>) {
          // F2-IR-059 gather family. Per lane (geometry from the
          // op's lane descriptor): if the mask lane's MSB is set,
          // load elem-width bits from
          // base + (sx64(index) << scale_shift) and insert into the
          // result; otherwise the lane keeps prev's value. Masked-off
          // lanes must not touch memory (their address may be
          // invalid by design), so each load hides behind a cbz.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecGather without result"};
          }
          arm64::Reg r_base;
          if (!reg_of(op.base, r_base))
            return {false, LowerError::DanglingRef, "VecGather.base"};
          Emitter::FpReg r_idx, r_mask, r_prev;
          if (!fp_reg_of(op.index, r_idx))
            return {false, LowerError::DanglingRef, "VecGather.index"};
          if (!fp_reg_of(op.mask, r_mask))
            return {false, LowerError::DanglingRef, "VecGather.mask"};
          if (!fp_reg_of(op.prev, r_prev))
            return {false, LowerError::DanglingRef, "VecGather.prev"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecGather"};
          }
          arm64::Reg t;
          if (!allocate_temporary(t)) {
            return {false, LowerError::OutOfScratchRegs, "VecGather tmp"};
          }
          emitter_.vmov_q(rd, r_prev);
          const bool e64 = op.elem_is64 != 0;
          const bool i64 = op.index_is64 != 0;
          const auto elane = e64 ? Emitter::VecLane::D2 : Emitter::VecLane::S4;
          const auto ilane = i64 ? Emitter::VecLane::D2 : Emitter::VecLane::S4;
          for (std::uint8_t lane = 0; lane < op.lane_count; ++lane) {
            const std::uint8_t dl = static_cast<std::uint8_t>(op.dest_lane_base + lane);
            const std::uint8_t xl = static_cast<std::uint8_t>(op.index_lane_base + lane);
            Emitter::Label skip = emitter_.create_label();
            // MSB of the elem-width mask lane decides participation.
            // vumov zero-extends into the X register, so a 64-bit
            // lsr by (lane width - 1) isolates exactly that bit.
            emitter_.vumov_w_from_lane(t, r_mask, dl, elane);
            emitter_.lsr_imm(t, t, e64 ? 63u : 31u);
            emitter_.cbz(t, skip);
            emitter_.vumov_w_from_lane(t, r_idx, xl, ilane);
            if (!i64)
              emitter_.sxtw(t, t);  // dword indices sign-extend
            emitter_.add_lsl(t, r_base, t, op.scale_shift);
            emitter_.load(t, t, e64 ? ir::OpSize::I64 : ir::OpSize::I32);
            emitter_.vins_lane_from_w(rd, rd, dl, t, elane);
            emitter_.bind(skip);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecAes>) {
          // F2-IR-055 AES round.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecAes without result"};
          }
          Emitter::FpReg r_src, r_key;
          if (!fp_reg_of(op.src, r_src))
            return {false, LowerError::DanglingRef, "VecAes.src"};
          if (!fp_reg_of(op.key, r_key))
            return {false, LowerError::DanglingRef, "VecAes.key"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecAes"};
          }
          emitter_.vaes(rd, r_src, r_key, op.kind);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecSha>) {
          // F2-IR-060 SHA-NI: thin dispatch onto the NEON-resident
          // emitter primitives (V29..V31 internal scratch).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecSha without result"};
          }
          Emitter::FpReg r_a, r_b, r_wk;
          if (!fp_reg_of(op.a, r_a))
            return {false, LowerError::DanglingRef, "VecSha.a"};
          if (!fp_reg_of(op.b, r_b))
            return {false, LowerError::DanglingRef, "VecSha.b"};
          if (!fp_reg_of(op.wk, r_wk))
            return {false, LowerError::DanglingRef, "VecSha.wk"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecSha"};
          }
          switch (op.kind) {
            case ir::VecShaKind::Sha1Rnds4:
              emitter_.vsha1_rnds4(rd, r_a, r_b, op.imm);
              break;
            case ir::VecShaKind::Sha1Nexte:
              emitter_.vsha1_nexte(rd, r_a, r_b);
              break;
            case ir::VecShaKind::Sha1Msg1:
              emitter_.vsha1_msg1(rd, r_a, r_b);
              break;
            case ir::VecShaKind::Sha1Msg2:
              emitter_.vsha1_msg2(rd, r_a, r_b);
              break;
            case ir::VecShaKind::Sha256Rnds2:
              emitter_.vsha256_rnds2(rd, r_a, r_b, r_wk);
              break;
            case ir::VecShaKind::Sha256Msg1:
              emitter_.vsha256_msg1(rd, r_a, r_b);
              break;
            case ir::VecShaKind::Sha256Msg2:
              emitter_.vsha256_msg2(rd, r_a, r_b);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecAesKeygenAssist>) {
          // F2-IR-058 AESKEYGENASSIST key-schedule helper.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecAesKeygenAssist without result"};
          }
          Emitter::FpReg r_src;
          if (!fp_reg_of(op.src, r_src)) {
            return {false, LowerError::DanglingRef, "VecAesKeygenAssist.src"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecAesKeygenAssist"};
          }
          emitter_.vaes_keygenassist(rd, r_src, op.rcon);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Bswap>) {
          // F2-IR-056 byte reverse — maps to ARM64 REV / REV16
          // depending on size. I8 is a no-op (single byte has no
          // byte order to reverse).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Bswap without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn))
            return {false, LowerError::DanglingRef, "Bswap.value"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Bswap"};
          }
          emitter_.bswap(rd, rn, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Crc32c>) {
          // F2-IR-057 CRC32C — direct ARM64 mapping.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Crc32c without result"};
          }
          arm64::Reg rcrc, rdata;
          if (!reg_of(op.crc, rcrc))
            return {false, LowerError::DanglingRef, "Crc32c.crc"};
          if (!reg_of(op.data, rdata))
            return {false, LowerError::DanglingRef, "Crc32c.data"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Crc32c"};
          }
          emitter_.crc32c(rd, rcrc, rdata, op.data_size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::X87Load>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "X87Load without result"};
          }
          if (op.st_index >= 8u) {
            return {false, LowerError::UnsupportedOp, "X87Load st_index"};
          }
          arm64::Reg rd, tos, slot;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "X87Load"};
          }
          if (!allocate_temporary(tos)) {
            return {false, LowerError::OutOfScratchRegs, "X87Load tos"};
          }
          if (!allocate_temporary(slot)) {
            return {false, LowerError::OutOfScratchRegs, "X87Load slot"};
          }
          emitter_.x87_load(arm64::Reg::X27, rd, tos, slot,
                            runtime::CpuStateFrame::x87_offset_bytes(0),
                            runtime::CpuStateFrame::kX87TosByteOffset, op.st_index);
          return {};
        } else if constexpr (std::is_same_v<T, ir::X87Store>) {
          if (op.st_index >= 8u) {
            return {false, LowerError::UnsupportedOp, "X87Store st_index"};
          }
          arm64::Reg value, tos, slot;
          if (!reg_of(op.value, value)) {
            return {false, LowerError::DanglingRef, "X87Store.value"};
          }
          if (!allocate_temporary(tos)) {
            return {false, LowerError::OutOfScratchRegs, "X87Store tos"};
          }
          if (!allocate_temporary(slot)) {
            return {false, LowerError::OutOfScratchRegs, "X87Store slot"};
          }
          emitter_.x87_store(arm64::Reg::X27, value, tos, slot,
                             runtime::CpuStateFrame::x87_offset_bytes(0),
                             runtime::CpuStateFrame::kX87TosByteOffset, op.st_index);
          return {};
        } else if constexpr (std::is_same_v<T, ir::X87Push>) {
          arm64::Reg value, tos, slot;
          if (!reg_of(op.value, value)) {
            return {false, LowerError::DanglingRef, "X87Push.value"};
          }
          if (!allocate_temporary(tos)) {
            return {false, LowerError::OutOfScratchRegs, "X87Push tos"};
          }
          if (!allocate_temporary(slot)) {
            return {false, LowerError::OutOfScratchRegs, "X87Push slot"};
          }
          emitter_.x87_push(arm64::Reg::X27, value, tos, slot,
                            runtime::CpuStateFrame::x87_offset_bytes(0),
                            runtime::CpuStateFrame::kX87TosByteOffset);
          return {};
        } else if constexpr (std::is_same_v<T, ir::X87Pop>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "X87Pop without result"};
          }
          arm64::Reg rd, tos, slot;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "X87Pop"};
          }
          if (!allocate_temporary(tos)) {
            return {false, LowerError::OutOfScratchRegs, "X87Pop tos"};
          }
          if (!allocate_temporary(slot)) {
            return {false, LowerError::OutOfScratchRegs, "X87Pop slot"};
          }
          emitter_.x87_pop(arm64::Reg::X27, rd, tos, slot,
                           runtime::CpuStateFrame::x87_offset_bytes(0),
                           runtime::CpuStateFrame::kX87TosByteOffset);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecBlend>) {
          // F2-IR-046 PBLENDVB / BLENDVPS / BLENDVPD.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecBlend without result"};
          }
          Emitter::FpReg rdst, rsrc, rmask;
          if (!fp_reg_of(op.dst, rdst))
            return {false, LowerError::DanglingRef, "VecBlend.dst"};
          if (!fp_reg_of(op.src, rsrc))
            return {false, LowerError::DanglingRef, "VecBlend.src"};
          if (!fp_reg_of(op.mask, rmask))
            return {false, LowerError::DanglingRef, "VecBlend.mask"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecBlend"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          emitter_.vblend(rd, rdst, rsrc, rmask, lane);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Lzcnt>) {
          // F2-IR-045 LZCNT — direct ARM64 clz.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Lzcnt without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn))
            return {false, LowerError::DanglingRef, "Lzcnt.value"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Lzcnt"};
          }
          emitter_.clz_gpr(rd, rn, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Tzcnt>) {
          // F2-IR-045 TZCNT — rbit + clz.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Tzcnt without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn))
            return {false, LowerError::DanglingRef, "Tzcnt.value"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Tzcnt"};
          }
          emitter_.rbit_clz_gpr(rd, rn, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::Popcnt>) {
          // F2-IR-044 POPCNT.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "Popcnt without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn))
            return {false, LowerError::DanglingRef, "Popcnt.value"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "Popcnt"};
          }
          emitter_.popcnt_gpr(rd, rn, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsCountZero>) {
          // F2-IR-045 follow-up. LZCNT/TZCNT exact flag write:
          //   Z = (result == 0)
          //   C = (src == 0)   ; CF is SET when the source operand is zero
          // The preceding StoreReg has already materialised the
          // count result, so we can reuse the same refs here.
          arm64::Reg rsrc, rres;
          if (!reg_of(op.src, rsrc))
            return {false, LowerError::DanglingRef, "WriteFlagsCountZero.src"};
          if (!reg_of(op.result, rres))
            return {false, LowerError::DanglingRef, "WriteFlagsCountZero.result"};
          arm64::Reg w_tmp;
          if (!allocate_temporary(w_tmp)) {
            return {false, LowerError::OutOfScratchRegs, "WriteFlagsCountZero tmp"};
          }
          emitter_.count_zero_flags(rres, rsrc, w_tmp);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecFpRound>) {
          // F2-IR-042 SSE4.1 ROUNDPS/PD/SS/SD.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpRound without result"};
          }
          Emitter::FpReg rs;
          if (!fp_reg_of(op.src, rs))
            return {false, LowerError::DanglingRef, "VecFpRound.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpRound"};
          }
          if (op.is_packed) {
            emitter_.vfrint_q(rd, rs, op.size, op.mode);
          } else {
            Emitter::FpReg rl;
            if (!fp_reg_of(op.lhs, rl))
              return {false, LowerError::DanglingRef, "VecFpRound.lhs"};
            emitter_.vfrint_scalar_with_upper(rd, rl, rs, op.size, op.mode);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecExtend>) {
          // F2-IR-041 SSE4.1 PMOVZX/PMOVSX.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecExtend without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn))
            return {false, LowerError::DanglingRef, "VecExtend.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecExtend"};
          }
          const auto map_lane = [](ir::VecLane l) {
            return l == ir::VecLane::B16  ? Emitter::VecLane::B16
                   : l == ir::VecLane::H8 ? Emitter::VecLane::H8
                   : l == ir::VecLane::S4 ? Emitter::VecLane::S4
                                          : Emitter::VecLane::D2;
          };
          emitter_.vextend(rd, rn, map_lane(op.narrow_lane), map_lane(op.wide_lane), op.is_signed);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecAlignr>) {
          // F2-IR-038 SSSE3 PALIGNR.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecAlignr without result"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "VecAlignr.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "VecAlignr.rhs"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecAlignr"};
          }
          emitter_.valignr(rd, rl, rr, op.count);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecAbs>) {
          // F2-IR-036 SSSE3 PABSB/W/D.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecAbs without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn))
            return {false, LowerError::DanglingRef, "VecAbs.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecAbs"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          emitter_.vabs_q(rd, rn, lane);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecFpCompare>) {
          // F2-IR-034. CMPxxPS/PD/SS/SD predicate compare.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpCompare without result"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "VecFpCompare.lhs"};
          if (!fp_reg_of(op.rhs, rr))
            return {false, LowerError::DanglingRef, "VecFpCompare.rhs"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpCompare"};
          }
          const std::uint8_t pred = static_cast<std::uint8_t>(op.pred);
          if (op.is_packed) {
            emitter_.vfcmp_packed(rd, rl, rr, op.size, pred);
          } else {
            emitter_.vfcmp_scalar_with_upper(rd, rl, rr, op.size, pred);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecMaskFp>) {
          // F2-IR-029. MOVMSKPS / MOVMSKPD.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecMaskFp without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src_xmm, rn))
            return {false, LowerError::DanglingRef, "VecMaskFp.src"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecMaskFp"};
          }
          arm64::Reg rt;
          if (!allocate_temporary(rt)) {
            return {false, LowerError::OutOfScratchRegs, "VecMaskFp temp"};
          }
          emitter_.vmask_fp(rd, rn, op.is_pd, rt);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecShuffleH4>) {
          // F2-IR-028. PSHUFLW / PSHUFHW.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecShuffleH4 without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn))
            return {false, LowerError::DanglingRef, "VecShuffleH4.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecShuffleH4"};
          }
          emitter_.vshuffle_h4(rd, rn, op.control, op.is_high);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecMaskMsb>) {
          // F2-IR-027. PMOVMSKB.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecMaskMsb without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src_xmm, rn))
            return {false, LowerError::DanglingRef, "VecMaskMsb.src"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecMaskMsb"};
          }
          emitter_.vmask_msb_b16(rd, rn);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecExtractLaneU>) {
          // F2-IR-022. Lane extract to a GPR (zero-extended).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecExtractLaneU without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src_xmm, rn))
            return {false, LowerError::DanglingRef, "VecExtractLaneU.src"};
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecExtractLaneU"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          emitter_.vumov_w_from_lane(rd, rn, op.lane_idx, lane);
          return {};
        } else if constexpr (std::is_same_v<T, ir::FpCvtScalar>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "FpCvtScalar without result"};
          }
          Emitter::FpReg rl, rs;
          if (!fp_reg_of(op.lhs, rl))
            return {false, LowerError::DanglingRef, "FpCvtScalar.lhs"};
          if (!fp_reg_of(op.src, rs))
            return {false, LowerError::DanglingRef, "FpCvtScalar.src"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "FpCvtScalar"};
          }
          emitter_.fcvt_scalar_with_upper(rd, rl, rs, op.src_size, op.dst_size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::IntToFpScalar>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "IntToFpScalar without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn)) {
            return {false, LowerError::DanglingRef, "IntToFpScalar.value"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "IntToFpScalar"};
          }
          emitter_.scvtf(rd, rn, op.int_size, op.fp_size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::FpToIntScalar>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "FpToIntScalar without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.value, rn)) {
            return {false, LowerError::DanglingRef, "FpToIntScalar.value"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "FpToIntScalar"};
          }
          emitter_.fcvtzs(rd, rn, op.fp_size, op.int_size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecShiftBytes>) {
          // F2-IR-014. PSLLDQ / PSRLDQ — whole-register byte shift.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecShiftBytes without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn)) {
            return {false, LowerError::DanglingRef, "VecShiftBytes.src"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecShiftBytes"};
          }
          if (op.is_left)
            emitter_.vshlb_imm_q(rd, rn, op.count);
          else
            emitter_.vshrb_imm_q(rd, rn, op.count);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecShuffle32x4>) {
          // F2-IR-010. PSHUFD: 4-way 32-bit lane permutation.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecShuffle32x4 without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.src, rn)) {
            return {false, LowerError::DanglingRef, "VecShuffle32x4.src"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecShuffle32x4"};
          }
          emitter_.vshuffle_s4(rd, rn, op.control);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecCmp>) {
          // F2-IR-009. cmeq / cmgt on V regs (lane-wise integer compare).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecCmp without result"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "VecCmp.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "VecCmp.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecCmp"};
          }
          const Emitter::VecLane lane = op.lane == ir::VecLane::B16  ? Emitter::VecLane::B16
                                        : op.lane == ir::VecLane::H8 ? Emitter::VecLane::H8
                                        : op.lane == ir::VecLane::S4 ? Emitter::VecLane::S4
                                                                     : Emitter::VecLane::D2;
          switch (op.kind) {
            case ir::VecCmpKind::Eq:
              emitter_.vcmeq_q(rd, rl, rr, lane);
              break;
            case ir::VecCmpKind::Gt:
              emitter_.vcmgt_q(rd, rl, rr, lane);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::XmmFromGpr>) {
          // F2-IR-008. fmov d/s_rd, x/w_rn — moves GPR low into V_rd
          // and zero-extends the upper 96/64 bits (fmov on
          // S/D register encodings).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "XmmFromGpr without result"};
          }
          arm64::Reg rn;
          if (!reg_of(op.value, rn)) {
            return {false, LowerError::DanglingRef, "XmmFromGpr.value"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "XmmFromGpr"};
          }
          const ir::FpSize fp_sz = op.size == ir::OpSize::I32 ? ir::FpSize::F32 : ir::FpSize::F64;
          emitter_.fmov_v_from_x(rd, rn, fp_sz);
          return {};
        } else if constexpr (std::is_same_v<T, ir::GprFromXmm>) {
          // F2-IR-008. fmov w/x_rd, s/d_rn — copies V_rn low lane to
          // GPR with zero-extension when size is I32.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "GprFromXmm without result"};
          }
          Emitter::FpReg rn;
          if (!fp_reg_of(op.value, rn)) {
            return {false, LowerError::DanglingRef, "GprFromXmm.value"};
          }
          arm64::Reg rd;
          if (!allocate_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "GprFromXmm"};
          }
          const ir::FpSize fp_sz = op.size == ir::OpSize::I32 ? ir::FpSize::F32 : ir::FpSize::F64;
          emitter_.fmov_x_from_v(rd, rn, fp_sz);
          return {};
        } else if constexpr (std::is_same_v<T, ir::LoadVec>) {
          // F2-IR-007. 128-bit aligned/unaligned load from guest mem.
          // ARM64 `ldr Q, [Xn]` accepts both — alignment fault only on
          // strict-alignment systems we don't target.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "LoadVec without result"};
          }
          arm64::Reg raddr;
          if (!reg_of(op.addr, raddr)) {
            return {false, LowerError::DanglingRef, "LoadVec.addr"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "LoadVec"};
          }
          emitter_.vld1_q(rd, raddr);
          return {};
        } else if constexpr (std::is_same_v<T, ir::StoreVec>) {
          arm64::Reg raddr;
          if (!reg_of(op.addr, raddr)) {
            return {false, LowerError::DanglingRef, "StoreVec.addr"};
          }
          Emitter::FpReg rv;
          if (!fp_reg_of(op.value, rv)) {
            return {false, LowerError::DanglingRef, "StoreVec.value"};
          }
          emitter_.vst1_q(rv, raddr);
          return {};
        } else if constexpr (std::is_same_v<T, ir::PcmpStrIndex>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "PcmpStrIndex without result"};
          }
          return lower_pcmpstr_index(op, *s.result);
        } else if constexpr (std::is_same_v<T, ir::PcmpStrMask>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "PcmpStrMask without result"};
          }
          return lower_pcmpstr_mask(op, *s.result);
        } else if constexpr (std::is_same_v<T, ir::PcmpStrFlags>) {
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "PcmpStrFlags without result"};
          }
          return lower_pcmpstr_flags(op, *s.result);
        } else if constexpr (std::is_same_v<T, ir::VecFpScalarFma>) {
          // F2-IR-006 — scalar FMA. ARM64 has 4-operand scalar FMA
          // primitives; the emitter wraps them with upper-lane
          // preservation (copies bits from `scalar_upper` into rd
          // before INS-ing the FMA result into rd's low lane).
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpScalarFma requires a result ref"};
          }
          Emitter::FpReg ra, rb, rc, rupper;
          if (!fp_reg_of(op.a, ra))
            return {false, LowerError::DanglingRef, "VecFpScalarFma.a"};
          if (!fp_reg_of(op.b, rb))
            return {false, LowerError::DanglingRef, "VecFpScalarFma.b"};
          if (!fp_reg_of(op.c, rc))
            return {false, LowerError::DanglingRef, "VecFpScalarFma.c"};
          if (!fp_reg_of(op.scalar_upper, rupper))
            return {false, LowerError::DanglingRef, "VecFpScalarFma.scalar_upper"};
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpScalarFma"};
          }
          emitter_.vfma_scalar(rd, rupper, ra, rb, rc, op.size, op.neg_addend, op.neg_mul);
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecFpFma>) {
          // F2-IR-006 — fused multiply-add. ARM64 FMLA is destructive
          // (Vd += Vn*Vm); we materialise the addend into Vd first.
          //   (neg_addend=F, neg_mul=F): Vd = Va; FMLA Vd, Vb, Vc
          //   (neg_addend=F, neg_mul=T): Vd = Va; FMLS Vd, Vb, Vc
          //   (neg_addend=T, neg_mul=F): FNEG Vd, Va; FMLA Vd, Vb, Vc
          //   (neg_addend=T, neg_mul=T): FNEG Vd, Va; FMLS Vd, Vb, Vc
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpFma requires a result ref"};
          }
          Emitter::FpReg ra, rb, rc;
          if (!fp_reg_of(op.a, ra)) {
            return {false, LowerError::DanglingRef, "VecFpFma.a"};
          }
          if (!fp_reg_of(op.b, rb)) {
            return {false, LowerError::DanglingRef, "VecFpFma.b"};
          }
          if (!fp_reg_of(op.c, rc)) {
            return {false, LowerError::DanglingRef, "VecFpFma.c"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpFma"};
          }
          const Emitter::VecLane lane =
              op.size == ir::VecFpSize::S4 ? Emitter::VecLane::S4 : Emitter::VecLane::D2;
          if (op.neg_addend) {
            emitter_.vfneg_q(rd, ra, lane);
          } else {
            emitter_.vmov_q(rd, ra);
          }
          if (op.neg_mul) {
            emitter_.vfmls_q(rd, rb, rc, lane);
          } else {
            emitter_.vfmla_q(rd, rb, rc, lane);
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::VecFpScalarBinOp>) {
          // F2-IR-006. ADDSS/ADDSD style: low lane = scalar op,
          // upper lanes preserved from lhs.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "VecFpScalarBinOp requires a result ref"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "VecFpScalarBinOp.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "VecFpScalarBinOp.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "VecFpScalarBinOp"};
          }
          switch (op.op) {
            case ir::VecFpBinOpKind::Add:
              emitter_.vfadd_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Sub:
              emitter_.vfsub_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Mul:
              emitter_.vfmul_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Div:
              emitter_.vfdiv_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Min:
              emitter_.vfmin_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Max:
              emitter_.vfmax_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::Sqrt:
              emitter_.vfsqrt_scalar(rd, rl, rr, op.size);
              break;
            case ir::VecFpBinOpKind::HAdd:
              return {false, LowerError::UnsupportedOp, "HAdd is packed-only; no scalar form"};
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::FpConstant>) {
          // F1-BK-013. Materialise an FP constant in a scratch V reg.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "FpConstant requires a result ref"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "FpConstant"};
          }
          emitter_.fmov_imm(rd, op.bits, op.size);
          return {};
        } else if constexpr (std::is_same_v<T, ir::FpBinOp>) {
          // F1-BK-013. fadd / fsub / fmul / fdiv on a freshly-allocated
          // V scratch.
          if (!s.result.has_value()) {
            return {false, LowerError::DanglingRef, "FpBinOp requires a result ref"};
          }
          Emitter::FpReg rl, rr;
          if (!fp_reg_of(op.lhs, rl)) {
            return {false, LowerError::DanglingRef, "FpBinOp.lhs"};
          }
          if (!fp_reg_of(op.rhs, rr)) {
            return {false, LowerError::DanglingRef, "FpBinOp.rhs"};
          }
          Emitter::FpReg rd;
          if (!allocate_fp_scratch(*s.result, rd)) {
            return {false, LowerError::OutOfScratchRegs, "FpBinOp"};
          }
          switch (op.op) {
            case ir::FpBinOpKind::Add:
              emitter_.fadd(rd, rl, rr, op.size);
              break;
            case ir::FpBinOpKind::Sub:
              emitter_.fsub(rd, rl, rr, op.size);
              break;
            case ir::FpBinOpKind::Mul:
              emitter_.fmul(rd, rl, rr, op.size);
              break;
            case ir::FpBinOpKind::Div:
              emitter_.fdiv(rd, rl, rr, op.size);
              break;
          }
          return {};
        } else if constexpr (std::is_same_v<T, ir::Fence>) {
          emitter_.fence(op.kind);
          return {};
        } else if constexpr (std::is_same_v<T, ir::CallRel> || std::is_same_v<T, ir::CallReg> ||
                             std::is_same_v<T, ir::RetAdjusted>) {
          const arm64::Reg rsp_host = arm64::host_reg_for(ir::Gpr::Rsp);
          arm64::Reg tmp;
          arm64::Reg tmp2;
          if (!allocate_temporary(tmp) || !allocate_temporary(tmp2)) {
            return {false, LowerError::OutOfScratchRegs, "Call/Ret temporaries"};
          }

          if constexpr (std::is_same_v<T, ir::RetAdjusted>) {
            // target = [RSP]; RSP += 8 + pop_bytes; x0 = target.
            emitter_.load(tmp, rsp_host, ir::OpSize::I64);
            emitter_.mov_imm64(tmp2, 8ULL + op.pop_bytes);
            emitter_.add(rsp_host, rsp_host, tmp2);
            emitter_.mov_reg_reg(arm64::Reg::X0, tmp);
          } else {
            // CALL pushes return_guest_pc, then transfers to the
            // callee target through x0 for the dispatcher.
            emitter_.mov_imm64(tmp, 8ULL);
            emitter_.sub(rsp_host, rsp_host, tmp);
            emitter_.mov_imm64(tmp2, op.return_guest_pc);
            emitter_.store(tmp2, rsp_host, ir::OpSize::I64);

            if constexpr (std::is_same_v<T, ir::CallRel>) {
              emitter_.mov_imm64(arm64::Reg::X0, op.target_guest_pc);
            } else {
              arm64::Reg rt;
              if (!reg_of(op.target, rt)) {
                return {false, LowerError::DanglingRef, "CallReg.target"};
              }
              emitter_.mov_reg_reg(arm64::Reg::X0, rt);
            }
          }
          if (options_.emit_ret_on_terminator)
            emitter_.ret();
          return {};
        } else {
          // Compare, LoadMem, StoreMem, LoadMemTSO, StoreMemTSO are
          // already lowered above. Anything reaching here is genuinely
          // unsupported.
          return {false, LowerError::UnsupportedOp, "op not yet lowered"};
        }
      },
      s.op);
}

}  // namespace prisma::backend
