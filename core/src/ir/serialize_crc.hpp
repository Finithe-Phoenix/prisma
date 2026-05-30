// core/src/ir/serialize_crc.hpp — internal CRC32C helper for the IR
// binary serializer. Not part of the public API; lives under src/.

#pragma once

#include <cstdint>
#include <span>

namespace prisma::ir::detail {

// CRC32C (Castagnoli, polynomial 0x1EDC6F41). Returns the standard
// final-XOR'd value matching iSCSI / Btrfs / SCTP usage.
[[nodiscard]] std::uint32_t crc32c(std::span<const std::uint8_t> bytes) noexcept;

}  // namespace prisma::ir::detail
