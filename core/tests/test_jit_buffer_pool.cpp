// core/tests/test_jit_buffer_pool.cpp — F1-RT-009.

#include <atomic>
#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <random>
#include <set>
#include <thread>
#include <utility>
#include <vector>

#include "prisma/jit_buffer_pool.hpp"

using namespace prisma;

namespace {

// AArch64 `ret` instruction encoding (always returns).
constexpr std::uint8_t kRetInstruction[] = {0xC0, 0x03, 0x5F, 0xD6};

}  // namespace

TEST_CASE("JitBufferPool: single acquire serves a small region") {
    runtime::JitSlabPool pool;
    auto blk = pool.acquire(std::span<const std::uint8_t>(kRetInstruction));
    REQUIRE(blk.entry != nullptr);
    REQUIRE(blk.size_bytes == 4);
    auto s = pool.stats();
    REQUIRE(s.total_slabs == 1);
    REQUIRE(s.acquire_count == 1);
    REQUIRE(s.total_bytes_used >= 4);
}

TEST_CASE("JitBufferPool: many small acquires fit in a single slab") {
    runtime::JitSlabPool pool;
    std::vector<runtime::JitBlock> blocks;
    blocks.reserve(1000);
    for (int i = 0; i < 1000; ++i) {
        blocks.push_back(pool.acquire(
            std::span<const std::uint8_t>(kRetInstruction)));
    }
    auto s = pool.stats();
    REQUIRE(s.total_slabs == 1);  // 1000 * 16-byte aligned = 16 KiB << 1 MiB
    REQUIRE(s.acquire_count == 1000);

    // Every block has a unique entry pointer.
    std::set<const std::uint8_t*> ptrs;
    for (const auto& b : blocks) ptrs.insert(b.entry);
    REQUIRE(ptrs.size() == 1000);
}

TEST_CASE("JitBufferPool: acquire past slab capacity allocates a new slab") {
    runtime::JitSlabPool pool;
    // Allocate close to a full slab worth.
    std::vector<std::uint8_t> big(runtime::kSlabBytes - 1024u, 0xFFu);
    // Fill the first slab with one big-ish allocation.
    auto a = pool.acquire(std::span<const std::uint8_t>(big));
    REQUIRE(a.entry != nullptr);
    REQUIRE(pool.stats().total_slabs == 1);

    // The next acquire of equal size cannot fit and forces a new slab.
    auto b = pool.acquire(std::span<const std::uint8_t>(big));
    REQUIRE(b.entry != nullptr);
    REQUIRE(pool.stats().total_slabs == 2);
    REQUIRE(b.entry != a.entry);
}

TEST_CASE("JitBufferPool: a > kSlabBytes acquire allocates a custom slab") {
    runtime::JitSlabPool pool;
    std::vector<std::uint8_t> huge(runtime::kSlabBytes + 4096u, 0u);
    auto blk = pool.acquire(std::span<const std::uint8_t>(huge));
    REQUIRE(blk.entry != nullptr);
    REQUIRE(blk.size_bytes == huge.size());
    auto s = pool.stats();
    REQUIRE(s.total_slabs == 1);
    REQUIRE(s.total_bytes_allocated >= huge.size());
}

TEST_CASE("JitBufferPool: concurrent acquires from N threads serialise") {
    runtime::JitSlabPool pool;
    constexpr int kThreads = 4;
    constexpr int kPerThread = 200;

    std::vector<std::thread> threads;
    std::vector<std::vector<runtime::JitBlock>> per_thread(kThreads);
    threads.reserve(kThreads);

    for (int t = 0; t < kThreads; ++t) {
        threads.emplace_back([&, t]() {
            std::mt19937 rng(static_cast<unsigned>(t * 31 + 7));
            std::uniform_int_distribution<unsigned> dist(16u, 256u);
            for (int i = 0; i < kPerThread; ++i) {
                const std::size_t n = dist(rng);
                std::vector<std::uint8_t> bytes(n, static_cast<std::uint8_t>(t));
                per_thread[static_cast<std::size_t>(t)].push_back(
                    pool.acquire(std::span<const std::uint8_t>(bytes)));
            }
        });
    }
    for (auto& th : threads) th.join();

    // Every thread saw its acquires complete; no entry pointer overlaps
    // with any other thread's range.
    std::set<const std::uint8_t*> all_starts;
    std::size_t total = 0;
    for (auto& v : per_thread) {
        REQUIRE(v.size() == kPerThread);
        for (const auto& b : v) {
            auto [it, inserted] = all_starts.insert(b.entry);
            REQUIRE(inserted);
            total += b.size_bytes;
        }
    }

    auto s = pool.stats();
    REQUIRE(s.acquire_count == kThreads * kPerThread);
    REQUIRE(s.total_bytes_used >= total);
}

TEST_CASE("JitBufferPool: release is a no-op (MVP)") {
    runtime::JitSlabPool pool;
    auto a = pool.acquire(std::span<const std::uint8_t>(kRetInstruction));
    pool.release(a);
    auto b = pool.acquire(std::span<const std::uint8_t>(kRetInstruction));
    // Future free-list reuse will let `b.entry == a.entry`. For now
    // they MUST differ.
    REQUIRE(b.entry != a.entry);
}
