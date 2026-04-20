// core/src/passes/pass_manager.cpp — implementation of PassManager.

#include "prisma/passes.hpp"

#include <utility>

namespace prisma::passes {

PassManager& PassManager::add(std::string name, PassFn fn) {
    passes_.push_back({std::move(name), std::move(fn)});
    return *this;
}

std::pair<std::vector<ir::Stmt>, PassRunStats>
PassManager::run(const std::vector<ir::Stmt>& input) const {
    PassRunStats stats;
    stats.initial_stmt_count = input.size();
    stats.passes.reserve(passes_.size());

    std::vector<ir::Stmt> current = input;
    for (const auto& entry : passes_) {
        current = entry.fn(current);
        stats.passes.push_back({entry.name, current.size()});
    }
    return {std::move(current), std::move(stats)};
}

PassManager default_pipeline() {
    // Order rationale:
    //   1. constant_propagate — folds two-constant BinOps into Constants.
    //                           Enables further simplification.
    //   2. algebraic_simplify — handles one-side identities that
    //                           const_prop can't see (x*0, x&0, |-1…).
    //   3. constant_propagate (again) — pick up new Constants the
    //                                   algebraic pass just created.
    //   4. common_subexpression_eliminate — kill duplicated BinOps
    //                                        post-simplification.
    //   5. dead_code_eliminate — sweep the defs that the earlier
    //                             passes made unreachable.
    //
    // This stays monotonic (each pass is purely refining) so a single
    // forward fixed-point iteration is enough for the current
    // optimisation set.
    PassManager pm;
    pm.add("constant_propagate",             constant_propagate);
    pm.add("algebraic_simplify",             algebraic_simplify);
    pm.add("constant_propagate_2",           constant_propagate);
    pm.add("common_subexpression_eliminate", common_subexpression_eliminate);
    pm.add("dead_code_eliminate",            dead_code_eliminate);
    return pm;
}

}  // namespace prisma::passes
