// prisma/smc_guard.hpp — F1-RT-010 page-protection-based SMC detection.
//
// Companion mechanism to the FNV-1a content hash in `TranslationCache`.
// The hash gives a robust per-lookup answer; the page guard catches
// guest writes *before* they invalidate state, so the dispatcher pays
// nothing on the hot path until SMC actually happens.
//
// Approach: every guest page that contains at least one cached
// translation is mapped READ | EXEC (no WRITE) via mprotect. A guest
// `mov [code_addr], al` traps SIGSEGV; our signal handler routes the
// fault into `SmcGuard::handle_fault`, which:
//   1. Looks up the page,
//   2. Tombstones it and queues it for invalidation (the actual cache
//      callbacks run later, in normal context, via `drain_pending` —
//      a signal handler must not allocate, free, or take a
//      std::mutex; TSan caught the original in-handler design),
//   3. Briefly flips the page back to RW so the guest store completes
//      on retry.
// Correctness before the drain happens is covered by the cache's
// content-hash check; the drain is the eviction/bookkeeping side.
//
// See docs/rfc/0010-smc-page-protection.md for the design rationale,
// trade-offs, and the alternatives considered.

#pragma once

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <unordered_map>
#include <vector>

namespace prisma::runtime {

// Guest page size for protection bookkeeping. We assume 4 KiB for
// Phase 1; revisited when we lift this to the actual guest's effective
// page size (16 KiB on Apple Silicon hosts is a separate concern that
// affects host-side mprotect granularity, not the guest model).
inline constexpr std::size_t kGuestPageSize = 4096;

class SmcGuard {
public:
    SmcGuard() = default;
    ~SmcGuard() = default;

    SmcGuard(const SmcGuard&)            = delete;
    SmcGuard& operator=(const SmcGuard&) = delete;
    SmcGuard(SmcGuard&&)                 = delete;
    SmcGuard& operator=(SmcGuard&&)      = delete;

    // Register a freshly-created translation. Marks every guest page
    // touched by `[guest_pc, guest_pc + guest_byte_len)` READ | EXEC
    // and remembers that `cache_key` covers each of those pages.
    //
    // If `mprotect` fails (returns -1) we log to stderr and continue
    // without protection on that page — never crash the runtime.
    void on_translate(std::uint64_t guest_pc,
                      std::size_t   guest_byte_len,
                      std::uint64_t cache_key);

    // Drop `cache_key` from every page that referenced it. If a page
    // becomes empty as a result, restore its RW protection.
    void on_invalidate(std::uint64_t cache_key);

    // Called from the SIGSEGV handler — async-signal-safe: no
    // allocation, no deallocation, no std::mutex (a spinlock guards
    // the table; mutators never run guest code while holding it, so
    // the faulting thread can never self-deadlock).
    // If `fault_addr` falls inside a tracked page:
    //   * Tombstones the page and queues it for `drain_pending`.
    //   * Flips the page back to RW so the faulting store can retry.
    //   * Returns true (the handler should resume execution rather
    //     than longjmp).
    // If `fault_addr` is unrelated to any tracked page (or already
    // tombstoned), returns false.
    bool handle_fault(void* fault_addr);

    // Normal-context companion to handle_fault: erases tombstoned
    // pages and invokes `invalidate_cb` once per cache_key that was
    // registered on them. Called by the dispatcher between blocks.
    // Returns the number of pages drained. Cheap when nothing is
    // pending (one spinlock acquire + counter check).
    std::size_t drain_pending(
        const std::function<void(std::uint64_t)>& invalidate_cb);

    // For tests / inspection. O(n) over live (non-tombstoned) pages.
    [[nodiscard]] std::size_t tracked_page_count() const;

    // For tests: returns true iff the page containing `addr` has at
    // least one tracked translation right now.
    [[nodiscard]] bool is_tracked(std::uint64_t addr) const;

    // Round `addr` down to the nearest guest-page boundary.
    [[nodiscard]] static constexpr std::uint64_t page_base(std::uint64_t addr) noexcept {
        return addr & ~static_cast<std::uint64_t>(kGuestPageSize - 1);
    }

private:
    // Apply mprotect to a single page. Returns true on success, false
    // (and logs to stderr) on failure. Centralised so on_translate /
    // on_invalidate / handle_fault use the same wrapper. mprotect is
    // async-signal-safe per POSIX.
    static bool protect_page(std::uint64_t page_base, bool writable) noexcept;

    // Signal-safe spinlock: handle_fault runs in SIGSEGV context where
    // std::mutex is not allowed. Mutators hold it only for short,
    // guest-code-free critical sections.
    class SpinLock {
    public:
        void lock() noexcept {
            while (flag_.test_and_set(std::memory_order_acquire)) {}
        }
        void unlock() noexcept { flag_.clear(std::memory_order_release); }

    private:
        std::atomic_flag flag_ = ATOMIC_FLAG_INIT;
    };

    struct PageEntry {
        std::vector<std::uint64_t> keys;
        // Set by handle_fault (no allocation in signal context);
        // erased + reported by drain_pending in normal context.
        bool dead{false};
    };

    mutable SpinLock mu_;
    std::unordered_map<std::uint64_t, PageEntry> protected_pages_;

    // Fault queue filled by handle_fault under mu_. Fixed capacity so
    // the handler never allocates; on overflow drain_pending falls
    // back to a full tombstone sweep (lossless, just slower).
    static constexpr std::size_t kPendingCap = 256;
    std::array<std::uint64_t, kPendingCap> pending_{};
    std::size_t pending_count_{0};
    bool        pending_overflow_{false};
};

// Process-global pointer the signal handler consults. The runtime sets
// this once during initialisation; tests usually leave it null.
void set_global_smc_guard(SmcGuard* guard) noexcept;
[[nodiscard]] SmcGuard* global_smc_guard() noexcept;

}  // namespace prisma::runtime
