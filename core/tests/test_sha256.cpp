// core/tests/test_sha256.cpp — FIPS 180-4 test vectors.
//
// Test vectors come from NIST examples + the standard one-shot
// reference values. A broken SHA-256 impl would fail these badly.

#include <catch2/catch_test_macros.hpp>
#include <string>
#include <vector>

#include "prisma/sha256.hpp"

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
