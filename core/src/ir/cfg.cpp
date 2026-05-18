// core/src/ir/cfg.cpp - flat IR to basic-block construction.

#include "prisma/ir_cfg.hpp"

#include <algorithm>
#include <cstdint>
#include <limits>
#include <optional>
#include <unordered_map>
#include <utility>
#include <variant>

namespace prisma::ir {

namespace {

using GuestPcMap = std::unordered_map<std::uint64_t, std::size_t>;

[[nodiscard]] CfgBuildResult err(CfgBuildCode code,
                                 std::size_t stmt_index,
                                 std::string message) {
    CfgBuildError e{code, stmt_index, std::move(message)};
    return {false, Function{}, std::move(e)};
}

[[nodiscard]] bool is_terminator(const Op& op) noexcept {
    return std::holds_alternative<Jump>(op)
        || std::holds_alternative<CondJump>(op)
        || std::holds_alternative<CondJumpFlags>(op)
        || std::holds_alternative<Return>(op)
        || std::holds_alternative<JumpReg>(op)
        || std::holds_alternative<JumpRel>(op)
        || std::holds_alternative<RetAdjusted>(op)
        || std::holds_alternative<Syscall>(op)
        || std::holds_alternative<Trap>(op)
        || std::holds_alternative<CondJumpRel>(op);
}

void add_boundary(std::vector<std::size_t>& boundaries,
                  std::size_t boundary,
                  std::size_t stmt_count) {
    if (boundary < stmt_count) {
        boundaries.push_back(boundary);
    }
}

[[nodiscard]] std::optional<std::size_t>
guest_pc_stmt_index(const GuestPcMap& guest_pcs, std::uint64_t pc) {
    const auto it = guest_pcs.find(pc);
    if (it == guest_pcs.end()) {
        return std::nullopt;
    }
    return it->second;
}

void add_guest_target_boundary(std::vector<std::size_t>& boundaries,
                               const GuestPcMap& guest_pcs,
                               std::uint64_t pc,
                               std::size_t stmt_count) {
    if (auto index = guest_pc_stmt_index(guest_pcs, pc)) {
        add_boundary(boundaries, *index, stmt_count);
    }
}

[[nodiscard]] std::vector<std::size_t> unique_sorted(std::vector<std::size_t> values) {
    std::sort(values.begin(), values.end());
    values.erase(std::unique(values.begin(), values.end()), values.end());
    return values;
}

}  // namespace

CfgBuildResult build_cfg(std::span<const Stmt> stmts) {
    if (stmts.empty()) {
        return err(CfgBuildCode::EmptyInput, 0u,
                   "cannot build a CFG from an empty statement list");
    }

    GuestPcMap guest_pcs;
    guest_pcs.reserve(stmts.size());
    for (std::size_t i = 0; i < stmts.size(); ++i) {
        if (const auto* pc = std::get_if<GuestPc>(&stmts[i].op)) {
            if (!guest_pcs.emplace(pc->pc, i).second) {
                return err(CfgBuildCode::DuplicateGuestPc, i,
                           "duplicate GuestPc marker makes CFG targets ambiguous");
            }
        }
    }

    std::vector<std::size_t> boundaries;
    boundaries.reserve(stmts.size());
    boundaries.push_back(0u);

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const Op& op = stmts[i].op;
        if (const auto* jump = std::get_if<JumpRel>(&op)) {
            add_guest_target_boundary(boundaries, guest_pcs,
                                      jump->target_guest_pc, stmts.size());
        } else if (const auto* cond = std::get_if<CondJumpRel>(&op)) {
            add_guest_target_boundary(boundaries, guest_pcs,
                                      cond->target_guest_pc, stmts.size());
            add_guest_target_boundary(boundaries, guest_pcs,
                                      cond->fallthrough_guest_pc, stmts.size());
        }

        if (is_terminator(op)) {
            add_boundary(boundaries, i + 1u, stmts.size());
        }
    }

    boundaries = unique_sorted(std::move(boundaries));
    if (boundaries.size() > static_cast<std::size_t>(std::numeric_limits<std::uint32_t>::max())) {
        return err(CfgBuildCode::TooManyBlocks, 0u,
                   "too many basic blocks for 32-bit block ids");
    }

    Function function;
    function.entry = 0u;
    function.blocks.reserve(boundaries.size());

    for (std::size_t block_index = 0; block_index < boundaries.size(); ++block_index) {
        const std::size_t start = boundaries[block_index];
        const std::size_t end = (block_index + 1u < boundaries.size())
            ? boundaries[block_index + 1u]
            : stmts.size();

        BasicBlock block;
        block.id = static_cast<std::uint32_t>(block_index);
        block.stmts.reserve(end - start);
        for (std::size_t stmt_index = start; stmt_index < end; ++stmt_index) {
            block.stmts.push_back(stmts[stmt_index]);
        }
        function.blocks.push_back(std::move(block));
    }

    return {true, std::move(function), std::nullopt};
}

std::vector<Stmt> flatten(const Function& function) {
    std::vector<const BasicBlock*> blocks;
    blocks.reserve(function.blocks.size());
    std::size_t stmt_count = 0;
    for (const auto& block : function.blocks) {
        blocks.push_back(&block);
        stmt_count += block.stmts.size();
    }

    std::sort(blocks.begin(), blocks.end(),
              [](const BasicBlock* lhs, const BasicBlock* rhs) {
                  return lhs->id < rhs->id;
              });

    std::vector<Stmt> out;
    out.reserve(stmt_count);
    for (const auto* block : blocks) {
        out.insert(out.end(), block->stmts.begin(), block->stmts.end());
    }
    return out;
}

}  // namespace prisma::ir
