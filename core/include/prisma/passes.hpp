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

#include <cstddef>
#include <functional>
#include <string>
#include <utility>
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

// ---------------------------------------------------------------------------
// PassManager — ordered pipeline of named passes, with run statistics.
// ---------------------------------------------------------------------------
//
// Why a manager at all: once we have more than a couple of passes, we
// want control over ordering, the ability to skip passes for debugging,
// and a place to hang telemetry for the Pillar 1 (NPU) classifier. The
// manager is intentionally simple today — no fixed-point iteration, no
// dependency declarations — so we can evolve it alongside actual needs.

struct PassRunStats {
    // Statement count after each pass (in order). The first entry is the
    // count AFTER the first pass ran; `initial_stmt_count` carries the
    // count BEFORE any pass. This makes it trivial to compute per-pass
    // deltas.
    struct PassEntry {
        std::string name;
        std::size_t stmts_after;
    };
    std::size_t initial_stmt_count{0};
    std::vector<PassEntry> passes;
};

class PassManager {
public:
    using PassFn = std::function<std::vector<ir::Stmt>(const std::vector<ir::Stmt>&)>;

    // Register a pass to be run in order. Insertion order == run order.
    // `name` should be unique within a manager — duplicates are legal but
    // make stats harder to read.
    PassManager& add(std::string name, PassFn fn);

    // Number of registered passes.
    [[nodiscard]] std::size_t size() const noexcept { return passes_.size(); }

    // Run all passes in order and return the final statements plus
    // per-pass statistics. Never mutates its argument.
    [[nodiscard]] std::pair<std::vector<ir::Stmt>, PassRunStats>
    run(const std::vector<ir::Stmt>& input) const;

private:
    struct Entry {
        std::string name;
        PassFn fn;
    };
    std::vector<Entry> passes_;
};

// Returns the default pipeline used by the rest of the codebase:
//   constant_propagate → dead_code_eliminate
// Fase 1+ will grow this; for now these two passes are the whole story.
[[nodiscard]] PassManager default_pipeline();

}  // namespace prisma::passes
