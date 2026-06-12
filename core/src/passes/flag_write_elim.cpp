// core/src/passes/flag_write_elim.cpp — F1-PS-012.
//
// Drops implicit flag writes whose NZCV is never consumed.
//
// Flag writers in our IR are `CmpFlags`, `AluFlags`, `Compare`, and
// `WriteFlagsCountZero`.
// Flag READERS are `CondJumpRel` (branches on NZCV) and `Select`
// (lowers to csel, which reads the NZCV set by the most recent
// flag writer — the CMOV / BZHI / CMPXCHG / BSF decode pattern).
// The pass walks forward, tracking the most-recent flag writer; on
// each flag reader it pins the writer as needed; on a fresh flag
// writer it drops the previous one if no reader pinned it.
// End-of-stream: a still-pending droppable flag write is dropped
// (no consumer follows).
//
// Compare writes flags too, but it has a result Ref that other code
// paths consume — we never drop a Compare. CmpFlags has no result
// Ref so dropping is safe.

#include "prisma/passes.hpp"

#include <cstddef>
#include <variant>
#include <vector>

namespace prisma::passes {

std::vector<ir::Stmt>
flag_write_elimination(const std::vector<ir::Stmt>& stmts) {
    const std::size_t n = stmts.size();
    std::vector<bool> drop(n, false);

    // Index of the most recent flag writer, or n if none. We only
    // ever drop CmpFlags / AluFlags / WriteFlagsCountZero (no result
    // Ref); Compare is always kept.
    std::size_t pending_writer    = n;
    bool        pending_is_droppable = false;

    for (std::size_t i = 0; i < n; ++i) {
        const auto& st = stmts[i];
        if (std::holds_alternative<ir::CmpFlags>(st.op)
            || std::holds_alternative<ir::AluFlags>(st.op)
            || std::holds_alternative<ir::WriteFlagsCountZero>(st.op)) {
            // A new implicit flag write supersedes the previous pending writer.
            // If the previous pending writer was an unread droppable write,
            // mark it for deletion now.
            if (pending_writer != n && pending_is_droppable) {
                drop[pending_writer] = true;
            }
            pending_writer      = i;
            pending_is_droppable = true;
        } else if (std::holds_alternative<ir::Compare>(st.op)) {
            // Compare also writes flags. Any pending droppable write becomes
            // stale and droppable. Compare itself stays (its result
            // Ref may have other consumers).
            if (pending_writer != n && pending_is_droppable) {
                drop[pending_writer] = true;
            }
            pending_writer      = i;
            pending_is_droppable = false;
        } else if (std::holds_alternative<ir::CondJumpRel>(st.op)
                   || std::holds_alternative<ir::Select>(st.op)) {
            // Flag reader: pin the most recent writer. Select lowers
            // to csel and consumes NZCV exactly like a conditional
            // branch — dropping its CmpFlags left csel reading stale
            // flags on hardware (caught by the gap-sweep ARM64 e2e).
            pending_writer      = n;
            pending_is_droppable = false;
        }
    }
    // End-of-block: a still-pending droppable write has no consumer.
    if (pending_writer != n && pending_is_droppable) {
        drop[pending_writer] = true;
    }

    std::vector<ir::Stmt> out;
    out.reserve(n);
    for (std::size_t i = 0; i < n; ++i) {
        if (!drop[i]) out.push_back(stmts[i]);
    }
    return out;
}

}  // namespace prisma::passes
