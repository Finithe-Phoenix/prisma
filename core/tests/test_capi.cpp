// tests/test_capi.cpp — C ABI surface (RFC 0014).
//
// Exercises the boundary the Rust shell consumes: handle lifecycle,
// argument validation, translate + cache behaviour, dispatcher run
// paths that do not execute JIT code (halt-at-entry, fetch fault),
// and — on ARM64 hosts only — a full guest program round trip.

#include <catch2/catch_test_macros.hpp>

#include <cstdint>
#include <cstring>
#include <vector>

#include "prisma/capi.h"

namespace {

constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif

// movabs rax, 42 ; ret  (B8+rd is the decoder's MOV-imm encoding)
const std::vector<std::uint8_t> kMovRax42Ret = {
    0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
};

struct GuestImage {
    std::uint64_t base{0};
    std::vector<std::uint8_t> bytes;
};

size_t image_reader(void* ctx, std::uint64_t pc,
                    const std::uint8_t** out_bytes) {
    const auto* img = static_cast<const GuestImage*>(ctx);
    if (pc < img->base || pc >= img->base + img->bytes.size()) return 0;
    const std::size_t off = static_cast<std::size_t>(pc - img->base);
    *out_bytes = img->bytes.data() + off;
    return img->bytes.size() - off;
}

size_t null_reader(void*, std::uint64_t, const std::uint8_t**) { return 0; }

}  // namespace

TEST_CASE("capi: version matches the header macro") {
    REQUIRE(prisma_capi_version() == PRISMA_CAPI_VERSION);
}

TEST_CASE("capi: translator lifecycle and argument validation") {
    REQUIRE(prisma_translator_create(nullptr) ==
            PRISMA_STATUS_INVALID_ARGUMENT);

    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);
    REQUIRE(t != nullptr);

    // NULL handle is rejected, not dereferenced.
    prisma_block_info info{};
    REQUIRE(prisma_translator_translate(nullptr, 0x1000,
                                        kMovRax42Ret.data(),
                                        kMovRax42Ret.size(),
                                        &info) ==
            PRISMA_STATUS_INVALID_ARGUMENT);
    REQUIRE(prisma_translator_translate(t, 0x1000, nullptr, 4, &info) ==
            PRISMA_STATUS_INVALID_ARGUMENT);

    // Empty input maps to the dedicated status.
    REQUIRE(prisma_translator_translate(t, 0x1000, kMovRax42Ret.data(), 0,
                                        &info) ==
            PRISMA_STATUS_EMPTY_INPUT);

    prisma_translator_destroy(t);
    prisma_translator_destroy(nullptr);  // no-op by contract
}

TEST_CASE("capi: translate populates block info and hits the cache") {
    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);

    prisma_block_info info{};
    REQUIRE(prisma_translator_translate(t, 0x1000, kMovRax42Ret.data(),
                                        kMovRax42Ret.size(), &info) ==
            PRISMA_OK);
    REQUIRE(info.guest_size == kMovRax42Ret.size());
    REQUIRE(info.code_size > 0);
    REQUIRE(info.exit_kind == PRISMA_BLOCK_EXIT_RET_ADJUSTED);
    REQUIRE(info.from_cache == 0);

    // Same bytes, same address: cache hit.
    prisma_block_info again{};
    REQUIRE(prisma_translator_translate(t, 0x1000, kMovRax42Ret.data(),
                                        kMovRax42Ret.size(), &again) ==
            PRISMA_OK);
    REQUIRE(again.from_cache == 1);

    // NULL out_info is allowed.
    REQUIRE(prisma_translator_translate(t, 0x1000, kMovRax42Ret.data(),
                                        kMovRax42Ret.size(), nullptr) ==
            PRISMA_OK);

    prisma_translator_stats stats{};
    REQUIRE(prisma_translator_get_stats(t, &stats) == PRISMA_OK);
    REQUIRE(stats.translations_attempted == 3);
    REQUIRE(stats.cache_misses == 1);
    REQUIRE(stats.cache_hits == 2);

    prisma_translator_destroy(t);
}

TEST_CASE("capi: legacy RET exit shape via set_real_call_ret(0)") {
    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);
    REQUIRE(prisma_translator_set_real_call_ret(t, 0) == PRISMA_OK);

    prisma_block_info info{};
    REQUIRE(prisma_translator_translate(t, 0x1000, kMovRax42Ret.data(),
                                        kMovRax42Ret.size(), &info) ==
            PRISMA_OK);
    REQUIRE(info.exit_kind == PRISMA_BLOCK_EXIT_RETURN);

    prisma_translator_destroy(t);
}

