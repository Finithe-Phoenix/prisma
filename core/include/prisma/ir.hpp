// prisma/ir.hpp — Prisma IR data structures (C++ runtime).
//
// This header mirrors the Lean 4 specification in `ir-spec/PrismaIR/Syntax.lean`.
// The Lean spec is authoritative; any discrepancy between the two is a bug in
// this header, not in the spec. RFC 0001 establishes this relationship.
//
// Status: early (Fase 0 week 1). Opcodes match the 12-op MVP from the spec.
// The full x86_64 ISA coverage arrives incrementally across Fase 1-2.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <variant>
#include <vector>

namespace prisma::ir {

// ---------------------------------------------------------------------------
// Basic enums — mirror `ir-spec/PrismaIR/Syntax.lean`.
// ---------------------------------------------------------------------------

enum class Gpr : std::uint8_t {
    Rax = 0, Rcx, Rdx, Rbx,
    Rsp,     Rbp, Rsi, Rdi,
    R8,      R9,  R10, R11,
    R12,     R13, R14, R15,
};

constexpr std::size_t kGprCount = 16;

enum class OpSize : std::uint8_t {
    I8  = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
};

// Width of an operand size in bits.
[[nodiscard]] constexpr std::uint32_t bit_width(OpSize s) noexcept {
    switch (s) {
        case OpSize::I8:  return 8;
        case OpSize::I16: return 16;
        case OpSize::I32: return 32;
        case OpSize::I64: return 64;
    }
    return 0;  // unreachable; compiler should warn if a case is added.
}

// Mask a 64-bit value to the given size.
[[nodiscard]] constexpr std::uint64_t mask_to_size(std::uint64_t v, OpSize s) noexcept {
    switch (s) {
        case OpSize::I8:  return v & 0xFFULL;
        case OpSize::I16: return v & 0xFFFFULL;
        case OpSize::I32: return v & 0xFFFF'FFFFULL;
        case OpSize::I64: return v;
    }
    return v;
}

enum class BinOpKind : std::uint8_t {
    Add = 0, Sub, Mul,
    And,     Or,  Xor,
    Shl,     Shr, Sar,
    Rol,     Ror,
    Rcl,     Rcr,
    UMulHi,  SMulHi,    // F2-BK-007 — high half of unsigned/signed
                        //              N×N multiply (for x86 MUL/IMUL
                        //              writeback to rdx).
    UDiv,    SDiv,      // F2-BK-007 — quotient (rax for x86 DIV/IDIV).
    UMod,    SMod,      // F2-BK-007 — remainder (rdx for x86 DIV/IDIV).
    Pdep,    Pext,      // BMI2 bit deposit/extract software-lowered ops.
};

enum class CondCode : std::uint8_t {
    Eq = 0, Ne,
    Ult,    Ule, Ugt, Uge,  // unsigned
    Slt,    Sle, Sgt, Sge,  // signed
    Cc,    Nc,              // carry clear / carry set pair
    Ov,   NoOv,             // overflow set / overflow clear pair
    Mi,   Pl,               // sign-set / sign-clear pair
};

// ---------------------------------------------------------------------------
// Ref — SSA value identifier.
// ---------------------------------------------------------------------------
//
// In the Lean spec `Ref := Nat`. Here it is a 32-bit unsigned offset into the
// containing function's statement list, matching the FEX OrderedNode design
// where references are compressed to 32-bit offsets for memory compactness
// (research_notes.md, 2026-04-19).
//
// A value of `kInvalidRef` means "not yet assigned" / "unused". Code that
// reads an invalid ref is buggy.

using Ref = std::uint32_t;
inline constexpr Ref kInvalidRef = 0xFFFF'FFFFu;

// ---------------------------------------------------------------------------
// Op — IR opcodes and their operands.
// ---------------------------------------------------------------------------
//
// Each variant's public fields match the corresponding `Op` constructor in
// the Lean spec. We use std::variant so a `Stmt` owns its Op by value, which
// keeps data-flow analysis cache-friendly.

struct Constant { std::uint64_t value; OpSize size; };

struct LoadReg  { Gpr reg; OpSize size; };
struct StoreReg { Gpr reg; Ref value; OpSize size; };

// Segment registers. x86-64 ignores the base of CS/DS/ES/SS for normal
// data accesses, but FS and GS still carry meaningful TLS pointers
// (Win64 stores TEB at GS:[0x30], Linux glibc stores TLS at FS).
enum class SegmentReg : std::uint8_t {
    Es = 0, Cs, Ss, Ds, Fs, Gs,
};

// Yields the 64-bit base address of the named segment as an SSA value.
// Pure: DCE-eligible. Lowering reads the base from a runtime-supplied
// table so changing the TEB pointer at run time only requires updating
// that table, not invalidating cached translations.
struct LoadSegBase { SegmentReg seg; };

struct BinOp    { BinOpKind op; Ref lhs; Ref rhs; OpSize size; };

struct Compare  { CondCode cc; Ref lhs; Ref rhs; OpSize size; };

// Conditional select (ternary): `result = (cond ? true_value : false_value)`.
// This is useful for CMOVcc / SETcc lowering in the current MVP slice and
// maps cleanly to ARM64 `csel`.
struct Select {
    CondCode cc;
    Ref true_value;
    Ref false_value;
    OpSize size;
};

struct LoadMem     { Ref addr; OpSize size; };
struct StoreMem    { Ref addr; Ref value; OpSize size; };
struct LoadMemTSO  { Ref addr; OpSize size; };
struct StoreMemTSO { Ref addr; Ref value; OpSize size; };

struct Jump     { std::uint32_t target_block; };
struct CondJump { Ref cond; std::uint32_t if_true; std::uint32_t if_false; };
struct Return   {};

// ---- Guest-PC-based control flow (MVP, no basic-block index yet) -------
//
// CmpFlags is a side-effecting op: it sets the x86 flags bank implicitly
// (no SSA result ref). Lowering emits an ARM64 `cmp` that leaves NZCV in
// a state CondJumpRel can immediately consume. The invariant is that
// CondJumpRel must follow a CmpFlags (or another flag-setting op in the
// future) with no flag-clobbering op in between. The Lowerer enforces
// this; the decoder produces IR that respects it by construction.
//
// JumpRel / CondJumpRel terminate the current block and cause the block
// to return `target_guest_pc` (taken) or `fallthrough_guest_pc` (not
// taken, cond only) to the caller in the convention x0. The dispatcher
// (Fase 1+) uses that value to resume translation at the next PC.

struct CmpFlags {
    Ref   lhs;
    Ref   rhs;
    OpSize size;
};

struct JumpReg {
    Ref target;
};

struct JumpRel {
    std::uint64_t target_guest_pc;
};

struct CondJumpRel {
    CondCode      cc;
    std::uint64_t target_guest_pc;
    std::uint64_t fallthrough_guest_pc;
};

// ---- Calls and returns -------------------------------------------------
//
// Side-effecting terminators that participate in the guest's call stack.
// CallRel pushes return_guest_pc onto the guest stack and transfers to
// target_guest_pc. CallReg does the same with an indirect target held
// in an SSA Ref. RetAdjusted pops the return address (and optionally
// `pop_bytes` extra stack bytes for stdcall callees) and transfers to
// the popped address. The dispatcher actually performs the transfer;
// the lowered code returns the next guest PC in x0 like other
// terminators.
//
// Both CallRel and CallReg carry the absolute return PC so the tail-call
// optimiser can fold a `CallRel; RetAdjusted{0}` pair into a `JumpRel`
// without losing any guest information.

struct CallRel {
    std::uint64_t target_guest_pc;
    std::uint64_t return_guest_pc;
};

struct CallReg {
    Ref           target;
    std::uint64_t return_guest_pc;
};

struct RetAdjusted {
    std::uint64_t pop_bytes;  // extra bytes popped after the return address.
};

// ---- Architectural placeholders ---------------------------------------
//
// CPUID, SYSCALL and INT3/UD2 (Trap) currently lower to placeholder
// machine code (zeroed registers / halt sentinel). They exist as
// first-class IR ops so the decoder can produce them today and the
// lowerer can swap to real semantics later without changing decoder
// output. See `core/src/backend/lowering.cpp` for the current shapes.

struct Cpuid   {};
struct Syscall {};

// ---- Width adjustment -------------------------------------------------
//
// Extend{value, from_size, to_size, signed} grows a narrower SSA value
// to a wider one. `signed=true` fills the upper bits with the sign bit
// (sign-extension, ARM64 `sxtw` / `sxth` / `sxtb` family); `signed=false`
// fills with zero (zero-extension, the AArch64 default for sub-64-bit
// register writes via `mov w*` or `uxt*`).
//
// Truncate{value, to_size} narrows a wider SSA value to a smaller one.
// On ARM64 this is implicit when storing or computing in a `W` register
// since the host hardware zeroes the upper 32 bits, so the lowerer
// often emits no instruction at all and just renames the register at
// the next use. Pure: DCE-eligible.
//
// Both ops have a single Ref operand and produce a Ref result. They
// participate in const folding (F1-PS-010, planned).

struct Extend {
    Ref    value;
    OpSize from_size;
    OpSize to_size;
    bool   is_signed;
};

struct Truncate {
    Ref    value;
    OpSize to_size;
};

// ---- Memory fences ----------------------------------------------------
//
// Explicit x86 LFENCE / SFENCE / MFENCE. Lowered to ARM64 DMB ISH /
// DMB ISHST / DMB ISHLD respectively. The pillar-3 TSO-adaptive pass
// can drop fences proven redundant under a region's restricted memory
// model.

enum class FenceKind : std::uint8_t {
    Mfence = 0,  // full barrier   → DMB ISH
    Lfence,      // load barrier   → DMB ISHLD
    Sfence,      // store barrier  → DMB ISHST
};

struct Fence {
    FenceKind kind;
};

// ---- Diagnostics / cache keying --------------------------------------
//
// `GuestPc{pc}` is a pseudo-op that records the original guest program
// counter for debugging, cache lookup, and (later) profile-guided
// inlining. It has no side effects in the IR semantics — the lowerer
// emits no code for it. Validators and serializers treat it as a
// 9-byte tag (1 byte op tag + 8 byte little-endian PC).
//
// Decoders may insert one at the start of every guest instruction to
// preserve original-source mapping across passes; passes preserve
// these in place but never duplicate them.

struct GuestPc {
    std::uint64_t pc;
};

// ---- Flags (F1-IR-003 / F1-IR-004 / F1-IR-005 / F1-IR-007) -----------
//
// SSA-tracked x86 EFLAGS bits. The naive "side-effecting CmpFlags +
// implicit NZCV consumer" model already in this header gets the
// common case, but it can't represent a SETcc that consumes flags
// from a non-adjacent producer, nor a CondJump that wants to test
// flags from an ALU operation rather than a dedicated CmpFlags.
//
// The new ops form a typed pillar:
//
//   WriteFlags{op, lhs, rhs, size}  — pure. Produces a "Flags" Ref by
//                                      doing a flag-setting variant of
//                                      `op` on (lhs, rhs).
//   ReadFlag{flags_ref, which}      — pure. Extracts a single FlagBit
//                                      as an I8 (0 or 1).
//   CondJumpFlags{flags, cc, t, f}  — terminator. Branches on the
//                                      Flags Ref using the CondCode.
//
// Lowering invariant: a Flags Ref is materialised in ARM64 NZCV.
// Between the WriteFlags and any consumer (ReadFlag / CondJumpFlags)
// the lowerer must not emit any flag-clobbering op. The validator
// gives a SizeMismatch (pseudo-typed Flags ≠ regular I8/I16/...)
// for any non-flag op that tries to consume a Flags ref.

enum class FlagBit : std::uint8_t {
    Carry = 0, Zero, Sign, Overflow, Parity, Aux
};

struct WriteFlags {
    BinOpKind op;
    Ref       lhs;
    Ref       rhs;
    OpSize    size;
};


struct ReadFlag {
    Ref     flags;
    FlagBit which;
};

struct CondJumpFlags {
    Ref           flags;
    CondCode      cc;
    std::uint32_t if_true;
    std::uint32_t if_false;
};

// ---- 128-bit SIMD (F2-IR-001 / F2-IR-002 / F2-IR-003) ----------------
//
// SSE2-class integer + bitwise SIMD on 128-bit vectors. The lane
// width (`VecLane`) decides how the bytes are interpreted by the
// op:
//
//   VecLane::B16  — 16 × i8
//   VecLane::H8   —  8 × i16
//   VecLane::S4   —  4 × i32
//   VecLane::D2   —  2 × i64
//
// The lowerer maps these directly to the `vadd_q / vsub_q / vand_q
// / vorr_q / veor_q` emitter helpers landed in F1-BK-012, on V0..V7
// from the FP scratch pool.

enum class VecLane : std::uint8_t { B16 = 0, H8, S4, D2 };

enum class VecBinOpKind : std::uint8_t {
    Add = 0, Sub,
    And, Or, Xor,
    Mul,            // F2-IR-013 — lane-wise integer multiply (PMULLW etc.)
    SqAdd,          // F2-IR-023 — signed saturating add (PADDSB/W).
    UqAdd,          //              unsigned saturating add (PADDUSB/W).
    SqSub,          //              signed saturating sub  (PSUBSB/W).
    UqSub,          //              unsigned saturating sub (PSUBUSB/W).
    UMin,           // F2-IR-024 — lane-wise min/max (PMINUB/PMAXUB H8 versions).
    UMax,
    SMin,
    SMax,
    SMulHi,         // F2-IR-025 — high half of signed   16x16 multiply (PMULHW).
    UMulHi,         //              high half of unsigned 16x16 multiply (PMULHUW).
    UMul32To64,     // F2-IR-030 — PMULUDQ: bottom-of-each-S4-pair u32×u32 → 2 D2 lanes.
    SadBw,          // F2-IR-031 — PSADBW: sum of |byte diffs| per 8-byte half → 2 D2 lanes.
    PairAddInt,     // F2-IR-037 — pairwise add (PHADDW / PHADDD, SSSE3).
    PairSubInt,     //              pairwise sub (PHSUBW / PHSUBD).
};

struct VecConstant {
    // 128-bit immediate, as two little-endian u64 lanes.
    std::uint64_t lo;
    std::uint64_t hi;
};

struct VecBinOp {
    VecBinOpKind op;
    Ref          lhs;
    Ref          rhs;
    VecLane      lane;   // ignored for And/Or/Xor — they're bitwise.
};

// SSE2 register file: 16 × 128-bit XMM registers, mapped onto the
// CpuStateFrame's `xmm[16]` slots. The decoder emits LoadVecReg /
// StoreVecReg pairs around `VecBinOp` to materialise SSE2 register
// semantics into our SSA IR. The lowerer translates these to vld1.16b
// / vst1.16b reads/writes against the state pointer + offset table.
inline constexpr std::size_t kXmmCount = 16;

struct LoadVecReg  { std::uint8_t xmm_index; };  // 0..15
struct StoreVecReg { std::uint8_t xmm_index; Ref value; };  // 0..15

// F2-IR-005 — AVX-256 high-lane access. The low 128 bits of YMMn
// live in xmm[n]; LoadVecRegHi/StoreVecRegHi address the upper
// 128 bits in CpuStateFrame::ymm_hi[]. Decoder emits these *in
// addition to* the low-lane LoadVecReg/StoreVecReg pair when
// VEX.L=1 — i.e. each AVX-256 op materialises as two IR triplets.
struct LoadVecRegHi  { std::uint8_t ymm_index; };  // 0..15
struct StoreVecRegHi { std::uint8_t ymm_index; Ref value; };  // 0..15

// F2-IR-007 — 128-bit memory load/store for SSE memory operands.
// `addr` is a 64-bit guest virtual address (already computed via the
// usual ModR/M EA Ref chain). Produces / consumes a 128-bit value in
// the same Ref namespace as LoadVecReg.
struct LoadVec  { Ref addr; };
struct StoreVec { Ref addr; Ref value; };

// F2-IR-008 — GPR ↔ XMM transfers (MOVD/MOVQ family).
//   XmmFromGpr{value, size}: produces a 128-bit value with the low
//     `size` bits = `value` and the upper bits zeroed. `size` must be
//     I32 or I64.
//   GprFromXmm{value, size}: extracts the low `size` bits of an
//     128-bit Ref into a GPR-shaped Ref. For I32 the upper 32 bits
//     are zero-extended.
struct XmmFromGpr { Ref value; OpSize size; };
struct GprFromXmm { Ref value; OpSize size; };

// F2-IR-009 — packed integer comparisons, lane-wise.
//   Eq  → all-1s where lanes equal, else 0.
//   Gt  → all-1s where lhs > rhs (signed), else 0.
// Models PCMPEQB/W/D (B16/H8/S4) and PCMPGTB/W/D — D2 lane is not
// supported (matches the SSE2 ISA: no PCMPEQQ/PCMPGTQ until SSE4.1).
enum class VecCmpKind : std::uint8_t { Eq = 0, Gt };
struct VecCmp {
    VecCmpKind kind;
    Ref        lhs;
    Ref        rhs;
    VecLane    lane;  // B16, H8, or S4
};

// F2-IR-010 — PSHUFD xmm1, xmm2, imm8. Permutes the 4 32-bit lanes of
// `src` according to `control`: result lane i = src lane ((control >>
// (2*i)) & 3). Pure 4-way 32-bit shuffle.
struct VecShuffle32x4 {
    Ref          src;
    std::uint8_t control;
};

// F2-IR-028 — PSHUFLW / PSHUFHW (4-way 16-bit lane shuffle of one half).
//   !is_high: result.h[0..3] = src.h[ctrl_lane(i)], h[4..7] passthrough.
//   is_high : result.h[4..7] = src.h[4+ctrl_lane(i)], h[0..3] passthrough.
struct VecShuffleH4 {
    bool         is_high;
    Ref          src;
    std::uint8_t control;
};

// F2-IR-020 — SHUFPS / SHUFPD (two-source FP shuffle).
//   SHUFPS: result lanes 0,1 from lhs (using control[0:1], [2:3]),
//           result lanes 2,3 from rhs (using control[4:5], [6:7]).
//   SHUFPD: result lane 0 from lhs (control[0] picks lane 0/1),
//           result lane 1 from rhs (control[1] picks lane 0/1).
// `is_pd` selects the D2 (SHUFPD) form; otherwise S4 (SHUFPS).
struct VecShuffle2Src {
    bool         is_pd;
    Ref          lhs;
    Ref          rhs;
    std::uint8_t control;
};

// F2-IR-022 — single-lane insert/extract.
//   VecInsertLane{lhs_xmm, value_gpr, lane_idx, lane} — produces a
//     128-bit value with `value_gpr` placed in lane `lane_idx`,
//     other lanes copied from `lhs_xmm`. Models PINSRW (lane=H8)
//     and the SSE4.1 family.
//   VecExtractLaneU{src_xmm, lane_idx, lane} — extracts the lane
//     into a GPR (zero-extended). Models PEXTRW.
struct VecInsertLane {
    Ref          lhs_xmm;
    Ref          value;
    std::uint8_t lane_idx;
    VecLane      lane;
};
struct VecExtractLaneU {
    Ref          src_xmm;
    std::uint8_t lane_idx;
    VecLane      lane;
};

// F2-IR-027 — PMOVMSKB. Extract MSB of each of the 16 bytes of an
// xmm into bits [0..15] of a GPR (zero-extended to 32-bit width).
struct VecMaskMsb {
    Ref src_xmm;
};

// F2-IR-029 — MOVMSKPS (4 S4 sign bits → bits 0..3 of GPR) and
// MOVMSKPD (2 D2 sign bits → bits 0..1 of GPR). is_pd selects D2.
struct VecMaskFp {
    Ref  src_xmm;
    bool is_pd;
};

// F2-IR-036 — SSSE3.
//   PSHUFB: lane-wise byte shuffle controlled by per-byte indices in
//           `mask`. If mask.b[i] has its MSB set, result.b[i] = 0;
//           otherwise result.b[i] = src.b[mask.b[i] & 0x0F].
//   VecAbs: lane-wise signed absolute value (PABSB / PABSW / PABSD).
struct VecPshufb { Ref src; Ref mask; };
struct VecAbs    { Ref src; VecLane lane; };

// F2-IR-038 — PALIGNR (SSSE3). Concat (lhs || rhs) as 32 bytes, shift
// right by `count` bytes, return the low 16 bytes as a 128-bit result.
// `count` >= 32 yields zero. Maps to NEON `ext` for count <= 16.
struct VecAlignr {
    Ref          lhs;
    Ref          rhs;
    std::uint8_t count;
};

// F2-IR-041 — PMOVZX/PMOVSX widening converts (SSE4.1).
// Takes the low N source lanes and zero/sign-extends each to a wider
// lane. `narrow_lane` says how to read the input (B16/H8/S4); `wide_lane`
// is the result lane (H8/S4/D2). Number of result lanes determined by
// the ratio (always: 16 / wide_bytes).
struct VecExtend {
    Ref     src;
    VecLane narrow_lane;
    VecLane wide_lane;
    bool    is_signed;
};

// F2-IR-011 — UNPCKL*/UNPCKH* (interleave low/high). Lane-wise pair
// merge of two source vectors: low form takes the bottom n/2 lanes of
// each, high form takes the top n/2. Lanes B16, H8, S4 or D2 select
// PUNPCK*BW / *WD / *DQ / *QDQ. Maps to NEON zip1 / zip2.
struct VecUnpack {
    bool    is_high;
    Ref     lhs;
    Ref     rhs;
    VecLane lane;
};

// F2-IR-012 — per-lane shift by immediate. Models PSLLW/D/Q (Shl),
// PSRLW/D/Q (LogicalShr) and PSRAW/D (ArithShr). Lanes H8/S4/D2.
// `count` is the SSE shift amount in bits; counts >= lane width
// saturate to lane width (PSRA semantics differ — see lowerer).
enum class VecShiftKind : std::uint8_t {
    ShiftL = 0,         // PSLLW/D/Q
    LogicalShr,         // PSRLW/D/Q
    ArithShr,           // PSRAW/D
};
struct VecShiftImm {
    VecShiftKind kind;
    Ref          src;
    std::uint8_t count;
    VecLane      lane;
};

// F2-IR-014 — whole-register byte shift (PSLLDQ / PSRLDQ).
// Shifts the entire 128-bit value left or right by `count` bytes.
// `count` >= 16 yields zero.
struct VecShiftBytes {
    bool         is_left;
    Ref          src;
    std::uint8_t count;
};


// F2-IR-005 — packed-FP binop. Single (S4 = 4×f32) and double
// (D2 = 2×f64) precision packed arithmetic, covering the SSE/SSE2
// hot path: ADDPS/SUBPS/MULPS/DIVPS and ADDPD/SUBPD/MULPD/DIVPD.
enum class VecFpBinOpKind : std::uint8_t {
    Add = 0, Sub, Mul, Div,
    Min, Max,
    Sqrt,            // F2-IR-019 — unary; uses rhs only (lhs supplies upper bits in scalar form).
    HAdd,            // F2-IR-032 — pairwise add (SSE3 HADDPS/HADDPD).
};
enum class VecFpSize : std::uint8_t { S4 = 0, D2 };

struct VecFpBinOp {
    VecFpBinOpKind op;
    Ref            lhs;
    Ref            rhs;
    VecFpSize      size;
};

// F2-IR-006 — fused multiply-add (FMA3). Single fused rounding step,
// matching ARM64 FMLA / FMLS semantics. Canonical form:
//   result = (neg_addend ? -a : a) + (neg_mul ? -(b*c) : b*c)
// The four x86 mnemonics map as:
//   VFMADD  → neg_addend=false, neg_mul=false
//   VFMSUB  → neg_addend=true,  neg_mul=false   (result = b*c - a)
//   VFNMADD → neg_addend=false, neg_mul=true    (result = a - b*c)
//   VFNMSUB → neg_addend=true,  neg_mul=true    (result = -a - b*c)
// Decoder normalises the 132/213/231 operand orderings into (a,b,c).
// Packed only in the first slice; scalar (SS/SD) deferred. The pass
// pipeline must NOT lower a separate Mul + Add into FMLA — fused vs
// decomposed differ in ULP.
struct VecFpFma {
    Ref          a;
    Ref          b;
    Ref          c;
    bool         neg_addend;
    bool         neg_mul;
    VecFpSize    size;
};

// (VecFpScalarFma — moved below FpSize declaration.)

// F2-BK-008 — REP STOSB / STOSW / STOSD / STOSQ as a native ARM64
// loop. No SSA operands: the lowerer reads/writes the pinned guest
// registers RCX (counter), RDI (dest pointer), and RAX (store
// value) directly. Side-effecting; treated as a barrier by DCE.
//
// Semantics (with DF=0, the common case; reverse=true handles DF=1):
//   while (RCX != 0) {
//     [RDI] = AL/AX/EAX/RAX   (size selects)
//     RDI  += sign * size
//     RCX  -= 1
//   }
//   ; on exit RCX is 0 and RDI has advanced by count*size.
//
// Maximum bytes a single REP STOS/MOVS invocation writes before
// returning to the dispatcher (Blocker A remediation). Bounded so a
// guest-controlled RCX cannot hang the host. When the clamp triggers,
// the lowering exits with `x0 = pc_of_rep` and the dispatcher
// re-enters the same REP instruction on the next hop with the
// remaining count, matching x86 REP-is-interruptible semantics.
// Per-call latency upper bound is dominated by `kRepMaxBytesPerCall`
// guest stores at ~1 ns each on ARM64 — ~16 ms for 16 MiB.
inline constexpr std::uint64_t kRepMaxBytesPerCall = 16ull << 20;  // 16 MiB

// `reverse=true` decrements RDI (DF=1). For now the decoder always
// emits reverse=false; a future commit can wire DF tracking.
//
// **Block-terminating with bounded iteration (Blocker A remediation):**
// `RepStos` is a block terminator (`is_block_terminator` returns true)
// because its lowering must control the next guest PC. The lowering
// clamps RCX to `kRepMaxBytesPerCall / step` iterations per dispatch
// hop so a guest-controlled RCX cannot hang the host. If RCX is
// non-zero after the clamped loop completes, the block exits with
// `x0 = pc_of_rep` (re-enters REP on next dispatch); otherwise
// `x0 = pc_after_rep`. This matches x86 REP-is-interruptible
// semantics: the guest IP only advances past REP when RCX reaches
// zero.
struct RepStos {
    OpSize        size;
    bool          reverse;
    std::uint64_t pc_of_rep;       // PC of the REP STOSB instruction
    std::uint64_t pc_after_rep;    // PC of the instruction after REP
};

// F2-BK-009 — REP MOVSB / MOVSW / MOVSD / MOVSQ. Same shape as
// RepStos but reads from [RSI] and writes to [RDI]; uses both as
// pinned operands. Same block-terminating + clamp semantics as
// `RepStos` (see above).
struct RepMovs {
    OpSize        size;
    bool          reverse;
    std::uint64_t pc_of_rep;       // PC of the REP MOVSB instruction
    std::uint64_t pc_after_rep;    // PC of the instruction after REP
};


// ---- Stack pointer adjustment (F1-RT-013) -----------------------------
//
// `RspAdjust{delta_bytes}` adds `delta_bytes` (signed, two's-complement
// in a u64) to the guest RSP. Decoders emit one for every PUSH/POP/CALL/
// RET that moves the stack. Lowered to `add x14, x14, #imm` or `sub`,
// using the pinned host register that holds guest RSP.
//
// Side-effecting; no result Ref. The dispatcher round-trip writes the
// up-to-date host x14 back into `state->gpr[Rsp]` so the next block
// (or signal handler) sees the correct guest stack pointer.

struct RspAdjust {
    std::int64_t delta_bytes;
};

// ---- Floating-point (F1-IR-026, lowered by F1-BK-013) ----------------
//
// Single- (S, 32-bit) and double- (D, 64-bit) precision IEEE-754 FP.
// We share the SSA Ref machinery with integer ops; the lowerer maps
// each FP Ref to an ARM64 Vn register (S- or D-view per FpSize) on a
// separate scratch pool from the integer x-pool to avoid interference.
//
// FpBinOpKind covers the four hot operations from x86 SSE/AVX
// scalar-single and scalar-double (ADDSS / ADDSD / MULSS / MULSD /
// SUBSS / SUBSD / DIVSS / DIVSD). Sqrt and the comparison family
// land later (F1-BK-029, planned).

enum class FpSize     : std::uint8_t { F32 = 0, F64 };
enum class FpBinOpKind: std::uint8_t { Add = 0, Sub, Mul, Div };

// F2-IR-044 — POPCNT (F3 0F B8 /r). Lane-agnostic; size selects 32 or 64-bit
// width. Result Ref is the population count value.
struct Popcnt { Ref value; OpSize size; };

// F2-IR-045 — LZCNT (F3 0F BD /r) leading-zero count, TZCNT (F3 0F BC /r)
// trailing-zero count. Both BMI1.
struct Lzcnt  { Ref value; OpSize size; };
struct Tzcnt  { Ref value; OpSize size; };

// F2-IR-046 — variable blend by implicit XMM0 mask (PBLENDVB / BLENDVPS / BLENDVPD).
// For each lane i: result[i] = mask[i].MSB ? src[i] : dst[i].
// Lane B16 / S4 / D2 sets the granularity.
struct VecBlend {
    Ref     dst;
    Ref     src;
    Ref     mask;
    VecLane lane;
};

// F2-IR-026 — FP compare → flags (UCOMISS / UCOMISD).
struct WriteFlagsFp {
    Ref    lhs;        // 128-bit xmm; only the low FP lane participates.
    Ref    rhs;
    FpSize size;
};

// F2-IR-047 — PTEST (SSE4.1) bitwise flag write.
//   ZF = (lhs AND rhs == 0)
//   CF = (lhs AND NOT rhs == 0)
//   SF = OF = AF = PF = 0.
// Lowering writes ARM NZCV directly via msr.
struct WriteFlagsPtest {
    Ref lhs;
    Ref rhs;
};

// F2-IR-049 — VPTEST ymm (VEX.256.66.0F38 17 /r). Same flag semantics as
// PTEST xmm, but over the full 256-bit register pair: ZF = ((lo_lhs &
// lo_rhs) | (hi_lhs & hi_rhs)) == 0; CF = ((~lo_lhs & lo_rhs) |
// (~hi_lhs & hi_rhs)) == 0. Lowered as 5 NEON ops + the existing scalar
// NZCV-build sequence — see `Emitter::vptest_ymm`.
struct WriteFlagsPtestYmm {
    Ref lo_lhs;
    Ref lo_rhs;
    Ref hi_lhs;
    Ref hi_rhs;
};

// F2-IR-051 — Lane-crossing byte permute from a 256-bit source pair.
//   For each output byte i (0..15):
//     k = idx[i]            // 0..31, wraps mod 32 by NEON TBL semantics
//     out[i] = (k < 16) ? src_lo[k] : src_hi[k - 16]
// Used to lower VPERMQ ymm (imm8-controlled qword permute; the
// decoder synthesises the byte-index VecConstant at decode time),
// and is general enough to host VPERMD / VPGATHER-style future
// extensions where the idx is computed at runtime. Two `VecTbl2`
// stmts (one per output half) build the full 256-bit result.
struct VecTbl2 {
    Ref src_lo;
    Ref src_hi;
    Ref idx;
};

// F2-IR-055 — AES round primitives (AES-NI). Each maps to a short
// NEON sequence; the kind enum selects the variant.
//   AesEnc      ← AESENC      = MixColumns(SubBytes(ShiftRows(src))) ^ key
//   AesEncLast  ← AESENCLAST  = SubBytes(ShiftRows(src)) ^ key
//   AesDec      ← AESDEC      = InvMixColumns(InvShiftRows(InvSubBytes(src))) ^ key
//   AesDecLast  ← AESDECLAST  = InvShiftRows(InvSubBytes(src)) ^ key
//   AesImc      ← AESIMC      = InvMixColumns(src)              (no key)
// For AesImc the `key` Ref is ignored at lower time but must still
// be a valid Ref (operand_refs walker visits it; pass src twice if
// no separate key value is available).
enum class VecAesKind : std::uint8_t {
    Enc = 0, EncLast,
    Dec,     DecLast,
    Imc,
};

struct VecAes {
    Ref        src;
    Ref        key;
    VecAesKind kind;
};

// F2-IR-058 — AESKEYGENASSIST. Builds the AES key-schedule helper
// vector from the source key material and the imm8 RCON byte:
//   dst.dword0 = SubWord(src.dword1)
//   dst.dword1 = RotWord(SubWord(src.dword1)) ^ rcon
//   dst.dword2 = SubWord(src.dword3)
//   dst.dword3 = RotWord(SubWord(src.dword3)) ^ rcon
// `rcon` is XORed into byte 0 of dword1 and dword3, matching Intel's
// little-endian architectural result.
struct VecAesKeygenAssist {
    Ref          src;
    std::uint8_t rcon;
};

// F2-IR-060 — SHA-NI (NP 0F 38 C8..CD + NP 0F 3A CC ib). One op per
// x86 instruction; the kind selects the variant. Operand roles:
//   a  = the destination register's prior value (RMW source 1)
//   b  = the xmm/m128 operand (source 2)
//   wk = the implicit XMM0 {WK0, WK1} — Sha256Rnds2 only. Other
//        kinds must still pass a valid Ref (pass b again); the
//        operand walkers visit every Ref (the VecAes convention).
//   imm = Sha1Rnds4's round-function/K selector (imm8 & 3); 0 else.
// Lane-order note: the SHA-1 kinds keep W0/A in the HIGH dword
// (lane 3) while the SHA-256 kinds are ascending (W0 in lane 0),
// matching the Intel SDM exactly.
enum class VecShaKind : std::uint8_t {
    Sha1Rnds4 = 0,
    Sha1Nexte,
    Sha1Msg1,
    Sha1Msg2,
    Sha256Rnds2,
    Sha256Msg1,
    Sha256Msg2,
};

struct VecSha {
    VecShaKind   kind;
    Ref          a;
    Ref          b;
    Ref          wk;
    std::uint8_t imm;
};

// F2-IR-056 — GPR byte-swap (the lowering target for x86 MOVBE).
// Reverses the byte order of `value` interpreted at `size`. Maps to
// ARM64 REV / REV (32-bit) / REV16 depending on size.
struct Bswap {
    Ref    value;
    OpSize size;
};

// F2-IR-057 — CRC32C (Castagnoli polynomial 0x11EDC6F41). Both x86's
// SSE4.2 CRC32 family and ARM64's CRC32C{B/H/W/X} compute the same
// polynomial, so the mapping is direct. `crc` carries the running
// 32-bit checksum (lhs); `data` supplies the next byte/word/dword/
// qword input (rhs, sized via `data_size`). Result is the updated
// 32-bit checksum.
struct Crc32c {
    Ref    crc;
    Ref    data;
    OpSize data_size;
};

// F2-IR-059 — VPGATHER/VGATHER family gather, one 128-bit half.
//   For each lane i (0..lane_count-1), with
//     d = dest_lane_base + i   (elem-width lanes; mask uses the same)
//     x = index_lane_base + i  (index-width lanes):
//     if (mask[d].MSB)  dst[d] = mem[base + (sx64(index[x]) << scale_shift)]
//     else              dst[d] = prev[d]
//   Lanes outside the dest window keep prev unchanged.
// Lanes whose mask MSB is clear MUST NOT touch memory — their address
// may be invalid by design — so lowering branches around each load.
// Architecturally the mask register is zeroed on completion; the
// decoder emits an explicit VecConstant{0} + StoreVecReg for that, so
// the op itself is a single-result gather with per-lane memory reads.
// ymm forms decode as two of these (lo/hi half); the lane bases
// express the split-width cases (VPGATHERDQ ymm's hi half reads index
// lanes 2..3, VPGATHERQD ymm's hi half writes dest/mask lanes 2..3).
struct VecGather {
    Ref          base;             // 64-bit base address (disp folded in)
    Ref          index;            // 128-bit vector of signed indices
    Ref          mask;             // 128-bit mask vector
    Ref          prev;             // previous dst; non-gathered lanes keep it
    std::uint8_t scale_shift;      // 0..3, scale = 1 << scale_shift
    std::uint8_t elem_is64   = 0;  // 0: dword elements, 1: qword
    std::uint8_t index_is64  = 0;  // 0: dword indices,  1: qword
    std::uint8_t lane_count  = 4;  // gathered elements per op: 2 or 4
    std::uint8_t dest_lane_base  = 0;  // first dest+mask lane
    std::uint8_t index_lane_base = 0;  // first index lane
};

// F2-IR-034 — CMPPS / CMPPD / CMPSS / CMPSD predicate compares.
//   Predicate = imm8 & 7 from the x86 encoding:
//     0=eq, 1=lt, 2=le, 3=unord, 4=neq, 5=nlt, 6=nle, 7=ord.
//   Packed forms produce per-lane all-1s/all-0s mask; scalar forms
//   only fill the low lane (upper bits = lhs).
//   `is_packed` toggles those two shapes.
enum class VecFpCmpPred : std::uint8_t {
    Eq = 0, Lt, Le, Unord,
    Neq, Nlt, Nle, Ord,
};
struct VecFpCompare {
    Ref          lhs;
    Ref          rhs;
    FpSize       size;       // F32 / F64
    VecFpCmpPred pred;
    bool         is_packed;  // false → scalar (upper preserved from lhs).
};

// F2-IR-042 — ROUNDPS/PD/SS/SD (SSE4.1 FP round to integer).
//   mode: 0=nearest, 1=down, 2=up, 3=truncate, 4..7=mxcsr (treated as nearest).
//   is_packed: true → ROUNDP*, false → ROUNDS* (scalar, upper preserved).
struct VecFpRound {
    Ref          lhs;     // upper-bits source for scalar form (ignored if packed).
    Ref          src;
    FpSize       size;
    std::uint8_t mode;
    bool         is_packed;
};

// F2-IR-016 — scalar int ↔ FP conversions (CVTSI2SS/SD + CVTTSS/SD2SI).
//   IntToFpScalar: signed int (I32 or I64) → low-lane FP (F32 or F64).
//                  Upper xmm bits are zeroed.
//   FpToIntScalar: low-lane FP → signed int (truncating).
struct IntToFpScalar { Ref value; OpSize int_size; FpSize fp_size; };
struct FpToIntScalar { Ref value; FpSize fp_size; OpSize int_size; };

// F2-IR-017 — CVTSS2SD / CVTSD2SS scalar precision conversion. Result
// low-lane FP = fcvt(src.low), upper xmm bits taken from `lhs`. Same
// upper-preserve pattern as VecFpScalarBinOp.
struct FpCvtScalar {
    Ref    lhs;        // 128-bit source for upper bits (the dest's old xmm).
    Ref    src;        // 128-bit source for the low FP value to convert.
    FpSize src_size;
    FpSize dst_size;
};

// F2-IR-006 — scalar-form SSE FP. Result is a 128-bit value where the
// low lane is `op(lhs.low, rhs.low)` and the upper bits are copied from
// `lhs`. Models ADDSS/SUBSS/MULSS/DIVSS (FpSize::F32) and the SD
// variants (FpSize::F64). Re-uses VecFpBinOpKind for the op.
struct VecFpScalarBinOp {
    VecFpBinOpKind op;
    Ref            lhs;
    Ref            rhs;
    FpSize         size;
};

// F2-IR-006 — scalar-form FMA (VFMADDxxxSS / VFMADDxxxSD and the
// MSUB / NMADD / NMSUB variants). Computes the canonical
//    low_lane(result) = (neg_addend ? -a.lo : a.lo)
//                     + (neg_mul ? -(b.lo*c.lo) : b.lo*c.lo)
// and copies the upper lane(s) of `scalar_upper` (the destination's
// original value) into the result. Decoder normalises 132/213/231
// into (a, b, c) and always sets `scalar_upper` to the destination's
// loaded xmm Ref so the lowerer doesn't need to know which form
// produced the IR.
struct VecFpScalarFma {
    Ref     a;
    Ref     b;
    Ref     c;
    Ref     scalar_upper;
    bool    neg_addend;
    bool    neg_mul;
    FpSize  size;
};

struct FpConstant { std::uint64_t bits; FpSize size; };
struct FpBinOp    { FpBinOpKind op; Ref lhs; Ref rhs; FpSize size; };

// F2-IR-007 — reduced-precision x87 stack primitives. The runtime model
// treats every slot as a 64-bit double, sacrificing the extra 16 bits of
// x87 extended precision for ARM64 fast-path use. Precision divergence
// is documented in RFC 0013.
//
// `st_index` is a logical x87 stack index: 0 = ST(0), 1 = ST(1), etc.
// The lowerer maps it to the physical frame slot `(TOS + st_index) mod 8`.
struct X87Load  { std::uint8_t st_index; };
struct X87Store { std::uint8_t st_index; Ref value; };

// Push a 64-bit value (FP bits) onto the x87 stack.
//
//   TOS = (TOS - 1) mod 8
//   ST(0) <- value (low 64 bits of slot)
struct X87Push { Ref value; };

// Pop the top of the x87 stack. Result Ref is the I64-typed FP bits read
// from ST(0); TOS post-increments modulo 8.
//
//   value = ST(0).lo
//   TOS = (TOS + 1) mod 8
struct X87Pop {};

// ---- InlineAsm escape hatch (F1-IR-013) ------------------------------
//
// Last resort for guest instructions our decoder + lowerer can't handle
// natively. Stores the raw guest bytes; the runtime emits a thunk that
// interprets the instruction via a software model (or jumps to a manual
// helper). The cache key includes these bytes verbatim so SMC remains
// detectable. Side-effecting; the validator treats it as a terminator
// (no later refs may rely on guest state staying coherent through it).

struct InlineAsm {
    std::vector<std::uint8_t> bytes;
};

enum class TrapKind : std::uint8_t {
    Sigtrap = 0,  // INT3 — debugger trap.
    Sigill,       // UD2 — illegal opcode.
    Sigfpe,       // DE  — divide error / floating-point.
};

struct Trap {
    TrapKind kind;
};

using Op = std::variant<
    Constant,
    LoadReg, StoreReg,
    LoadSegBase,
    BinOp,
    Compare,
    Select,
    LoadMem, StoreMem,
    LoadMemTSO, StoreMemTSO,
    Jump, CondJump, Return,
    JumpReg,
    CmpFlags, JumpRel, CondJumpRel,
    CallRel, CallReg, RetAdjusted,
    Cpuid, Syscall, Trap,
    Extend, Truncate, Fence,
    GuestPc, InlineAsm,
    FpConstant, FpBinOp,
    WriteFlags, ReadFlag, CondJumpFlags,
    RspAdjust,
    VecConstant, VecBinOp,
    LoadVecReg, StoreVecReg,
    LoadVec, StoreVec,
    VecFpBinOp, VecFpScalarBinOp,
    XmmFromGpr, GprFromXmm,
    VecCmp, VecShuffle32x4,
    VecUnpack, VecShiftImm,
    VecShiftBytes,
    IntToFpScalar, FpToIntScalar,
    FpCvtScalar,
    VecShuffle2Src,
    VecInsertLane, VecExtractLaneU,
    VecMaskMsb,
    WriteFlagsFp,
    VecShuffleH4,
    VecMaskFp,
    VecFpCompare,
    VecPshufb, VecAbs,
    VecAlignr,
    VecExtend,
    VecFpRound,
    Popcnt,
    Lzcnt, Tzcnt,
    VecBlend,
    WriteFlagsPtest,
    WriteFlagsPtestYmm,
    VecTbl2,
    VecAes,
    VecAesKeygenAssist,
    VecSha,
    Bswap,
    Crc32c,
    VecGather,
    LoadVecRegHi, StoreVecRegHi,
    VecFpFma, VecFpScalarFma,
    RepStos, RepMovs,
    X87Load, X87Store, X87Push, X87Pop
>;

// ---------------------------------------------------------------------------
// Stmt, BasicBlock, Function.
// ---------------------------------------------------------------------------

struct Stmt {
    std::optional<Ref> result;  // none for stores, jumps, ret.
    Op op;
};

struct BasicBlock {
    std::uint32_t id{0};
    std::vector<Stmt> stmts;
};

struct Function {
    std::vector<BasicBlock> blocks;
    std::uint32_t entry{0};
};

// ---------------------------------------------------------------------------
// Pretty printing (debug only — not performance critical).
// ---------------------------------------------------------------------------

[[nodiscard]] std::string pretty_print(const Op& op);
[[nodiscard]] std::string pretty_print(const Stmt& stmt);
[[nodiscard]] std::string pretty_print(const BasicBlock& block);
[[nodiscard]] std::string pretty_print(const Function& fn);

// F1-IR-019: memoised variant of `pretty_print(const Stmt&)` for test
// stability and debugger-loop performance. The cache is a per-thread
// FIFO of bounded capacity (default 256 entries) keyed by structural
// equality on Stmt; identical stmts return identical strings without
// recomputation. Call `pretty_print_memoised_clear()` between unrelated
// test runs to keep behaviour deterministic.
[[nodiscard]] std::string pretty_print_memoised(const Stmt& stmt);
void pretty_print_memoised_clear() noexcept;
[[nodiscard]] std::size_t pretty_print_memoised_size() noexcept;

// ---------------------------------------------------------------------------
// Equality — structural, not identity.
// ---------------------------------------------------------------------------
//
// std::variant comparison requires each alternative to be comparable. The
// simple structs above use aggregate-equality once we mark the operators.
// We declare them here; implementations are trivial `=default` in the .cpp.

bool operator==(const Constant& a, const Constant& b) noexcept;
bool operator==(const LoadReg& a, const LoadReg& b) noexcept;
bool operator==(const StoreReg& a, const StoreReg& b) noexcept;
bool operator==(const LoadSegBase& a, const LoadSegBase& b) noexcept;
bool operator==(const BinOp& a, const BinOp& b) noexcept;
bool operator==(const Compare& a, const Compare& b) noexcept;
bool operator==(const Select& a, const Select& b) noexcept;
bool operator==(const LoadMem& a, const LoadMem& b) noexcept;
bool operator==(const StoreMem& a, const StoreMem& b) noexcept;
bool operator==(const LoadMemTSO& a, const LoadMemTSO& b) noexcept;
bool operator==(const StoreMemTSO& a, const StoreMemTSO& b) noexcept;
bool operator==(const Jump& a, const Jump& b) noexcept;
bool operator==(const CondJump& a, const CondJump& b) noexcept;
bool operator==(const JumpReg& a, const JumpReg& b) noexcept;
bool operator==(const Return&, const Return&) noexcept;

bool operator==(const CmpFlags& a, const CmpFlags& b) noexcept;
bool operator==(const JumpRel& a, const JumpRel& b) noexcept;
bool operator==(const CondJumpRel& a, const CondJumpRel& b) noexcept;

bool operator==(const CallRel& a, const CallRel& b) noexcept;
bool operator==(const CallReg& a, const CallReg& b) noexcept;
bool operator==(const RetAdjusted& a, const RetAdjusted& b) noexcept;
bool operator==(const Cpuid&, const Cpuid&) noexcept;
bool operator==(const Syscall&, const Syscall&) noexcept;
bool operator==(const Trap& a, const Trap& b) noexcept;
bool operator==(const Extend& a, const Extend& b) noexcept;
bool operator==(const Truncate& a, const Truncate& b) noexcept;
bool operator==(const Fence& a, const Fence& b) noexcept;
bool operator==(const GuestPc& a, const GuestPc& b) noexcept;
bool operator==(const InlineAsm& a, const InlineAsm& b) noexcept;
bool operator==(const FpConstant& a, const FpConstant& b) noexcept;
bool operator==(const FpBinOp&    a, const FpBinOp&    b) noexcept;
bool operator==(const WriteFlags& a, const WriteFlags& b) noexcept;
bool operator==(const ReadFlag&   a, const ReadFlag&   b) noexcept;
bool operator==(const CondJumpFlags& a, const CondJumpFlags& b) noexcept;
bool operator==(const RspAdjust&     a, const RspAdjust&     b) noexcept;
bool operator==(const VecConstant&   a, const VecConstant&   b) noexcept;
bool operator==(const VecBinOp&      a, const VecBinOp&      b) noexcept;
bool operator==(const LoadVecReg&    a, const LoadVecReg&    b) noexcept;
bool operator==(const StoreVecReg&   a, const StoreVecReg&   b) noexcept;
bool operator==(const VecFpBinOp&    a, const VecFpBinOp&    b) noexcept;
bool operator==(const VecFpScalarBinOp& a, const VecFpScalarBinOp& b) noexcept;
bool operator==(const LoadVec&       a, const LoadVec&       b) noexcept;
bool operator==(const StoreVec&      a, const StoreVec&      b) noexcept;
bool operator==(const XmmFromGpr&    a, const XmmFromGpr&    b) noexcept;
bool operator==(const GprFromXmm&    a, const GprFromXmm&    b) noexcept;
bool operator==(const VecCmp&        a, const VecCmp&        b) noexcept;
bool operator==(const VecShuffle32x4& a, const VecShuffle32x4& b) noexcept;
bool operator==(const VecUnpack&     a, const VecUnpack&     b) noexcept;
bool operator==(const VecShiftImm&   a, const VecShiftImm&   b) noexcept;
bool operator==(const VecShiftBytes& a, const VecShiftBytes& b) noexcept;
bool operator==(const IntToFpScalar& a, const IntToFpScalar& b) noexcept;
bool operator==(const FpToIntScalar& a, const FpToIntScalar& b) noexcept;
bool operator==(const FpCvtScalar&   a, const FpCvtScalar&   b) noexcept;
bool operator==(const VecShuffle2Src& a, const VecShuffle2Src& b) noexcept;
bool operator==(const VecInsertLane& a, const VecInsertLane& b) noexcept;
bool operator==(const VecExtractLaneU& a, const VecExtractLaneU& b) noexcept;
bool operator==(const VecMaskMsb&    a, const VecMaskMsb&    b) noexcept;
bool operator==(const WriteFlagsFp&  a, const WriteFlagsFp&  b) noexcept;
bool operator==(const VecShuffleH4&  a, const VecShuffleH4&  b) noexcept;
bool operator==(const VecMaskFp&     a, const VecMaskFp&     b) noexcept;
bool operator==(const VecFpCompare&  a, const VecFpCompare&  b) noexcept;
bool operator==(const VecPshufb&     a, const VecPshufb&     b) noexcept;
bool operator==(const VecAbs&        a, const VecAbs&        b) noexcept;
bool operator==(const VecAlignr&     a, const VecAlignr&     b) noexcept;
bool operator==(const VecExtend&     a, const VecExtend&     b) noexcept;
bool operator==(const VecFpRound&    a, const VecFpRound&    b) noexcept;
bool operator==(const Popcnt&        a, const Popcnt&        b) noexcept;
bool operator==(const Lzcnt&         a, const Lzcnt&         b) noexcept;
bool operator==(const Tzcnt&         a, const Tzcnt&         b) noexcept;
bool operator==(const VecBlend&      a, const VecBlend&      b) noexcept;
bool operator==(const WriteFlagsPtest& a, const WriteFlagsPtest& b) noexcept;
bool operator==(const WriteFlagsPtestYmm& a, const WriteFlagsPtestYmm& b) noexcept;
bool operator==(const VecTbl2& a, const VecTbl2& b) noexcept;
bool operator==(const VecAes& a, const VecAes& b) noexcept;
bool operator==(const VecAesKeygenAssist& a, const VecAesKeygenAssist& b) noexcept;
bool operator==(const Bswap& a, const Bswap& b) noexcept;
bool operator==(const Crc32c& a, const Crc32c& b) noexcept;
bool operator==(const VecSha& a, const VecSha& b) noexcept;
bool operator==(const VecGather& a, const VecGather& b) noexcept;
bool operator==(const LoadVecRegHi&  a, const LoadVecRegHi&  b) noexcept;
bool operator==(const StoreVecRegHi& a, const StoreVecRegHi& b) noexcept;
bool operator==(const VecFpFma&      a, const VecFpFma&      b) noexcept;
bool operator==(const VecFpScalarFma& a, const VecFpScalarFma& b) noexcept;
bool operator==(const RepStos&       a, const RepStos&       b) noexcept;
bool operator==(const RepMovs&       a, const RepMovs&       b) noexcept;
bool operator==(const X87Load&       a, const X87Load&       b) noexcept;
bool operator==(const X87Store&      a, const X87Store&      b) noexcept;
bool operator==(const X87Push&       a, const X87Push&       b) noexcept;
bool operator==(const X87Pop&,         const X87Pop&)          noexcept;

bool operator==(const Stmt& a, const Stmt& b) noexcept;

}  // namespace prisma::ir
