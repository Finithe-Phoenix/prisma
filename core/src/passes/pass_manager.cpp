// core/src/passes/pass_manager.cpp — implementation of PassManager.

#include "prisma/passes.hpp"

#include <chrono>
#include <utility>

namespace prisma::passes {

PassManager& PassManager::add(std::string name, PassFn fn) {
    passes_.push_back({std::move(name), std::move(fn)});
    return *this;
}

PassManager& PassManager::on_pass_run(DumpHook hook) {
    hooks_.push_back(std::move(hook));
    return *this;
}

std::pair<std::vector<ir::Stmt>, PassRunStats>
PassManager::run(const std::vector<ir::Stmt>& input) const {
    PassRunStats stats;
    stats.initial_stmt_count = input.size();
    stats.passes.reserve(passes_.size());

    std::vector<ir::Stmt> current = input;
    for (const auto& entry : passes_) {
        const auto t0 = std::chrono::steady_clock::now();
        current = entry.fn(current);
        const auto t1 = std::chrono::steady_clock::now();
        const auto ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                            t1 - t0).count();
        stats.passes.push_back({
            entry.name,
            current.size(),
            static_cast<std::uint64_t>(ns),
        });
        for (const auto& hook : hooks_) hook(entry.name, current);
    }
    return {std::move(current), std::move(stats)};
}

PassManager default_pipeline() {
    // Order rationale:
    //   1. constant_propagate — folds two-constant BinOps into Constants.
    //                           Enables further simplification.
    //   2. algebraic_simplify — handles one-side identities that
    //                           const_prop can't see (x*0, x&0, |-1…).
    //   3. strength_reduce — turns `x * (1<<k)` into shifts. Runs after
    //                        algebraic so obvious x*0 / x*1 are already gone.
    //   4. constant_propagate (again) — pick up new Constants the
    //                                   earlier passes just created.
    //   5. redundant_load_eliminate — dedupe repeated LoadMems through
    //                                  a stable addr ref. Emits copy
    //                                  idioms that copy_propagate
    //                                  consumes in step 7.
    //   6. common_subexpression_eliminate — kill duplicated BinOps
    //                                        post-simplification. Emits
    //                                        `Or x, x` copy idioms.
    //   7. copy_propagate — chase the copy idioms RLE + CSE emit so
    //                        downstream refs see the canonical source.
    //   8. dead_store_eliminate — drop StoreMems overwritten before any
    //                              read. Runs AFTER copy_propagate so
    //                              addr refs are canonical.
    //   9. branch_fold — statically-resolve CondJumpRel whose CmpFlags
    //                     compares two now-Constant operands.
    //  10. dead_code_eliminate — sweep defs that the earlier passes
    //                             made unreachable (including the
    //                             post-CSE/copy-prop/RLE copies).
    //
    // This stays monotonic (each pass is purely refining) so a single
    // forward iteration is enough for the current optimisation set.
    PassManager pm;
    pm.add("constant_propagate",             constant_propagate);
    pm.add("algebraic_simplify",             algebraic_simplify);
    pm.add("strength_reduce",                strength_reduce);
    pm.add("peephole",                       peephole_optimise_default);
    pm.add("constant_propagate_2",           constant_propagate);
    pm.add("redundant_load_eliminate",       redundant_load_eliminate);
    pm.add("common_subexpression_eliminate", common_subexpression_eliminate);
    pm.add("copy_propagate",                 copy_propagate);
    pm.add("dead_store_eliminate",           dead_store_eliminate);
    pm.add("branch_fold",                    branch_fold);
    pm.add("flag_write_elimination",         flag_write_elimination);
    pm.add("dead_code_eliminate",            dead_code_eliminate);
    return pm;
}

}  // namespace prisma::passes
