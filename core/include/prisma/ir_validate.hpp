// prisma/ir_validate.hpp — F1-IR-016 IR validator.
//
// Structural sanity-check for a statement list. This is not a pass: it
// never rewrites the IR. It returns the first problem it finds so
// callers (tests, fuzzer hooks, debug builds of the translator) can
// assert correctness before handing IR off to passes or the lowerer.
//
// Checks (MVP):
//   1. Every operand Ref references a statement earlier in the list
//      whose `result` is that same Ref (SSA-def-precedes-use).
//   2. Every statement's `result` is unique — no Ref is redefined.
//   3. Side-effecting ops (StoreReg/StoreMem*/Jump*/Return/CondJump*/
//      CmpFlags/Fence) have `result == nullopt`. Pure ops (Constant/LoadReg/
//      BinOp/Extend/Truncate/Compare/Select/LoadMem*) have
//      `result != nullopt`.
//
// Out of scope for MVP (future IR-015 work):
//   * Cross-ref size consistency (BinOp.size vs lhs.size vs rhs.size).
//   * CFG-level validation (block entry/exit coherence).
//
// The validator is O(n) in the number of statements and allocates one
// flat-hash set + one map. Cheap enough to run in debug builds on every
// translation.

#pragma once

#include <optional>
#include <string>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

enum class ValidationCode {
    UndefinedRef,        // an operand ref has no earlier def
    DuplicateResult,     // two stmts define the same ref
    ImpureHasResult,     // a store/jump/return has result != nullopt
    PureLacksResult,     // a Constant/BinOp/... has result == nullopt
};

struct ValidationError {
    ValidationCode code;
    std::size_t    stmt_index;  // first stmt that violates
    Ref            bad_ref;     // the offending ref (0 for code-only errors)
    std::string    message;     // human-readable detail
};

struct ValidationResult {
    bool                            ok{true};
    std::optional<ValidationError>  error;  // populated iff !ok
};

[[nodiscard]] ValidationResult validate(const std::vector<Stmt>& stmts);

}  // namespace prisma::ir
