// core/src/ir/validate.cpp — implementation of F1-IR-016 validator
// + F1-IR-015 typed-Ref consistency checks.

#include "prisma/ir_validate.hpp"

#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <variant>

namespace prisma::ir {

namespace {

ValidationResult err(ValidationCode code,
                     std::size_t    stmt_index,
                     Ref            bad_ref,
                     std::string    message) {
    ValidationError e{code, stmt_index, bad_ref, std::move(message)};
    return {false, std::move(e)};
}

// What size does this op produce? `nullopt` for ops without a result
// (the validator already rejects pure-without-result above) or for
// ops whose result size depends on something the validator can't
// know without the ref-size map (which is only possible during the
// forward pass — handled in `result_size_for` below).
std::optional<OpSize> result_size_static(const Op& op) {
    return std::visit([](const auto& x) -> std::optional<OpSize> {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, Constant>)    return x.size;
        else if constexpr (std::is_same_v<T, LoadReg>)     return x.size;
        else if constexpr (std::is_same_v<T, LoadSegBase>) return OpSize::I64;
        else if constexpr (std::is_same_v<T, BinOp>)       return x.size;
        else if constexpr (std::is_same_v<T, Compare>)     return OpSize::I8;
        else if constexpr (std::is_same_v<T, Select>)      return x.size;
        else if constexpr (std::is_same_v<T, LoadMem>)     return x.size;
        else if constexpr (std::is_same_v<T, LoadMemTSO>)  return x.size;
        else if constexpr (std::is_same_v<T, X87Load>)     return OpSize::I64;
        else if constexpr (std::is_same_v<T, X87Pop>)      return OpSize::I64;
        else if constexpr (std::is_same_v<T, Extend>)      return x.to_size;
        else if constexpr (std::is_same_v<T, Truncate>)    return x.to_size;
        else if constexpr (std::is_same_v<T, ReadFlag>)    return OpSize::I8;
        // WriteFlags / FpConstant / FpBinOp produce non-integer-typed
        // refs (Flags pseudo-type or FP); the size table doesn't apply.
        else                                                return std::nullopt;
    }, op);
}

// Walk every Ref-valued field of `op` and invoke `visit(ref)`.
template <typename F>
void for_each_operand_ref(const Op& op, F&& visit) {
    std::visit([&](const auto& x) {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, BinOp>)      { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, Compare>)    { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, Select>)     { visit(x.true_value); visit(x.false_value); }
        else if constexpr (std::is_same_v<T, LoadMem>)    { visit(x.addr); }
        else if constexpr (std::is_same_v<T, StoreMem>)   { visit(x.addr); visit(x.value); }
        else if constexpr (std::is_same_v<T, LoadMemTSO>) { visit(x.addr); }
        else if constexpr (std::is_same_v<T, StoreMemTSO>){ visit(x.addr); visit(x.value); }
        else if constexpr (std::is_same_v<T, StoreReg>)   { visit(x.value); }
        else if constexpr (std::is_same_v<T, CmpFlags>)   { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, CondJump>)   { visit(x.cond); }
        else if constexpr (std::is_same_v<T, JumpReg>)    { visit(x.target); }
        else if constexpr (std::is_same_v<T, CallReg>)    { visit(x.target); }
        else if constexpr (std::is_same_v<T, Extend>)     { visit(x.value); }
        else if constexpr (std::is_same_v<T, Truncate>)   { visit(x.value); }
        else if constexpr (std::is_same_v<T, FpBinOp>)    { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, WriteFlags>)    { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, ReadFlag>)      { visit(x.flags); }
        else if constexpr (std::is_same_v<T, CondJumpFlags>) { visit(x.flags); }
        else if constexpr (std::is_same_v<T, VecBinOp>)      { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, StoreVecReg>)   { visit(x.value); }
        else if constexpr (std::is_same_v<T, VecFpBinOp>)    { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecFpScalarBinOp>) { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, LoadVec>)       { visit(x.addr); }
        else if constexpr (std::is_same_v<T, StoreVec>)      { visit(x.addr); visit(x.value); }
        else if constexpr (std::is_same_v<T, XmmFromGpr>)    { visit(x.value); }
        else if constexpr (std::is_same_v<T, GprFromXmm>)    { visit(x.value); }
        else if constexpr (std::is_same_v<T, VecCmp>)        { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecShuffle32x4>) { visit(x.src); }
        else if constexpr (std::is_same_v<T, VecUnpack>)     { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecShiftImm>)   { visit(x.src); }
        else if constexpr (std::is_same_v<T, VecShiftBytes>) { visit(x.src); }
        else if constexpr (std::is_same_v<T, IntToFpScalar>) { visit(x.value); }
        else if constexpr (std::is_same_v<T, FpToIntScalar>) { visit(x.value); }
        else if constexpr (std::is_same_v<T, FpCvtScalar>)   { visit(x.lhs); visit(x.src); }
        else if constexpr (std::is_same_v<T, VecShuffle2Src>) { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecInsertLane>)  { visit(x.lhs_xmm); visit(x.value); }
        else if constexpr (std::is_same_v<T, VecExtractLaneU>) { visit(x.src_xmm); }
        else if constexpr (std::is_same_v<T, VecMaskMsb>)    { visit(x.src_xmm); }
        else if constexpr (std::is_same_v<T, WriteFlagsFp>)  { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecShuffleH4>)  { visit(x.src); }
        else if constexpr (std::is_same_v<T, VecMaskFp>)     { visit(x.src_xmm); }
        else if constexpr (std::is_same_v<T, VecFpCompare>)  { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecPshufb>)     { visit(x.src); visit(x.mask); }
        else if constexpr (std::is_same_v<T, VecAbs>)        { visit(x.src); }
        else if constexpr (std::is_same_v<T, VecAlignr>)     { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, VecExtend>)     { visit(x.src); }
        else if constexpr (std::is_same_v<T, VecFpRound>)    { visit(x.lhs); visit(x.src); }
        else if constexpr (std::is_same_v<T, Popcnt>)        { visit(x.value); }
        else if constexpr (std::is_same_v<T, Lzcnt>)         { visit(x.value); }
        else if constexpr (std::is_same_v<T, Tzcnt>)         { visit(x.value); }
        else if constexpr (std::is_same_v<T, VecBlend>)      { visit(x.dst); visit(x.src); visit(x.mask); }
        else if constexpr (std::is_same_v<T, WriteFlagsPtest>) { visit(x.lhs); visit(x.rhs); }
        else if constexpr (std::is_same_v<T, WriteFlagsPtestYmm>) {
            visit(x.lo_lhs); visit(x.lo_rhs); visit(x.hi_lhs); visit(x.hi_rhs);
        }
        else if constexpr (std::is_same_v<T, VecTbl2>) {
            visit(x.src_lo); visit(x.src_hi); visit(x.idx);
        }
        else if constexpr (std::is_same_v<T, VecAes>) {
            visit(x.src); visit(x.key);
        }
        else if constexpr (std::is_same_v<T, VecAesKeygenAssist>) {
            visit(x.src);
        }
        else if constexpr (std::is_same_v<T, VecSha>) {
            visit(x.a); visit(x.b); visit(x.wk);
        }
        else if constexpr (std::is_same_v<T, Bswap>) {
            visit(x.value);
        }
        else if constexpr (std::is_same_v<T, Crc32c>) {
            visit(x.crc); visit(x.data);
        }
        else if constexpr (std::is_same_v<T, VecGather>) {
            visit(x.base); visit(x.index); visit(x.mask); visit(x.prev);
        }
        else if constexpr (std::is_same_v<T, StoreVecRegHi>) { visit(x.value); }
        else if constexpr (std::is_same_v<T, VecFpFma>)      { visit(x.a); visit(x.b); visit(x.c); }
        else if constexpr (std::is_same_v<T, VecFpScalarFma>) { visit(x.a); visit(x.b); visit(x.c); visit(x.scalar_upper); }
        else if constexpr (std::is_same_v<T, RepStos>)       { (void)x; }   // no operand refs
        else if constexpr (std::is_same_v<T, RepMovs>)       { (void)x; }
        else if constexpr (std::is_same_v<T, X87Load>)       { (void)x; }   // no operand refs
        else if constexpr (std::is_same_v<T, X87Store>)      { visit(x.value); }
        else if constexpr (std::is_same_v<T, X87Push>)       { visit(x.value); }
        else if constexpr (std::is_same_v<T, X87Pop>)        { (void)x; }   // no operand refs
        // Constant, LoadReg, LoadSegBase, Jump, JumpRel, CondJumpRel,
        // Return, CallRel, RetAdjusted, Cpuid, Syscall, Trap, Fence,
        // GuestPc, InlineAsm, FpConstant, VecConstant, LoadVecReg,
        // LoadVecRegHi — no operand refs.
    }, op);
}

