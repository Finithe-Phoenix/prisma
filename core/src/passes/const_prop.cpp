// core/src/passes/const_prop.cpp — constant propagation + folding.
//
// Single sweep over the input statement list. Maintains a ref→u64 map for
// every Ref that is known to be a constant. When a BinOp's two operands
// are both in that map, we replace the BinOp with a Constant whose value
// is mask_to_size(evalBinOp(op, a, b)). Extend and Truncate fold the same
// way when their source Ref is known.
//
// Not yet in scope for this pass:
//   * Propagation across basic blocks (no CFG reasoning).
//   * Folding of LoadReg whose register has a known last-written constant
//     (requires a register-value analysis; future work).
//   * Algebraic identities (x + 0 = x, x ^ x = 0 with non-const x, etc.).
//   * Dead-code elimination of the now-unused original defs.

#include "prisma/passes.hpp"

#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

struct KnownConst {
    std::uint64_t value;
    ir::OpSize size;
};

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
    }
    return 0;  // unreachable
}

std::uint64_t eval_extend(std::uint64_t value,
                          ir::OpSize from_size,
                          ir::OpSize to_size,
                          bool is_signed) noexcept {
    const std::uint64_t masked = ir::mask_to_size(value, from_size);
    if (!is_signed || from_size == ir::OpSize::I64) {
        return ir::mask_to_size(masked, to_size);
    }

    const std::uint32_t from_bits = ir::bit_width(from_size);
    const std::uint64_t sign_bit = 1ULL << (from_bits - 1u);
    if ((masked & sign_bit) == 0) {
        return ir::mask_to_size(masked, to_size);
    }

    const std::uint64_t extended = masked | (~0ULL << from_bits);
    return ir::mask_to_size(extended, to_size);
}

}  // namespace

std::vector<ir::Stmt>
constant_propagate(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    // Known-constant values indexed by the Ref they are bound to.
    std::unordered_map<ir::Ref, KnownConst> consts;

    for (const auto& s : stmts) {
        // By default we pass through unchanged. We only rewrite the op
        // when the Constant shortcut applies.
        bool rewritten = false;

        if (std::holds_alternative<ir::Constant>(s.op) && s.result.has_value()) {
            const auto& c = std::get<ir::Constant>(s.op);
            consts[*s.result] = KnownConst{ir::mask_to_size(c.value, c.size), c.size};
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
                    ir::mask_to_size(eval_binop(b.op, ia->second.value, ib->second.value),
                                     b.size);
                consts[*s.result] = KnownConst{folded, b.size};
                out.push_back(ir::Stmt{s.result, ir::Constant{folded, b.size}});
                rewritten = true;
            }
        } else if (std::holds_alternative<ir::Extend>(s.op) && s.result.has_value()) {
            const auto& e = std::get<ir::Extend>(s.op);
            const auto it = consts.find(e.value);
            if (it != consts.end()) {
                const std::uint64_t folded =
                    eval_extend(it->second.value, e.from_size, e.to_size, e.is_signed);
                consts[*s.result] = KnownConst{folded, e.to_size};
                out.push_back(ir::Stmt{s.result, ir::Constant{folded, e.to_size}});
                rewritten = true;
            }
        } else if (std::holds_alternative<ir::Truncate>(s.op) && s.result.has_value()) {
            const auto& t = std::get<ir::Truncate>(s.op);
            const auto it = consts.find(t.value);
            if (it != consts.end()) {
                const std::uint64_t folded = ir::mask_to_size(it->second.value, t.to_size);
                consts[*s.result] = KnownConst{folded, t.to_size};
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
