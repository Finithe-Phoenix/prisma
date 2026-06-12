// core/tests/test_smc_guard.cpp — F1-RT-010 SmcGuard surface tests.
//
// Exercises the page-tracking + invalidation API in isolation from
// the JIT runtime. The signal-handler integration is *not* covered
// here — re-entering a SIGSEGV handler under Catch2 is brittle on
// macOS. The handler hook is unit-tested only via the synthetic
// `handle_fault` path.
//
// Pages handed to `on_translate` must be real, mappable memory so
// `mprotect` actually succeeds. We `mmap` a couple of anonymous
// pages, hand their addresses to the SmcGuard, and observe through
// the public API.

#include <catch2/catch_test_macros.hpp>

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <vector>

#include <sys/mman.h>
#include <unistd.h>

#include "prisma/smc_guard.hpp"

using namespace prisma::runtime;

namespace {

// Host page size lookup — mprotect granularity. On Apple Silicon
// this is 16 KiB; on x86-64 Linux it is 4 KiB. The SmcGuard rounds
// up internally; the test mmap must hand out enough memory for two
// distinct *host* pages so the multi-page tests still exercise two
// independent protected regions.
std::size_t host_page() {
    long v = ::sysconf(_SC_PAGESIZE);
    return v > 0 ? static_cast<std::size_t>(v) : 4096;
}

// RAII wrapper around two adjacent anonymous host pages. The two
// "guest pages" we present to the SmcGuard line up with the start
// of each host page so that mprotect operates cleanly.
class TwoPages {
public:
    TwoPages() {
        bytes_ = 2 * host_page();
        void* p = ::mmap(nullptr, bytes_,
                         PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS,
                         -1, 0);
        REQUIRE(p != MAP_FAILED);
        base_ = reinterpret_cast<std::uint8_t*>(p);
    }
    ~TwoPages() {
        if (base_ != nullptr) {
            ::munmap(base_, bytes_);
        }
    }

    [[nodiscard]] std::uint64_t page0() const noexcept {
        return reinterpret_cast<std::uint64_t>(base_);
    }
    [[nodiscard]] std::uint64_t page1() const noexcept {
        return page0() + host_page();
    }
    [[nodiscard]] std::uint8_t* raw0() const noexcept { return base_; }

private:
    std::uint8_t* base_{nullptr};
    std::size_t   bytes_{0};
};

}  // namespace

TEST_CASE("smc_guard: on_translate records and on_invalidate clears single-page entries") {
    TwoPages pages;
    SmcGuard g;

    g.on_translate(pages.page0(), 16, /*cache_key=*/42);
    REQUIRE(g.tracked_page_count() == 1);
    REQUIRE(g.is_tracked(pages.page0()));
    REQUIRE_FALSE(g.is_tracked(pages.page1()));

    g.on_invalidate(/*cache_key=*/42);
    REQUIRE(g.tracked_page_count() == 0);
    REQUIRE_FALSE(g.is_tracked(pages.page0()));
}

TEST_CASE("smc_guard: multi-page translation registers every page it covers") {
    TwoPages pages;
    SmcGuard g;

    // Translation that spans the boundary between page1 and the next
    // host page. We use page1 (start of the second host page) so the
    // guest-page split lands cleanly on a host-page boundary too,
    // letting mprotect succeed on both. The tracked pages reported
    // back are the two guest pages straddled by the translation.
    const std::uint64_t pc = pages.page1() - 4;
    g.on_translate(pc, /*guest_byte_len=*/8, /*cache_key=*/7);
    REQUIRE(g.tracked_page_count() == 2);
    REQUIRE(g.is_tracked(pc));
    REQUIRE(g.is_tracked(pages.page1()));

    g.on_invalidate(/*cache_key=*/7);
    REQUIRE(g.tracked_page_count() == 0);
}

