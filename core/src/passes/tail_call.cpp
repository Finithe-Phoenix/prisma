// core/src/passes/tail_call.cpp — F1-PS-015 tail-call optimisation.
//
// Pattern:
//
//   CallRel{T, R}              ┐ replaced by
//   RetAdjusted{0}             ┘ JumpRel{T}
//
// The CallRel + RetAdjusted{0} pair is observably equivalent to the
// JumpRel because, at the point of CallRel, the top of stack already
// holds OUR caller's return address. CallRel pushes R; T eventually
// pops R via its own RetAdjusted, transfers control to R (the byte
// after our CallRel), where our RetAdjusted{0} pops the caller's
// return address and goes there. JumpRel{T} skips the push-then-pop
// of R: T's RetAdjusted reads our caller's return address directly.
//
// Same final destination, same final stack — just one fewer round
// trip through the dispatcher.

#include "prisma/passes.hpp"

#include <variant>

namespace prisma::passes {

std::vector<ir::Stmt>
tail_call_optimise(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];
        // Look for `CallRel` immediately followed by `RetAdjusted{0}`.
        if (i + 1 < stmts.size()
            && std::holds_alternative<ir::CallRel>(s.op)
            && std::holds_alternative<ir::RetAdjusted>(stmts[i + 1].op)) {
            const auto& call = std::get<ir::CallRel>(s.op);
            const auto& ret  = std::get<ir::RetAdjusted>(stmts[i + 1].op);
            if (ret.pop_bytes == 0u) {
                out.push_back({std::nullopt,
                    ir::Op{ir::JumpRel{call.target_guest_pc}}});
                ++i;  // skip the RetAdjusted (replaced by the jump)
                continue;
            }
        }
        out.push_back(s);
    }

    return out;
}

}  // namespace prisma::passes
