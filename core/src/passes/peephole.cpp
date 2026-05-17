// core/src/passes/peephole.cpp — F1-PS-009 local IR patterns.

#include "prisma/passes.hpp"

#include <variant>

namespace prisma::passes {

std::vector<ir::Stmt>
peephole_match(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& cur = stmts[i];

        if (i + 1 < stmts.size()
            && std::holds_alternative<ir::StoreReg>(cur.op)) {
            const auto& store = std::get<ir::StoreReg>(cur.op);
            const auto& next = stmts[i + 1];

            if (std::holds_alternative<ir::LoadReg>(next.op) && next.result) {
                const auto& load = std::get<ir::LoadReg>(next.op);
                if (load.reg == store.reg && load.size == store.size) {
                    out.push_back(cur);
                    out.push_back(ir::Stmt{
                        next.result,
                        ir::Op{ir::BinOp{
                            ir::BinOpKind::Or,
                            store.value,
                            store.value,
                            store.size}}});
                    ++i;
                    continue;
                }
            }

            if (std::holds_alternative<ir::StoreReg>(next.op)) {
                const auto& next_store = std::get<ir::StoreReg>(next.op);
                if (next_store.reg == store.reg
                    && next_store.size == store.size) {
                    continue;
                }
            }
        }

        out.push_back(cur);
    }

    return out;
}

}  // namespace prisma::passes
