// prisma/ir_serialize.hpp — Compact binary serialization for Prisma IR.
//
// On-disk / on-wire representation of `prisma::ir::Stmt`,
// `prisma::ir::BasicBlock`, and `prisma::ir::Function`. The format is
// specified by RFC 0009 (`docs/rfc/0009-ir-binary-format.md`); the
// header here is a thin C++ surface around it. Use this when you want
// to stash post-passes IR into the persistent cache (RFC 0007) or
// hand it across a process boundary deterministically.
//
// Invariants:
//   * Format magic is the four ASCII bytes "PIRB".
//   * Version is `kVersion` (currently 1, little-endian u16).
//   * OpKind tag values are stable forever — see RFC 0009 §"OpKind
//     tag table" — so a future reader can decode older streams.
//   * The trailing CRC32C covers every byte from the magic up to and
//     including the final statement payload.
//
// Status: F1-IR-017 / F1-IR-018. No cache integration yet (see
// "Future work" in RFC 0009).

#pragma once

#include <cstdint>
#include <optional>
#include <span>
#include <utility>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::ir {

// Binary format constants. Bumping `kVersion` is a one-way door; old
// readers must reject newer versions (RFC 0009 §"Version policy").
inline constexpr std::uint32_t kSerializeMagic   = 0x42524950u;  // 'PIRB' LE
inline constexpr std::uint16_t kSerializeVersion = 0x0001u;

// Failure modes for `deserialize_*`. Each has a single canonical cause
// — see RFC 0009 §"Error taxonomy".
enum class DeserializeError : std::uint8_t {
    Ok            = 0,
    BadMagic      = 1,
    BadVersion    = 2,
    Truncated     = 3,
    UnknownOpKind = 4,
    BadCrc        = 5,
    BadSize       = 6,  // OpSize / enum value out of range.
};

// Result of `deserialize_stmts`. On `error != Ok`, `stmts` is empty.
struct DeserializeResult {
    DeserializeError       error;
    std::vector<Stmt>      stmts;
};

// Serialize a flat statement list to a self-contained byte stream.
// The returned bytes start with the four-byte magic and end with a
// CRC32C trailer.
[[nodiscard]] std::vector<std::uint8_t> serialize(const std::vector<Stmt>& stmts);

// Serialize a `Function`. Encodes the entry block id, the block
// count, then each block as `(id, stmt_count, stmt*)`. Wraps the
// whole thing in the same magic / version / CRC envelope.
[[nodiscard]] std::vector<std::uint8_t> serialize(const Function& fn);

// Inverse of `serialize(const std::vector<Stmt>&)`.
[[nodiscard]] DeserializeResult deserialize_stmts(std::span<const std::uint8_t> bytes);

// Inverse of `serialize(const Function&)`. Returns the populated
// function on success; `nullopt` on any error.
[[nodiscard]] std::pair<DeserializeError, std::optional<Function>>
deserialize_function(std::span<const std::uint8_t> bytes);

}  // namespace prisma::ir
