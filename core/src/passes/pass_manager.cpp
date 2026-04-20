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
    PassManager pm;
    pm.add("constant_propagate", constant_propagate);
    pm.add("dead_code_eliminate", dead_code_eliminate);
    return pm;
}

}  // namespace prisma::passes
