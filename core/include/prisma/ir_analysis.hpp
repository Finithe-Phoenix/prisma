// prisma/ir_analysis.hpp - CFG traversal and dominator utilities.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

enum class CfgAnalysisCode {
    DuplicateBlockId,
    EntryNotFound,
    InvalidBlockTarget,
    DuplicateGuestPc,
};

struct CfgAnalysisError {
    CfgAnalysisCode code;
    std::uint32_t block_id;
    std::uint32_t target_block;
    std::string message;
};

struct CfgGraph {
    std::uint32_t entry{0};
    std::vector<std::uint32_t> blocks;
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> successors;
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> predecessors;
};

struct CfgGraphResult {
    bool ok{true};
    CfgGraph graph{};
    std::optional<CfgAnalysisError> error;
};

struct BlockOrderResult {
    bool ok{true};
    std::vector<std::uint32_t> order;
    std::optional<CfgAnalysisError> error;
};

struct DominatorTree {
    std::uint32_t entry{0};
    std::vector<std::uint32_t> reachable;
    std::unordered_map<std::uint32_t, std::optional<std::uint32_t>> immediate_dominators;
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> children_by_block;

    [[nodiscard]] bool is_reachable(std::uint32_t block_id) const;
    [[nodiscard]] std::optional<std::uint32_t>
    immediate_dominator(std::uint32_t block_id) const;
    [[nodiscard]] std::vector<std::uint32_t> children(std::uint32_t block_id) const;
    [[nodiscard]] bool dominates(std::uint32_t dominator,
                                 std::uint32_t block_id) const;
};

struct DominatorResult {
    bool ok{true};
    DominatorTree tree{};
    std::optional<CfgAnalysisError> error;
};

struct BackEdge {
    std::uint32_t from;
    std::uint32_t to;
};

struct NaturalLoop {
    std::uint32_t header;
    std::vector<std::uint32_t> latches;
    std::vector<std::uint32_t> blocks;
};

struct LoopAnalysis {
    std::vector<BackEdge> back_edges;
    std::vector<NaturalLoop> loops;
};

struct LoopAnalysisResult {
    bool ok{true};
    LoopAnalysis analysis{};
    std::optional<CfgAnalysisError> error;
};

[[nodiscard]] CfgGraphResult build_cfg_graph(const Function& function);
[[nodiscard]] BlockOrderResult postorder(const Function& function);
[[nodiscard]] BlockOrderResult reverse_postorder(const Function& function);
[[nodiscard]] DominatorResult compute_dominators(const Function& function);
[[nodiscard]] LoopAnalysisResult detect_natural_loops(const Function& function);

}  // namespace prisma::ir
