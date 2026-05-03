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
//   2. Invalidates every translation that covers that page (callback),
//   3. Briefly flips the page back to RW so the guest store completes
//      on retry, recording the page in a per-thread "needs reprotect"
//      flag the dispatcher consults on its next round trip.
//
// See docs/rfc/0010-smc-page-protection.md for the design rationale,
// trade-offs, and the alternatives considered.

#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <mutex>
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

    // Called from the SIGSEGV handler. If `fault_addr` falls inside a
    // tracked page:
    //   * Invokes `invalidate_cb` once per `cache_key` registered for
    //     the page, then forgets those keys.
    //   * Flips the page back to RW so the faulting store can retry.
    //   * Returns true (the handler should resume execution rather
    //     than longjmp).
    // If `fault_addr` is unrelated to any tracked page, returns false
    // (the handler should propagate the fault as it normally would).
    bool handle_fault(void* fault_addr,
                      const std::function<void(std::uint64_t)>& invalidate_cb);

    // For tests / inspection. O(1).
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
    // on_invalidate / handle_fault use the same wrapper.
    static bool protect_page(std::uint64_t page_base, bool writable) noexcept;

    mutable std::mutex mu_;
    std::unordered_map<std::uint64_t, std::vector<std::uint64_t>> protected_pages_;
};

// Process-global pointer the signal handler consults. The runtime sets
// this once during initialisation; tests usually leave it null.
void set_global_smc_guard(SmcGuard* guard) noexcept;
[[nodiscard]] SmcGuard* global_smc_guard() noexcept;

}  // namespace prisma::runtime
