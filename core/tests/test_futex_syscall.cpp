#include "prisma/cpu_state.hpp"
#include "prisma/futex_wait_table.hpp"
#include "prisma/ir.hpp"
#include "prisma/syscall_handler.hpp"

#include <catch2/catch_test_macros.hpp>
#include <catch2/generators/catch_generators.hpp>

#include <cerrno>
#include <chrono>
#include <cstdint>
#include <future>
#include <optional>
#include <thread>

using namespace prisma::ir;
using namespace prisma::runtime;

namespace {

constexpr std::uint64_t kGuestFutex = 0x1000;
constexpr std::uint64_t kFutexWait = 0;
constexpr std::uint64_t kFutexWake = 1;
constexpr std::uint64_t kFutexPrivate = 128;

std::int64_t futex_syscall(std::uint64_t address, std::uint64_t operation, std::uint64_t value,
                           std::uint64_t timeout = 0) {
  CpuStateFrame frame{};
  frame[Gpr::Rax] = 202;
  frame[Gpr::Rdi] = address;
  frame[Gpr::Rsi] = operation;
  frame[Gpr::Rdx] = value;
  frame[Gpr::R10] = timeout;
  prisma_syscall_handler(&frame);
  return static_cast<std::int64_t>(frame[Gpr::Rax]);
}
}  // namespace

TEST_CASE("syscall_handler: futex validates guest words through the bound reader") {
  FutexWaitTable table;
  SyscallRuntimeContext context{table, [](std::uint64_t address) {
                                  return address == kGuestFutex ? std::optional<std::uint32_t>{7}
                                                                : std::nullopt;
                                }};
  const auto private_flag = GENERATE(std::uint64_t{0}, kFutexPrivate);
  REQUIRE(futex_syscall(kGuestFutex, kFutexWait | private_flag, 6) == -ENOSYS);
  {
    ScopedSyscallRuntimeContext binding{context};
    CHECK(futex_syscall(kGuestFutex, kFutexWait | private_flag, 6) == -EAGAIN);
    CHECK(futex_syscall(kGuestFutex + 4, kFutexWait | private_flag, 6) == -EFAULT);
    CHECK(futex_syscall(kGuestFutex + 1, kFutexWake | private_flag, 1) == -EFAULT);
    CHECK(futex_syscall(kGuestFutex, kFutexWake | private_flag, 8) == 0);
    CHECK(table.waiter_count(kGuestFutex) == 0);
    auto unbound_thread =
        std::async(std::launch::async, [] { return futex_syscall(kGuestFutex, kFutexWake, 1); });
    CHECK(unbound_thread.get() == -ENOSYS);
  }
  CHECK(futex_syscall(kGuestFutex, kFutexWake | private_flag, 1) == -ENOSYS);
}

TEST_CASE("syscall_handler: futex rejects unsupported operations before reading guest memory") {
  FutexWaitTable table;
  unsigned reads = 0;
  SyscallRuntimeContext context{table, [&](std::uint64_t) {
                                  ++reads;
                                  return std::optional<std::uint32_t>{};
                                }};
  ScopedSyscallRuntimeContext binding{context};
  constexpr std::uint64_t kFutexRequeue = 3;
  constexpr std::uint64_t kFutexClockRealtime = 256;
  CHECK(futex_syscall(kGuestFutex, kFutexRequeue, 0) == -ENOSYS);
  CHECK(futex_syscall(kGuestFutex, kFutexWait | kFutexClockRealtime, 0) == -ENOSYS);
  CHECK(futex_syscall(kGuestFutex, kFutexWait, 0, 1) == -ENOSYS);
  CHECK(futex_syscall(kGuestFutex + 1, kFutexWait, 0) == -EFAULT);
  CHECK(reads == 0);
}

TEST_CASE("syscall_handler: futex wakes and interrupts waiters across host threads") {
  using namespace std::chrono_literals;
  const bool interrupt = GENERATE(false, true);
  FutexWaitTable table;
  SyscallRuntimeContext context{table, [](std::uint64_t address) {
                                  return address == kGuestFutex ? std::optional<std::uint32_t>{42}
                                                                : std::nullopt;
                                }};
  ScopedSyscallRuntimeContext binding{context};
  auto waiter = std::async(std::launch::async, [&] {
    ScopedSyscallRuntimeContext thread_binding{context};
    return futex_syscall(kGuestFutex, kFutexWait | kFutexPrivate, 42);
  });

  const auto deadline = std::chrono::steady_clock::now() + 5s;
  while (table.waiter_count(kGuestFutex) == 0 && std::chrono::steady_clock::now() < deadline) {
    std::this_thread::yield();
  }
  const bool queued = table.waiter_count(kGuestFutex) == 1;
  std::int64_t woken = 0;
  if (!interrupt && queued) {
    woken = futex_syscall(kGuestFutex, kFutexWake | kFutexPrivate, 1);
  }
  if (interrupt || !queued || woken != 1 || waiter.wait_for(5s) != std::future_status::ready) {
    table.shutdown();
  }
  const auto result = waiter.get();
  REQUIRE(queued);
  CHECK(woken == (interrupt ? 0 : 1));
  CHECK(result == (interrupt ? -EINTR : 0));
  CHECK(table.waiter_count(kGuestFutex) == 0);
}
