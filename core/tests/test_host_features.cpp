// core/tests/test_host_features.cpp — F1-RT-016 / F1-RT-017 coverage.
//
// We can't assert which features ARE present on the host (that depends
// on the CI runner's silicon) but we CAN assert:
//   * detection doesn't crash and returns a well-formed struct,
//   * the detected result is stable across calls (cached),
//   * the test override + clear path restores the live value.

#include <catch2/catch_test_macros.hpp>

#include "prisma/host_features.hpp"

using namespace prisma::runtime;

TEST_CASE("host_features: returns a stable result across calls") {
    const auto& a = host_features();
    const auto& b = host_features();
    // Bitwise equality across fields — same pointer is an even stronger
    // guarantee that the static cache is singleton.
    REQUIRE(&a == &b);
    REQUIRE(a.feat_lse     == b.feat_lse);
    REQUIRE(a.feat_lse2    == b.feat_lse2);
    REQUIRE(a.feat_lrcpc   == b.feat_lrcpc);
    REQUIRE(a.feat_lrcpc2  == b.feat_lrcpc2);
    REQUIRE(a.feat_flagm   == b.feat_flagm);
    REQUIRE(a.feat_flagm2  == b.feat_flagm2);
    REQUIRE(a.feat_dotprod == b.feat_dotprod);
    REQUIRE(a.feat_crc32   == b.feat_crc32);
    REQUIRE(a.feat_sha1    == b.feat_sha1);
    REQUIRE(a.feat_sha256  == b.feat_sha256);
    REQUIRE(a.feat_aes     == b.feat_aes);
}

TEST_CASE("host_features: test override returns the injected struct") {
    HostFeatures synthetic{};
    synthetic.feat_lse   = true;
    synthetic.feat_flagm = true;

    override_host_features_for_test(synthetic);
    const auto& view = host_features();
    REQUIRE(view.feat_lse);
    REQUIRE(view.feat_flagm);
    REQUIRE_FALSE(view.feat_lse2);

    clear_host_features_override();
    // After clearing, we're back to the detected (cached) values. We
    // don't assert what those are, only that a later call still works.
    (void)host_features();
}

TEST_CASE("host_features: apple-silicon hosts should expose FEAT_LSE") {
    // Skip on non-Apple hosts — the check is a sanity canary for
    // macOS/Apple Silicon CI runners, where FEAT_LSE has been standard
    // since M1 (2020). Every Apple Silicon mac we care about has it.
    clear_host_features_override();
#if defined(__APPLE__) && defined(__arm64__)
    REQUIRE(host_features().feat_lse);
#else
    SUCCEED("not an Apple Silicon host — assertion skipped");
#endif
}

TEST_CASE("host_features: apple-silicon hosts should expose SHA crypto") {
    // Same canary as FEAT_LSE: every M-series core ships the SHA-1 +
    // SHA-256 crypto extensions the F2-IR-060 lowering relies on.
    clear_host_features_override();
#if defined(__APPLE__) && defined(__arm64__)
    REQUIRE(host_features().feat_sha1);
    REQUIRE(host_features().feat_sha256);
    REQUIRE(host_features().feat_aes);
#else
    SUCCEED("not an Apple Silicon host — assertion skipped");
#endif
}
