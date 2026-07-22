#include "prisma/futex_wait_table.hpp"

#include <catch2/catch_test_macros.hpp>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <optional>
#include <thread>

using namespace prisma::runtime;
using namespace std::chrono_literals;

namespace {

bool wait_for_waiter_count(
    const FutexWaitTable& table,
    std::uint64_t guest_addr,
    std::size_t expected) {
    const auto deadline = std::chrono::steady_clock::now() + 2s;
    while (std::chrono::steady_clock::now() < deadline) {
        if (table.waiter_count(guest_addr) == expected) {
            return true;
        }
        std::this_thread::sleep_for(1ms);
    }
    return table.waiter_count(guest_addr) == expected;
}

bool wait_for_completed(
    const std::atomic<std::size_t>& completed,
    std::size_t expected) {
    const auto deadline = std::chrono::steady_clock::now() + 2s;
    while (std::chrono::steady_clock::now() < deadline) {
        if (completed.load(std::memory_order_acquire) == expected) {
            return true;
        }
        std::this_thread::sleep_for(1ms);
    }
    return completed.load(std::memory_order_acquire) == expected;
}

}  // namespace

TEST_CASE("FutexWaitTable: validates address and expected word") {
    FutexWaitTable table;
    std::atomic<std::uint32_t> word{7};
    std::size_t reads = 0;
    const auto read_word = [&](std::uint64_t) {
        ++reads;
        return std::optional<std::uint32_t>{
            word.load(std::memory_order_relaxed)};
    };

    REQUIRE(table.wait(0x1001, 7, read_word, 0ns)
            == FutexWaitStatus::InvalidAddress);
    REQUIRE(reads == 0);

    REQUIRE(table.wait(0x1000, 8, read_word, 0ns)
            == FutexWaitStatus::ValueMismatch);
    REQUIRE(reads == 1);

    const auto invalid_read = [](std::uint64_t) {
        return std::optional<std::uint32_t>{};
    };
    REQUIRE(table.wait(0x1000, 7, invalid_read, 0ns)
            == FutexWaitStatus::InvalidAddress);
    REQUIRE(table.waiter_count(0x1000) == 0);
}

TEST_CASE("FutexWaitTable: wake releases one matching waiter") {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x2000;
    std::atomic<std::uint32_t> word{42};
    FutexWaitStatus status = FutexWaitStatus::TimedOut;

    std::thread waiter([&]() {
        status = table.wait(
            kAddress,
            42,
            [&](std::uint64_t) {
                return std::optional<std::uint32_t>{
                    word.load(std::memory_order_relaxed)};
            },
            2s);
    });

    REQUIRE(wait_for_waiter_count(table, kAddress, 1));
    REQUIRE(table.wake(kAddress, 1) == 1);
    waiter.join();

    REQUIRE(status == FutexWaitStatus::Woken);
    REQUIRE(table.waiter_count(kAddress) == 0);
    REQUIRE(table.wake(kAddress, 1) == 0);
}

TEST_CASE("FutexWaitTable: wake count is bounded by sleeping waiters") {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x3000;
    std::atomic<std::uint32_t> word{9};
    std::array<FutexWaitStatus, 2> statuses{
        FutexWaitStatus::TimedOut,
        FutexWaitStatus::TimedOut,
    };
    std::atomic<std::size_t> completed{0};

    std::array<std::thread, 2> waiters{
        std::thread([&]() {
            statuses[0] = table.wait(
                kAddress,
                9,
                [&](std::uint64_t) {
                    return std::optional<std::uint32_t>{
                        word.load(std::memory_order_relaxed)};
                },
                2s);
            completed.fetch_add(1, std::memory_order_release);
        }),
        std::thread([&]() {
            statuses[1] = table.wait(
                kAddress,
                9,
                [&](std::uint64_t) {
                    return std::optional<std::uint32_t>{
                        word.load(std::memory_order_relaxed)};
                },
                2s);
            completed.fetch_add(1, std::memory_order_release);
        }),
    };

    REQUIRE(wait_for_waiter_count(table, kAddress, 2));
    REQUIRE(table.wake(kAddress, 1) == 1);
    REQUIRE(wait_for_completed(completed, 1));
    REQUIRE(wait_for_waiter_count(table, kAddress, 1));
    REQUIRE(table.wake(kAddress, 8) == 1);

    for (std::thread& waiter : waiters) {
        waiter.join();
    }

    REQUIRE(statuses[0] == FutexWaitStatus::Woken);
    REQUIRE(statuses[1] == FutexWaitStatus::Woken);
    REQUIRE(completed.load(std::memory_order_acquire) == 2);
    REQUIRE(table.waiter_count(kAddress) == 0);
}

TEST_CASE("FutexWaitTable: timeout removes the waiter cleanly") {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x4000;

    const FutexWaitStatus status = table.wait(
        kAddress,
        1,
        [](std::uint64_t) {
            return std::optional<std::uint32_t>{1};
        },
        10ms);

    REQUIRE(status == FutexWaitStatus::TimedOut);
    REQUIRE(table.waiter_count(kAddress) == 0);
    REQUIRE(table.wake(kAddress, 1) == 0);
}

TEST_CASE("FutexWaitTable: shutdown interrupts all current and future waits") {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x5000;
    FutexWaitStatus status = FutexWaitStatus::TimedOut;

    std::thread waiter([&]() {
        status = table.wait(
            kAddress,
            3,
            [](std::uint64_t) {
                return std::optional<std::uint32_t>{3};
            });
    });

    REQUIRE(wait_for_waiter_count(table, kAddress, 1));
    table.shutdown();
    waiter.join();

    REQUIRE(status == FutexWaitStatus::Shutdown);
    REQUIRE(table.is_shutdown());
    REQUIRE(table.waiter_count(kAddress) == 0);
    REQUIRE(table.wake(kAddress, 1) == 0);
    REQUIRE(table.wait(
                kAddress,
                3,
                [](std::uint64_t) {
                    return std::optional<std::uint32_t>{3};
                },
                0ns)
            == FutexWaitStatus::Shutdown);
}
