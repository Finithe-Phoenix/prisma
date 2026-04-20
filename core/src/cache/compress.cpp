// core/src/cache/compress.cpp — zstd wrapper implementation.

#include "prisma/compress.hpp"

#include <zstd.h>

namespace prisma::cache {

std::vector<std::uint8_t>
zstd_compress(std::span<const std::uint8_t> bytes, int level) {
    if (bytes.empty()) return {};
    const std::size_t bound = ZSTD_compressBound(bytes.size());
    std::vector<std::uint8_t> out(bound);
    const std::size_t produced = ZSTD_compress(
        out.data(), bound,
        bytes.data(), bytes.size(),
        level);
    if (ZSTD_isError(produced)) return {};
    out.resize(produced);
    return out;
}

std::optional<std::vector<std::uint8_t>>
zstd_decompress(std::span<const std::uint8_t> frame) {
    if (frame.empty()) return std::vector<std::uint8_t>{};

    // Query the declared decompressed size. Bail if zstd can't tell —
    // a streamed frame without embedded size would force us to loop,
    // and our writer always records the size, so "unknown" here is a
    // sign of corruption.
    const std::uint64_t expected =
        ZSTD_getFrameContentSize(frame.data(), frame.size());
    if (expected == ZSTD_CONTENTSIZE_ERROR) return std::nullopt;
    if (expected == ZSTD_CONTENTSIZE_UNKNOWN) return std::nullopt;

    std::vector<std::uint8_t> out(static_cast<std::size_t>(expected));
    const std::size_t produced = ZSTD_decompress(
        out.data(), out.size(),
        frame.data(), frame.size());
    if (ZSTD_isError(produced)) return std::nullopt;
    if (produced != expected)   return std::nullopt;
    return out;
}

}  // namespace prisma::cache
