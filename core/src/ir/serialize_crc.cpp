// core/src/ir/serialize_crc.cpp — CRC32C (Castagnoli, poly 0x1EDC6F41).
//
// Internal helper for the IR binary serializer. Picked Castagnoli to
// match the polynomial family already used by the cache layer's hash
// (FNV-1a is unrelated; CRC32C is what the SHA-256 trust envelope
// path will eventually pair with). Table-driven, slice-by-1 — fast
// enough for an IR stream that is at most a few KB and small enough
// to keep the build dependency-free.
//
// Reference: RFC 3720 §B.4 (iSCSI). The polynomial is reversed
// (0x82F63B78) so the table can be built bottom-up; this matches the
// LE-byte-stream convention used by the rest of the file format.

#include "serialize_crc.hpp"

#include <array>
#include <cstdint>

namespace prisma::ir::detail {

namespace {

constexpr std::uint32_t kReversedPoly = 0x82F63B78u;  // bit-reversed 0x1EDC6F41

constexpr std::array<std::uint32_t, 256> make_table() {
    std::array<std::uint32_t, 256> t{};
    for (std::uint32_t i = 0; i < 256; ++i) {
        std::uint32_t c = i;
        for (int k = 0; k < 8; ++k) {
            c = (c & 1u) ? ((c >> 1) ^ kReversedPoly) : (c >> 1);
        }
        t[i] = c;
    }
    return t;
}

constexpr auto kTable = make_table();

}  // namespace

std::uint32_t crc32c(std::span<const std::uint8_t> bytes) noexcept {
    std::uint32_t c = 0xFFFF'FFFFu;
    for (auto b : bytes) {
        const std::uint32_t idx = (c ^ static_cast<std::uint32_t>(b)) & 0xFFu;
        c = kTable[idx] ^ (c >> 8);
    }
    return c ^ 0xFFFF'FFFFu;
}

}  // namespace prisma::ir::detail
