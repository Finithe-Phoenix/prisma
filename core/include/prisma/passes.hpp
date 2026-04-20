// prisma/passes.hpp — IR optimisation passes.
//
// Each pass is a pure function `Function → Function` (or a statement list
// transformer when useful). Passes NEVER mutate their input; they return
// the transformed IR. This lets us reason about soundness per-pass in Lean
// (Pillar 2) without worrying about ownership.
//
// Status: Fase 0 MVP — one pass (constant propagation). The pass manager
// arrives in Fase 1 once we have more than a couple of passes.

#pragma once

#include <vector>

#include "prisma/ir.hpp"

namespace prisma::passes {

// Constant propagation + folding for the pure arithmetic fragment.
//
// Transformations:
//   * `BinOp(op, ca, cb)` where both operands are Constants collapses to
//     a new Constant whose value is `mask_to_size(evalBinOp(op, ca, cb))`.
//   * Any StoreReg / Return / side-effecting op passes through unchanged,
//     possibly with updated refs where its operand was folded.
//
// The pass is local: it reasons within a single statement list, does not
// cross basic-block boundaries, and does not delete defs (a separate DCE
// pass handles that). A folded BinOp leaves its original LoadReg / other
// dependencies in place — they become dead but correctness is unchanged.
//
// See docs/rfc/0001-ir-ssa-over-template-based.md for the soundness
// obligation this pass will eventually satisfy formally in Lean.
[[nodiscard]] std::vector<ir::Stmt>
constant_propagate(const std::vector<ir::Stmt>& stmts);

}  // namespace prisma::passes
