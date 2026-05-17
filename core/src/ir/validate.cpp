// core/src/ir/validate.cpp — implementation of F1-IR-016 validator.

#include "prisma/ir_validate.hpp"

#include <optional>
#include <unordered_map>
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

// Classify whether `op` is allowed to have a result ref.
// Returns true iff the op is pure (must produce a ref).
bool op_is_pure(const Op& op) {
    return std::visit([](const auto& x) {
        using T = std::decay_t<decltype(x)>;
        return std::is_same_v<T, Constant>
            || std::is_same_v<T, LoadReg>
            || std::is_same_v<T, LoadSegBase>
            || std::is_same_v<T, BinOp>
            || std::is_same_v<T, Extend>
            || std::is_same_v<T, Truncate>
            || std::is_same_v<T, Compare>
            || std::is_same_v<T, Select>
            || std::is_same_v<T, LoadMem>
            || std::is_same_v<T, LoadMemTSO>;
    }, op);
}

[[nodiscard]] const char* size_name(OpSize size) {
    switch (size) {
        case OpSize::I8:  return "i8";
        case OpSize::I16: return "i16";
        case OpSize::I32: return "i32";
        case OpSize::I64: return "i64";
    }
    return "unknown";
}

[[nodiscard]] std::optional<OpSize> result_size_of(const Op& op) {
    return std::visit([](const auto& x) -> std::optional<OpSize> {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, Constant>)    return x.size;
        else if constexpr (std::is_same_v<T, LoadReg>)     return x.size;
        else if constexpr (std::is_same_v<T, LoadSegBase>) return OpSize::I64;
        else if constexpr (std::is_same_v<T, BinOp>)       return x.size;
        else if constexpr (std::is_same_v<T, Extend>)      return x.to_size;
        else if constexpr (std::is_same_v<T, Truncate>)    return x.to_size;
        // Compare materializes 0/1 in a general register today; tighten this
        // to Bool/I1 if the IR grows a boolean size.
        else if constexpr (std::is_same_v<T, Compare>)     return OpSize::I64;
        else if constexpr (std::is_same_v<T, Select>)      return x.size;
        else if constexpr (std::is_same_v<T, LoadMem>)     return x.size;
        else if constexpr (std::is_same_v<T, LoadMemTSO>)  return x.size;
        else return std::nullopt;
    }, op);
}

using RefSizes = std::unordered_map<Ref, OpSize>;

[[nodiscard]] ValidationResult check_ref_size(const RefSizes& defs,
                                              std::size_t     stmt_index,
                                              Ref             ref,
                                              OpSize          expected,
                                              const char*     context) {
    const auto it = defs.find(ref);
    if (it == defs.end()) {
        return err(ValidationCode::UndefinedRef, stmt_index, ref,
                   "operand ref used before its def");
    }
    if (it->second != expected) {
        return err(ValidationCode::SizeMismatch, stmt_index, ref,
                   std::string(context) + " expected " + size_name(expected)
                       + " but ref is " + size_name(it->second));
    }
    return {true, std::nullopt};
}

[[nodiscard]] ValidationResult check_ref_defined(const RefSizes& defs,
                                                 std::size_t     stmt_index,
                                                 Ref             ref) {
    const auto it = defs.find(ref);
    if (it == defs.end()) {
        return err(ValidationCode::UndefinedRef, stmt_index, ref,
                   "operand ref used before its def");
    }
    return {true, std::nullopt};
}

[[nodiscard]] ValidationResult check_ref_min_size(const RefSizes& defs,
                                                  std::size_t     stmt_index,
                                                  Ref             ref,
                                                  OpSize          minimum,
                                                  const char*     context) {
    const auto it = defs.find(ref);
    if (it == defs.end()) {
        return err(ValidationCode::UndefinedRef, stmt_index, ref,
                   "operand ref used before its def");
    }
    if (bit_width(it->second) < bit_width(minimum)) {
        return err(ValidationCode::SizeMismatch, stmt_index, ref,
                   std::string(context) + " expected at least "
                       + size_name(minimum) + " but ref is " + size_name(it->second));
    }
    return {true, std::nullopt};
}

[[nodiscard]] bool is_shift_or_rotate(BinOpKind op) {
    return op == BinOpKind::Shl
        || op == BinOpKind::Shr
        || op == BinOpKind::Sar
        || op == BinOpKind::Rol
        || op == BinOpKind::Ror
        || op == BinOpKind::Rcl
        || op == BinOpKind::Rcr;
}

