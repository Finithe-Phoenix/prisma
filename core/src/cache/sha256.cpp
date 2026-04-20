// core/src/cache/sha256.cpp — clean-room FIPS 180-4 SHA-256.
//
// Written from the standard; no copied code. Follows NIST SP 800-107
// message padding:
//   * Append a single 1 bit (0x80 byte).
//   * Zero-pad until length mod 512 == 448 (mod 64 == 56 bytes).
//   * Append the original message length as a 64-bit big-endian int.
// Then digest 64-byte blocks with the round constants + message
// schedule in section 6.2.

#include "prisma/sha256.hpp"

#include <cstring>

namespace prisma::cache {

namespace {

constexpr std::uint32_t K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

constexpr std::uint32_t rotr(std::uint32_t x, unsigned n) noexcept {
    return (x >> n) | (x << (32u - n));
}

void process_block(std::uint32_t H[8],
                   const std::uint8_t block[64]) noexcept {
    std::uint32_t W[64];
    for (unsigned i = 0; i < 16; ++i) {
        W[i] = (static_cast<std::uint32_t>(block[i * 4    ]) << 24)
             | (static_cast<std::uint32_t>(block[i * 4 + 1]) << 16)
             | (static_cast<std::uint32_t>(block[i * 4 + 2]) <<  8)
             | (static_cast<std::uint32_t>(block[i * 4 + 3]));
    }
    for (unsigned i = 16; i < 64; ++i) {
        const std::uint32_t s0 = rotr(W[i-15], 7) ^ rotr(W[i-15], 18) ^ (W[i-15] >> 3);
        const std::uint32_t s1 = rotr(W[i-2], 17) ^ rotr(W[i-2],  19) ^ (W[i-2] >> 10);
        W[i] = W[i-16] + s0 + W[i-7] + s1;
    }

    std::uint32_t a = H[0], b = H[1], c = H[2], d = H[3];
    std::uint32_t e = H[4], f = H[5], g = H[6], h = H[7];
    for (unsigned i = 0; i < 64; ++i) {
        const std::uint32_t S1   = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        const std::uint32_t ch   = (e & f) ^ (~e & g);
        const std::uint32_t tmp1 = h + S1 + ch + K[i] + W[i];
        const std::uint32_t S0   = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        const std::uint32_t maj  = (a & b) ^ (a & c) ^ (b & c);
        const std::uint32_t tmp2 = S0 + maj;
        h = g; g = f; f = e; e = d + tmp1;
        d = c; c = b; b = a; a = tmp1 + tmp2;
    }
    H[0] += a; H[1] += b; H[2] += c; H[3] += d;
    H[4] += e; H[5] += f; H[6] += g; H[7] += h;
}

}  // namespace

Sha256Digest sha256(std::span<const std::uint8_t> bytes) noexcept {
    std::uint32_t H[8] = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };

    const std::uint64_t total_bits = static_cast<std::uint64_t>(bytes.size()) * 8ULL;

    // Chew full blocks first.
    std::size_t off = 0;
    while (off + 64 <= bytes.size()) {
        process_block(H, bytes.data() + off);
        off += 64;
    }

    // Tail + padding. Up to two trailing blocks.
    std::uint8_t tail[128];
    const std::size_t rem = bytes.size() - off;
    if (rem > 0) std::memcpy(tail, bytes.data() + off, rem);
    tail[rem] = 0x80u;
    std::size_t pad_end = rem + 1;

    // If there's no room for the 8-byte length in the current block,
    // pad to 64 then start a second block.
    std::size_t blocks = (rem + 1 <= 56) ? 1 : 2;
    const std::size_t total_tail = blocks * 64;
    for (std::size_t i = pad_end; i < total_tail - 8; ++i) tail[i] = 0u;

    // Length in bits, big-endian.
    for (unsigned i = 0; i < 8; ++i) {
        tail[total_tail - 1 - i] = static_cast<std::uint8_t>(total_bits >> (i * 8));
    }

    for (std::size_t i = 0; i < blocks; ++i) {
        process_block(H, tail + i * 64);
    }

    // Serialise H big-endian.
    Sha256Digest out{};
    for (unsigned i = 0; i < 8; ++i) {
        out[i * 4    ] = static_cast<std::uint8_t>(H[i] >> 24);
        out[i * 4 + 1] = static_cast<std::uint8_t>(H[i] >> 16);
        out[i * 4 + 2] = static_cast<std::uint8_t>(H[i] >>  8);
        out[i * 4 + 3] = static_cast<std::uint8_t>(H[i]);
    }
    return out;
}

std::string to_hex(const Sha256Digest& d) {
    static constexpr char kHex[] = "0123456789abcdef";
    std::string out(64, '0');
    for (std::size_t i = 0; i < 32; ++i) {
        out[i * 2    ] = kHex[(d[i] >> 4) & 0xF];
        out[i * 2 + 1] = kHex[ d[i]       & 0xF];
    }
    return out;
}

}  // namespace prisma::cache
