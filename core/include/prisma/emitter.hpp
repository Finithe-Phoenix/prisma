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
    // Rotate-left is not a native ARM64 op. `rol(rd, rn, rm, tmp)`
    // implements it as `neg tmp, rm; ror rd, rn, tmp` — the caller
    // supplies the scratch register so the Lowerer keeps its
    // register allocation explicit. F1-BK-014.
    void rol (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm, arm64::Reg tmp);
    void neg (arm64::Reg rd, arm64::Reg rn);                 // neg xd, xn (alias of sub xd, xzr, xn)

    // Bit-manipulation ops (F1-BK-015). All are 2-register ARM64 primitives.
    //   clz — count leading zeros
    //   cls — count leading sign bits (one fewer than clz for signed ints)
    //   rbit — bit-reverse (useful when building a 64-bit clz-of-trailing-zeros)
    void clz  (arm64::Reg rd, arm64::Reg rn);
    void cls  (arm64::Reg rd, arm64::Reg rn);
    void rbit (arm64::Reg rd, arm64::Reg rn);

    // Multi-output mul/div (F1-BK-011).
    //
    // x86 IMUL / MUL write a 128-bit result split across RDX:RAX. On ARM64
    // we compute that in two steps: the low 64 bits via `mul`, the high
    // 64 bits via `umulh` (unsigned) or `smulh` (signed).
    //
    // x86 DIV / IDIV read RDX:RAX and write quotient to RAX, remainder to
    // RDX. On ARM64 there is no 128/64 divide; our lowering will narrow
    // to 64/64 for MVP and emit the pair `udiv rq, rn, rm` + `msub rr,
    // rq, rm, rn` (remainder = n - q*m). `msub(rd, rn, rm, ra)` gives
    // `rd = ra - rn*rm` which is the canonical AArch64 idiom.
    void umulh(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // unsigned high 64
    void smulh(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // signed   high 64
    void udiv (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // unsigned 64/64
    void sdiv (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);  // signed   64/64
    void msub (arm64::Reg rd, arm64::Reg rn, arm64::Reg rm,
               arm64::Reg ra);                                 // ra - rn*rm

    // Atomic RMW via exclusive-monitor pair (F1-BK-016).
    //
    // `ldxr(rd, raddr, size)` is a load-exclusive: grabs the value and
    // reserves the cache line. `stxr(rs, rv, raddr, size)` is a
    // store-exclusive: writes `rv` to `[raddr]` iff the reservation
    // still holds, storing 0 (success) or 1 (failure) into `rs`. The
    // acquire / release variants (`ldaxr` / `stlxr`) add C++11-style
    // memory ordering for TSO-safe use.
    //
    // Typical atomic CAS loop emitted by the lowerer:
    //   retry:
    //     ldaxr  r_current, [r_addr]        (size)
    //     cmp    r_current, r_expected
    //     b.ne   fail
    //     stlxr  r_status, r_new, [r_addr]  (size)
    //     cbnz   r_status, retry
    //   fail:
    void ldxr  (arm64::Reg rd, arm64::Reg raddr, ir::OpSize size);
    void stxr  (arm64::Reg rs, arm64::Reg rv, arm64::Reg raddr, ir::OpSize size);
    void ldaxr (arm64::Reg rd, arm64::Reg raddr, ir::OpSize size);
    void stlxr (arm64::Reg rs, arm64::Reg rv, arm64::Reg raddr, ir::OpSize size);

    // LSE atomics (F1-BK-017). One-instruction CAS + fetch-add.
    // Requires `host_features().feat_lse`; the lowerer should check
    // before emitting. `casal` is the sequentially-consistent variant
    // (acquire+release). `ldaddal` is LSE fetch-add with the same
    // ordering.
    //
    //   casal(rs, rt, raddr, size):
    //     if *raddr == rs: *raddr = rt
    //     rs = *raddr (old value, always)
    //
    //   ldaddal(rs, rt, raddr, size):
    //     rt = *raddr; *raddr += rs
    void casal   (arm64::Reg rs, arm64::Reg rt, arm64::Reg raddr, ir::OpSize size);
    void ldaddal (arm64::Reg rs, arm64::Reg rt, arm64::Reg raddr, ir::OpSize size);

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

    // [base, #imm] forms for the 64-bit load/store only. Used by the
    // block prologue/epilogue to read and write guest GPRs from a
    // CpuStateFrame*. The immediate is signed; the vixl MacroAssembler
    // picks the best encoding or falls back to a scratch-based form.
    void load_offset   (arm64::Reg rd, arm64::Reg rbase, std::int32_t imm);
    void store_offset  (arm64::Reg rv, arm64::Reg rbase, std::int32_t imm);

    // Stack push/pop as register pairs (the ARM64 idiom for prologue /
    // epilogue sequences). `push_pair(r1, r2)` emits
    //   stp r1, r2, [sp, #-16]!   ; sp -= 16, store both
    // `pop_pair(r1, r2)` emits
    //   ldp r1, r2, [sp], #16     ; load both, sp += 16
    // Used by the Translator to save / restore AAPCS64 callee-saved
    // registers that we clobber (x19..x26 + x27 state ptr + x29/x30).
    void push_pair (arm64::Reg r1, arm64::Reg r2);
    void pop_pair  (arm64::Reg r1, arm64::Reg r2);

    // ret xN  (default x30)
    void ret(arm64::Reg rn = arm64::Reg::X30);

    // --- Label management (F1-BK-005) --------------------------------------
    //
    // Opaque handle to a vixl label. A Label is:
    //   1. created via `create_label()`,
    //   2. referenced by zero or more branches (forward or backward),
    //   3. bound exactly once via `bind()` to set its target PC to the
    //      current emit position.
    //
    // Emit unresolved forward branches before `bind()`; the vixl
    // MacroAssembler records them as fix-ups and rewrites the
    // instructions when the label is bound. `finalize()` asserts that
    // every label has been bound.
    //
    // Labels outlive the statement list being lowered — one Label per
    // basic block is the expected usage pattern for CFG lowering
    // (F1-BK-006).
    struct Label {
        std::size_t id{0};  // 0 is the sentinel "not a label"
    };

    [[nodiscard]] Label create_label();

    // Place `label` at the current emit position. Must be called exactly
    // once per label, before finalize().
    void bind(Label label);

    // Unconditional branch to `label`. The vixl MacroAssembler picks
    // between a direct `b` and a longer veneer depending on the
    // expected distance.
    void branch(Label label);

    // Conditional branch: `b.<cc> label`. Reads the NZCV set by the
    // most-recent flag-producing instruction (CmpFlags in our IR).
    void branch_cc(Label label, ir::CondCode cc);

    // --- Literal pool management (F1-BK-018) -------------------------------
    //
    // vixl accumulates literals (64-bit immediates used by `ldr r, =#imm`
    // forms) in a per-MacroAssembler pool. The pool auto-flushes when
    // its size or the distance-to-first-use would exceed the load's
    // reach (±1 MiB on AArch64), wrapping the literals in a branch so
    // execution skips over them.
    //
    // Most Prisma code doesn't need to touch these — mov_imm64 uses
    // movz/movk rather than ldr-literal. Expose them anyway so block
    // boundaries (F1-BK-020 prologue / F1-BK-021 epilogue) can flush
    // deterministically and so tests can assert pool size stays at 0.
    //
    // `flush_literal_pool()` emits any pending literals with a branch
    // veneer so fallthrough code is unaffected. Safe to call at any
    // valid instruction boundary.
    [[nodiscard]] std::size_t literal_pool_size() const noexcept;
    void flush_literal_pool();

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
