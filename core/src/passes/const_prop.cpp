// core/src/passes/const_prop.cpp — constant propagation + folding.
//
// Includes:
//   <limits> for std::numeric_limits in the SDiv/SMod fast-paths.
//
// Single sweep over the input statement list. Maintains a ref→u64 map for
// every Ref that is known to be a constant. When a BinOp's two operands
// are both in that map, we replace the BinOp with a Constant whose value
// is mask_to_size(evalBinOp(op, a, b)) — matching the Lean spec's
// `evalBinOp` exactly (`ir-spec/PrismaIR/Semantics.lean`).
//
// Not yet in scope for this pass:
//   * Propagation across basic blocks (no CFG reasoning).
//   * Folding of LoadReg whose register has a known last-written constant
//     (requires a register-value analysis; future work).
//   * Algebraic identities (x + 0 = x, x ^ x = 0 with non-const x, etc.).
//   * Dead-code elimination of the now-unused original defs.

#include "prisma/passes.hpp"

#ifdef _MSC_VER
#include <intrin.h>
#endif

#include <limits>
#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

std::uint64_t bit_deposit(std::uint64_t src, std::uint64_t mask) noexcept {
    std::uint64_t out = 0;
    std::uint64_t bit = 1;
    while (mask != 0) {
        const std::uint64_t low = mask & (0 - mask);
        if ((src & bit) != 0) out |= low;
        mask &= mask - 1;
        bit <<= 1;
    }
    return out;
}

std::uint64_t bit_extract(std::uint64_t src, std::uint64_t mask) noexcept {
    std::uint64_t out = 0;
    std::uint64_t bit = 1;
    while (mask != 0) {
        const std::uint64_t low = mask & (0 - mask);
        if ((src & low) != 0) out |= bit;
        mask &= mask - 1;
        bit <<= 1;
    }
    return out;
}

// Portable 64-bit high-half multiply without relying on compiler extensions.
std::uint64_t mul_high_u64(std::uint64_t a, std::uint64_t b) noexcept {
    const std::uint64_t a_lo = static_cast<std::uint32_t>(a);
    const std::uint64_t a_hi = a >> 32;
    const std::uint64_t b_lo = static_cast<std::uint32_t>(b);
    const std::uint64_t b_hi = b >> 32;

    const std::uint64_t w0 = a_lo * b_lo;
    const std::uint64_t t = a_hi * b_lo + (w0 >> 32);
    const std::uint64_t w1 = a_lo * b_hi + static_cast<std::uint32_t>(t);
    return a_hi * b_hi + (t >> 32) + (w1 >> 32);
}

std::uint64_t mul_high_s64(std::int64_t a, std::int64_t b) noexcept {
    const std::uint64_t a_u = static_cast<std::uint64_t>(a);
    const std::uint64_t b_u = static_cast<std::uint64_t>(b);
    const std::uint64_t a_sign = static_cast<std::uint64_t>(a >> 63);
    const std::uint64_t b_sign = static_cast<std::uint64_t>(b >> 63);
    return mul_high_u64(a_u, b_u) - (a_sign & b_u) - (b_sign & a_u);
}

// Exact mirror of the Lean `evalBinOp` definition, with size-masking
// applied here so the producer of the Constant sees a canonical value.
std::uint64_t eval_binop(ir::BinOpKind op, std::uint64_t a, std::uint64_t b) noexcept {
    switch (op) {
        case ir::BinOpKind::Add: return a + b;
        case ir::BinOpKind::Sub: return a - b;
        case ir::BinOpKind::Mul: return a * b;
        case ir::BinOpKind::And: return a & b;
        case ir::BinOpKind::Or:  return a | b;
        case ir::BinOpKind::Xor: return a ^ b;
        case ir::BinOpKind::Shl: return a << (b & 0x3Fu);
        case ir::BinOpKind::Shr: return a >> (b & 0x3Fu);
        case ir::BinOpKind::Sar: {
            // Arithmetic right shift on 64-bit: sign-extend.
            const std::int64_t sa = static_cast<std::int64_t>(a);
            const std::uint64_t shift = b & 0x3Fu;
            return static_cast<std::uint64_t>(sa >> shift);
        }
        case ir::BinOpKind::Rol: {
            const std::uint64_t n = b & 0x3Fu;
            if (n == 0) return a;
            return (a << n) | (a >> (64 - n));
        }
        case ir::BinOpKind::Ror: {
            const std::uint64_t n = b & 0x3Fu;
            if (n == 0) return a;
            return (a >> n) | (a << (64 - n));
        }
        case ir::BinOpKind::Rcl: {
            const std::uint64_t n = b & 0x3Fu;
            if (n == 0) return a;
            return (a << n) | (a >> (64 - n));
        }
        case ir::BinOpKind::Rcr: {
            const std::uint64_t n = b & 0x3Fu;
            if (n == 0) return a;
            return (a >> n) | (a << (64 - n));
        }
        case ir::BinOpKind::UMulHi: {
            return mul_high_u64(a, b);
        }
        case ir::BinOpKind::SMulHi: {
            return mul_high_s64(static_cast<std::int64_t>(a),
                               static_cast<std::int64_t>(b));
        }
        case ir::BinOpKind::UDiv: {
            if (b == 0) return 0;  // ARM64 udiv returns 0 on /0.
            return a / b;
        }
        case ir::BinOpKind::SDiv: {
            const std::int64_t sa = static_cast<std::int64_t>(a);
            const std::int64_t sb = static_cast<std::int64_t>(b);
            if (sb == 0) return 0;
            // INT_MIN / -1 wraps in ARM64 sdiv (no trap); mirror that.
            if (sa == std::numeric_limits<std::int64_t>::min() && sb == -1) {
                return static_cast<std::uint64_t>(sa);
            }
            return static_cast<std::uint64_t>(sa / sb);
        }
        case ir::BinOpKind::UMod: {
            if (b == 0) return a;  // ARM64 mod via udiv+msub: r = a - 0*b = a.
            return a % b;
        }
        case ir::BinOpKind::SMod: {
            const std::int64_t sa = static_cast<std::int64_t>(a);
            const std::int64_t sb = static_cast<std::int64_t>(b);
            if (sb == 0) return static_cast<std::uint64_t>(sa);
            if (sa == std::numeric_limits<std::int64_t>::min() && sb == -1) {
                return 0;
            }
            return static_cast<std::uint64_t>(sa % sb);
        }
        case ir::BinOpKind::Pdep: return bit_deposit(a, b);
        case ir::BinOpKind::Pext: return bit_extract(a, b);
    }
    return 0;  // unreachable
}