// Classify whether `op` is allowed to have a result ref.
// Returns true iff the op is pure (must produce a ref).
bool op_is_pure(const Op& op) {
    return std::visit([](const auto& x) {
        using T = std::decay_t<decltype(x)>;
        return std::is_same_v<T, Constant>
            || std::is_same_v<T, LoadReg>
            || std::is_same_v<T, LoadSegBase>
            || std::is_same_v<T, BinOp>
            || std::is_same_v<T, Compare>
            || std::is_same_v<T, Select>
            || std::is_same_v<T, LoadMem>
            || std::is_same_v<T, LoadMemTSO>
            || std::is_same_v<T, Extend>
            || std::is_same_v<T, Truncate>
            || std::is_same_v<T, FpConstant>
            || std::is_same_v<T, FpBinOp>
            || std::is_same_v<T, WriteFlags>
            || std::is_same_v<T, ReadFlag>
            || std::is_same_v<T, VecConstant>
            || std::is_same_v<T, VecBinOp>
            || std::is_same_v<T, LoadVecReg>
            || std::is_same_v<T, VecFpBinOp>
            || std::is_same_v<T, VecFpScalarBinOp>
            || std::is_same_v<T, LoadVec>
            || std::is_same_v<T, XmmFromGpr>
            || std::is_same_v<T, GprFromXmm>
            || std::is_same_v<T, VecCmp>
            || std::is_same_v<T, VecShuffle32x4>
            || std::is_same_v<T, VecUnpack>
            || std::is_same_v<T, VecShiftImm>
            || std::is_same_v<T, VecShiftBytes>
            || std::is_same_v<T, IntToFpScalar>
            || std::is_same_v<T, FpToIntScalar>
            || std::is_same_v<T, FpCvtScalar>
            || std::is_same_v<T, VecShuffle2Src>
            || std::is_same_v<T, VecInsertLane>
            || std::is_same_v<T, VecExtractLaneU>
            || std::is_same_v<T, VecMaskMsb>
            || std::is_same_v<T, WriteFlagsFp>
            || std::is_same_v<T, VecShuffleH4>
            || std::is_same_v<T, VecMaskFp>
            || std::is_same_v<T, VecFpCompare>
            || std::is_same_v<T, VecPshufb>
            || std::is_same_v<T, VecAbs>
            || std::is_same_v<T, VecAlignr>
            || std::is_same_v<T, VecExtend>
            || std::is_same_v<T, VecFpRound>
            || std::is_same_v<T, Popcnt>
            || std::is_same_v<T, Lzcnt>
            || std::is_same_v<T, Tzcnt>
            || std::is_same_v<T, VecBlend>
            || std::is_same_v<T, WriteFlagsPtest>
            || std::is_same_v<T, WriteFlagsPtestYmm>
            || std::is_same_v<T, VecTbl2>
            || std::is_same_v<T, VecAes>
            || std::is_same_v<T, VecAesKeygenAssist>
            || std::is_same_v<T, VecSha>
            || std::is_same_v<T, Bswap>
            || std::is_same_v<T, Crc32c>
            || std::is_same_v<T, VecGather>
            || std::is_same_v<T, LoadVecRegHi>
            || std::is_same_v<T, VecFpFma>
            || std::is_same_v<T, VecFpScalarFma>
            || std::is_same_v<T, X87Load>
            || std::is_same_v<T, X87Pop>
            // Result-bearing but NOT optimizable-pure: the time
            // source must keep its result slot (DCE/CSE handle it
            // separately by never listing it).
            || std::is_same_v<T, Rdtsc>;
    }, op);
}

}  // namespace

