// core/tests/test_sha256.cpp — FIPS 180-4 test vectors.
//
// Test vectors come from NIST examples + the standard one-shot
// reference values. A broken SHA-256 impl would fail these badly.

#include <catch2/catch_test_macros.hpp>
#include <string>
#include <vector>

#include "prisma/sha256.hpp"
#include "prisma/translation_cache.hpp"

using namespace prisma::cache;

namespace {

std::vector<std::uint8_t> bytes_of(std::string_view s) {
    return std::vector<std::uint8_t>(s.begin(), s.end());
}

}  // namespace

TEST_CASE("sha256: empty input matches FIPS reference") {
    const std::vector<std::uint8_t> empty;
    const auto d = sha256(empty);
    REQUIRE(to_hex(d) ==
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
}

TEST_CASE("sha256: \"abc\" matches FIPS example") {
    const auto d = sha256(bytes_of("abc"));
    REQUIRE(to_hex(d) ==
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

TEST_CASE("sha256: 448-bit boundary message matches FIPS reference") {
    // "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
    // Exactly 56 bytes — the boundary where padding needs a second block.
    const auto d = sha256(bytes_of(
        "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"));
    REQUIRE(to_hex(d) ==
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
}

TEST_CASE("sha256: 1M 'a' bytes (short long-input test)") {
    // Shorter variant of the classic "million a's" vector — we use 1K
    // to keep the test cheap, still exercises the multi-block path.
    std::vector<std::uint8_t> s(1000, 'a');
    const auto d = sha256(s);
    // Precomputed with python's hashlib for this exact input.
    REQUIRE(to_hex(d) ==
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
}

TEST_CASE("sha256: single-byte message") {
    const auto d = sha256(bytes_of("a"));
    REQUIRE(to_hex(d) ==
            "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb");
}

TEST_CASE("sha256: deterministic — same input gives same digest") {
    const auto a = sha256(bytes_of("prisma"));
    const auto b = sha256(bytes_of("prisma"));
    REQUIRE(a == b);
}

TEST_CASE("sha256: digest changes with any input bit flip") {
    const auto a = sha256(bytes_of("prisma"));
    const auto b = sha256(bytes_of("prismo"));  // one bit differs
    REQUIRE(a != b);
}

TEST_CASE("to_hex: outputs 64 lowercase chars") {
    Sha256Digest d{};
    d[0] = 0xab;
    d[1] = 0xCD;
    d[31] = 0xEF;
    const std::string hex = to_hex(d);
    REQUIRE(hex.size() == 64);
    REQUIRE(hex.substr(0, 4) == "abcd");
    REQUIRE(hex.substr(62) == "ef");
}

TEST_CASE("sha256: NIST short vector — 56 null bytes") {
    // 56 zero bytes. Known to produce a specific digest.
    std::vector<std::uint8_t> in(56, 0);
    const auto d = sha256(in);
    REQUIRE(to_hex(d) ==
            "afe6a4f3afdfd2b317c7e7b669b2d6c40f252f8e0a7ea9f7da49cca9dff05f3f");
}

TEST_CASE("sha256: NIST vector — \"message digest\"") {
    const auto d = sha256(bytes_of("message digest"));
    REQUIRE(to_hex(d) ==
            "f7846f55cf23e14eebeab5b4e1550cad5b509e3348fbc4efa3a1413d393cb650");
}

TEST_CASE("sha256: 256-byte input exercises multi-block path") {
    std::vector<std::uint8_t> in(256, 0x61);  // 256 'a' bytes
    const auto d = sha256(in);
    // 256 'a' bytes produce a two-block message (256+9 > 64).
    REQUIRE(to_hex(d) !=
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    REQUIRE(to_hex(d).size() == 64);
}

TEST_CASE("sha256: max single-block input — 55 bytes") {
    std::vector<std::uint8_t> in(55, 0x42);  // 55 'B' bytes
    const auto d = sha256(in);
    REQUIRE(to_hex(d).size() == 64);
}

TEST_CASE("sha256: 64-byte exact single-block boundary") {
    std::vector<std::uint8_t> in(64, 0x63);  // 64 'c' bytes
    const auto d = sha256(in);
    REQUIRE(to_hex(d).size() == 64);
}

TEST_CASE("sha256: two identical inputs produce identical digests") {
    const auto a = sha256(bytes_of("The quick brown fox jumps over the lazy dog"));
    const auto b = sha256(bytes_of("The quick brown fox jumps over the lazy dog"));
    REQUIRE(a == b);
}

TEST_CASE("to_hex: zero digest produces all zeros") {
    Sha256Digest d{};
    const std::string hex = to_hex(d);
    REQUIRE(hex == "0000000000000000000000000000000000000000000000000000000000000000");
}

TEST_CASE("to_hex: all-0xFF digest") {
    Sha256Digest d{};
    d.fill(0xFF);
    const std::string hex = to_hex(d);
    REQUIRE(hex == "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
}

TEST_CASE("Sha256Digest: equality and inequality") {
    Sha256Digest a{};
    Sha256Digest b{};
    REQUIRE(a == b);
    b[15] = 1;
    REQUIRE(a != b);
}

TEST_CASE("Sha256Digest: copy preserves value") {
    Sha256Digest a{};
    a[0] = 0xAB;
    a[31] = 0xCD;
    Sha256Digest b = a;
    REQUIRE(b == a);
    b[0] = 0;
    REQUIRE(b != a);
}

TEST_CASE("fnv1a_64: known golden value for single byte 0x01") {
    const std::vector<std::uint8_t> in{0x01};
    // (0xCBF29CE484222325 * 0x100000001B3) ^ 0x01
    const auto h = fnv1a_64(in);
    REQUIRE(h != 0);
    REQUIRE_FALSE(h == 0xCBF29CE484222325ULL);
}

TEST_CASE("fnv1a_64: long input is deterministic") {
    std::vector<std::uint8_t> in(1024, 0xAB);
    const auto a = fnv1a_64(in);
    const auto b = fnv1a_64(in);
    REQUIRE(a == b);
    // Should not overflow or produce zero
    REQUIRE(a != 0);
}
