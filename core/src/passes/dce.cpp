// core/src/passes/dce.cpp — Dead Code Elimination for the IR.
//
// Two-pass algorithm over a single statement list:
//
//   1. Backward sweep: seed `live` with every Ref appearing as an operand
//      of a side-effecting statement. Then iterate: for each statement
//      whose result is in `live`, also add its operands to `live`. We
//      implement this by walking statements in reverse once, which is
//      sufficient because our IR is in SSA and statements are already in
//      program order (operand defs precede their uses).
//
//   2. Forward filter: emit every statement that is side-effecting, or
//      whose result is in `live`. Drop the rest.
//
// See `prisma/passes.hpp` for the contract on LoadMemTSO handling.

#include "prisma/passes.hpp"

#include <unordered_set>
#include <variant>

namespace prisma::passes {

namespace {

// Classify an Op: true iff removing its statement would NOT change program
// observable behaviour (assuming its result is unused). Side-effecting
// ops stay regardless of whether their result Ref is live.
bool is_pure_for_dce(const ir::Op& op) noexcept {
    return std::visit([](auto const& x) -> bool {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::Constant>) return true;
        else if constexpr (std::is_same_v<T, ir::LoadReg>) return true;
        else if constexpr (std::is_same_v<T, ir::BinOp>) return true;
        else if constexpr (std::is_same_v<T, ir::Compare>) return true;
        else if constexpr (std::is_same_v<T, ir::Select>) return true;
        else if constexpr (std::is_same_v<T, ir::LoadMem>) return true;
        else if constexpr (std::is_same_v<T, ir::JumpReg>) return false;
        // Everything else is impure: StoreReg, StoreMem*, LoadMemTSO,
        // Jump, JumpReg, CondJump, Return, CmpFlags (sets implicit flags),
        // JumpRel / CondJumpRel (control-flow transfer).
        else return false;
    }, op);
}

// Collect every Ref that an Op reads as an operand. The Op's own result
// (if any) is not included — only operands.
void collect_operand_refs(const ir::Op& op, std::unordered_set<ir::Ref>& into) {
    std::visit([&](auto const& x) {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::Constant>) {
            (void)x;
        } else if constexpr (std::is_same_v<T, ir::LoadReg>) {
            (void)x;
        } else if constexpr (std::is_same_v<T, ir::StoreReg>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::BinOp>) {
            into.insert(x.lhs);
            into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::Compare>) {
            into.insert(x.lhs);
            into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::Select>) {
            into.insert(x.true_value);
            into.insert(x.false_value);
        } else if constexpr (std::is_same_v<T, ir::LoadMem>) {
            into.insert(x.addr);
        } else if constexpr (std::is_same_v<T, ir::StoreMem>) {
            into.insert(x.addr);
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::LoadMemTSO>) {
            into.insert(x.addr);
        } else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) {
            into.insert(x.addr);
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::Jump>) {
            (void)x;
        } else if constexpr (std::is_same_v<T, ir::JumpReg>) {
            into.insert(x.target);
        } else if constexpr (std::is_same_v<T, ir::CondJump>) {
            into.insert(x.cond);
        } else if constexpr (std::is_same_v<T, ir::Return>) {
            (void)x;
        } else if constexpr (std::is_same_v<T, ir::CmpFlags>) {
            into.insert(x.lhs);
            into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::JumpRel>) {
            (void)x;
        } else if constexpr (std::is_same_v<T, ir::CondJumpRel>) {
            (void)x;  // no SSA operands; cc + PCs are constants.
        } else if constexpr (std::is_same_v<T, ir::VecBinOp>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecFpBinOp>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecFpScalarBinOp>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::StoreVecReg>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::LoadVec>) {
            into.insert(x.addr);
        } else if constexpr (std::is_same_v<T, ir::StoreVec>) {
            into.insert(x.addr); into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::XmmFromGpr>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::GprFromXmm>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::VecCmp>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecShuffle32x4>) {
            into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::VecUnpack>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecShiftImm>) {
            into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::VecShiftBytes>) {
            into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::IntToFpScalar>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::FpToIntScalar>) {
            into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::FpCvtScalar>) {
            into.insert(x.lhs); into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::VecShuffle2Src>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecInsertLane>) {
            into.insert(x.lhs_xmm); into.insert(x.value);
        } else if constexpr (std::is_same_v<T, ir::VecExtractLaneU>) {
            into.insert(x.src_xmm);
        } else if constexpr (std::is_same_v<T, ir::VecMaskMsb>) {
            into.insert(x.src_xmm);
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsFp>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecShuffleH4>) {
            into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::VecMaskFp>) {
            into.insert(x.src_xmm);
        } else if constexpr (std::is_same_v<T, ir::VecFpCompare>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecPshufb>) {
            into.insert(x.src); into.insert(x.mask);
        } else if constexpr (std::is_same_v<T, ir::VecAbs>) {
            into.insert(x.src);
        } else if constexpr (std::is_same_v<T, ir::VecAlignr>) {
            into.insert(x.lhs); into.insert(x.rhs);
        } else if constexpr (std::is_same_v<T, ir::VecExtend>) {
            into.insert(x.src);
        }
    }, op);
}

}  // namespace

std::vector<ir::Stmt>
dead_code_eliminate(const std::vector<ir::Stmt>& stmts) {
    // -------- Pass 1: collect live refs by walking statements in reverse.
    std::unordered_set<ir::Ref> live;

    for (auto it = stmts.rbegin(); it != stmts.rend(); ++it) {
        const ir::Stmt& s = *it;
        const bool has_result = s.result.has_value();
        const bool impure     = !is_pure_for_dce(s.op);
        const bool result_is_live = has_result && live.count(*s.result) != 0;

        // A statement's operands become live if:
        //   * the op is side-effecting (we must keep it, so its operands
        //     are needed), OR
        //   * its result is already live (so computing it requires the
        //     operands).
        if (impure || result_is_live) {
            collect_operand_refs(s.op, live);
        }
    }

    // -------- Pass 2: filter.
    std::vector<ir::Stmt> out;
    out.reserve(stmts.size());
    for (const ir::Stmt& s : stmts) {
        const bool impure = !is_pure_for_dce(s.op);
        const bool result_is_live =
            s.result.has_value() && live.count(*s.result) != 0;

        if (impure || result_is_live) {
            out.push_back(s);
        }
    }
    return out;
}

}  // namespace prisma::passes
