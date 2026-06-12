// prisma/cfg.hpp — flat decoded IR → control-flow graph (F1-IR-023).
//
// The decoder emits a flat std::vector<Stmt>. Most analyses (and the
// future register allocator that respects basic-block boundaries) want
// to see the same statements grouped into BasicBlocks with explicit
// edges between them. `build_cfg` does that without rewriting any of
// the original statement payloads:
//
//   * Block 0 begins at the first statement.
//   * Every statement after a terminator (Return, JumpRel, JumpReg,
//     CondJumpRel, CallRel, CallReg, RetAdjusted, Trap, Cpuid,
//     InlineAsm) starts a new block.
//   * Block IDs are assigned in source order; entry = 0.
//
// Edges are implicit via the original guest-PC-based control-flow ops
// (`JumpRel`, `CondJumpRel`, etc.). A second pass — F1-IR-024
// dominator analysis — can lift those into explicit `Jump`/`CondJump`
// over block IDs once the decoder supplies a complete address-to-id
// map for the in-region targets.
//
// `build_cfg` is pure: same input → same output. No allocation beyond
// the returned vectors.

#pragma once

#include <span>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

// Returns true if `op` is a block terminator. Defined here (rather
// than buried inside the cpp) so passes that want to do their own
// block-boundary reasoning can share the predicate.
[[nodiscard]] bool is_terminator(const Op& op) noexcept;

// Build a Function whose blocks group `stmts` between terminators.
// The Function's `entry` is always 0 (flat decoded input always
// starts at block 0). Block IDs are 0..N-1 in source order.
[[nodiscard]] Function build_cfg(std::span<const Stmt> stmts);

}  // namespace prisma::ir
