// core/src/ir/analysis.cpp - CFG traversal and dominator utilities.

#include "prisma/ir_analysis.hpp"

#include <algorithm>
#include <functional>
#include <iterator>
#include <optional>
#include <set>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <variant>

namespace prisma::ir {

namespace {

using BlockMap = std::unordered_map<std::uint32_t, const BasicBlock*>;
using GuestPcMap = std::unordered_map<std::uint64_t, std::uint32_t>;

[[nodiscard]] CfgAnalysisError make_error(CfgAnalysisCode code,
                                          std::uint32_t block_id,
                                          std::uint32_t target_block,
                                          std::string message) {
    return {code, block_id, target_block, std::move(message)};
}

[[nodiscard]] CfgGraphResult graph_error(CfgAnalysisError error) {
    return {false, CfgGraph{}, std::move(error)};
}

[[nodiscard]] BlockOrderResult order_error(CfgAnalysisError error) {
    return {false, {}, std::move(error)};
}

[[nodiscard]] DominatorResult dom_error(CfgAnalysisError error) {
    return {false, DominatorTree{}, std::move(error)};
}

[[nodiscard]] LoopAnalysisResult loop_error(CfgAnalysisError error) {
    return {false, LoopAnalysis{}, std::move(error)};
}

void add_unique(std::vector<std::uint32_t>& values, std::uint32_t value) {
    if (std::find(values.begin(), values.end(), value) == values.end()) {
        values.push_back(value);
    }
}

[[nodiscard]] std::optional<std::uint32_t>
next_block_id(const std::vector<std::uint32_t>& block_ids, std::uint32_t block_id) {
    const auto it = std::find(block_ids.begin(), block_ids.end(), block_id);
    if (it == block_ids.end() || std::next(it) == block_ids.end()) {
        return std::nullopt;
    }
    return *std::next(it);
}

[[nodiscard]] std::optional<CfgAnalysisError>
add_direct_successor(CfgGraph& graph,
                     const BlockMap& blocks,
                     std::uint32_t block_id,
                     std::uint32_t target) {
    if (blocks.find(target) == blocks.end()) {
        return make_error(CfgAnalysisCode::InvalidBlockTarget,
                          block_id, target,
                          "terminator targets a missing block id");
    }
    add_unique(graph.successors[block_id], target);
    add_unique(graph.predecessors[target], block_id);
    return std::nullopt;
}

void add_optional_successor(CfgGraph& graph,
                            std::uint32_t block_id,
                            std::optional<std::uint32_t> target) {
    if (!target) {
        return;
    }
    add_unique(graph.successors[block_id], *target);
    add_unique(graph.predecessors[*target], block_id);
}

[[nodiscard]] bool is_exit_terminator(const Op& op) noexcept {
    return std::holds_alternative<Return>(op)
        || std::holds_alternative<JumpReg>(op)
        || std::holds_alternative<RetAdjusted>(op)
        || std::holds_alternative<Syscall>(op)
        || std::holds_alternative<Trap>(op);
}

[[nodiscard]] std::optional<std::uint32_t>
guest_target(const GuestPcMap& guest_pcs, std::uint64_t pc) {
    const auto it = guest_pcs.find(pc);
    if (it == guest_pcs.end()) {
        return std::nullopt;
    }
    return it->second;
}

[[nodiscard]] std::optional<std::uint32_t>
processed_pred(const DominatorTree& tree, std::uint32_t entry, std::uint32_t pred) {
    if (pred == entry || tree.immediate_dominators.find(pred) != tree.immediate_dominators.end()) {
        return pred;
    }
    return std::nullopt;
}

[[nodiscard]] std::uint32_t intersect_idoms(const DominatorTree& tree,
                                            std::uint32_t lhs,
                                            std::uint32_t rhs) {
    std::unordered_set<std::uint32_t> ancestors;
    for (std::uint32_t current = lhs;;) {
        ancestors.insert(current);
        const auto it = tree.immediate_dominators.find(current);
        if (it == tree.immediate_dominators.end() || !it->second) {
            break;
        }
        current = *it->second;
    }

    for (std::uint32_t current = rhs;;) {
        if (ancestors.find(current) != ancestors.end()) {
            return current;
        }
        const auto it = tree.immediate_dominators.find(current);
        if (it == tree.immediate_dominators.end() || !it->second) {
            return current;
        }
        current = *it->second;
    }
}

void add_natural_loop_nodes(std::set<std::uint32_t>& nodes,
                            const CfgGraph& graph,
                            const DominatorTree& tree,
                            std::uint32_t header,
                            std::uint32_t latch) {
    std::vector<std::uint32_t> worklist{latch};
    nodes.insert(header);
    nodes.insert(latch);

    while (!worklist.empty()) {
        const auto current = worklist.back();
        worklist.pop_back();

        const auto pred_it = graph.predecessors.find(current);
        if (pred_it == graph.predecessors.end()) {
            continue;
        }

        for (const auto pred : pred_it->second) {
            if (pred == header || !tree.is_reachable(pred)) {
                continue;
            }
            if (nodes.insert(pred).second) {
                worklist.push_back(pred);
            }
        }
    }
}

}  // namespace

bool DominatorTree::is_reachable(std::uint32_t block_id) const {
    return std::find(reachable.begin(), reachable.end(), block_id) != reachable.end();
}

std::optional<std::uint32_t>
DominatorTree::immediate_dominator(std::uint32_t block_id) const {
    const auto it = immediate_dominators.find(block_id);
    if (it == immediate_dominators.end()) {
        return std::nullopt;
    }
    return it->second;
}

std::vector<std::uint32_t> DominatorTree::children(std::uint32_t block_id) const {
    const auto it = children_by_block.find(block_id);
    if (it == children_by_block.end()) {
        return {};
    }
    return it->second;
}

bool DominatorTree::dominates(std::uint32_t dominator,
                              std::uint32_t block_id) const {
    if (!is_reachable(dominator) || !is_reachable(block_id)) {
        return false;
    }

    for (std::uint32_t current = block_id;;) {
        if (current == dominator) {
            return true;
        }
        const auto it = immediate_dominators.find(current);
        if (it == immediate_dominators.end() || !it->second) {
            return false;
        }
        current = *it->second;
    }
}

CfgGraphResult build_cfg_graph(const Function& function) {
    BlockMap block_by_id;
    block_by_id.reserve(function.blocks.size());
    for (const auto& block : function.blocks) {
        if (!block_by_id.emplace(block.id, &block).second) {
            return graph_error(make_error(CfgAnalysisCode::DuplicateBlockId,
                                          block.id, block.id,
                                          "duplicate block id in Function"));
        }
    }

    if (block_by_id.find(function.entry) == block_by_id.end()) {
        return graph_error(make_error(CfgAnalysisCode::EntryNotFound,
                                      function.entry, function.entry,
                                      "entry block id not found"));
    }

    GuestPcMap guest_pcs;
    guest_pcs.reserve(function.blocks.size());
    for (const auto& block : function.blocks) {
        for (const auto& stmt : block.stmts) {
            if (const auto* pc = std::get_if<GuestPc>(&stmt.op)) {
                if (!guest_pcs.emplace(pc->pc, block.id).second) {
                    return graph_error(make_error(CfgAnalysisCode::DuplicateGuestPc,
                                                  block.id, block.id,
                                                  "duplicate GuestPc marker in Function"));
                }
            }
        }
    }

    CfgGraph graph;
    graph.entry = function.entry;
    graph.blocks.reserve(function.blocks.size());
    for (const auto& block : function.blocks) {
        graph.blocks.push_back(block.id);
    }
    for (const auto id : graph.blocks) {
        graph.successors.emplace(id, std::vector<std::uint32_t>{});
        graph.predecessors.emplace(id, std::vector<std::uint32_t>{});
    }

    for (const auto id : graph.blocks) {
        const auto& block = *block_by_id.at(id);
        const auto fallthrough = next_block_id(graph.blocks, id);

        if (block.stmts.empty()) {
            add_optional_successor(graph, id, fallthrough);
            continue;
        }

        const Op& op = block.stmts.back().op;
        if (const auto* jump = std::get_if<Jump>(&op)) {
            if (auto error = add_direct_successor(graph, block_by_id,
                                                  id, jump->target_block)) {
                return graph_error(std::move(*error));
            }
        } else if (const auto* cond = std::get_if<CondJump>(&op)) {
            if (auto error = add_direct_successor(graph, block_by_id,
                                                  id, cond->if_false)) {
                return graph_error(std::move(*error));
            }
            if (auto error = add_direct_successor(graph, block_by_id,
                                                  id, cond->if_true)) {
                return graph_error(std::move(*error));
            }
        } else if (const auto* jump_rel = std::get_if<JumpRel>(&op)) {
            add_optional_successor(graph, id,
                                   guest_target(guest_pcs, jump_rel->target_guest_pc));
        } else if (const auto* call_rel = std::get_if<CallRel>(&op)) {
            add_optional_successor(graph, id,
                                   guest_target(guest_pcs, call_rel->return_guest_pc));
        } else if (const auto* call_reg = std::get_if<CallReg>(&op)) {
            add_optional_successor(graph, id,
                                   guest_target(guest_pcs, call_reg->return_guest_pc));
        } else if (const auto* cond_rel = std::get_if<CondJumpRel>(&op)) {
            add_optional_successor(graph, id,
                                   guest_target(guest_pcs, cond_rel->fallthrough_guest_pc));
            add_optional_successor(graph, id,
                                   guest_target(guest_pcs, cond_rel->target_guest_pc));
        } else if (!is_exit_terminator(op)) {
            add_optional_successor(graph, id, fallthrough);
        }
    }

    return {true, std::move(graph), std::nullopt};
}

BlockOrderResult postorder(const Function& function) {
    auto graph_result = build_cfg_graph(function);
    if (!graph_result.ok) {
        return order_error(*graph_result.error);
    }
    const auto& graph = graph_result.graph;

    std::vector<std::uint32_t> order;
    std::unordered_set<std::uint32_t> visited;
    visited.reserve(graph.blocks.size());

    std::function<void(std::uint32_t)> dfs = [&](std::uint32_t block_id) {
        if (!visited.insert(block_id).second) {
            return;
        }
        const auto succ_it = graph.successors.find(block_id);
        if (succ_it != graph.successors.end()) {
            for (const auto successor : succ_it->second) {
                dfs(successor);
            }
        }
        order.push_back(block_id);
    };

    dfs(graph.entry);
    return {true, std::move(order), std::nullopt};
}

BlockOrderResult reverse_postorder(const Function& function) {
    auto order = postorder(function);
    if (!order.ok) {
        return order;
    }
    std::reverse(order.order.begin(), order.order.end());
    return order;
}

DominatorResult compute_dominators(const Function& function) {
    auto graph_result = build_cfg_graph(function);
    if (!graph_result.ok) {
        return dom_error(*graph_result.error);
    }
    const auto& graph = graph_result.graph;

    auto rpo_result = reverse_postorder(function);
    if (!rpo_result.ok) {
        return dom_error(*rpo_result.error);
    }
    const auto& rpo = rpo_result.order;

    DominatorTree tree;
    tree.entry = graph.entry;
    tree.reachable = rpo;
    tree.immediate_dominators.emplace(graph.entry, std::nullopt);

    bool changed = true;
    while (changed) {
        changed = false;
        for (const auto block_id : rpo) {
            if (block_id == graph.entry) {
                continue;
            }

            std::optional<std::uint32_t> new_idom;
            const auto pred_it = graph.predecessors.find(block_id);
            if (pred_it == graph.predecessors.end()) {
                continue;
            }

            for (const auto pred : pred_it->second) {
                auto candidate = processed_pred(tree, graph.entry, pred);
                if (!candidate) {
                    continue;
                }
                if (!new_idom) {
                    new_idom = *candidate;
                } else {
                    new_idom = intersect_idoms(tree, *new_idom, *candidate);
                }
            }

            if (new_idom) {
                const auto current = tree.immediate_dominators.find(block_id);
                if (current == tree.immediate_dominators.end() || current->second != new_idom) {
                    tree.immediate_dominators[block_id] = *new_idom;
                    changed = true;
                }
            }
        }
    }

    for (const auto& entry : tree.immediate_dominators) {
        const auto block_id = entry.first;
        const auto idom = entry.second;
        if (idom) {
            add_unique(tree.children_by_block[*idom], block_id);
        }
    }
    for (auto& entry : tree.children_by_block) {
        std::sort(entry.second.begin(), entry.second.end());
    }

    return {true, std::move(tree), std::nullopt};
}

LoopAnalysisResult detect_natural_loops(const Function& function) {
    auto graph_result = build_cfg_graph(function);
    if (!graph_result.ok) {
        return loop_error(*graph_result.error);
    }
    const auto& graph = graph_result.graph;

    auto dom_result = compute_dominators(function);
    if (!dom_result.ok) {
        return loop_error(*dom_result.error);
    }
    const auto& tree = dom_result.tree;

    struct LoopAccumulator {
        std::set<std::uint32_t> latches;
        std::set<std::uint32_t> blocks;
    };

    LoopAnalysis analysis;
    std::unordered_map<std::uint32_t, LoopAccumulator> by_header;

    for (const auto block_id : graph.blocks) {
        if (!tree.is_reachable(block_id)) {
            continue;
        }
        const auto succ_it = graph.successors.find(block_id);
        if (succ_it == graph.successors.end()) {
            continue;
        }

        for (const auto successor : succ_it->second) {
            if (!tree.is_reachable(successor) || !tree.dominates(successor, block_id)) {
                continue;
            }

            analysis.back_edges.push_back(BackEdge{block_id, successor});
            auto& loop = by_header[successor];
            loop.latches.insert(block_id);
            add_natural_loop_nodes(loop.blocks, graph, tree, successor, block_id);
        }
    }

    std::vector<std::uint32_t> headers;
    headers.reserve(by_header.size());
    for (const auto& entry : by_header) {
        headers.push_back(entry.first);
    }
    std::sort(headers.begin(), headers.end());

    analysis.loops.reserve(headers.size());
    for (const auto header : headers) {
        const auto& loop = by_header.at(header);
        NaturalLoop natural_loop;
        natural_loop.header = header;
        natural_loop.latches.assign(loop.latches.begin(), loop.latches.end());
        natural_loop.blocks.assign(loop.blocks.begin(), loop.blocks.end());
        analysis.loops.push_back(std::move(natural_loop));
    }

    return {true, std::move(analysis), std::nullopt};
}

}  // namespace prisma::ir
