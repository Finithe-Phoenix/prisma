#include "prisma/futex_wait_table.hpp"

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <optional>
#include <stdexcept>
#include <thread>

using namespace prisma::runtime;
using namespace std::chrono_literals;

namespace {

void check(bool condition, const char* message) {
    if (!condition) {
        throw std::runtime_error(message);
    }
}

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

void test_validation() {
    FutexWaitTable table;
    std::atomic<std::uint32_t> word{7};
    std::size_t reads = 0;
    const auto read_word = [&](std::uint64_t) {
        ++reads;
        return std::optional<std::uint32_t>{
            word.load(std::memory_order_relaxed)};
    };

    check(table.wait(0x1001, 7, read_word, 0ns)
              == FutexWaitStatus::InvalidAddress,
          "misaligned futex address was accepted");
    check(reads == 0, "misaligned wait read guest memory");
    check(table.wait(0x1000, 8, read_word, 0ns)
              == FutexWaitStatus::ValueMismatch,
          "mismatched futex value did not fail immediately");
    check(table.waiter_count(0x1000) == 0,
          "non-blocking validation left a waiter behind");
}

void test_single_wake() {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x2000;
    FutexWaitStatus status = FutexWaitStatus::TimedOut;

    std::thread waiter([&]() {
        status = table.wait(
            kAddress,
            42,
            [](std::uint64_t) {
                return std::optional<std::uint32_t>{42};
            },
            2s);
    });

    check(wait_for_waiter_count(table, kAddress, 1),
          "waiter did not enter the futex queue");
    check(table.wake(kAddress, 1) == 1,
          "wake did not release one waiter");
    waiter.join();
    check(status == FutexWaitStatus::Woken,
          "waiter returned the wrong wake status");
    check(table.wake(kAddress, 1) == 0,
          "empty futex queue reported a wake");
}

void test_bounded_wake() {
    FutexWaitTable table;
    constexpr std::uint64_t kAddress = 0x3000;
    std::array<FutexWaitStatus, 2> statuses{
        FutexWaitStatus::TimedOut,
        FutexWaitStatus::TimedOut,
    };
    std::atomic<std::size_t> completed{0};

    const auto run_waiter = [&](std::size_t index) {
        statuses[index] = table.wait(
            kAddress,
            9,
            [](std::uint64_t) {
                return std::optional<std::uint32_t>{9};
            },
            2s);
        completed.fetch_add(1, std::memory_order_release);
    };

    std::array<std::thread, 2> waiters{
        std::thread(run_waiter, 0),
        std::thread(run_waiter, 1),
    };

    check(wait_for_waiter_count(table, kAddress, 2),
          "both waiters did not enter the futex queue");
    check(table.wake(kAddress, 1) == 1,
          "bounded wake released an unexpected count");
    check(wait_for_completed(completed, 1),
          "first bounded wake did not complete exactly one waiter");
    check(wait_for_waiter_count(table, kAddress, 1),
          "bounded wake did not leave one waiter sleeping");
    check(table.wake(kAddress, 8) == 1,
          "second wake did not release the remaining waiter");

    for (std::thread& waiter : waiters) {
        waiter.join();
    }
    check(statuses[0] == FutexWaitStatus::Woken
              && statuses[1] == FutexWaitStatus::Woken,
          "a bounded waiter returned the wrong status");
}

void test_timeout_and_shutdown() {
    {
        FutexWaitTable table;
        check(table.wait(
                  0x4000,
                  1,
                  [](std::uint64_t) {
                      return std::optional<std::uint32_t>{1};
                  },
                  10ms)
                  == FutexWaitStatus::TimedOut,
              "futex timeout did not expire");
        check(table.waiter_count(0x4000) == 0,
              "timed-out waiter was not removed");
    }

    FutexWaitTable table;
    FutexWaitStatus status = FutexWaitStatus::TimedOut;
    std::thread waiter([&]() {
        status = table.wait(
            0x5000,
            3,
            [](std::uint64_t) {
                return std::optional<std::uint32_t>{3};
            });
    });

    check(wait_for_waiter_count(table, 0x5000, 1),
          "shutdown waiter did not enter the queue");
    table.shutdown();
    waiter.join();
    check(status == FutexWaitStatus::Shutdown,
          "shutdown did not interrupt the waiter");
    check(table.is_shutdown(), "shutdown state was not retained");
    check(table.wake(0x5000, 1) == 0,
          "shutdown table accepted a wake");
}

}  // namespace

int main() {
    test_validation();
    test_single_wake();
    test_bounded_wake();
    test_timeout_and_shutdown();
    return 0;
}
