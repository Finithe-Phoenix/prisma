// prisma/compress.hpp — F1-CA-010 / RFC 0008 zstd wrapper.
//
// Thin wrappers around the zstd one-shot API. Not a general-purpose
// compression facility — these exist so the translation cache can
// shrink its on-disk entries without surfacing zstd internals to
// callers.

#pragma once

#include <cstdint>
#include <optional>
#include <span>
#include <vector>

namespace prisma::cache {

// Compress `bytes` with zstd at the given level. Default level 3 is
// zstd's recommended "balanced" setting — ~3× typical ratio, ~200
// MB/s per core on ARM64. Levels 1..22 are legal; higher is slower +
// denser. Returns the compressed frame as a new vector. Does not
// fail under normal use; an empty return means zstd refused the
// input (practically: never).
[[nodiscard]] std::vector<std::uint8_t>
zstd_compress(std::span<const std::uint8_t> bytes, int level = 3);

// Decompress a zstd frame. Returns nullopt on any failure (bad
// frame, truncated input, memory allocation, unknown format). The
// caller decides what to do — for cache loads we want to reject the
// file rather than substitute garbage.
[[nodiscard]] std::optional<std::vector<std::uint8_t>>
zstd_decompress(std::span<const std::uint8_t> frame);

}  // namespace prisma::cache
