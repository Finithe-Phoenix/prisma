// core/src/passes/flag_write_elim_stub.cpp — link-only placeholder for
// F1-PS-012 (claimed by codex). Returns the input unchanged so the rest
// of the binary links and other tests run. Codex's real implementation
// replaces this file when their claim lands.

#include "prisma/passes.hpp"

namespace prisma::passes {

std::vector<ir::Stmt>
flag_write_elimination(const std::vector<ir::Stmt>& stmts) {
    return stmts;
}

}  // namespace prisma::passes