TEST_CASE("capi: dispatcher halt-at-entry runs zero blocks") {
    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);

    GuestImage img{0x1000, kMovRax42Ret};
    prisma_dispatcher* d = nullptr;
    REQUIRE(prisma_dispatcher_create(t, &image_reader, &img, &d) ==
            PRISMA_OK);

    REQUIRE(prisma_dispatcher_add_halt_pc(d, 0x1000) == PRISMA_OK);

    prisma_run_result r{};
    REQUIRE(prisma_dispatcher_run(d, 0x1000, 16, &r) == PRISMA_OK);
    REQUIRE(r.exit == PRISMA_DISPATCH_HALTED);
    REQUIRE(r.final_pc == 0x1000);
    REQUIRE(r.stats.blocks_executed == 0);
    REQUIRE(r.message[0] == '\0');

    prisma_dispatcher_destroy(d);
    prisma_dispatcher_destroy(nullptr);  // no-op by contract
    prisma_translator_destroy(t);
}

TEST_CASE("capi: reader returning 0 surfaces a fetch fault") {
    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);

    prisma_dispatcher* d = nullptr;
    REQUIRE(prisma_dispatcher_create(t, &null_reader, nullptr, &d) ==
            PRISMA_OK);

    prisma_run_result r{};
    REQUIRE(prisma_dispatcher_run(d, 0x4000, 16, &r) == PRISMA_OK);
    REQUIRE(r.exit == PRISMA_DISPATCH_FETCH_FAILED);
    REQUIRE(r.final_pc == 0x4000);
    REQUIRE(std::strlen(r.message) > 0);

    prisma_dispatcher_destroy(d);
    prisma_translator_destroy(t);
}

TEST_CASE("capi: gpr and guest_pc accessors validate and round-trip") {
    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);
    prisma_dispatcher* d = nullptr;
    REQUIRE(prisma_dispatcher_create(t, &null_reader, nullptr, &d) ==
            PRISMA_OK);

    REQUIRE(prisma_dispatcher_gpr_set(d, PRISMA_GPR_COUNT, 1) ==
            PRISMA_STATUS_INVALID_ARGUMENT);
    std::uint64_t v = 0;
    REQUIRE(prisma_dispatcher_gpr_get(d, PRISMA_GPR_COUNT, &v) ==
            PRISMA_STATUS_INVALID_ARGUMENT);

    REQUIRE(prisma_dispatcher_gpr_set(d, PRISMA_GPR_RDI, 0xDEADBEEFULL) ==
            PRISMA_OK);
    REQUIRE(prisma_dispatcher_gpr_get(d, PRISMA_GPR_RDI, &v) == PRISMA_OK);
    REQUIRE(v == 0xDEADBEEFULL);

    std::uint64_t pc = 1;
    REQUIRE(prisma_dispatcher_guest_pc(d, &pc) == PRISMA_OK);
    REQUIRE(pc == 0);

    prisma_dispatcher_destroy(d);
    prisma_translator_destroy(t);
}

TEST_CASE("capi: full guest program executes on ARM64") {
    if (!is_arm64) {
        SUCCEED("JIT execution requires an ARM64 host; translation-only "
                "coverage runs in the other cases");
        return;
    }

    prisma_translator* t = nullptr;
    REQUIRE(prisma_translator_create(&t) == PRISMA_OK);

    GuestImage img{0x1000, kMovRax42Ret};
    prisma_dispatcher* d = nullptr;
    REQUIRE(prisma_dispatcher_create(t, &image_reader, &img, &d) ==
            PRISMA_OK);
    REQUIRE(prisma_dispatcher_install_halt_return_stack(d) == PRISMA_OK);

    prisma_run_result r{};
    REQUIRE(prisma_dispatcher_run(d, 0x1000, 64, &r) == PRISMA_OK);
    REQUIRE(r.exit == PRISMA_DISPATCH_HALTED);
    REQUIRE(r.stats.blocks_executed == 1);

    std::uint64_t rax = 0;
    REQUIRE(prisma_dispatcher_gpr_get(d, PRISMA_GPR_RAX, &rax) == PRISMA_OK);
    REQUIRE(rax == 42);

    prisma_dispatcher_destroy(d);
    prisma_translator_destroy(t);
}
