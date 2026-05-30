// prisma/dominators.hpp — F1-IR-024 dominator tree + F1-IR-025 loop
// detection over an `ir::Function`.
//
// Edges in the CFG are implicit in each block's terminator:
//
//   Jump{target_block}                → one successor: target_block
//   CondJump{cond, if_true, if_false} → two successors: if_true, if_false
//   any other terminator              → zero successors (out-of-region:
//                                        the block hands off to the
//                                        dispatcher via the next-PC value)
//
// Algorithms:
//
//   * `postorder(fn)` — DFS postorder traversal from `fn.entry`, using
//     successors() above. Returns a vector of block IDs.
//
//   * `dominators(fn)` — Cooper-Harvey-Kennedy iterative idom solver
//     (CHK 2001, "A Simple, Fast Dominance Algorithm"). For each
//     reachable block, idom[b] is its immediate dominator (idom[entry]
//     == entry). Unreachable blocks get `kNoDominator`.
//
//   * `back_edges(fn)` — pairs (tail, head) where head dominates tail.
//     Each such edge identifies a natural-loop header.
//
//   * `natural_loops(fn)` — for every back edge (tail → head) returns
//     the set of blocks in the loop body (header plus everything that
//     reaches tail without crossing header). Computed by reverse BFS
//     on the predecessor graph from `tail`, stopping at `header`.
//
// All four functions are pure and `[[nodiscard]]`. They tolerate
// disconnected blocks (idom undefined → kNoDominator), self-edges
// (Jump{self.id} → trivial loop with header == tail), and empty
// functions (return empty vectors).

#pragma once

#include <cstdint>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

inline constexpr std::uint32_t kNoDominator = 0xFFFF'FFFFu;

// Successors of `block_id` derived from its terminator. Returns up to
// two entries. If `block_id` is out-of-range, returns empty.
[[nodiscard]] std::vector<std::uint32_t>
successors(const Function& fn, std::uint32_t block_id);

[[nodiscard]] std::vector<std::uint32_t> postorder(const Function& fn);

// `idom[b] == kNoDominator` means b is unreachable from fn.entry.
// `idom[fn.entry] == fn.entry` (the entry self-dominates by convention).
[[nodiscard]] std::vector<std::uint32_t> dominators(const Function& fn);

struct BackEdge {
    std::uint32_t tail;    // edge source
    std::uint32_t header;  // edge target = loop header
};

[[nodiscard]] std::vector<BackEdge> back_edges(const Function& fn);

struct NaturalLoop {
    std::uint32_t              header;
    std::uint32_t              tail;    // the back-edge source
    std::vector<std::uint32_t> body;    // includes header and tail
};

[[nodiscard]] std::vector<NaturalLoop> natural_loops(const Function& fn);

}  // namespace prisma::ir
