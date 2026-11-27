// prisma/futex_wait_table.hpp — portable FUTEX_WAIT/FUTEX_WAKE backing.
//
// RFC 0022 maps guest futexes to host condition variables instead of exposing
// the guest arena directly to the host futex ABI. The table is keyed by guest
// virtual address and closes the lost-wakeup window by re-reading the guest
// word while holding the per-address wait-queue lock.

#pragma once

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <list>
#include <memory>
#include <mutex>
#include <optional>
#include <unordered_map>
#include <utility>

namespace prisma::runtime {

enum class FutexWaitStatus : std::uint8_t {
  Woken,
  ValueMismatch,
  TimedOut,
  InvalidAddress,
  Shutdown,
};

class FutexWaitTable {
 public:
  using Duration = std::chrono::nanoseconds;
  using ReadWord = std::function<std::optional<std::uint32_t>(std::uint64_t guest_addr)>;

  FutexWaitTable() : state_(std::make_shared<State>()) {}

  FutexWaitTable(const FutexWaitTable&) = delete;
  FutexWaitTable& operator=(const FutexWaitTable&) = delete;
  FutexWaitTable(FutexWaitTable&&) = delete;
  FutexWaitTable& operator=(FutexWaitTable&&) = delete;

  ~FutexWaitTable() { shutdown(); }

  [[nodiscard]] FutexWaitStatus wait(std::uint64_t guest_addr, std::uint32_t expected,
                                     const ReadWord& read_word,
                                     std::optional<Duration> timeout = std::nullopt) {
    if ((guest_addr % alignof(std::uint32_t)) != 0 || !read_word) {
      return FutexWaitStatus::InvalidAddress;
    }

    const std::shared_ptr<State> state = state_;
    const std::shared_ptr<Entry> entry = acquire_entry(state, guest_addr, /*create=*/true);
    if (!entry) {
      return FutexWaitStatus::Shutdown;
    }

    std::unique_lock entry_lock(entry->mutex);
    std::optional<std::uint32_t> observed;
    try {
      observed = read_word(guest_addr);
    } catch (...) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      throw;
    }

    if (!observed.has_value()) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      return FutexWaitStatus::InvalidAddress;
    }
    if (*observed != expected) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      return FutexWaitStatus::ValueMismatch;
    }
    if (state->shutting_down.load(std::memory_order_acquire)) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      return FutexWaitStatus::Shutdown;
    }

    // Grant wakes to specific threads already parked here. A later WAIT
    // on the same address must never consume an earlier thread's wake.
    std::list<Waiter>::iterator slot;
    try {
      slot = entry->waiters.emplace(entry->waiters.end());
    } catch (...) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      throw;
    }
    const auto ready = [&]() noexcept {
      return state->shutting_down.load(std::memory_order_acquire) || slot->woken;
    };
    bool signaled = true;
    if (timeout.has_value()) {
      signaled = slot->cv.wait_for(entry_lock, *timeout, ready);
    } else {
      slot->cv.wait(entry_lock, ready);
    }
    FutexWaitStatus status = FutexWaitStatus::Woken;
    if (!signaled) {
      status = FutexWaitStatus::TimedOut;
    } else if (state->shutting_down.load(std::memory_order_acquire)) {
      status = FutexWaitStatus::Shutdown;
    }
    entry->waiters.erase(slot);

    entry_lock.unlock();
    release_entry(state, guest_addr, entry);
    return status;
  }

  [[nodiscard]] std::size_t wake(std::uint64_t guest_addr, std::size_t max_count) {
    if (max_count == 0 || state_->shutting_down.load(std::memory_order_acquire)) {
      return 0;
    }

    const std::shared_ptr<State> state = state_;
    const std::shared_ptr<Entry> entry = acquire_entry(state, guest_addr, /*create=*/false);
    if (!entry) {
      return 0;
    }

    std::unique_lock entry_lock(entry->mutex);
    if (state->shutting_down.load(std::memory_order_acquire)) {
      entry_lock.unlock();
      release_entry(state, guest_addr, entry);
      return 0;
    }

    std::size_t wake_count = 0;
    for (Waiter& waiter : entry->waiters) {
      if (!waiter.woken) {
        waiter.woken = true;
        waiter.cv.notify_one();
        if (++wake_count == max_count) {
          break;
        }
      }
    }
    entry_lock.unlock();
    release_entry(state, guest_addr, entry);
    return wake_count;
  }

  void shutdown() noexcept {
    const std::shared_ptr<State> state = state_;
    if (!state || state->shutting_down.exchange(true, std::memory_order_acq_rel)) {
      return;
    }

    decltype(state->entries) entries;
    {
      std::lock_guard table_lock(state->table_mutex);
      entries.swap(state->entries);
    }

    for (const auto& [address, entry] : entries) {
      (void)address;
      // Synchronize with the final predicate check before parking.
      std::lock_guard entry_lock(entry->mutex);
      for (Waiter& waiter : entry->waiters) {
        waiter.cv.notify_one();
      }
    }
  }

  [[nodiscard]] bool is_shutdown() const noexcept {
    return state_->shutting_down.load(std::memory_order_acquire);
  }

  [[nodiscard]] std::size_t waiter_count(std::uint64_t guest_addr) const noexcept {
    const std::shared_ptr<State> state = state_;
    std::shared_ptr<Entry> entry;
    {
      std::lock_guard table_lock(state->table_mutex);
      const auto found = state->entries.find(guest_addr);
      if (found == state->entries.end()) {
        return 0;
      }
      entry = found->second;
    }

    std::lock_guard entry_lock(entry->mutex);
    return entry->waiters.size();
  }

 private:
  struct Waiter {
    std::condition_variable cv;
    bool woken{false};
  };

  struct Entry {
    std::mutex mutex;
    std::list<Waiter> waiters;
    std::size_t users{0};
  };

  struct State {
    std::mutex table_mutex;
    std::unordered_map<std::uint64_t, std::shared_ptr<Entry>> entries;
    std::atomic<bool> shutting_down{false};
  };

  [[nodiscard]] static std::shared_ptr<Entry> acquire_entry(const std::shared_ptr<State>& state,
                                                            std::uint64_t guest_addr, bool create) {
    std::lock_guard table_lock(state->table_mutex);
    if (state->shutting_down.load(std::memory_order_acquire)) {
      return {};
    }

    const auto found = state->entries.find(guest_addr);
    if (found != state->entries.end()) {
      ++found->second->users;
      return found->second;
    }
    if (!create) {
      return {};
    }

    auto entry = std::make_shared<Entry>();
    entry->users = 1;
    state->entries.emplace(guest_addr, entry);
    return entry;
  }

  static void release_entry(const std::shared_ptr<State>& state, std::uint64_t guest_addr,
                            const std::shared_ptr<Entry>& entry) noexcept {
    std::lock_guard table_lock(state->table_mutex);
    const auto found = state->entries.find(guest_addr);
    if (found == state->entries.end() || found->second != entry) {
      return;
    }

    if (entry->users != 0) {
      --entry->users;
    }
    if (entry->users == 0) {
      state->entries.erase(found);
    }
  }

  std::shared_ptr<State> state_;
};

}  // namespace prisma::runtime
