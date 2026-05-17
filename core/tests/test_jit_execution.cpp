// core/tests/test_jit_execution.cpp — actually execute code we emit.
//
// This is the fourth and final test layer:
//
//   test_arm64_encoding  — bits correct
//   test_emitter         — vixl produces the same bits
//   test_decoder         — x86 bytes → IR correct
//   test_e2e             — bytes → IR → ARM64 bytes chain
//   test_jit_execution   — ARM64 bytes actually run and return what we expect
//
// Only meaningful on ARM64 hosts (Apple silicon macOS, Linux aarch64).
// On x86_64 CI we skip the execute step but still validate that the
// JitBuffer mmap/mprotect sequence succeeds.

#include <catch2/catch_test_macros.hpp>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <thread>
#include <vector>

#include "prisma/arm64_encoding.hpp"
#include "prisma/emitter.hpp"
#include "prisma/jit_memory.hpp"

using namespace prisma;

namespace {
constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif
}  // namespace

TEST_CASE("JitBuffer allocates, accepts writes, and flips to executable") {
    runtime::JitBuffer buf(64);
    REQUIRE(buf.capacity() >= 64);
    REQUIRE(buf.written_size() == 0);
    REQUIRE_FALSE(buf.is_executable());

    const std::uint8_t nops[] = {0x1F, 0x20, 0x03, 0xD5};  // ARM64 NOP, 4 bytes
    REQUIRE(buf.write(nops));
    REQUIRE(buf.written_size() == sizeof(nops));

    buf.make_executable();
    REQUIRE(buf.is_executable());
    REQUIRE(buf.entry() != nullptr);

    // Second make_executable is a no-op.
    buf.make_executable();
    REQUIRE(buf.is_executable());

    // write() after make_executable must refuse.
    REQUIRE_FALSE(buf.write(nops));
    REQUIRE(buf.written_size() == sizeof(nops));
}

TEST_CASE("JitBuffer rejects oversized writes without changing written size") {
    runtime::JitBuffer buf(4);
    const std::uint8_t one_word[] = {0x1F, 0x20, 0x03, 0xD5};
    REQUIRE(buf.write(one_word));
    REQUIRE(buf.written_size() == sizeof(one_word));

    const std::vector<std::uint8_t> too_large(buf.capacity() + 1u, 0x00);
    REQUIRE_FALSE(buf.write(too_large));
    REQUIRE(buf.written_size() == sizeof(one_word));
}

TEST_CASE("JitBufferPool publishes executable buffers with stable indices") {
    runtime::JitBufferPool pool;
    const std::uint8_t nops[] = {0x1F, 0x20, 0x03, 0xD5};

    const auto first = pool.add(nops);
    const auto second = pool.add(nops);

    REQUIRE(first.has_value());
    REQUIRE(second.has_value());
    REQUIRE(first->index == 0);
    REQUIRE(second->index == 1);
    REQUIRE(pool.size() == 2);

    const runtime::JitBuffer* first_buf = pool.get(first->index);
    const runtime::JitBuffer* second_buf = pool.get(second->index);
    REQUIRE(first_buf != nullptr);
    REQUIRE(second_buf != nullptr);
    REQUIRE(first_buf->entry() == first->entry);
    REQUIRE(second_buf->entry() == second->entry);
    REQUIRE(first_buf->is_executable());
    REQUIRE(second_buf->is_executable());
}

TEST_CASE("JitBufferPool accepts concurrent publishers") {
    runtime::JitBufferPool pool;
    const std::vector<std::uint8_t> nops = {0x1F, 0x20, 0x03, 0xD5};
    constexpr std::size_t kThreads = 4;
    constexpr std::size_t kPerThread = 8;

    std::atomic<std::size_t> successful{0};
    std::vector<std::thread> workers;
    workers.reserve(kThreads);
    for (std::size_t t = 0; t < kThreads; ++t) {
        workers.emplace_back([&] {
            for (std::size_t i = 0; i < kPerThread; ++i) {
                const auto allocation = pool.add(nops);
                if (allocation && allocation->entry != nullptr
                    && allocation->code_size == nops.size()) {
                    successful.fetch_add(1, std::memory_order_relaxed);
                }
            }
        });
    }

    for (auto& worker : workers) worker.join();

    REQUIRE(successful.load(std::memory_order_relaxed) == kThreads * kPerThread);
    REQUIRE(pool.size() == kThreads * kPerThread);
    for (std::size_t i = 0; i < pool.size(); ++i) {
        REQUIRE(pool.get(i) != nullptr);
    }
}

TEST_CASE("JIT execution: emit and run `uint64_t f() { return 42; }`",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // AAPCS64: function return value goes in x0. Emit `movz x0, #42; ret`.
    backend::Emitter em;
    em.movz(arm64::Reg::X0, 42, 0);
    em.ret();
    em.finalize();

    const auto bytes = em.code_bytes();
    REQUIRE(bytes.size() == 8);

    runtime::JitBuffer jit(bytes.size());
    REQUIRE(jit.write(bytes));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    const std::uint64_t result = fn();
    REQUIRE(result == 42u);
}

TEST_CASE("JIT execution: larger immediate via mov_imm64", "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    backend::Emitter em;
    em.mov_imm64(arm64::Reg::X0, 0xBADC0FFEE0DDF00DULL);
    em.ret();
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    REQUIRE(fn() == 0xBADC0FFEE0DDF00DULL);
}
