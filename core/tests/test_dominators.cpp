// core/tests/test_dominators.cpp — F1-IR-024 dominators + F1-IR-025
// natural loops.

#include <algorithm>
#include <catch2/catch_test_macros.hpp>

#include "prisma/dominators.hpp"
#include "prisma/ir.hpp"

using namespace prisma;
using namespace prisma::ir;

namespace {

// Helpers to build small CFGs by hand. Each function takes a list of
// (block_id, terminator) and stitches them together with no body
// statements except the terminator.
Function f_jump_chain() {
    // bb0 → bb1 → bb2 (Return)
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {{std::nullopt, Jump{1u}}}});
    fn.blocks.push_back(BasicBlock{1u, {{std::nullopt, Jump{2u}}}});
    fn.blocks.push_back(BasicBlock{2u, {{std::nullopt, Return{}}}});
    return fn;
}

Function f_diamond() {
    // bb0  CondJump bb1, bb2
    // bb1  Jump bb3
    // bb2  Jump bb3
    // bb3  Return
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{1, OpSize::I64}},
        {std::nullopt, CondJump{0u, 1u, 2u}}}});
    fn.blocks.push_back(BasicBlock{1u, {{std::nullopt, Jump{3u}}}});
    fn.blocks.push_back(BasicBlock{2u, {{std::nullopt, Jump{3u}}}});
    fn.blocks.push_back(BasicBlock{3u, {{std::nullopt, Return{}}}});
    return fn;
}

Function f_self_loop() {
    // bb0 CondJump bb0, bb1
    // bb1 Return
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{0, OpSize::I64}},
        {std::nullopt, CondJump{0u, 0u, 1u}}}});
    fn.blocks.push_back(BasicBlock{1u, {{std::nullopt, Return{}}}});
    return fn;
}

Function f_while_loop() {
    // bb0 (preheader)  Jump bb1
    // bb1 (header)     CondJump bb2, bb3
    // bb2 (body)       Jump bb1     ← back edge
    // bb3 (exit)       Return
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {{std::nullopt, Jump{1u}}}});
    fn.blocks.push_back(BasicBlock{1u, {
        {0u, Constant{0, OpSize::I64}},
        {std::nullopt, CondJump{0u, 2u, 3u}}}});
    fn.blocks.push_back(BasicBlock{2u, {{std::nullopt, Jump{1u}}}});
    fn.blocks.push_back(BasicBlock{3u, {{std::nullopt, Return{}}}});
    return fn;
}

}  // namespace

TEST_CASE("successors: Jump → 1, CondJump → 2, Return → 0") {
    auto fn = f_jump_chain();
    REQUIRE(successors(fn, 0u) == std::vector<std::uint32_t>{1u});
    REQUIRE(successors(fn, 1u) == std::vector<std::uint32_t>{2u});
    REQUIRE(successors(fn, 2u).empty());

    auto fd = f_diamond();
    REQUIRE(successors(fd, 0u) == std::vector<std::uint32_t>{1u, 2u});
}

TEST_CASE("postorder: linear chain is reverse of source order") {
    auto fn = f_jump_chain();
    const auto po = postorder(fn);
    // DFS from 0: visit 0, recurse to 1, recurse to 2, finish 2, 1, 0.
    REQUIRE(po == std::vector<std::uint32_t>{2u, 1u, 0u});
}

TEST_CASE("postorder: diamond has the expected shape") {
    auto fn = f_diamond();
    const auto po = postorder(fn);
    // Some valid orders: {3,1,2,0} or {3,2,1,0}. Check structural
    // invariants instead of pinning a specific tie-break.
    REQUIRE(po.size() == 4);
    REQUIRE(po.back() == 0u);                                    // entry last
    REQUIRE(std::find(po.begin(), po.end(), 3u) != po.end());    // 3 visited
}

TEST_CASE("dominators: linear chain — each block dominates the next") {
    auto fn = f_jump_chain();
    auto idoms = dominators(fn);
    REQUIRE(idoms[0] == 0u);  // entry self-dominates
    REQUIRE(idoms[1] == 0u);
    REQUIRE(idoms[2] == 1u);
}

TEST_CASE("dominators: diamond — bb3's idom is bb0 (the merge point)") {
    auto fn = f_diamond();
    auto idoms = dominators(fn);
    REQUIRE(idoms[0] == 0u);
    REQUIRE(idoms[1] == 0u);
    REQUIRE(idoms[2] == 0u);
    REQUIRE(idoms[3] == 0u);  // join below the diamond, dominated by 0
}

TEST_CASE("back_edges: jump_chain has none") {
    REQUIRE(back_edges(f_jump_chain()).empty());
}

TEST_CASE("back_edges: self-loop yields one edge (0 → 0)") {
    auto edges = back_edges(f_self_loop());
    REQUIRE(edges.size() == 1);
    REQUIRE(edges[0].tail == 0u);
    REQUIRE(edges[0].header == 0u);
}

TEST_CASE("back_edges: while loop yields edge bb2 → bb1") {
    auto edges = back_edges(f_while_loop());
    REQUIRE(edges.size() == 1);
    REQUIRE(edges[0].tail == 2u);
    REQUIRE(edges[0].header == 1u);
}

TEST_CASE("natural_loops: while loop body is {bb1, bb2}") {
    auto loops = natural_loops(f_while_loop());
    REQUIRE(loops.size() == 1);
    REQUIRE(loops[0].header == 1u);
    REQUIRE(loops[0].tail == 2u);
    REQUIRE(loops[0].body == std::vector<std::uint32_t>{1u, 2u});
}

TEST_CASE("natural_loops: self-loop body is just the header") {
    auto loops = natural_loops(f_self_loop());
    REQUIRE(loops.size() == 1);
    REQUIRE(loops[0].header == 0u);
    REQUIRE(loops[0].tail == 0u);
    REQUIRE(loops[0].body == std::vector<std::uint32_t>{0u});
}

TEST_CASE("natural_loops: diamond has no loops") {
    REQUIRE(natural_loops(f_diamond()).empty());
}

TEST_CASE("dominators: empty function returns empty vector") {
    Function fn;
    fn.entry = 0;
    auto idoms = dominators(fn);
    REQUIRE(idoms.empty());
}
