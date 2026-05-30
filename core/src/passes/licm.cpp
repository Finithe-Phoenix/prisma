// core/src/passes/licm.cpp — F2-PS-003 Loop-Invariant Code Motion.
//
// For each natural loop in the function, find statements whose result
// does not change across iterations and hoist them to the loop's
// pre-header. A statement qualifies as loop-invariant iff:
//
//   1. It is pure (no observable side effect — the same set DCE uses).
//   2. All of its operand Refs are defined outside the loop body.
//
// Iteration: we re-scan each loop's body after a successful hoist,
// because the just-hoisted stmt's result is now "outside the body" and
// may unlock further hoists. This converges in O(stmts_in_loop) per
// loop (each stmt can leave the body at most once).
//
// Pre-header detection is conservative: the loop is hoisted iff the
// header has exactly one CFG predecessor outside the loop body. If
// the natural-loop entry has multiple incoming non-loop edges (rare
// in compiler output but possible in hand-written assembly), we skip
// — synthesising a new preheader block changes CFG shape and is
// deferred to a follow-up.
//
// **Wiring caveat (carries from F2-PS-004 / Global CSE):** today's
// translator emits single-block functions, which trivially have zero
// loops. LICM on those is a no-op. The plumbing here is the
// deliverable; the algorithmic gain unlocks once
// `core/src/translator/` gains multi-instruction fusion or the
// translator learns to merge a backward-jump target with its
// predecessor.

#include "prisma/passes.hpp"

#include <algorithm>
#include <unordered_set>
#include <variant>

#include "prisma/dominators.hpp"

namespace prisma::passes {

namespace {

bool stmt_is_pure(const ir::Op& op) noexcept {
    return std::visit([](auto const& x) -> bool {
        using T = std::decay_t<decltype(x)>;
        return std::is_same_v<T, ir::Constant>
            || std::is_same_v<T, ir::LoadReg>
            || std::is_same_v<T, ir::LoadSegBase>
            || std::is_same_v<T, ir::BinOp>
            || std::is_same_v<T, ir::Compare>
            || std::is_same_v<T, ir::Extend>
            || std::is_same_v<T, ir::Truncate>;
    }, op);
}

void collect_operand_refs(const ir::Op& op, std::vector<ir::Ref>& into) {
    std::visit([&](auto const& x) {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::BinOp>) {
            into.push_back(x.lhs); into.push_back(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::Compare>) {
            into.push_back(x.lhs); into.push_back(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::Extend>) {
            into.push_back(x.value);
        } else if constexpr (std::is_same_v<T, ir::Truncate>) {
            into.push_back(x.value);
        }
        // Constant / LoadReg / LoadSegBase have no operand refs.
    }, op);
}

}  // namespace

ir::Function loop_invariant_motion(const ir::Function& fn) {
    if (fn.blocks.empty()) return fn;

    const auto loops = ir::natural_loops(fn);
    if (loops.empty()) return fn;

    // id → position index, used to address `out.blocks` directly.
    std::unordered_map<std::uint32_t, std::size_t> idx;
    idx.reserve(fn.blocks.size());
    for (std::size_t i = 0; i < fn.blocks.size(); ++i) {
        idx[fn.blocks[i].id] = i;
    }

    // Predecessor map for preheader detection.
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> preds;
    for (const auto& b : fn.blocks) {
        for (auto s : ir::successors(fn, b.id)) preds[s].push_back(b.id);
    }

    ir::Function out = fn;

    for (const auto& loop : loops) {
        // Set of block ids in this loop, for fast membership checks.
        std::unordered_set<std::uint32_t> body_ids(
            loop.body.begin(), loop.body.end());

        // Identify the preheader: the unique non-loop predecessor of
        // `loop.header`. Skip the whole loop if 0 or >1 candidates.
        const auto pit = preds.find(loop.header);
        if (pit == preds.end()) continue;
        std::vector<std::uint32_t> outside_preds;
        for (auto p : pit->second) {
            if (body_ids.find(p) == body_ids.end()) {
                outside_preds.push_back(p);
            }
        }
        if (outside_preds.size() != 1) continue;
        const std::uint32_t preheader_id = outside_preds.front();
        const auto preh_it = idx.find(preheader_id);
        if (preh_it == idx.end()) continue;
        auto& preheader = out.blocks[preh_it->second];
        if (preheader.stmts.empty()) continue;  // no terminator slot

        // Set of Refs defined in the loop body. Rebuilt each pass
        // because successful hoists shrink it.
        auto rebuild_body_refs = [&](std::unordered_set<ir::Ref>& set) {
            set.clear();
            for (auto bid : loop.body) {
                const auto i = idx[bid];
                for (const auto& st : out.blocks[i].stmts) {
                    if (st.result) set.insert(*st.result);
                }
            }
        };

        std::unordered_set<ir::Ref> body_refs;
        rebuild_body_refs(body_refs);

        // Iterate to fixed point per loop.
        bool changed = true;
        while (changed) {
            changed = false;
            for (auto bid : loop.body) {
                auto& block = out.blocks[idx[bid]];
                // Don't hoist the terminator (last stmt of any block).
                if (block.stmts.empty()) continue;
                const std::size_t terminator_pos = block.stmts.size() - 1;
                std::size_t i = 0;
                while (i < terminator_pos) {
                    const auto& stmt = block.stmts[i];
                    if (!stmt.result || !stmt_is_pure(stmt.op)) {
                        ++i;
                        continue;
                    }
                    std::vector<ir::Ref> operand_refs;
                    collect_operand_refs(stmt.op, operand_refs);
                    bool all_outside = true;
                    for (auto r : operand_refs) {
                        if (body_refs.count(r) != 0) {
                            all_outside = false;
                            break;
                        }
                    }
                    if (!all_outside) {
                        ++i;
                        continue;
                    }
                    // Hoist: erase from body block, insert at the
                    // preheader's end-1 (before its terminator). Save
                    // the result Ref before the move-out so we can
                    // update body_refs without touching the moved-from
                    // statement.
                    const ir::Ref hoisted_result = *stmt.result;
                    ir::Stmt hoisted = std::move(block.stmts[i]);
                    block.stmts.erase(block.stmts.begin()
                                      + static_cast<std::ptrdiff_t>(i));
                    auto& ph_stmts = preheader.stmts;
                    if (ph_stmts.empty()) {
                        ph_stmts.push_back(std::move(hoisted));
                    } else {
                        ph_stmts.insert(
                            ph_stmts.begin()
                                + static_cast<std::ptrdiff_t>(
                                    ph_stmts.size() - 1),
                            std::move(hoisted));
                    }
                    body_refs.erase(hoisted_result);
                    changed = true;
                    // Do not increment i; the next stmt has shifted up.
                }
            }
        }
    }

    return out;
}

}  // namespace prisma::passes
