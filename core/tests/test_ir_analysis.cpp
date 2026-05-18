// core/tests/test_ir_analysis.cpp - CFG traversal and dominator tests.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/ir_analysis.hpp"

#include <optional>
#include <vector>

using namespace prisma::ir;

namespace {

const std::vector<std::uint32_t>& successors_of(const CfgGraph& graph,
                                                std::uint32_t block_id) {
    const auto it = graph.successors.find(block_id);
    REQUIRE(it != graph.successors.end());
    return it->second;
}

const std::vector<std::uint32_t>& predecessors_of(const CfgGraph& graph,
                                                  std::uint32_t block_id) {
    const auto it = graph.predecessors.find(block_id);
    REQUIRE(it != graph.predecessors.end());
    return it->second;
}

Function diamond_function() {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{0u, Constant{1u, OpSize::I64}},
                        {std::nullopt, CondJump{0u, 2u, 1u}}}},
        BasicBlock{1u, {{std::nullopt, Jump{3u}}}},
        BasicBlock{2u, {{std::nullopt, Jump{3u}}}},
        BasicBlock{3u, {{std::nullopt, Return{}}}},
        BasicBlock{4u, {{std::nullopt, Return{}}}},
    };
    return function;
}

}  // namespace

TEST_CASE("IR analysis: graph derives direct and fallthrough successors") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{0u, Constant{1u, OpSize::I64}}}},
        BasicBlock{1u, {{std::nullopt, CondJump{0u, 3u, 2u}}}},
        BasicBlock{2u, {{std::nullopt, Return{}}}},
        BasicBlock{3u, {{std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    const auto& graph = graph_result.graph;
    REQUIRE(successors_of(graph, 0u) == std::vector<std::uint32_t>{1u});
    REQUIRE(successors_of(graph, 1u) == std::vector<std::uint32_t>{2u, 3u});
    REQUIRE(successors_of(graph, 2u).empty());
    REQUIRE(predecessors_of(graph, 2u) == std::vector<std::uint32_t>{1u});
}

TEST_CASE("IR analysis: fallthrough uses Function block order") {
    Function function;
    function.entry = 10u;
    function.blocks = {
        BasicBlock{10u, {{0u, Constant{1u, OpSize::I64}}}},
        BasicBlock{20u, {{std::nullopt, Return{}}}},
        BasicBlock{5u, {{std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    REQUIRE(graph_result.graph.blocks == std::vector<std::uint32_t>{10u, 20u, 5u});
    REQUIRE(successors_of(graph_result.graph, 10u) == std::vector<std::uint32_t>{20u});
    REQUIRE(successors_of(graph_result.graph, 20u).empty());
    REQUIRE(successors_of(graph_result.graph, 5u).empty());
}

TEST_CASE("IR analysis: graph derives local guest-PC branch targets") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, GuestPc{0x1000u}},
                        {std::nullopt, CondJumpRel{CondCode::Eq, 0x3000u, 0x2000u}}}},
        BasicBlock{1u, {{std::nullopt, GuestPc{0x2000u}},
                        {std::nullopt, Return{}}}},
        BasicBlock{2u, {{std::nullopt, GuestPc{0x3000u}},
                        {std::nullopt, JumpRel{0x2000u}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    const auto& graph = graph_result.graph;
    REQUIRE(successors_of(graph, 0u) == std::vector<std::uint32_t>{1u, 2u});
    REQUIRE(successors_of(graph, 1u).empty());
    REQUIRE(successors_of(graph, 2u) == std::vector<std::uint32_t>{1u});
    REQUIRE(predecessors_of(graph, 1u) == std::vector<std::uint32_t>{0u, 2u});
}

TEST_CASE("IR analysis: CallRel and CallReg use local return edges") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, CallRel{0x9000u, 0x2000u}}}},
        BasicBlock{1u, {{std::nullopt, GuestPc{0x2000u}},
                        {0u, Constant{0x3000u, OpSize::I64}},
                        {std::nullopt, CallReg{0u, 0x3000u}}}},
        BasicBlock{2u, {{std::nullopt, GuestPc{0x3000u}},
                        {std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    REQUIRE(successors_of(graph_result.graph, 0u) == std::vector<std::uint32_t>{1u});
    REQUIRE(successors_of(graph_result.graph, 1u) == std::vector<std::uint32_t>{2u});
    REQUIRE(successors_of(graph_result.graph, 2u).empty());
}

TEST_CASE("IR analysis: external guest-PC branches have no local successor") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, GuestPc{0x1000u}},
                        {std::nullopt, JumpRel{0xDEADu}}}},
        BasicBlock{1u, {{std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    REQUIRE(successors_of(graph_result.graph, 0u).empty());
    REQUIRE(predecessors_of(graph_result.graph, 1u).empty());
}

TEST_CASE("IR analysis: conditional self-targets are deduplicated") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{0u, Constant{1u, OpSize::I64}},
                        {std::nullopt, CondJump{0u, 1u, 1u}}}},
        BasicBlock{1u, {{std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE(graph_result.ok);
    REQUIRE(successors_of(graph_result.graph, 0u) == std::vector<std::uint32_t>{1u});
    REQUIRE(predecessors_of(graph_result.graph, 1u) == std::vector<std::uint32_t>{0u});
}

TEST_CASE("IR analysis: graph rejects invalid direct block targets") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, Jump{99u}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE_FALSE(graph_result.ok);
    REQUIRE(graph_result.error);
    REQUIRE(graph_result.error->code == CfgAnalysisCode::InvalidBlockTarget);
    REQUIRE(graph_result.error->block_id == 0u);
    REQUIRE(graph_result.error->target_block == 99u);
}

TEST_CASE("IR analysis: graph rejects duplicate block ids and missing entry") {
    Function duplicate;
    duplicate.entry = 0u;
    duplicate.blocks = {
        BasicBlock{0u, {{std::nullopt, Return{}}}},
        BasicBlock{0u, {{std::nullopt, Return{}}}},
    };

    const auto duplicate_result = build_cfg_graph(duplicate);
    REQUIRE_FALSE(duplicate_result.ok);
    REQUIRE(duplicate_result.error);
    REQUIRE(duplicate_result.error->code == CfgAnalysisCode::DuplicateBlockId);

    Function missing_entry;
    missing_entry.entry = 7u;
    missing_entry.blocks = {
        BasicBlock{0u, {{std::nullopt, Return{}}}},
    };

    const auto missing_result = build_cfg_graph(missing_entry);
    REQUIRE_FALSE(missing_result.ok);
    REQUIRE(missing_result.error);
    REQUIRE(missing_result.error->code == CfgAnalysisCode::EntryNotFound);
}

TEST_CASE("IR analysis: postorder traversals visit only reachable blocks") {
    const auto function = diamond_function();

    const auto po = postorder(function);
    const auto rpo = reverse_postorder(function);

    REQUIRE(po.ok);
    REQUIRE(rpo.ok);
    REQUIRE(po.order.size() == 4u);
    REQUIRE(rpo.order.size() == 4u);
    REQUIRE(po.order.back() == 0u);
    REQUIRE(rpo.order.front() == 0u);
    REQUIRE(rpo.order == std::vector<std::uint32_t>{0u, 2u, 1u, 3u});
}

TEST_CASE("IR analysis: dominators handle a diamond") {
    const auto doms = compute_dominators(diamond_function());

    REQUIRE(doms.ok);
    const auto& tree = doms.tree;
    REQUIRE(tree.immediate_dominator(0u) == std::nullopt);
    REQUIRE(tree.immediate_dominator(1u) == std::optional<std::uint32_t>{0u});
    REQUIRE(tree.immediate_dominator(2u) == std::optional<std::uint32_t>{0u});
    REQUIRE(tree.immediate_dominator(3u) == std::optional<std::uint32_t>{0u});
    REQUIRE(tree.children(0u) == std::vector<std::uint32_t>{1u, 2u, 3u});
    REQUIRE(tree.children(1u).empty());
    REQUIRE_FALSE(tree.is_reachable(4u));
    REQUIRE(tree.dominates(0u, 3u));
    REQUIRE_FALSE(tree.dominates(1u, 3u));
    REQUIRE_FALSE(tree.dominates(4u, 4u));
}

TEST_CASE("IR analysis: dominators handle a natural loop") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, Jump{1u}}}},
        BasicBlock{1u, {{0u, Constant{1u, OpSize::I64}},
                        {std::nullopt, CondJump{0u, 1u, 2u}}}},
        BasicBlock{2u, {{std::nullopt, Return{}}}},
    };

    const auto doms = compute_dominators(function);

    REQUIRE(doms.ok);
    const auto& tree = doms.tree;
    REQUIRE(tree.immediate_dominator(1u) == std::optional<std::uint32_t>{0u});
    REQUIRE(tree.immediate_dominator(2u) == std::optional<std::uint32_t>{1u});
    REQUIRE(tree.dominates(1u, 2u));
    REQUIRE(tree.dominates(1u, 1u));
    REQUIRE_FALSE(tree.dominates(2u, 1u));
}

TEST_CASE("IR analysis: duplicate GuestPc markers are rejected") {
    Function function;
    function.entry = 0u;
    function.blocks = {
        BasicBlock{0u, {{std::nullopt, GuestPc{0x1000u}},
                        {std::nullopt, Jump{1u}}}},
        BasicBlock{1u, {{std::nullopt, GuestPc{0x1000u}},
                        {std::nullopt, Return{}}}},
    };

    const auto graph_result = build_cfg_graph(function);

    REQUIRE_FALSE(graph_result.ok);
    REQUIRE(graph_result.error);
    REQUIRE(graph_result.error->code == CfgAnalysisCode::DuplicateGuestPc);
}