TEST_CASE("smc_guard: handle_fault returns false for an unrelated address") {
    TwoPages pages;
    SmcGuard g;

    g.on_translate(pages.page0(), 8, /*cache_key=*/1);

    int callbacks = 0;
    auto cb = [&](std::uint64_t) { ++callbacks; };

    // Address one full page past anything we registered — definitely not
    // in our map.
    void* unrelated = reinterpret_cast<void*>(pages.page1() + kGuestPageSize);
    REQUIRE_FALSE(g.handle_fault(unrelated));
    REQUIRE(g.drain_pending(cb) == 0);
    REQUIRE(callbacks == 0);
    REQUIRE(g.is_tracked(pages.page0()));  // still tracked
}

TEST_CASE("smc_guard: handle_fault fires invalidate callback for each cache_key on the page") {
    TwoPages pages;
    SmcGuard g;

    g.on_translate(pages.page0(), 8,  /*cache_key=*/100);
    g.on_translate(pages.page0(), 12, /*cache_key=*/101);
    g.on_translate(pages.page1(), 4,  /*cache_key=*/200);

    std::vector<std::uint64_t> seen;
    auto cb = [&](std::uint64_t key) { seen.push_back(key); };

    void* fault = reinterpret_cast<void*>(pages.page0() + 2);
    // handle_fault is signal-safe and only tombstones + queues; the
    // callbacks fire on the normal-context drain.
    REQUIRE(g.handle_fault(fault));
    REQUIRE(seen.empty());
    REQUIRE(g.drain_pending(cb) == 1);
    REQUIRE(seen.size() == 2);
    // Order is implementation-defined; check both keys present.
    bool saw100 = false, saw101 = false;
    for (auto k : seen) {
        if (k == 100) saw100 = true;
        if (k == 101) saw101 = true;
    }
    REQUIRE(saw100);
    REQUIRE(saw101);

    // page0 entry was consumed; page1 untouched.
    REQUIRE_FALSE(g.is_tracked(pages.page0()));
    REQUIRE(g.is_tracked(pages.page1()));
}

TEST_CASE("smc_guard: round-trip — shared-page translations, partial then full invalidate") {
    TwoPages pages;
    SmcGuard g;

    // Two translations that both live on page0.
    g.on_translate(pages.page0() + 0,  16, /*cache_key=*/1);
    g.on_translate(pages.page0() + 64, 16, /*cache_key=*/2);
    REQUIRE(g.tracked_page_count() == 1);

    // Invalidate one — the page is still tracked because the other
    // translation keeps it referenced.
    g.on_invalidate(/*cache_key=*/1);
    REQUIRE(g.tracked_page_count() == 1);
    REQUIRE(g.is_tracked(pages.page0()));

    // Invalidate the second — page is now clean and writable again.
    g.on_invalidate(/*cache_key=*/2);
    REQUIRE(g.tracked_page_count() == 0);

    // Confirm the page is genuinely RW: a store must not fault.
    volatile std::uint8_t* p = pages.raw0();
    *p = 0xAA;
    REQUIRE(*p == 0xAA);
}

TEST_CASE("smc_guard: re-translating the same key on a page does not duplicate it") {
    TwoPages pages;
    SmcGuard g;

    g.on_translate(pages.page0(), 8, /*cache_key=*/9);
    g.on_translate(pages.page0(), 8, /*cache_key=*/9);  // dedup

    int callbacks = 0;
    auto cb = [&](std::uint64_t) { ++callbacks; };
    void* fault = reinterpret_cast<void*>(pages.page0() + 4);
    REQUIRE(g.handle_fault(fault));
    REQUIRE(g.drain_pending(cb) == 1);
    REQUIRE(callbacks == 1);
}

TEST_CASE("smc_guard: global pointer setter / getter round-trip") {
    REQUIRE(global_smc_guard() == nullptr);
    SmcGuard g;
    set_global_smc_guard(&g);
    REQUIRE(global_smc_guard() == &g);
    set_global_smc_guard(nullptr);
    REQUIRE(global_smc_guard() == nullptr);
}
