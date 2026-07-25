// core/src/passes/inline_short_helpers.cpp — F2-PS-005 stub
//
// Inlines short helper blocks at the call site.
// A full implementation requires cross-block IR analysis which is
// still being developed. For now, this is a stub pass that scans
// for CallRel operations and logs candidates.

#include "prisma/passes.hpp"

#include <iostream>
#include <variant>

namespace prisma::passes {

std::vector<ir::Stmt>
inline_short_helpers(const std::vector<ir::Stmt>& stmts) {
    // In Fase 2, realistic cross-block inlining is still being developed.
    // This pass scans for CallRel ops and logs them as potential candidates.
    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];
        
        // Look for CallRel as a potential inlining candidate
        if (std::holds_alternative<ir::CallRel>(s.op)) {
            const auto& call = std::get<ir::CallRel>(s.op);
            // Log the candidate
            std::cerr << "[inline_short_helpers] Candidate CallRel to target: 0x" 
                      << std::hex << call.target_guest_pc << std::dec << "\n";
        }
    }

    // Return the unmodified statements.
    return stmts;
}

}  // namespace prisma::passes
