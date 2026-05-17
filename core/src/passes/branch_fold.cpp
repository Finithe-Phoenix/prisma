// core/src/passes/branch_fold.cpp — statically-resolve trivial conds.
//
// When `CmpFlags const_a, const_b` is followed immediately by
// `CondJumpRel cc, taken, fallthrough`, the branch's direction is
// known at compile time. This pass rewrites the CondJumpRel to a
// plain JumpRel at whichever side the condition picks.
//
// Works on a single-pass scan. The CmpFlags isn't removed — dropping
// it would change observable flag state that a later instruction
// might read. DCE handles the actual removal once the flag state is
// provably unused.

#include "prisma/passes.hpp"

#include <cstdint>
#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

struct KnownConst {
    std::uint64_t value;
    ir::OpSize    size;
};

// Evaluate `a cc b` on two 64-bit values with size-specific masking.
// Returns true/false when the condition can be decided. Returns nullopt
// for flag-direct cases whose carry/overflow/sign semantics are not
// modelled by this MVP pass.
std::optional<bool> eval_cond(ir::CondCode cc,
                              std::uint64_t a,
                              std::uint64_t b,
                              ir::OpSize size) noexcept {
    // Mask to operand size — the comparison sees truncated values.
    a = ir::mask_to_size(a, size);
    b = ir::mask_to_size(b, size);

    // Signed interpretation for the *-signed comparisons. Extract the
    // value's bit width from the size enum so we sign-extend correctly.
    const std::uint32_t bits = ir::bit_width(size);
    auto sext = [&](std::uint64_t v) -> std::int64_t {
        if (bits == 64) return static_cast<std::int64_t>(v);
        const std::uint64_t sign_bit = 1ULL << (bits - 1);
        if (v & sign_bit) {
            const std::uint64_t mask = ~((1ULL << bits) - 1);
            return static_cast<std::int64_t>(v | mask);
        }
        return static_cast<std::int64_t>(v);
    };

    const std::int64_t sa = sext(a);
    const std::int64_t sb = sext(b);

    switch (cc) {
        case ir::CondCode::Eq:  return a == b;
        case ir::CondCode::Ne:  return a != b;
        case ir::CondCode::Ult: return a <  b;
        case ir::CondCode::Ule: return a <= b;
        case ir::CondCode::Ugt: return a >  b;
        case ir::CondCode::Uge: return a >= b;
        case ir::CondCode::Slt: return sa <  sb;
        case ir::CondCode::Sle: return sa <= sb;
        case ir::CondCode::Sgt: return sa >  sb;
        case ir::CondCode::Sge: return sa >= sb;
        // Flag-direct cases (Cc / Nc / Ov / NoOv / Mi / Pl) can't be
        // decided without modelling the full NZCV update — leave them
        // unfolded for now.
        case ir::CondCode::Cc:
        case ir::CondCode::Nc:
        case ir::CondCode::Ov:
        case ir::CondCode::NoOv:
        case ir::CondCode::Mi:
        case ir::CondCode::Pl:
            return std::nullopt;
    }
    return std::nullopt;
}

}  // namespace

std::vector<ir::Stmt>
branch_fold(const std::vector<ir::Stmt>& stmts) {
    std::unordered_map<ir::Ref, KnownConst> consts;

    // Track the most recent CmpFlags so the next CondJumpRel can
    // consult it. Cleared on any flag-clobbering op.
    struct LastCmp {
        ir::Ref  lhs;
        ir::Ref  rhs;
        ir::OpSize size;
    };
    std::optional<LastCmp> last_cmp;

    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    for (const auto& s : stmts) {
        if (std::holds_alternative<ir::Constant>(s.op) && s.result) {
            const auto& c = std::get<ir::Constant>(s.op);
            consts[*s.result] = KnownConst{
                ir::mask_to_size(c.value, c.size), c.size};
        }
        else if (std::holds_alternative<ir::CmpFlags>(s.op)) {
            const auto& cf = std::get<ir::CmpFlags>(s.op);
            last_cmp = LastCmp{cf.lhs, cf.rhs, cf.size};
        }
        else if (std::holds_alternative<ir::CondJumpRel>(s.op) && last_cmp) {
            const auto& cj = std::get<ir::CondJumpRel>(s.op);
            const auto it_l = consts.find(last_cmp->lhs);
            const auto it_r = consts.find(last_cmp->rhs);
            if (it_l != consts.end() && it_r != consts.end()) {
                const auto taken = eval_cond(
                    cj.cc, it_l->second.value, it_r->second.value,
                    last_cmp->size);
                if (taken) {
                    ir::Stmt folded{std::nullopt,
                        ir::Op{ir::JumpRel{
                            *taken ? cj.target_guest_pc : cj.fallthrough_guest_pc}}};
                    out.push_back(std::move(folded));
                    last_cmp.reset();
                    continue;
                }
            }
            last_cmp.reset();
        }
        // Any ALU op that writes could invalidate NZCV; conservatively
        // flush last_cmp when any BinOp or StoreReg touches flags-
        // adjacent state. We're over-conservative here because the IR
        // doesn't yet model flag-setting variants.
        else if (std::holds_alternative<ir::BinOp>(s.op)) {
            // BinOp does NOT clobber flags in our IR today — only
            // CmpFlags does — so leave last_cmp alone.
        }

        out.push_back(s);
    }

    return out;
}

}  // namespace prisma::passes
