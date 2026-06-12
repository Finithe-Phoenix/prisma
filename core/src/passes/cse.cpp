// core/src/passes/cse.cpp — Common Subexpression Elimination within
// a single statement list.
//
// Algorithm: scan linearly. For each BinOp, look up a canonical key
// `(op, lhs, rhs, size)` in a hash map. If we've already seen that
// key, rewrite the current stmt's op to a no-op copy — in the current
// IR, expressed as `BinOp Or, prev_ref, prev_ref` so the Lowerer
// emits `orr xd, xprev, xprev` which is effectively `mov xd, xprev`.
// If not seen, record it.
//
// Invalidation: a StoreReg or a Store* flushes affected map entries
// conservatively. For MVP (which only operates on Refs, not memory
// cells), any StoreReg / StoreMem / StoreMemTSO / CmpFlags (clobbers
// implicit flags — unrelated but simple) is treated as a full flush.

#include "prisma/passes.hpp"

#include <tuple>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

// Hashable canonical key for a BinOp expression.
struct BinOpKey {
    ir::BinOpKind op;
    ir::Ref       lhs;
    ir::Ref       rhs;
    ir::OpSize    size;

    bool operator==(const BinOpKey& o) const noexcept = default;
};

struct BinOpKeyHash {
    std::size_t operator()(const BinOpKey& k) const noexcept {
        // FNV-1a over the 4 fields. Good enough; collisions are
        // disambiguated by operator== on the full tuple.
        std::uint64_t h = 0xcbf29ce484222325ULL;
        const auto mix = [&](std::uint64_t v) {
            h ^= v;
            h *= 0x100000001b3ULL;
        };
        mix(static_cast<std::uint64_t>(k.op));
        mix(k.lhs);
        mix(k.rhs);
        mix(static_cast<std::uint64_t>(k.size));
        return static_cast<std::size_t>(h);
    }
};

bool is_flushing_op(const ir::Op& op) noexcept {
    return std::visit([](auto const& x) -> bool {
        using T = std::decay_t<decltype(x)>;
        return std::is_same_v<T, ir::StoreReg>
            || std::is_same_v<T, ir::StoreMem>
            || std::is_same_v<T, ir::StoreMemTSO>
            || std::is_same_v<T, ir::CmpFlags>
            || std::is_same_v<T, ir::AluFlags>
            || std::is_same_v<T, ir::Syscall>;
    }, op);
}

}  // namespace

std::vector<ir::Stmt>
common_subexpression_eliminate(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    std::unordered_map<BinOpKey, ir::Ref, BinOpKeyHash> seen;

    for (const auto& s : stmts) {
        ir::Stmt new_stmt = s;

        if (is_flushing_op(s.op)) {
            // Conservative: any side effect invalidates the table. A
            // stricter pass would only flush refs that depend on the
            // changed register; that's future work.
            seen.clear();
        }
        else if (std::holds_alternative<ir::BinOp>(s.op) && s.result) {
            const auto& b = std::get<ir::BinOp>(s.op);
            const BinOpKey key{b.op, b.lhs, b.rhs, b.size};
            const auto it = seen.find(key);
            if (it != seen.end()) {
                // Redundant computation — rewrite to a copy. We don't
                // have a dedicated Copy op; use `BinOp Or prev,prev`
                // which the Lowerer renders as `orr xd, xprev, xprev`
                // (equivalent to `mov xd, xprev`).
                new_stmt.op = ir::Op{ir::BinOp{
                    ir::BinOpKind::Or, it->second, it->second, b.size}};
            } else {
                seen[key] = *s.result;
            }
        }

        out.push_back(std::move(new_stmt));
    }

    return out;
}

}  // namespace prisma::passes
