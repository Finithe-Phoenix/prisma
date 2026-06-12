// core/src/passes/global_cse.cpp — F2-PS-004 Global Common Subexpression
// Elimination over an `ir::Function`.
//
// Forwards the intra-block CSE's available-expression hash table along
// dominator-tree edges where the dominated child has exactly one
// predecessor in the CFG, and that predecessor is its immediate
// dominator. Diamond / join blocks (multiple predecessors) start with
// an empty entry table — see `prisma/passes.hpp` for the soundness
// discussion and the deferred follow-up (classical available-expressions
// dataflow) that would tighten this.
//
// Within each block the algorithm is identical to
// `common_subexpression_eliminate` in `cse.cpp`. We re-implement
// rather than share a helper because the per-block state lifecycle
// here (entry table from idom, end table cached for descendants)
// makes the loop body different in subtle ways.

#include "prisma/passes.hpp"

#include <tuple>
#include <unordered_map>
#include <variant>

#include "prisma/dominators.hpp"

namespace prisma::passes {

namespace {

struct BinOpKey {
    ir::BinOpKind op;
    ir::Ref       lhs;
    ir::Ref       rhs;
    ir::OpSize    size;

    bool operator==(const BinOpKey& o) const noexcept = default;
};

struct BinOpKeyHash {
    std::size_t operator()(const BinOpKey& k) const noexcept {
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

using AvailTable = std::unordered_map<BinOpKey, ir::Ref, BinOpKeyHash>;

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

ir::Function global_cse(const ir::Function& fn) {
    if (fn.blocks.empty()) return fn;

    // id → position in `fn.blocks`. `idoms[idx[id]]` then yields the
    // immediate dominator's block ID.
    std::unordered_map<std::uint32_t, std::size_t> idx;
    idx.reserve(fn.blocks.size());
    for (std::size_t i = 0; i < fn.blocks.size(); ++i) {
        idx[fn.blocks[i].id] = i;
    }

    // Predecessor map derived from the public `successors()` API. Each
    // block id maps to the (possibly empty) list of predecessor ids.
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> preds;
    for (const auto& b : fn.blocks) {
        for (auto s : ir::successors(fn, b.id)) {
            preds[s].push_back(b.id);
        }
    }

    const auto idoms = ir::dominators(fn);
    if (idx.find(fn.entry) == idx.end()) return fn;

    // Reverse postorder visitation: every block is visited after all of
    // its dominators (this is the standard CHK property). Unreachable
    // blocks don't appear and are left untouched in the output.
    const auto po = ir::postorder(fn);
    std::vector<std::uint32_t> rpo(po.rbegin(), po.rend());

    std::unordered_map<std::uint32_t, AvailTable> end_table;

    ir::Function out = fn;

    for (auto block_id : rpo) {
        AvailTable seen;
        if (block_id != fn.entry) {
            const auto pit = preds.find(block_id);
            if (pit != preds.end() && pit->second.size() == 1) {
                const std::uint32_t pred = pit->second.front();
                if (pred == idoms[idx[block_id]]) {
                    const auto eit = end_table.find(pred);
                    if (eit != end_table.end()) {
                        seen = eit->second;
                    }
                }
            }
        }

        auto& block = out.blocks[idx[block_id]];
        for (auto& stmt : block.stmts) {
            if (is_flushing_op(stmt.op)) {
                seen.clear();
                continue;
            }
            if (!std::holds_alternative<ir::BinOp>(stmt.op) || !stmt.result) {
                continue;
            }
            const auto& b = std::get<ir::BinOp>(stmt.op);
            const BinOpKey key{b.op, b.lhs, b.rhs, b.size};
            const auto it = seen.find(key);
            if (it != seen.end()) {
                stmt.op = ir::Op{ir::BinOp{
                    ir::BinOpKind::Or, it->second, it->second, b.size}};
            } else {
                seen[key] = *stmt.result;
            }
        }

        end_table[block_id] = std::move(seen);
    }

    return out;
}

}  // namespace prisma::passes
