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

// F2-IR-005 — packed-FP binop. Single (S4 = 4×f32) and double
// (D2 = 2×f64) precision packed arithmetic, covering the SSE/SSE2
// hot path: ADDPS/SUBPS/MULPS/DIVPS and ADDPD/SUBPD/MULPD/DIVPD.
enum class VecFpBinOpKind : std::uint8_t {
    Add = 0, Sub, Mul, Div,
};
enum class VecFpSize : std::uint8_t { S4 = 0, D2 };

struct VecFpBinOp {
    VecFpBinOpKind op;
    Ref            lhs;
    Ref            rhs;
    VecFpSize      size;
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

struct FpConstant { std::uint64_t bits; FpSize size; };
struct FpBinOp    { FpBinOpKind op; Ref lhs; Ref rhs; FpSize size; };

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
    VecFpBinOp
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

bool operator==(const Stmt& a, const Stmt& b) noexcept;

}  // namespace prisma::ir