// Helper: returns the size that operand `r` must have for op `op`'s
// declared semantics. When `op` says nothing about the size (e.g.
// JumpReg.target may be any 64-bit ref, conservatively treated as
// I64), returns nullopt so the check is skipped.
std::optional<OpSize> required_operand_size(const Op& op, Ref r) {
    return std::visit([&](const auto& x) -> std::optional<OpSize> {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, BinOp>) {
            const bool rhs_is_count = r == x.rhs
                && (x.op == BinOpKind::Shl || x.op == BinOpKind::Shr
                    || x.op == BinOpKind::Sar || x.op == BinOpKind::Rol
                    || x.op == BinOpKind::Ror || x.op == BinOpKind::Rcl
                    || x.op == BinOpKind::Rcr);
            if (rhs_is_count) return std::nullopt;
            if (r == x.lhs || r == x.rhs) return x.size;
        } else if constexpr (std::is_same_v<T, Compare>) {
            if (r == x.lhs || r == x.rhs) return x.size;
        } else if constexpr (std::is_same_v<T, CmpFlags>) {
            if (r == x.lhs || r == x.rhs) return x.size;
        } else if constexpr (std::is_same_v<T, Select>) {
            if (r == x.true_value || r == x.false_value) return x.size;
        } else if constexpr (std::is_same_v<T, StoreReg>) {
            (void)x; (void)r;
        } else if constexpr (std::is_same_v<T, StoreMem>) {
            if (r == x.value) return x.size;
            if (r == x.addr)  return OpSize::I64;
        } else if constexpr (std::is_same_v<T, StoreMemTSO>) {
            if (r == x.value) return x.size;
            if (r == x.addr)  return OpSize::I64;
        } else if constexpr (std::is_same_v<T, X87Store>) {
            if (r == x.value) return OpSize::I64;
        } else if constexpr (std::is_same_v<T, X87Push>) {
            if (r == x.value) return OpSize::I64;
        } else if constexpr (std::is_same_v<T, LoadMem>) {
            if (r == x.addr)  return OpSize::I64;
        } else if constexpr (std::is_same_v<T, LoadMemTSO>) {
            if (r == x.addr)  return OpSize::I64;
        } else if constexpr (std::is_same_v<T, JumpReg>) {
            if (r == x.target) return OpSize::I64;
        } else if constexpr (std::is_same_v<T, CallReg>) {
            if (r == x.target) return OpSize::I64;
        } else if constexpr (std::is_same_v<T, Extend>) {
            if (r == x.value) return x.from_size;
        } else if constexpr (std::is_same_v<T, Truncate>) {
            // Truncate accepts any source size strictly wider than
            // to_size; the validator can't pin it without per-op
            // logic. For now skip the check.
            (void)x; (void)r;
        } else if constexpr (std::is_same_v<T, WriteFlags>) {
            if (r == x.lhs || r == x.rhs) return x.size;
        }
        // ReadFlag / CondJumpFlags consume Flags-typed refs; the
        // integer-size table doesn't apply, so return nullopt to
        // skip the check.
        return std::nullopt;
    }, op);
}

