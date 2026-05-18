// prisma/ir_cfg.hpp - Basic block construction helpers.
//
// The builder is the migration bridge from the current flat statement
// representation to Function/BasicBlock form. It preserves statement ops:
// guest-PC based branches stay as JumpRel/CondJumpRel, while block-indexed
// Jump/CondJump are already in CFG form and are copied through.

#pragma once

#include <cstddef>
#include <optional>
#include <span>
#include <string>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

enum class CfgBuildCode {
    EmptyInput,
    DuplicateGuestPc,
    TooManyBlocks,
};

struct CfgBuildError {
    CfgBuildCode code;
    std::size_t stmt_index;
    std::string message;
};

struct CfgBuildResult {
    bool ok{true};
    Function function{};
    std::optional<CfgBuildError> error;
};

[[nodiscard]] CfgBuildResult build_cfg(std::span<const Stmt> stmts);
[[nodiscard]] std::vector<Stmt> flatten(const Function& function);

}  // namespace prisma::ir
