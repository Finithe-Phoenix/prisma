// prisma/emitter.hpp — ARM64 code emitter built on vixl.
//
// This is the thin Prisma-facing API over vixl's MacroAssembler. Day 1 it
// exposes just enough to lower our 12-opcode IR MVP to bytes. As the IR
// grows (Fase 1-2), this API grows along with it.
//
// Invariants:
//   * One `Emitter` per translation unit (basic block or function). Not
//     thread-safe — each thread must own its Emitter.
//   * `finalize()` must be called before reading `code_bytes()`. After
//     finalize the Emitter is read-only.
//   * All emitted code targets 64-bit ARM64. 32-bit W-regs and SIMD come
//     later.

#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

#include "prisma/arm64_encoding.hpp"  // for Reg enum & host_reg_for
#include "prisma/ir.hpp"              // for ir::CondCode

namespace prisma::backend {

class Emitter {
public:
    Emitter();
    ~Emitter();

    Emitter(const Emitter&) = delete;
    Emitter& operator=(const Emitter&) = delete;
    Emitter(Emitter&&) = delete;
    Emitter& operator=(Emitter&&) = delete;

    // --- instruction-level API (grows as IR grows) ---

    // movz Xd, #imm16, lsl #(hw*16)
    void movz(arm64::Reg rd, std::uint16_t imm16, unsigned hw = 0);

    // mov Xd, #imm64 (may emit up to 4 instructions: movz + movk*)
    void mov_imm64(arm64::Reg rd, std::uint64_t imm);

    // mov Xd, Xs  (register-to-register copy)
    void mov_reg_reg(arm64::Reg rd, arm64::Reg rs);

    // --- 64-bit ALU, 3-register form (AArch64 canonical for BinOp lowering) ---
    // Each maps 1-1 to our IR BinOpKind on OpSize::I64.
    void add (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Add
    void sub (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Sub
    void mul (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Mul
    void and_(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // And  (trailing _: `and` is a keyword)
    void orr (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Or   (AArch64 spells it `orr`)
    void eor (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Xor  (AArch64 spells it `eor`)

    // Shifts: shift amount comes from low 6 bits of rm (matches x86 behaviour
    // for 64-bit shifts after we mask in the IR/lowerer).
    void lsl (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Shl
    void lsr (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Shr
    void asr (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Sar
    void ror (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // Ror (ARM64 native)
    // Rol is not a native ARM64 op. Callers that want it must either:
    //   * lower via `neg tmp, count; ror rd, rn, tmp` at a higher layer, or
    //   * use constant folding when the count is known at compile time.
    // The Lowerer currently rejects Rol with UnsupportedOp for MVP.
    void neg (arm64::Reg rd, arm64::Reg rn);                 // neg xd, xn (alias of sub xd, xzr, xn)

    // Compare (SUBS with discard) + materialise 0/1 from flags.
    //   cmp(xn, xm)                — sets NZCV.
    //   cset(rd, CondCode)         — rd = 1 if condition holds, else 0.
    // Together these lower IR `Compare{cc, lhs, rhs, size}` to
    //   cmp xlhs, xrhs
    //   cset xresult, <arm-cond(cc)>
    void cmp  (arm64::Reg rn, arm64::Reg rm);
    void cset (arm64::Reg rd, ir::CondCode cc);

    // csel xd, xn_true, xn_false, <cc>
    //   xd = flag_true ? xn_true : xn_false
    // Used by CondJumpRel lowering to pick between taken / fallthrough
    // target PCs without emitting a real branch. movz/movk don't touch
    // flags, so the NZCV set by a preceding cmp stays valid through
    // the two target-load instructions.
    void csel (arm64::Reg rd, arm64::Reg rn_true, arm64::Reg rn_false,
               ir::CondCode cc);

    // --- Memory access -----------------------------------------------------
    //
    // Plain load / store (no barriers):
    //   load(xd, [xaddr], size)            — LoadMem
    //   store(xv, [xaddr], size)           — StoreMem
    //
    // TSO (x86 memory model) load / store:
    //   load_acquire(xd, [xaddr], size)    — LoadMemTSO
    //   store_release(xv, [xaddr], size)   — StoreMemTSO
    //
    // Size dispatch matches the IR:
    //   I8  → ldrb / strb  (byte, zero-extended result for loads)
    //   I16 → ldrh / strh  (halfword)
    //   I32 → ldr w / str w (word; zero-extends to 64-bit)
    //   I64 → ldr x / str x
    // Acquire/release variants use ldar*/stlr* which ARM ARM B2 documents
    // as equivalent to the cheap LRCPC path on capable CPUs and to a
    // post-DMB fence otherwise.
    //
    // The address is assumed to be a full 64-bit register holding the
    // effective address; we do not yet emit [base + offset] forms from
    // the Emitter API (callers compute the address in a scratch first,
    // matching what the decoder already does).
    void load          (arm64::Reg rd, arm64::Reg raddr, ir::OpSize size);
    void store         (arm64::Reg rv, arm64::Reg raddr, ir::OpSize size);
    void load_acquire  (arm64::Reg rd, arm64::Reg raddr, ir::OpSize size);
    void store_release (arm64::Reg rv, arm64::Reg raddr, ir::OpSize size);

    // ret xN  (default x30)
    void ret(arm64::Reg rn = arm64::Reg::X30);

    // --- lifecycle ---

    // Finalize the buffer: resolve labels, emit any literal pool, flush
    // internal state. Call exactly once before reading bytes.
    void finalize();

    // Raw code bytes. Only valid after finalize().
    [[nodiscard]] std::span<const std::uint8_t> code_bytes() const noexcept;

    // Disassemble the emitted code (after finalize). One instruction per
    // line. Primarily for debugging / test failure messages.
    [[nodiscard]] std::string disassemble() const;

private:
    struct Impl;             // pimpl — hides vixl headers from our API
    Impl* impl_;
};

}  // namespace prisma::backend
