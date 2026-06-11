// core/src/ir/cfg.cpp — flat → CFG translation, F1-IR-023.

#include "prisma/cfg.hpp"

#include <variant>

namespace prisma::ir {

bool is_terminator(const Op& op) noexcept {
    return std::holds_alternative<Return>(op)
        || std::holds_alternative<Jump>(op)
        || std::holds_alternative<CondJump>(op)
        || std::holds_alternative<JumpReg>(op)
        || std::holds_alternative<JumpRel>(op)
        || std::holds_alternative<CondJumpRel>(op)
        || std::holds_alternative<CallRel>(op)
        || std::holds_alternative<CallReg>(op)
        || std::holds_alternative<RetAdjusted>(op)
        || std::holds_alternative<Trap>(op)
        || std::holds_alternative<Cpuid>(op)
        || std::holds_alternative<Xgetbv>(op)
        || std::holds_alternative<InlineAsm>(op)
        || std::holds_alternative<CondJumpFlags>(op)
        || std::holds_alternative<RepStos>(op)
        || std::holds_alternative<RepMovs>(op);
}

Function build_cfg(std::span<const Stmt> stmts) {
    Function fn;
    fn.entry = 0;
    if (stmts.empty()) return fn;

    BasicBlock current;
    current.id = 0;
    std::uint32_t next_id = 1;

    for (const auto& s : stmts) {
        current.stmts.push_back(s);
        if (is_terminator(s.op)) {
            fn.blocks.push_back(std::move(current));
            current = BasicBlock{};
            current.id = next_id++;
        }
    }

    // Trailing non-terminator stmts still need a block (the decoder
    // may have run off the end of the buffer mid-instruction).
    if (!current.stmts.empty()) {
        fn.blocks.push_back(std::move(current));
    }

    return fn;
}

}  // namespace prisma::ir
