// prisma/decoder.hpp — x86_64 → Prisma IR decoder.
//
// Status: Fase 0 MVP. Recognises four instructions:
//
//   * MOV r64, imm64   (48 B8+rd imm64)             10 bytes
//   * ADD r64, r64     (48 01 /r, mod=11)            3 bytes
//   * SUB r64, r64     (48 29 /r, mod=11)            3 bytes
//   * RET              (C3)                          1 byte
//
// The decoder is deliberately minimal: no prefixes other than REX.W, no
// memory operands (mod != 11 is rejected), no R8..R15 (REX.R / REX.B must
// be zero). Each of these will be lifted in future RFCs as the opcode set
// grows. The shape of the API does not change.
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
[[nodiscard]] std::variant<Decoded, DecodeError> decode_one(
    std::span<const std::uint8_t> bytes,
    ir::Ref& next_ref
);

}  // namespace prisma::decoder
