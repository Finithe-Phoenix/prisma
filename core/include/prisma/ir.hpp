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

enum class SegmentReg : std::uint8_t {
    Fs = 0,
    Gs = 1,
};

enum class TrapKind : std::uint8_t {
    Sigtrap = 0,
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
struct LoadSegBase { SegmentReg segment; };
struct StoreReg { Gpr reg; Ref value; OpSize size; };

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

struct CallRel {
    std::uint64_t target_guest_pc;
    std::uint64_t return_guest_pc;
};

struct CallReg {
    Ref target;
    std::uint64_t return_guest_pc;
};

struct RetAdjusted {
    std::uint32_t pop_bytes;
};

struct Cpuid {};
struct Syscall {};
struct Trap {
    TrapKind kind;
};

struct CondJumpRel {
    CondCode      cc;
    std::uint64_t target_guest_pc;
    std::uint64_t fallthrough_guest_pc;
};

using Op = std::variant<
    Constant,
    LoadReg, LoadSegBase, StoreReg,
    BinOp,
    Compare,
    Select,
    LoadMem, StoreMem,
    LoadMemTSO, StoreMemTSO,
    Jump, CondJump, Return,
    JumpReg,
    CmpFlags, JumpRel, CallRel, CallReg, RetAdjusted,
    Cpuid, Syscall, Trap,
    CondJumpRel
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

// ---------------------------------------------------------------------------
// Equality — structural, not identity.
// ---------------------------------------------------------------------------
//
// std::variant comparison requires each alternative to be comparable. The
// simple structs above use aggregate-equality once we mark the operators.
// We declare them here; implementations are trivial `=default` in the .cpp.

bool operator==(const Constant& a, const Constant& b) noexcept;
bool operator==(const LoadReg& a, const LoadReg& b) noexcept;
bool operator==(const LoadSegBase& a, const LoadSegBase& b) noexcept;
bool operator==(const StoreReg& a, const StoreReg& b) noexcept;
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
bool operator==(const CallRel& a, const CallRel& b) noexcept;
bool operator==(const CallReg& a, const CallReg& b) noexcept;
bool operator==(const RetAdjusted& a, const RetAdjusted& b) noexcept;
bool operator==(const Cpuid&, const Cpuid&) noexcept;
bool operator==(const Syscall&, const Syscall&) noexcept;
bool operator==(const Trap& a, const Trap& b) noexcept;
bool operator==(const CondJumpRel& a, const CondJumpRel& b) noexcept;

bool operator==(const Stmt& a, const Stmt& b) noexcept;

}  // namespace prisma::ir
