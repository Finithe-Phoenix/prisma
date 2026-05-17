// core/src/passes/dead_store.cpp — F1-PS-008.
//
// A StoreMem whose value is overwritten by a subsequent StoreMem to the
// same (addr_ref, size) pair — with NO intervening memory operation
// that could observe it — is dead. We drop the earlier one.
//
// Implementation:
//   * Two-pass: first scan forward to collect dead-store indices,
//     second pass filters them out. Collecting indices first avoids
//     the fiddly "update pending-store iterator mid-emit" dance and
//     keeps the rewrite trivially linear.
//   * `pending_store[(addr, size)] = last_store_stmt_idx`. On a new
//     StoreMem to the same `(addr, size)`, the previous entry becomes
//     a dead-store index and is replaced.
//   * Any LoadMem / LoadMemTSO / StoreMemTSO clears `pending_store`
//     entirely — the pending store might have been observed.
//   * StoreMem to a DIFFERENT `(addr, size)` under this MVP simply
//     adds itself to the table without clearing — different keys don't
//     alias each other in the address-ref space. (This is the
//     soundness-relevant invariant: two StoreMems with distinct addr
//     refs are guaranteed by SSA to write different concrete addresses
//     within a single call path, but not across all schedules. Still
//     safe per-block since we don't cross block boundaries.)
//
// Scope decisions:
//   * StoreMemTSO is release-ordered — another thread may observe it
//     even in a block that appears to overwrite it. Never killed.
//   * `addr_ref` equality, not value equality: %a == %a counts, but
//     two distinct LoadReg results pointing at the same guest reg
//     don't. Missing some opportunities is fine; soundness first.

#include "prisma/passes.hpp"

#include <cstddef>
#include <unordered_map>
#include <unordered_set>
#include <variant>

namespace prisma::passes {

namespace {

struct StoreKey {
    ir::Ref    addr;
    ir::OpSize size;
    bool operator==(const StoreKey&) const = default;
};

struct StoreKeyHash {
    std::size_t operator()(const StoreKey& k) const noexcept {
        return std::hash<ir::Ref>{}(k.addr) * 31u
             + static_cast<std::size_t>(k.size);
    }
};

}  // namespace

std::vector<ir::Stmt>
dead_store_eliminate(const std::vector<ir::Stmt>& stmts) {
    std::unordered_map<StoreKey, std::size_t, StoreKeyHash> pending;
    std::unordered_set<std::size_t> dead_indices;

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];
        if (std::holds_alternative<ir::StoreMem>(s.op)) {
            const auto& st = std::get<ir::StoreMem>(s.op);
            const StoreKey k{st.addr, st.size};
            auto it = pending.find(k);
            if (it != pending.end()) {
                dead_indices.insert(it->second);
                it->second = i;
            } else {
                pending[k] = i;
            }
        }
        else if (std::holds_alternative<ir::LoadMem>(s.op)
              || std::holds_alternative<ir::LoadMemTSO>(s.op)
              || std::holds_alternative<ir::StoreMemTSO>(s.op)
              || std::holds_alternative<ir::Fence>(s.op)) {
            // Could observe / synchronise with a pending store.
            pending.clear();
        }
        // Pure ops don't interact with memory — table survives.
    }

    // Filter.
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size() - dead_indices.size());
    for (std::size_t i = 0; i < stmts.size(); ++i) {
        if (!dead_indices.contains(i)) out.push_back(stmts[i]);
    }
    return out;
}

}  // namespace prisma::passes
