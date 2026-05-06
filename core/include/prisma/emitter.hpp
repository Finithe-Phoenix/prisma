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

    // Flag-setting ALU forms (F1-IR-004 lowering).
    //   adds rd, rn, rm   — like add but also sets NZCV.
    //   ands rd, rn, rm   — like and but also sets N + Z (clears C/V).
    // The destination is mandatory because vixl's macro path doesn't
    // expose a "discard-result" form; the lowerer passes XZR-equivalent
    // (any scratch the lowerer is willing to clobber) when only the
    // flags are wanted. Currently we expose a 3-reg form.
    void adds(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void ands(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);

    // 32-bit W-register ALU forms (F1-BK-010). AArch64 implicitly
    // zero-extends the upper 32 bits when writing through a W-view,
    // so these are the canonical lowering for x86 32-bit ops without
    // an explicit Truncate. Each is a thin wrapper over vixl
    // operating on `WRegister(reg_id)`.
    void add_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void sub_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void and_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void orr_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void eor_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm);
    void mov_w_reg_reg(arm64::Reg rd, arm64::Reg rs);

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

    // sp-relative 64-bit load/store (F1-BK-008). SP is not part of our
    // Reg enum — it shares encoding 31 with XZR and needs vixl's
    // dedicated `sp` singleton. These helpers exist so the Lowerer
    // can spill/reload to a stack-frame slot without teaching the
    // pool allocator about SP.
    void sp_load       (arm64::Reg rd, std::int32_t imm);
    void sp_store      (arm64::Reg rv, std::int32_t imm);

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

    // --- Width adjustment (F1-BK-022) -------------------------------------
    //
    // sign- and zero-extension from a narrower view of `rn` into the
    // 64-bit `rd`. ARM64 has dedicated single-cycle instructions for
    // each width: SXTB / SXTH / SXTW for sign, UXTB / UXTH for zero;
    // 32→64 zero-extension is implicit on any `mov w*` write so the
    // emitter codegens that as `mov wd, wn`. The lowerer drives these.
    void sxtb (arm64::Reg rd, arm64::Reg rn);   //  8 → 64 signed
    void sxth (arm64::Reg rd, arm64::Reg rn);   // 16 → 64 signed
    void sxtw (arm64::Reg rd, arm64::Reg rn);   // 32 → 64 signed
    void uxtb (arm64::Reg rd, arm64::Reg rn);   //  8 → 64 unsigned
    void uxth (arm64::Reg rd, arm64::Reg rn);   // 16 → 64 unsigned

    // 32 → 64 zero-extension via `mov wd, wn`. AArch64 zeroes the upper
    // 32 bits of any 64-bit register written through its W-view.
    void uxtw (arm64::Reg rd, arm64::Reg rn);

    // Truncate Xn to a narrower view in Xd by ANDing with a 64-bit
    // mask. For 32-bit truncation we use `uxtw` (cheaper). For I8/I16
    // the lowerer materialises the mask into a scratch first, and then
    // calls and_().
    // Truncations are not needed when the consumer already reads only
    // the W-view; the lowerer only emits one when the SSA result has
    // to live as a clean narrow value.

    // --- Floating-point ALU (F1-BK-013) -----------------------------------
    //
    // ARM64 has 32 vector/FP registers V0..V31. Each register has
    // sub-views: B (8b), H (16b), S (32b), D (64b), Q (128b). Scalar
    // FP ops use the S- or D-view per `ir::FpSize`.
    //
    // We expose a small `FpReg` enum (V0..V31) and the four hot
    // scalar binops. NEON 128-bit forms come later (F1-BK-012).
    enum class FpReg : std::uint8_t {
        V0 = 0,  V1,  V2,  V3,  V4,  V5,  V6,  V7,
        V8,  V9, V10, V11, V12, V13, V14, V15,
        V16, V17, V18, V19, V20, V21, V22, V23,
        V24, V25, V26, V27, V28, V29, V30, V31,
    };

    void fadd(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void fsub(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void fmul(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void fdiv(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);

    // Materialise an FP constant. The bits are the IEEE-754 encoding
    // of the value (single → low 32 bits, double → all 64). vixl
    // picks fmov-immediate / ldr-literal as appropriate.
    void fmov_imm(FpReg rd, std::uint64_t bits, ir::FpSize sz);

    // F2-IR-008. GPR ↔ FP register transfers (fmov x_d, d_s and inverse).
    //   fmov_v_from_x(rd, rn, sz): writes low lane of V_rd from X_rn,
    //     zero-extending the upper bits of the V register. sz is F32 or F64.
    //   fmov_x_from_v(rd, rn, sz): writes X_rd from low lane of V_rn.
    //     For F32, the upper 32 bits of X_rd are zero-extended.
    void fmov_v_from_x(FpReg rd, arm64::Reg rn, ir::FpSize sz);
    void fmov_x_from_v(arm64::Reg rd, FpReg rn, ir::FpSize sz);

    // F2-IR-016. scvtf / fcvtzs scalar conversions.
    void scvtf(FpReg rd, arm64::Reg rn, ir::OpSize int_sz, ir::FpSize fp_sz);
    void fcvtzs(arm64::Reg rd, FpReg rn, ir::FpSize fp_sz, ir::OpSize int_sz);
    // F2-IR-017. Scalar FP precision convert with upper-preserve.
    // Steps: fcvt scratch.dst, src.src; mov rd, lhs; ins rd.dst[0],
    // scratch.dst[0]. Uses V31 internal scratch.
    void fcvt_scalar_with_upper(FpReg rd, FpReg lhs, FpReg src,
                                ir::FpSize src_sz, ir::FpSize dst_sz);

    // --- 128-bit NEON SIMD (F1-BK-012) ------------------------------------
    //
    // Same V0..V31 register file as scalar FP, viewed as 16 bytes (B16),
    // 8 halfwords (H8), 4 words (S4), or 2 doublewords (D2). The lane
    // size is fixed by the call (`VecLane`); SSE/AVX-style integer
    // SIMD lowers through these.
    //
    // Initial coverage: ALU integer (add/sub/and/or/xor) and a 16-byte
    // load/store. Multiplies, shuffles and reductions land in
    // F1-BK-028+ as we unblock real SSE2 binaries.
    enum class VecLane : std::uint8_t {
        B16 = 0,  // 16 × i8
        H8,       //  8 × i16
        S4,       //  4 × i32
        D2,       //  2 × i64
    };
    void vadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vand_q(FpReg rd, FpReg rn, FpReg rm);  // bitwise: lane-agnostic
    void vorr_q(FpReg rd, FpReg rn, FpReg rm);
    void veor_q(FpReg rd, FpReg rn, FpReg rm);
    void vmul_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);  // F2-IR-013
    // F2-IR-023. Saturating integer arithmetic (B16, H8).
    void vsqadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vuqadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vsqsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vuqsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    // F2-IR-024. Lane-wise min/max (PMINUB / PMAXUB / PMINSW / PMAXSW).
    void vumin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vumax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vsmin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vsmax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    // F2-IR-025. High half of 16x16 multiply, lane-wise (8 H8 results).
    // is_signed selects PMULHW vs PMULHUW. Uses V31 internal scratch.
    void vmulhi_h8(FpReg rd, FpReg rn, FpReg rm, bool is_signed);

    // Packed-FP arithmetic (F2-IR-005). `lane` must be S4 (4×f32) or
    // D2 (2×f64); other values are rejected by an assert.
    void vfadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfmul_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfdiv_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfmin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfmax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vfsqrt_q(FpReg rd, FpReg rn, VecLane lane);

    // F2-IR-006 — scalar SSE FP semantics: result.low = op(rn.low, rm.low),
    // result.upper = rn.upper (untouched). `sz` selects S (32-bit) or D
    // (64-bit) lane. Internally uses V31 as a fixed scratch — it must
    // not appear in the SSA scratch pool (V0..V7).
    void vfadd_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfsub_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfmul_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfdiv_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfmin_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfmax_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);
    void vfsqrt_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz);  // unary; rm is the source.

    // F2-IR-009. Lane-wise integer compare (cmeq / cmgt).
    void vcmeq_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vcmgt_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);

    // F2-IR-010. 4-way 32-bit lane shuffle (PSHUFD). Result lane i =
    // src lane ((control >> (2*i)) & 3). Implemented as four INS
    // (mov v.s[i], src.s[lane]) into V31 scratch, then mov to dst.
    void vshuffle_s4(FpReg rd, FpReg rn, std::uint8_t control);

    // F2-IR-020. SHUFPS / SHUFPD lowering — picks lanes from rn for
    // the first half of the result and from rm for the second half.
    void vshuffle_2src_s4(FpReg rd, FpReg rn, FpReg rm, std::uint8_t control);
    void vshuffle_2src_d2(FpReg rd, FpReg rn, FpReg rm, std::uint8_t control);

    // F2-IR-022. Per-lane insert/extract to/from a GPR.
    void vins_lane_from_w(FpReg rd, FpReg rn, std::uint8_t lane_idx,
                          arm64::Reg w_value, VecLane lane);
    void vumov_w_from_lane(arm64::Reg w_dst, FpReg rn,
                           std::uint8_t lane_idx, VecLane lane);

    // F2-IR-027. PMOVMSKB: byte MSB extraction → 16-bit mask in w_dst.
    void vmask_msb_b16(arm64::Reg w_dst, FpReg rn);

    // F2-IR-011. NEON zip1/zip2 (interleave low/high lanes).
    void vzip1_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);
    void vzip2_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane);

    // F2-IR-012. Per-lane shift by immediate (PSLLW/D/Q-style).
    // For ShiftL `count` may be >= lane bits; the lowerer clamps to
    // lane width via the SSE rule (count >= bits → lane = 0).
    void vshl_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane);
    void vushr_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane);
    void vsshr_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane);

    // F2-IR-014. Whole-register byte shift (PSLLDQ / PSRLDQ).
    // count >= 16 → result is zero.
    void vshlb_imm_q(FpReg rd, FpReg rn, std::uint8_t count);
    void vshrb_imm_q(FpReg rd, FpReg rn, std::uint8_t count);

    // 128-bit aligned load/store from [base]. `base` is a 64-bit X-reg
    // already holding the effective address.
    void vld1_q(FpReg rd, arm64::Reg base);
    void vst1_q(FpReg rs, arm64::Reg base);

    // [base, #imm] forms — used by SSE2 lowering to read/write the
    // CpuStateFrame's xmm[] table. The immediate is signed 32-bit;
    // vixl picks the cheapest encoding or falls back to a scratch.
    void vld1_q_offset(FpReg rd, arm64::Reg base, std::int32_t imm);
    void vst1_q_offset(FpReg rs, arm64::Reg base, std::int32_t imm);

    // --- Memory fences (F1-BK-023) ----------------------------------------
    //
    // ARM64 DMB / DSB barrier emission. In our IR:
    //   FenceKind::Mfence → dmb ish        (full barrier)
    //   FenceKind::Lfence → dmb ishld      (load-only)
    //   FenceKind::Sfence → dmb ishst      (store-only)
    // The TSO-adaptive pass (Pillar 3) can drop a fence proven
    // redundant under a region's restricted memory model.
    enum class BarrierKind : std::uint8_t { Ish, IshLd, IshSt };
    void dmb(BarrierKind k);

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

    // Compare-and-branch on a 64-bit register without touching NZCV.
    // `cbnz(r, label)` branches when r != 0; `cbz` when r == 0. These
    // are how we lower `CondJump{cond_ref, true, false}` where the
    // condition is an SSA Ref (a 0/1 value materialised by Compare),
    // not a flag. Range is ±1 MiB; vixl picks the encoding.
    void cbnz(arm64::Reg r, Label label);
    void cbz (arm64::Reg r, Label label);

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
