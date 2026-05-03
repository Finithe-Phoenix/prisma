// core/src/ir/validate.cpp — implementation of F1-IR-016 validator.

#include "prisma/ir_validate.hpp"

#include <unordered_set>
#include <variant>

namespace prisma::ir {

namespace {

ValidationResult err(ValidationCode code,
                     std::size_t    stmt_index,
                     Ref            bad_ref,
                     std::string    message) {
    ValidationError e{code, stmt_index, bad_ref, std::move(message)};
    return {false, std::move(e)};
}

// Walk every Ref-valued field of `op` and invoke `visit(ref)`.
template <typename F>
void for_each_operand_ref(const Op& op, F&& visit) {
    std::visit([&](const auto& x) {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, BinOp>)      { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, Compare>)    { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, Select>)     { visit(x.true_value); visit(x.false_value); }
        else if constexpr (std::is_same_v<T, LoadMem>)    { visit(x.addr); }
        else if constexpr (std::is_same_v<T, StoreMem>)   { visit(x.addr); visit(x.value); }
        else if constexpr (std::is_same_v<T, LoadMemTSO>) { visit(x.addr); }
        else if constexpr (std::is_same_v<T, StoreMemTSO>){ visit(x.addr); visit(x.value); }
        else if constexpr (std::is_same_v<T, StoreReg>)   { visit(x.value); }
        else if constexpr (std::is_same_v<T, CmpFlags>)   { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, CondJump>)   { visit(x.cond); }
        else if constexpr (std::is_same_v<T, JumpReg>)    { visit(x.target); }
        else if constexpr (std::is_same_v<T, CallReg>)    { visit(x.target); }
        // Constant, LoadReg, Jump, JumpRel, CondJumpRel, Return,
        // CallRel, RetAdjusted, Cpuid, Syscall, Trap — no refs.
    }, op);
}

// Classify whether `op` is allowed to have a result ref.
// Returns true iff the op is pure (must produce a ref).
bool op_is_pure(const Op& op) {
    return std::visit([](const auto& x) {
        using T = std::decay_t<decltype(x)>;
        return std::is_same_v<T, Constant>
            || std::is_same_v<T, LoadReg>
            || std::is_same_v<T, LoadSegBase>
            || std::is_same_v<T, BinOp>
            || std::is_same_v<T, Compare>
            || std::is_same_v<T, Select>
            || std::is_same_v<T, LoadMem>
            || std::is_same_v<T, LoadMemTSO>;
    }, op);
}

}  // namespace

ValidationResult validate(const std::vector<Stmt>& stmts) {
    std::unordered_set<Ref> defs;
    defs.reserve(stmts.size());

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];

        // Rule 3/4: result presence must match op category.
        const bool pure = op_is_pure(s.op);
        if (pure && !s.result) {
            return err(ValidationCode::PureLacksResult, i, 0,
                       "pure op has no result ref");
        }
        if (!pure && s.result) {
            return err(ValidationCode::ImpureHasResult, i, *s.result,
                       "side-effecting op has a result ref");
        }

        // Rule 1: every operand ref must be defined by an earlier stmt.
        ValidationResult undef{true, std::nullopt};
        for_each_operand_ref(s.op, [&](Ref r) {
            if (undef.ok && defs.find(r) == defs.end()) {
                undef = err(ValidationCode::UndefinedRef, i, r,
                            "operand ref used before its def");
            }
        });
        if (!undef.ok) return undef;

        // Rule 2: result ref must be unique.
        if (s.result) {
            if (!defs.insert(*s.result).second) {
                return err(ValidationCode::DuplicateResult, i, *s.result,
                           "result ref already defined by an earlier stmt");
            }
        }
    }
    return {true, std::nullopt};
}

}  // namespace prisma::ir
