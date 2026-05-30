// core/src/passes/flag_write_elim.cpp — F1-PS-012.
//
// Drops `CmpFlags` writes whose implicit NZCV is never consumed.
//
// Flag writers in our IR are `CmpFlags` and `Compare`. The only flag
// reader exposed today is `CondJumpRel`. The pass walks forward,
// tracking the most-recent flag writer; on each flag reader it pins
// the writer as needed; on a fresh flag writer it drops the previous
// one if no reader pinned it. End-of-stream: a still-pending
// CmpFlags is dropped (no consumer follows).
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
    // ever drop CmpFlags (no result Ref); Compare is always kept.
    std::size_t pending_writer    = n;
    bool        pending_is_cmpflags = false;

    for (std::size_t i = 0; i < n; ++i) {
        const auto& st = stmts[i];
        if (std::holds_alternative<ir::CmpFlags>(st.op)) {
            // A new CmpFlags supersedes the previous pending writer.
            // If the previous pending writer was an unread CmpFlags,
            // mark it for deletion now.
            if (pending_writer != n && pending_is_cmpflags) {
                drop[pending_writer] = true;
            }
            pending_writer      = i;
            pending_is_cmpflags = true;
        } else if (std::holds_alternative<ir::Compare>(st.op)) {
            // Compare also writes flags. Any pending CmpFlags becomes
            // stale and droppable. Compare itself stays (its result
            // Ref may have other consumers).
            if (pending_writer != n && pending_is_cmpflags) {
                drop[pending_writer] = true;
            }
            pending_writer      = i;
            pending_is_cmpflags = false;
        } else if (std::holds_alternative<ir::CondJumpRel>(st.op)) {
            // Flag reader: pin the most recent writer.
            pending_writer      = n;
            pending_is_cmpflags = false;
        }
    }
    // End-of-block: a still-pending CmpFlags has no consumer.
    if (pending_writer != n && pending_is_cmpflags) {
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