[[nodiscard]] ValidationResult validate_operand_sizes(const Op&      op,
                                                      const RefSizes& defs,
                                                      std::size_t     stmt_index) {
    return std::visit([&](const auto& x) -> ValidationResult {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, BinOp>) {
            if (auto r = check_ref_size(defs, stmt_index, x.lhs, x.size, "BinOp.lhs"); !r.ok) {
                return r;
            }
            if (is_shift_or_rotate(x.op)) {
                return check_ref_defined(defs, stmt_index, x.rhs);
            }
            return check_ref_size(defs, stmt_index, x.rhs, x.size, "BinOp.rhs");
        } else if constexpr (std::is_same_v<T, Extend>) {
            return check_ref_min_size(defs, stmt_index, x.value, x.from_size, "Extend.value");
        } else if constexpr (std::is_same_v<T, Truncate>) {
            return check_ref_min_size(defs, stmt_index, x.value, x.to_size, "Truncate.value");
        } else if constexpr (std::is_same_v<T, Compare>) {
            if (auto r = check_ref_size(defs, stmt_index, x.lhs, x.size, "Compare.lhs"); !r.ok) {
                return r;
            }
            return check_ref_size(defs, stmt_index, x.rhs, x.size, "Compare.rhs");
        } else if constexpr (std::is_same_v<T, Select>) {
            if (auto r = check_ref_size(defs, stmt_index, x.true_value, x.size,
                                        "Select.true_value"); !r.ok) {
                return r;
            }
            return check_ref_size(defs, stmt_index, x.false_value, x.size,
                                  "Select.false_value");
        } else if constexpr (std::is_same_v<T, LoadMem>) {
            return check_ref_size(defs, stmt_index, x.addr, OpSize::I64, "LoadMem.addr");
        } else if constexpr (std::is_same_v<T, StoreMem>) {
            if (auto r = check_ref_size(defs, stmt_index, x.addr, OpSize::I64,
                                        "StoreMem.addr"); !r.ok) {
                return r;
            }
            return check_ref_size(defs, stmt_index, x.value, x.size, "StoreMem.value");
        } else if constexpr (std::is_same_v<T, LoadMemTSO>) {
            return check_ref_size(defs, stmt_index, x.addr, OpSize::I64, "LoadMemTSO.addr");
        } else if constexpr (std::is_same_v<T, StoreMemTSO>) {
            if (auto r = check_ref_size(defs, stmt_index, x.addr, OpSize::I64,
                                        "StoreMemTSO.addr"); !r.ok) {
                return r;
            }
            return check_ref_size(defs, stmt_index, x.value, x.size, "StoreMemTSO.value");
        } else if constexpr (std::is_same_v<T, StoreReg>) {
            return check_ref_min_size(defs, stmt_index, x.value, x.size, "StoreReg.value");
        } else if constexpr (std::is_same_v<T, CmpFlags>) {
            if (auto r = check_ref_size(defs, stmt_index, x.lhs, x.size, "CmpFlags.lhs"); !r.ok) {
                return r;
            }
            return check_ref_size(defs, stmt_index, x.rhs, x.size, "CmpFlags.rhs");
        } else if constexpr (std::is_same_v<T, CondJump>) {
            return check_ref_size(defs, stmt_index, x.cond, OpSize::I64, "CondJump.cond");
        } else if constexpr (std::is_same_v<T, JumpReg>) {
            return check_ref_size(defs, stmt_index, x.target, OpSize::I64, "JumpReg.target");
        } else if constexpr (std::is_same_v<T, CallReg>) {
            return check_ref_size(defs, stmt_index, x.target, OpSize::I64, "CallReg.target");
        } else {
            return {true, std::nullopt};
        }
    }, op);
}

}  // namespace

ValidationResult validate(const std::vector<Stmt>& stmts) {
    RefSizes defs;
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

        // Rules 1 and 4: every operand ref must be defined by an earlier stmt
        // and consumed at a compatible size.
        if (auto r = validate_operand_sizes(s.op, defs, i); !r.ok) {
            return r;
        }

        // Rule 2: result ref must be unique.
        if (s.result) {
            const auto result_size = result_size_of(s.op);
            if (!result_size) {
                return err(ValidationCode::ImpureHasResult, i, *s.result,
                           "side-effecting op has a result ref");
            }
            if (!defs.emplace(*s.result, *result_size).second) {
                return err(ValidationCode::DuplicateResult, i, *s.result,
                           "result ref already defined by an earlier stmt");
            }
        }
    }
    return {true, std::nullopt};
}

}  // namespace prisma::ir