// Sign-extend a value from `from_size` bits to 64 bits by replicating
// the top bit of the masked low bits up through the high bits. The
// algorithm:
//   1. Mask off the high bits we don't have.
//   2. If the top bit of `from_size` is set, OR in the high bits.
std::uint64_t sign_extend_from(std::uint64_t v, ir::OpSize from_size) noexcept {
    const std::uint64_t mask = ir::mask_to_size(~0ULL, from_size);
    const std::uint64_t low  = v & mask;
    const std::uint32_t bits = ir::bit_width(from_size);
    if (bits >= 64) return low;
    const std::uint64_t top_bit = 1ULL << (bits - 1);
    if ((low & top_bit) == 0) return low;          // positive — no fill needed
    return low | (~mask);                          // negative — fill upper bits
}

}  // namespace

std::vector<ir::Stmt>
constant_propagate(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    // Known-constant values indexed by the Ref they are bound to.
    std::unordered_map<ir::Ref, std::uint64_t> consts;

    for (const auto& s : stmts) {
        // By default we pass through unchanged. We only rewrite the op
        // when the Constant shortcut applies.
        bool rewritten = false;

        if (std::holds_alternative<ir::Constant>(s.op) && s.result.has_value()) {
            const auto& c = std::get<ir::Constant>(s.op);
            consts[*s.result] = ir::mask_to_size(c.value, c.size);
            // Preserve the Constant stmt itself so dead defs are still
            // identifiable for a later DCE pass.
            out.push_back(s);
            rewritten = true;
        } else if (std::holds_alternative<ir::BinOp>(s.op) && s.result.has_value()) {
            const auto& b = std::get<ir::BinOp>(s.op);
            const auto ia = consts.find(b.lhs);
            const auto ib = consts.find(b.rhs);
            if (ia != consts.end() && ib != consts.end()) {
                const std::uint64_t folded =
                    ir::mask_to_size(eval_binop(b.op, ia->second, ib->second), b.size);
                consts[*s.result] = folded;
                out.push_back(ir::Stmt{s.result, ir::Constant{folded, b.size}});
                rewritten = true;
            }
        } else if (std::holds_alternative<ir::Extend>(s.op) && s.result.has_value()) {
            // F1-PS-010. If the source ref is a known constant, fold:
            //   * signed   → sign-extend from `from_size` then mask to to_size
            //   * unsigned → zero-extend (just mask to the wider size)
            const auto& e = std::get<ir::Extend>(s.op);
            const auto src = consts.find(e.value);
            if (src != consts.end()) {
                std::uint64_t folded = e.is_signed
                    ? sign_extend_from(src->second, e.from_size)
                    : ir::mask_to_size(src->second, e.from_size);
                folded = ir::mask_to_size(folded, e.to_size);
                consts[*s.result] = folded;
                out.push_back(ir::Stmt{s.result, ir::Constant{folded, e.to_size}});
                rewritten = true;
            }
        } else if (std::holds_alternative<ir::Truncate>(s.op) && s.result.has_value()) {
            // F1-PS-010. Truncate of a known constant just masks down.
            const auto& t = std::get<ir::Truncate>(s.op);
            const auto src = consts.find(t.value);
            if (src != consts.end()) {
                const std::uint64_t folded = ir::mask_to_size(src->second, t.to_size);
                consts[*s.result] = folded;
                out.push_back(ir::Stmt{s.result, ir::Constant{folded, t.to_size}});
                rewritten = true;
            }
        }

        if (!rewritten) {
            out.push_back(s);
        }
    }

    return out;
}

}  // namespace prisma::passes
