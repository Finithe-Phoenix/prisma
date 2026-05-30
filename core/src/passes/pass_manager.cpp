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

// ---------------------------------------------------------------------------
// FunctionPassManager — same shape as PassManager but operates on
// ir::Function. Both lists kept side-by-side rather than unified so
// the existing PassManager surface stays stable.
// ---------------------------------------------------------------------------

FunctionPassManager& FunctionPassManager::add(std::string name, PassFn fn) {
    passes_.push_back({std::move(name), std::move(fn)});
    return *this;
}

FunctionPassManager& FunctionPassManager::on_pass_run(DumpHook hook) {
    hooks_.push_back(std::move(hook));
    return *this;
}

namespace {
std::size_t total_stmts(const ir::Function& fn) {
    std::size_t n = 0;
    for (const auto& b : fn.blocks) n += b.stmts.size();
    return n;
}
}  // namespace

std::pair<ir::Function, FunctionPassRunStats>
FunctionPassManager::run(const ir::Function& input) const {
    FunctionPassRunStats stats;
    stats.initial_block_count = input.blocks.size();
    stats.initial_stmt_count  = total_stmts(input);
    stats.passes.reserve(passes_.size());

    ir::Function current = input;
    for (const auto& entry : passes_) {
        const auto t0 = std::chrono::steady_clock::now();
        current = entry.fn(current);
        const auto t1 = std::chrono::steady_clock::now();
        const auto ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                            t1 - t0).count();
        stats.passes.push_back({
            entry.name,
            current.blocks.size(),
            total_stmts(current),
            static_cast<std::uint64_t>(ns),
        });
        for (const auto& hook : hooks_) hook(entry.name, current);
    }
    return {std::move(current), std::move(stats)};
}

FunctionPassManager default_function_pipeline() {
    // Order rationale:
    //   1. global_cse runs first so duplicate computations across
    //      blocks collapse to copies before LICM scans for invariants
    //      — a hoist-then-cse pass would either miss the copy idiom
    //      or hoist redundant work.
    //   2. loop_invariant_motion follows. Future additions (GVN,
    //      partial-redundancy elimination) go here in order.
    FunctionPassManager pm;
    pm.add("global_cse",           global_cse);
    pm.add("loop_invariant_motion", loop_invariant_motion);
    return pm;
}

PassManager default_pipeline() {
    // Order rationale:
    //   1. constant_propagate folds two-constant BinOps into Constants.
    //   2. algebraic_simplify handles one-side identities that const_prop
    //      cannot see (x*0, x&0, |-1).
    //   3. strength_reduce turns x * (1<<k) into shifts after simple
    //      algebraic identities are gone.
    //   4. peephole performs local target-independent cleanup.
    //   5. constant_propagate_2 picks up Constants created above.
    //   6. redundant_load_eliminate dedupes repeated LoadMems and emits
    //      copy idioms for copy_propagate.
    //   7. common_subexpression_eliminate removes duplicate BinOps and emits
    //      `Or x, x` copy idioms.
    //   8. x87_stack_eliminate forwards known x87 ST slots into copy idioms
    //      before copy_propagate canonicalises downstream refs.
    //   9. copy_propagate chases copy idioms from RLE, CSE, and x87 stack
    //      forwarding.
    //  10. dead_store_eliminate drops StoreMems overwritten before any read.
    //  11. branch_fold resolves CondJumpRel with constant CmpFlags inputs.
    //  12. flag_write_elimination removes overwritten flag writes.
    //  13. dead_code_eliminate sweeps defs made unreachable above.
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
    pm.add("x87_stack_eliminate",            x87_stack_eliminate);
    pm.add("copy_propagate",                 copy_propagate);
    pm.add("dead_store_eliminate",           dead_store_eliminate);
    pm.add("branch_fold",                    branch_fold);
    pm.add("flag_write_elimination",         flag_write_elimination);
    pm.add("dead_code_eliminate",            dead_code_eliminate);
    return pm;
}

}  // namespace prisma::passes
