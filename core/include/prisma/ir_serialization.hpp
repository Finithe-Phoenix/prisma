// prisma/ir_serialization.hpp - compact binary Prisma IR serialization.
//
// F1-IR-017 produces a deterministic, little-endian byte stream for cache
// storage and future cross-process exchange. Deserialization is intentionally
// a separate backlog item so this API only writes bytes.

#pragma once

#include <array>
#include <cstdint>
#include <span>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

inline constexpr std::array<std::uint8_t, 4> kIrBinaryMagic{
    static_cast<std::uint8_t>('P'),
    static_cast<std::uint8_t>('I'),
    static_cast<std::uint8_t>('R'),
    static_cast<std::uint8_t>('B'),
};
inline constexpr std::uint16_t kIrBinaryVersion = 1;

enum class IrBinaryKind : std::uint8_t {
    StmtList = 1,
    Function = 2,
};

[[nodiscard]] std::vector<std::uint8_t> serialize_stmts(std::span<const Stmt> stmts);
[[nodiscard]] std::vector<std::uint8_t> serialize_op(const Op& op);
[[nodiscard]] std::vector<std::uint8_t> serialize_function(const Function& function);

}  // namespace prisma::ir
