// core/src/passes/algebraic.cpp — algebraic simplification.
//
// Identities that rewrite Stmt.op in place when one operand is a known
// constant with a special value. The pass walks the statement list once,
// tracks refs that bind to a known Constant, and rewrites matching
// BinOps to their identity form.

#include "prisma/passes.hpp"

#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

// Track refs whose value is a compile-time-known constant. This is the
// same shape const_prop uses. The two passes are complementary:
// const_prop folds two-constant BinOps; this one fires when only one
// side is constant with a special value.
struct KnownConst {
    std::uint64_t value;
    ir::OpSize    size;
};

// Canonical masked -1 per size.
constexpr std::uint64_t all_ones(ir::OpSize sz) noexcept {
    switch (sz) {
        case ir::OpSize::I8:  return 0xFFu;
        case ir::OpSize::I16: return 0xFFFFu;
        case ir::OpSize::I32: return 0xFFFF'FFFFULL;
        case ir::OpSize::I64: return 0xFFFF'FFFF'FFFF'FFFFULL;
    }
    return 0;
}

// Rewrite the BinOp if an identity applies. Returns the new op (which
// may be a Constant) or std::nullopt if no identity fires.
std::optional<ir::Op> try_simplify(
    const ir::BinOp& b,
    const std::unordered_map<ir::Ref, KnownConst>& consts) {

    const auto it_l = consts.find(b.lhs);
    const auto it_r = consts.find(b.rhs);
    const bool l_const = it_l != consts.end();
    const bool r_const = it_r != consts.end();
    const bool same_ref = b.lhs == b.rhs;

    using K = ir::BinOpKind;

    // Patterns where BOTH operands are known constants are handled by
    // constant_propagate. We deliberately do NOT fire here to keep
    // contracts disjoint.

    // x - x → 0    ;   x ^ x → 0
    if (same_ref) {
        if (b.op == K::Sub || b.op == K::Xor) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // x & x → x (becomes an identity we express as Or x,x → x; simpler
        // to just rewrite with a move via algebraic `or`. Skip for MVP.)
    }

    // ADD / OR / XOR / SUB / SHL / SHR / SAR / ROL / ROR / RCL / RCR:
    // right-side 0 is identity → result is lhs.
    // Express "result = lhs" as: `lhs | 0` pattern — actually simpler
    // approach: emit a move by reusing OR with itself and lhs. But the
    // cleanest thing is to rewrite to an alias via Constant: we can't
    // express a "copy" directly in our IR. So for these identities we
    // just synthesize the LHS ref's operation.
    //
    // Since we don't have a Copy op, we rewrite `x + 0` to a no-op form
    // by preserving the original. For MVP, just handle the pure
    // "result-becomes-a-constant" patterns and "result-becomes-zero"
    // patterns which we CAN express. The move-through cases (x + 0 → x)
    // are left for copy propagation to pick up — they don't save
    // instructions at the lowering level here.

    if (r_const) {
        const KnownConst& rhs = it_r->second;
        // x * 0 → 0
        if (b.op == K::Mul && rhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // x & 0 → 0
        if (b.op == K::And && rhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // x | -1 → -1
        if (b.op == K::Or && rhs.value == all_ones(b.size)) {
            return ir::Op{ir::Constant{all_ones(b.size), b.size}};
        }
        // F2-BK-007 — high half of x*0 = 0 (unsigned and signed).
        if ((b.op == K::UMulHi || b.op == K::SMulHi) && rhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // F2-BK-007 — high half of x*1 = 0 (unsigned). Signed only when
        // x is non-negative; we can't prove it without range info, so
        // skip the SMulHi case.
        if (b.op == K::UMulHi && rhs.value == 1) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // F2-BK-007 — x % 1 = 0 (both signed and unsigned).
        if ((b.op == K::UMod || b.op == K::SMod) && rhs.value == 1) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // x / 1 = x — would need a copy op; skip.
    }

    if (l_const) {
        const KnownConst& lhs = it_l->second;
        // 0 * x → 0
        if (b.op == K::Mul && lhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // 0 & x → 0
        if (b.op == K::And && lhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
        // -1 | x → -1
        if (b.op == K::Or && lhs.value == all_ones(b.size)) {
            return ir::Op{ir::Constant{all_ones(b.size), b.size}};
        }
        // F2-BK-007 — high half of 0*x = 0; 0/x = 0; 0%x = 0.
        if ((b.op == K::UMulHi || b.op == K::SMulHi ||
             b.op == K::UDiv  || b.op == K::SDiv  ||
             b.op == K::UMod  || b.op == K::SMod) && lhs.value == 0) {
            return ir::Op{ir::Constant{0, b.size}};
        }
    }

    return std::nullopt;
}

}  // namespace

std::vector<ir::Stmt>
algebraic_simplify(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    std::unordered_map<ir::Ref, KnownConst> consts;

    for (const auto& s : stmts) {
        ir::Stmt new_stmt = s;

        if (std::holds_alternative<ir::Constant>(s.op) && s.result) {
            const auto& c = std::get<ir::Constant>(s.op);
            consts[*s.result] = KnownConst{
                ir::mask_to_size(c.value, c.size), c.size};
        }
        else if (std::holds_alternative<ir::BinOp>(s.op) && s.result) {
            const auto& b = std::get<ir::BinOp>(s.op);
            if (auto simp = try_simplify(b, consts)) {
                new_stmt.op = *simp;
                // If the rewrite produced a Constant, learn it.
                if (std::holds_alternative<ir::Constant>(*simp)) {
                    const auto& c = std::get<ir::Constant>(*simp);
                    consts[*s.result] = KnownConst{
                        ir::mask_to_size(c.value, c.size), c.size};
                }
            }
        }

        out.push_back(std::move(new_stmt));
    }

    return out;
}

}  // namespace prisma::passes
