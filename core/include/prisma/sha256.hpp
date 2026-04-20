// prisma/sha256.hpp — F1-CA-004 SHA-256 for cache content hashing.
//
// Clean-room FIPS 180-4 implementation. Used by the P2P cache trust
// protocol in Fase 2.5: each translation entry will ship with a SHA-256
// digest of its guest bytes so receiving agents can verify integrity
// before running untrusted precompiled code.
//
// This module intentionally does NOT replace `fnv1a_64` — that remains
// the intra-process cache key hash because it's ~20x faster and
// collision-resistant enough for a single process. SHA-256 kicks in
// when entries cross a trust boundary.

#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>

namespace prisma::cache {

// 32-byte SHA-256 digest. Big-endian by construction; byte[0] is the
// most-significant byte of the first 32-bit word.
using Sha256Digest = std::array<std::uint8_t, 32>;

// Compute the SHA-256 digest of `bytes`. No allocation beyond the
// fixed-size output and the stack-local state block.
[[nodiscard]] Sha256Digest sha256(std::span<const std::uint8_t> bytes) noexcept;

// Lowercase hex rendering of a digest. 64 chars, no separator.
[[nodiscard]] std::string to_hex(const Sha256Digest& d);

}  // namespace prisma::cache
