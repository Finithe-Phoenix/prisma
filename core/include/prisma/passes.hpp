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

// Dead Code Elimination.
//
// Removes pure statements whose bound Ref is never read by any subsequent
// statement. "Pure" here means: Constant, LoadReg, BinOp, Compare, LoadMem
// (non-TSO), LoadMemTSO (see note below). Side-effecting statements
// (StoreReg, StoreMem*, Jump, CondJump, Return) are never removed.
//
// Note on LoadMemTSO: strictly speaking a TSO load is observable under a
// weak memory model (it synchronises), so removing it when its value is
// dead changes the observable behaviour. For now we do keep LoadMemTSO
// alive even if the result ref is dead — it is not in the "pure" set for
// DCE. When the TSO-adaptive pass (Pillar 3) proves a region is
// single-threaded, those loads downgrade to plain LoadMem, and then
// become eligible for DCE.
//
// Algorithm: one backward pass that seeds `live_refs` from every side-
// effecting op's operands, plus one forward pass that filters. Correct-
// ness follows from the invariant that every Ref has a unique def.
[[nodiscard]] std::vector<ir::Stmt>
dead_code_eliminate(const std::vector<ir::Stmt>& stmts);

}  // namespace prisma::passes
