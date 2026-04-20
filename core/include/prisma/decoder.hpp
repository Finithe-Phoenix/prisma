// prisma/decoder.hpp — x86_64 → Prisma IR decoder.
//
// Status: Fase 0 MVP. Recognises the following opcodes:
//
//   No operands / immediates
//     * NOP              (90)                        1 byte
//     * RET              (C3)                        1 byte
//     * MOV r64, imm64   (48 B8+rd imm64)           10 bytes
//
//   Register / register, 64-bit, mod=11
//     * ADD r/m64, r64   (48 01 /r)                  3 bytes
//     * ADC r/m64, r64   (48 11 /r)                  3 bytes
//     * OR  r/m64, r64   (48 09 /r)                  3 bytes
//     * AND r/m64, r64   (48 21 /r)                  3 bytes
//     * SUB r/m64, r64   (48 29 /r)                  3 bytes
//     * SBB r/m64, r64   (48 19 /r)                  3 bytes
//     * XOR r/m64, r64   (48 31 /r)                  3 bytes
//     * TEST r/m64, r64  (48 85 /r)                  3 bytes
//     * INC r/m64        (48 FF /0)                  3 bytes
//     * DEC r/m64        (48 FF /1)                  3 bytes
//     * NOT r/m64        (48 F7 /2)                  3 bytes
//     * NEG r/m64        (48 F7 /3)                  3 bytes
//     * MUL r/m64        (48 F7 /4)                  3 bytes
//     * IMUL r/m64       (48 F7 /5)                  3 bytes
//     * DIV r/m64        (48 F7 /6)                  3 bytes
//     * IDIV r/m64       (48 F7 /7)                  3 bytes
//     * SHL r/m64, imm8  (48 C1 /4)                  4 bytes
//     * RCL r/m64, imm8  (48 C1 /2)                  4 bytes
//     * RCR r/m64, imm8  (48 C1 /3)                  4 bytes
//     * ROL r/m64, imm8  (48 C1 /0)                  4 bytes
//     * ROR r/m64, imm8  (48 C1 /1)                  4 bytes
//     * SHR r/m64, imm8  (48 C1 /5)                  4 bytes
//     * SAR r/m64, imm8  (48 C1 /7)                  4 bytes
//     * SHL r/m64, CL    (48 D3 /4)                  3 bytes
//     * RCL r/m64, CL    (48 D3 /2)                  3 bytes
//     * RCR r/m64, CL    (48 D3 /3)                  3 bytes
//     * ROL r/m64, CL    (48 D3 /0)                  3 bytes
//     * ROR r/m64, CL    (48 D3 /1)                  3 bytes
//     * SHR r/m64, CL    (48 D3 /5)                  3 bytes
//     * SAR r/m64, CL    (48 D3 /7)                  3 bytes
//     * IMUL r64, r/m64  (48 0F AF /r)              4 bytes
//     * IMUL r64, r/m64, imm32 (48 69 /r)           7 bytes
//     * IMUL r64, r/m64, imm8  (48 6B /r)           4 bytes
//     * BT r/m64, imm8    (0F BA /4)                  4 bytes
//     * BTS r/m64, imm8   (0F BA /5)                  4 bytes
//     * BTR r/m64, imm8   (0F BA /6)                  4 bytes
//     * BTC r/m64, imm8   (0F BA /7)                  4 bytes
//     * BSF r64, r/m64    (0F BC /r)                  4 bytes
//     * BSR r64, r/m64    (0F BD /r)                  4 bytes
//     * LZCNT r64, r/m64  (F3 48 0F BD)              5 bytes
//     * TZCNT r64, r/m64  (F3 48 0F BC)              5 bytes
//     * POPCNT r64, r/m64  (F3 48 0F B8)              5 bytes
//     * PUSHFQ / POPFQ    (9C / 9D)                      1 byte
//     * PUSH r64          (50+rd)                     1 byte
//     * POP r64           (58+rd)                     1 byte
//     * PUSH imm8 / imm32 (6A / 68)                   2 / 5 bytes
//     * LEA r64, [mem]    (48 8D /r)                   4..10 bytes
// 
//   MOV, both register and simple memory forms
//     * MOV r/m64, r64   (48 89 /r)                  3..7 bytes
//     * MOVSX r64, r/m8   (0F BE /r)                 4 bytes
//     * MOVSX r64, r/m16  (0F BF /r)                 4 bytes
//     * MOV r64, r/m64   (48 8B /r)                  3..7 bytes
//         - This form also supports MOVSXD with opcode 63 /r when REX.W is 1:
//           sign-extend r/m32 to 64-bit and write r64.
//         - mod=00: [base]            (no disp)
//         - mod=01: [base + disp8]    (signed 8-bit disp)
//         - mod=10: [base + disp32]   (signed 32-bit disp)
//         - mod=11: register direct
//     * XCHG r64, r/m64   (48 87 /r)                  3..7 bytes
//     * CMPXCHG r/m64, r64 (48 0F B1 /r)              4..8 bytes
//     * XADD r/m64, r64   (48 0F C1 /r)               4..8 bytes
//     * CMPXCHG16B m128   (48 0F C7 /1)               4..8 bytes
//     * STOSB/STOSW/STOSD/STOSQ (AA / AB)             1..2 bytes
//     * MOVSB/MOVSW/MOVSD/MOVSQ (A4 / A5)             1..2 bytes
//     * CMPSB/CMPSW/CMPSD/CMPSQ (A6 / A7)             1..2 bytes
//     * SCASB/SCASW/SCASD/SCASQ (AE / AF)             1..2 bytes
//     * MOVZX r64, r/m8   (0F B6 /r)                 4 bytes
//     * MOVZX r64, r/m16  (0F B7 /r)                 4 bytes
//         Memory forms always emit *_TSO variants of load/store. The
//         TSO-adaptive pass (Pillar 3, later) may rewrite to non-TSO.
//         Rejected for MVP: rm=100 (SIB required), mod=00 rm=101
//         (disp32 absolute).
//
//   Control flow (produce absolute-PC ir ops)
//     * CMP r/m64, r64   (48 39 /r, mod=11)          3 bytes  → CmpFlags
//     * JMP rel8         (EB cb)                     2 bytes  → JumpRel
//     * JMP rel32        (E9 cd)                     5 bytes  → JumpRel
//     * JE  rel8         (74 cb)                     2 bytes  → CondJumpRel(Eq)
//     * JNE rel8         (75 cb)                     2 bytes  → CondJumpRel(Ne)
//
// The decoder is deliberately minimal but now supports a small subset of
// prefixes:
//   * 0x66 for selected size-sensitive MOV forms (I16),
//   * 0xF0 (LOCK) for CMPXCHG / CMPXCHG16B / XADD,
//   * 0xF2 / 0xF3 as ignored HLE hints when paired with LOCK on that same
//     exchange-family atomic subset,
//   * no REP string-loop semantics yet; string ops decode as single-step only,
//   * 0xF3 for LZCNT/TZCNT/POPCNT family placeholder support,
//   * REX.W for the 64-bit MOV/ALU variants.
// No SIB / RIP-relative / R8..R15 yet (REX.R / REX.B / REX.X must be
// zero). All of these constraints remain future-work items; the API stays
// stable.
//
// The decoder is pure: it does not mmap, execute, or touch global state.
// Given the same input + `next_ref`, it produces the same output.

