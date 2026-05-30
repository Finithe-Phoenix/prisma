// prisma/abi.hpp — Prisma ↔ host AAPCS64 calling-convention helpers.
//
// Block prologue / epilogue. Every Translator-produced block is invoked
// as a host function with the signature
//
//     uint64_t block(CpuStateFrame* state);
//        entry: x0 = state pointer
//        exit:  x0 = next guest PC
//
// On entry we must save the AAPCS64 callee-saved registers we will
// clobber: x19..x28 (general callee-saved that we use to pin guest
// GPRs r8..r15 and to hold the state pointer), x29 (frame pointer)
// and x30 (link register). That is six register pairs, 96 bytes of
// stack frame, 16-byte aligned as required.
//
// `emit_block_prologue(em)` emits those six `stp` pairs (each a
// pre-indexed `stp r1, r2, [sp, #-16]!`), then moves the state
// pointer from x0 into the pinned holder (x27), then loads each
// guest GPR from `state->gpr[i]` into its pinned host register.
//
// `emit_block_epilogue_and_ret(em)` does the reverse: stores the
// pinned host regs back to the state frame, pops the six register
// pairs in reverse push order, and emits a final `ret`.
//
// These helpers exist as a separate API (rather than living inside
// translator.cpp) so future inline guest-CALL sites — which need the
// same callee-saved discipline — can reuse them. F1-BK-009.
//
// `kStatePtrReg` is the host register that holds the state pointer
// across the body. It is one of the saved callee-saved regs so the
// body can read it freely without re-loading from the frame.

#pragma once

#include "prisma/arm64_encoding.hpp"
#include "prisma/emitter.hpp"

namespace prisma::backend::abi {

// Host register that holds the state pointer (CpuStateFrame*) across
// the block body. Must be one of the AAPCS64 callee-saved registers
// that the prologue saves. Currently x27.
constexpr arm64::Reg kStatePtrReg = arm64::Reg::X27;

// Number of register pairs the prologue saves (and the epilogue
// restores). Six = 96 bytes of stack frame, 16-byte aligned.
constexpr unsigned kCalleeSavedPairCount = 6;

// Emit the full block prologue: 6 stp pairs + mov x27, x0 + 16 ldr
// loading the guest GPRs from the state frame.
void emit_block_prologue(Emitter& em);

// Emit the full block epilogue: 16 str storing the pinned host regs
// back to the state frame + 6 ldp pairs (reverse order) + ret.
void emit_block_epilogue_and_ret(Emitter& em);

}  // namespace prisma::backend::abi
