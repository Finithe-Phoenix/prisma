// core/src/passes/redundant_load.cpp — F1-PS-007.
//
// Re-reading the same memory location through the same address ref
// with no intervening memory write is redundant. We rewrite the second
// LoadMem to `Or %v1, %v1` (our copy idiom) so copy_propagate + DCE
// handle the downstream cleanup uniformly.
//
// Scope decisions:
//   * LoadMemTSO is untouched. Acquire semantics make a second load a
//     real synchronisation event whose elimination would change
//     observable behaviour even on single-threaded code under the
//     adaptive TSO pass.
//   * Any StoreMem / StoreMemTSO / fence-like op flushes the whole
//     load table — we have no alias analysis, so any write is assumed
//     to possibly alias every tracked address.
//   * Size matters: `LoadMem %a, I32` and `LoadMem %a, I64` describe
//     different values at the same address; only `(addr, size)` tuples
//     dedupe.

#include "prisma/passes.hpp"

#include <unordered_map>
#include <utility>
#include <variant>

namespace prisma::passes {

namespace {

struct LoadKey {
    ir::Ref    addr;
    ir::OpSize size;
    bool operator==(const LoadKey&) const = default;
};

struct LoadKeyHash {
    std::size_t operator()(const LoadKey& k) const noexcept {
        return std::hash<ir::Ref>{}(k.addr) * 31u
             + static_cast<std::size_t>(k.size);
    }
};

}  // namespace

std::vector<ir::Stmt>
redundant_load_eliminate(const std::vector<ir::Stmt>& stmts) {
    std::unordered_map<LoadKey, ir::Ref, LoadKeyHash> last_load;

    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    for (const auto& s : stmts) {
        if (std::holds_alternative<ir::LoadMem>(s.op) && s.result) {
            const auto& l = std::get<ir::LoadMem>(s.op);
            const LoadKey k{l.addr, l.size};
            auto it = last_load.find(k);
            if (it != last_load.end()) {
                // Rewrite as a copy of the prior load's result.
                ir::Stmt rewritten{s.result,
                    ir::Op{ir::BinOp{
                        ir::BinOpKind::Or, it->second, it->second, l.size}}};
                out.push_back(std::move(rewritten));
                continue;
            }
            last_load[k] = *s.result;
        }
        else if (std::holds_alternative<ir::StoreMem>(s.op)
              || std::holds_alternative<ir::StoreMemTSO>(s.op)
              || std::holds_alternative<ir::Fence>(s.op)) {
            // Any store could alias any tracked address. A fence can make
            // surrounding memory operations observable, so do not forward
            // plain loads across it.
            last_load.clear();
        }
        // LoadMemTSO, Jumps, Calls, CmpFlags, pure ops — no effect on the
        // memory image visible via plain LoadMem, so the table survives.

        out.push_back(s);
    }

    return out;
}

}  // namespace prisma::passes
