// core/src/passes/flag_write_elim.cpp — F1-PS-012.
//
// `CmpFlags` writes the implicit x86 flag bank. A later CondJumpRel is
// the only current reader. Walking backward lets us keep only the nearest
// flag write that can feed each reader and drop older or orphaned writes.

#include "prisma/passes.hpp"

#include <algorithm>
#include <variant>

namespace prisma::passes {

std::vector<ir::Stmt>
flag_write_elimination(const std::vector<ir::Stmt>& stmts) {
    std::vector<ir::Stmt> reversed;
    reversed.reserve(stmts.size());

    bool flags_needed = false;

    for (auto it = stmts.rbegin(); it != stmts.rend(); ++it) {
        const ir::Stmt& s = *it;

        if (std::holds_alternative<ir::CondJumpRel>(s.op)) {
            flags_needed = true;
            reversed.push_back(s);
            continue;
        }

        if (std::holds_alternative<ir::CmpFlags>(s.op)) {
            if (flags_needed) {
                flags_needed = false;
                reversed.push_back(s);
            }
            continue;
        }

        if (std::holds_alternative<ir::Compare>(s.op)) {
            flags_needed = false;
            reversed.push_back(s);
            continue;
        }

        reversed.push_back(s);
    }

    std::reverse(reversed.begin(), reversed.end());
    return reversed;
}

}  // namespace prisma::passes