ValidationResult validate(const std::vector<Stmt>& stmts) {
    std::unordered_set<Ref>           defs;
    std::unordered_map<Ref, OpSize>   ref_size;  // F1-IR-015
    defs.reserve(stmts.size());
    ref_size.reserve(stmts.size());

    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];

        // Rule 3/4: result presence must match op category.
        const bool pure = op_is_pure(s.op);
        if (pure && !s.result) {
            return err(ValidationCode::PureLacksResult, i, 0,
                       "pure op has no result ref");
        }
        if (!pure && s.result) {
            return err(ValidationCode::ImpureHasResult, i, *s.result,
                       "side-effecting op has a result ref");
        }

        // Rule 1: every operand ref must be defined by an earlier stmt.
        ValidationResult undef{true, std::nullopt};
        for_each_operand_ref(s.op, [&](Ref r) {
            if (undef.ok && defs.find(r) == defs.end()) {
                undef = err(ValidationCode::UndefinedRef, i, r,
                            "operand ref used before its def");
            }
        });
        if (!undef.ok) return undef;

        // Rule 5 (F1-IR-015): operand sizes must match the consuming
        // op's declared expectation.
        ValidationResult mism{true, std::nullopt};
        for_each_operand_ref(s.op, [&](Ref r) {
            if (!mism.ok) return;
            const auto want = required_operand_size(s.op, r);
            if (!want.has_value()) {
                const auto relaxed_it = ref_size.find(r);
                if (relaxed_it == ref_size.end()) return;
                const bool relaxed_ok = std::visit([&](const auto& op) {
                    using T = std::decay_t<decltype(op)>;
                    if constexpr (std::is_same_v<T, StoreReg>) {
                        return r != op.value
                            || bit_width(relaxed_it->second) >= bit_width(op.size);
                    } else {
                        return true;
                    }
                }, s.op);
                if (!relaxed_ok) {
                    mism = err(ValidationCode::SizeMismatch, i, r,
                               "operand size is too narrow for consuming op");
                }
                return;
            }
            const auto it = ref_size.find(r);
            if (it == ref_size.end()) return;  // size unknown — skip
            if (it->second != *want) {
                mism = err(ValidationCode::SizeMismatch, i, r,
                           "operand size disagrees with consuming op");
            }
        });
        if (!mism.ok) return mism;

        // Rule 2: result ref must be unique. Record its inferred size.
        if (s.result) {
            if (!defs.insert(*s.result).second) {
                return err(ValidationCode::DuplicateResult, i, *s.result,
                           "result ref already defined by an earlier stmt");
            }
            if (auto sz = result_size_static(s.op)) {
                ref_size[*s.result] = *sz;
            }
        }
    }
    return {true, std::nullopt};
}

}  // namespace prisma::ir
