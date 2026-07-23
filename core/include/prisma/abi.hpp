// prisma/abi.hpp — Prisma ↔ host AAPCS64 calling-convention helpers.
//
// Block prologue / epilogue. Every Translator-produced block is invoked
// as a host function with the signature
//
//     uint64_t block(CpuStateFrame* state);
//        entry: x0 = state pointer
//        exit:  x0 = next guest PC
//
// On entry we save the AAPCS64 callee-saved registers we will clobber:
// x19..x27 plus x30. The existing six-pair frame is 96 bytes and remains
// 16-byte aligned. x28 and x29 are deliberately never modified by generated
// code; their two saved lanes are therefore available as bounded translator
// spill slots while their original live register values remain untouched.
//
// `emit_block_prologue(em)` emits those six `stp` pairs, then moves the
// state pointer from x0 into the pinned holder (x27), then loads each guest
// GPR from `state->gpr[i]` into its pinned host register.
//
// `emit_block_epilogue_and_ret(em)` stores the pinned guest registers and
// restores x19..x27 and x30. The x28/x29 stack lanes are discarded into
// caller-saved x8/x9 because generated code preserves the live x28/x29
// registers directly.
//
// These helpers exist as a separate API (rather than living inside
// translator.cpp) so future inline guest-CALL sites — which need the
// same callee-saved discipline — can reuse them. F1-BK-009.
//
// `kStatePtrReg` is the host register that holds the state pointer
// across the body. It is one of the saved callee-saved regs so the
// body can read it freely without re-loading from the frame.

#pragma once

#include <cstddef>
#include <cstdint>

#include "prisma/arm64_encoding.hpp"
#include "prisma/emitter.hpp"

namespace prisma::backend::abi {

// Host register that holds the state pointer (CpuStateFrame*) across
// the block body. Must be one of the AAPCS64 callee-saved registers
// that the prologue saves. Currently x27.
constexpr arm64::Reg kStatePtrReg = arm64::Reg::X27;

// Number of register pairs the prologue saves (and the epilogue consumes).
// Six pairs = 96 bytes, 16-byte aligned.
constexpr unsigned kCalleeSavedPairCount = 6;

// The saved x28 and x29 lanes are not needed for restoration because
// generated code never writes those registers. Reuse their frame locations
// as two 64-bit spill slots for Translator-owned blocks.
constexpr unsigned kTranslatorSpillSlotCount = 2;
constexpr std::int32_t kTranslatorSpillSlotBaseOffset = 72;

// Emit the full block prologue: 6 stp pairs + mov x27, x0 + 16 ldr
// loading the guest GPRs from the state frame.
void emit_block_prologue(Emitter& em);

// Emit the full block epilogue: 16 str storing the pinned host regs
// back to the state frame + frame restoration + ret.
void emit_block_epilogue_and_ret(Emitter& em);

// Patchable tail epilogue for direct block chaining.
//
// Entry: x0 holds the guest next PC, same as the normal epilogue.
// Before the patch site, this epilogue stores pinned GPRs, preserves
// next_pc in x1, moves the CpuStateFrame* back into x0, and restores
// all callee-saved state. The patch site itself is a single AArch64
// unconditional `b fallback`. Unpatched execution falls through to
// `mov x0, x1; ret`, preserving the public block ABI. Once patched,
// the branch can jump directly to a successor block whose entry ABI is
// x0 = CpuStateFrame*.
struct PatchableTailEpilogue {
    std::size_t branch_offset{0};
    std::size_t fallback_offset{0};
};

[[nodiscard]] PatchableTailEpilogue emit_block_epilogue_patchable_tail(
    Emitter& em);

}  // namespace prisma::backend::abi
