// prisma/lowering.hpp — IR → ARM64 lowering.
//
// First real lowering. Replaces the ad-hoc "trivial lowering" that lived
// inside test_e2e.cpp with a proper Lowerer class that consumes IR
// statements and drives the Emitter.
//
// Scope for this version (Fase 0):
//
//   Supported IR ops:
//     Constant, LoadReg, StoreReg, BinOp (Add/Sub/And/Or/Xor/Shl/Shr/Sar),
//     Return.
//
//   Rejected IR ops (returns LowerError::Unsupported):
//     Compare, Jump, CondJump, LoadMem, StoreMem, LoadMemTSO, StoreMemTSO.
//     These land in subsequent sessions (flags, CFG, memory).
//
// Register allocation strategy:
//
//   Guest GPRs rax..r15 are mapped to host x10..x25 per the Fase 1 fixed
//   mapping (`arm64::host_reg_for`). Those are "pinned" — StoreReg writes
//   the host register directly, and LoadReg reads from it.
//
//   SSA values produced by Constant / LoadReg / BinOp live in a pool of
//   scratch registers x0..x9 (10 registers). Each new Ref allocates the
//   next scratch; we don't yet free scratches (short basic blocks only).
//   If a block needs more than 10 scratches, lowering fails with
//   LowerError::OutOfScratchRegs. Constrained register allocation lands
//   in Fase 2.

#pragma once

#include <cstdint>
#include <span>
#include <string>
#include <unordered_map>

#include "prisma/arm64_encoding.hpp"  // for arm64::Reg
#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"

namespace prisma::backend {

enum class LowerError {
    UnsupportedOp,       // IR op we do not lower yet (e.g. Compare, Jump).
    OutOfScratchRegs,    // More than 10 simultaneously-live SSA values.
    DanglingRef,         // A statement references a Ref that was never defined.
};

struct LowerResult {
    bool success{true};
    LowerError error{LowerError::UnsupportedOp};
    std::string message{};  // human-readable context when !success
};

class Lowerer {
public:
    explicit Lowerer(Emitter& emitter) : emitter_(emitter) {}

    // Lower the given statement list onto the underlying Emitter. Returns
    // a result indicating success or the first error encountered. After
    // an error the Emitter state is undefined — throw it away.
    [[nodiscard]] LowerResult lower(std::span<const ir::Stmt> stmts);

    // For tests: how many scratch registers have been allocated so far.
    [[nodiscard]] unsigned scratch_used() const noexcept { return next_scratch_; }

private:
    Emitter& emitter_;

    // Map IR Ref → host scratch register.
    std::unordered_map<ir::Ref, arm64::Reg> ref_to_scratch_;

    // Next scratch register to hand out (x0..x9).
    unsigned next_scratch_{0};

    [[nodiscard]] LowerResult lower_stmt(const ir::Stmt& s);

    // Allocate the next scratch register. Returns false on exhaustion.
    [[nodiscard]] bool allocate_scratch(ir::Ref ref, arm64::Reg& out);

    // Look up the host register that currently holds an SSA Ref's value.
    // Returns false if the Ref was never bound.
    [[nodiscard]] bool reg_of(ir::Ref ref, arm64::Reg& out) const;
};

}  // namespace prisma::backend
