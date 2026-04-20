// core/src/passes/strength_reduce.cpp — strength reduction (F1-PS-011).
//
// Replaces expensive BinOps with cheaper primitives that compute the
// same value. MVP handles one pattern only:
//
//   Mul x, (1 << k)   →   Shl x, k            for k in 1..63
//
// Both directions are considered (constant left or right). The new
// Shl gets a freshly-allocated shift-count Constant; the old
// pow-of-two Constant is left for DCE to clean up.
//
// Signed/unsigned division-by-power-of-two strength reduction is
// intentionally NOT done yet — the rounding direction differs for
// signed division (we'd need a conditional rounding step) and we
// don't model signedness on BinOp size. That's future work.

#include "prisma/passes.hpp"

#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

struct KnownConst {
    std::uint64_t value;
    ir::OpSize    size;
};

// If v is a non-zero power of two, return its log2. Otherwise nullopt.
std::optional<std::uint64_t> log2_pow2(std::uint64_t v) noexcept {
    if (v == 0 || (v & (v - 1)) != 0) return std::nullopt;
    // count trailing zeros — v is a single bit.
    std::uint64_t k = 0;
    while ((v & 1ULL) == 0) { v >>= 1; ++k; }
    return k;
}

}  // namespace

std::vector<ir::Stmt>
strength_reduce(const std::vector<ir::Stmt>& stmts) {
    std::unordered_map<ir::Ref, KnownConst> consts;

    // Find the highest ref in use so we can mint a new one for the
    // shift-count Constants this pass creates.
    ir::Ref next_ref = 0;
    for (const auto& s : stmts) {
        if (s.result && *s.result >= next_ref) next_ref = *s.result + 1;
    }

    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    for (const auto& s : stmts) {
        // Learn Constants as we go.
        if (std::holds_alternative<ir::Constant>(s.op) && s.result) {
            const auto& c = std::get<ir::Constant>(s.op);
            consts[*s.result] = KnownConst{
                ir::mask_to_size(c.value, c.size), c.size};
        }

        if (std::holds_alternative<ir::BinOp>(s.op) && s.result) {
            const auto& b = std::get<ir::BinOp>(s.op);
            if (b.op == ir::BinOpKind::Mul) {
                // x * (1<<k)
                const auto it_r = consts.find(b.rhs);
                const auto it_l = consts.find(b.lhs);
                std::optional<std::uint64_t> k;
                ir::Ref shift_operand = 0;
                if (it_r != consts.end()) {
                    if ((k = log2_pow2(it_r->second.value))) {
                        shift_operand = b.lhs;
                    }
                } else if (it_l != consts.end()) {
                    if ((k = log2_pow2(it_l->second.value))) {
                        shift_operand = b.rhs;
                    }
                }

                if (k) {
                    // Emit a fresh Constant for the shift count, then
                    // rewrite this stmt's op to Shl.
                    const ir::Ref ref_k = next_ref++;
                    ir::Stmt k_stmt{ref_k,
                        ir::Constant{*k, b.size}};
                    out.push_back(std::move(k_stmt));
                    // Remember it so later iterations can see it.
                    consts[ref_k] = KnownConst{*k, b.size};

                    ir::Stmt rewritten{s.result,
                        ir::Op{ir::BinOp{
                            ir::BinOpKind::Shl, shift_operand, ref_k, b.size}}};
                    out.push_back(std::move(rewritten));
                    continue;
                }
            }
        }

        out.push_back(s);
    }

    return out;
}

}  // namespace prisma::passes