#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <variant>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::decoder {

enum class DecodeError {
    TruncatedInput,        // ran out of bytes before the instruction ended.
    UnknownOpcode,         // first opcode byte not in our MVP set.
    UnsupportedEncoding,   // ModR/M with memory operand, REX.R/B set, etc.
};

[[nodiscard]] constexpr std::string_view describe(DecodeError e) noexcept {
    switch (e) {
        case DecodeError::TruncatedInput:      return "truncated-input";
        case DecodeError::UnknownOpcode:       return "unknown-opcode";
        case DecodeError::UnsupportedEncoding: return "unsupported-encoding";
    }
    return "?";
}

// Successful decode of exactly one instruction.
struct Decoded {
    // IR statements this instruction lowers to. Always contains at least
    // one statement; compound instructions can produce several.
    std::vector<ir::Stmt> stmts;
    // How many guest bytes were consumed. Advance the caller's cursor by
    // this many.
    std::size_t bytes_consumed{0};
};

// Decode one x86_64 instruction from the start of `bytes`. On success,
// returns the statements + byte count. On failure, returns the error.
//
// `next_ref` is used-and-incremented for every new SSA value bound by the
// returned statements. Caller owns the counter so it can be shared across
// multiple `decode_one` calls.
//
// `instruction_guest_pc` is the absolute guest PC of the first byte of
// the instruction. Only relevant for branch instructions (JMP / Jcc),
// which need to translate rel8 / rel32 displacements into absolute
// target PCs. Callers that don't care about branch targets (the many
// tests decoding non-jump instructions) can leave it at the default 0.
[[nodiscard]] std::variant<Decoded, DecodeError> decode_one(
    std::span<const std::uint8_t> bytes,
    ir::Ref& next_ref,
    std::uint64_t instruction_guest_pc = 0
);

}  // namespace prisma::decoder
