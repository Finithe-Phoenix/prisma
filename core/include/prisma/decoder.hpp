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
//
//   MOV, both register and simple memory forms
//     * MOV r/m64, r64   (48 89 /r)                  3..7 bytes
//     * MOV r64, r/m64   (48 8B /r)                  3..7 bytes
//         - mod=00: [base]            (no disp)
//         - mod=01: [base + disp8]    (signed 8-bit disp)
//         - mod=10: [base + disp32]   (signed 32-bit disp)
//         - mod=11: register direct
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
