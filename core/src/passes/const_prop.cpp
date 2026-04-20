// core/src/passes/const_prop.cpp — constant propagation + folding.
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

#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

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
    }
    return 0;  // unreachable
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
        }

        if (!rewritten) {
            out.push_back(s);
        }
    }

    return out;
}

}  // namespace prisma::passes
