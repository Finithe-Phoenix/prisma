// core/src/runtime/smc_guard.cpp — page-protection SMC tracking.
//
// POSIX-only (Linux + Darwin). Windows path lands in the host
// integration when we do that work; until then `mprotect` is the
// only platform call we make.
//
// Locking: a single mutex around `protected_pages_`. The fault rate
// is dominated by SMC events (very low for non-self-modifying guests),
// so contention is not a concern; lock-free is deferred.
//
// On `mprotect` failure we log via `fprintf(stderr)` and continue
// without protection on that page. This is documented in the RFC's
// "Trade-offs" section: silent degradation to "always pay the hash
// cost" is preferable to crashing the dispatcher.

#include "prisma/smc_guard.hpp"

#include <algorithm>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <functional>
#include <mutex>
#include <vector>

#include <sys/mman.h>
#include <unistd.h>

namespace prisma::runtime {

namespace {

std::atomic<SmcGuard*> g_smc_guard{nullptr};

// Host page size (e.g. 4 KiB on x86-64 Linux, 16 KiB on Apple
// Silicon). mprotect operates in units of this size on every POSIX
// host; if the guest page is smaller, we round up.
std::size_t host_page_size() noexcept {
    static const std::size_t cached = []() -> std::size_t {
        long v = ::sysconf(_SC_PAGESIZE);
        return v > 0 ? static_cast<std::size_t>(v) : 4096;
    }();
    return cached;
}

}  // namespace

void set_global_smc_guard(SmcGuard* guard) noexcept {
    g_smc_guard.store(guard, std::memory_order_release);
}

SmcGuard* global_smc_guard() noexcept {
    return g_smc_guard.load(std::memory_order_acquire);
}

bool SmcGuard::protect_page(std::uint64_t page_addr, bool writable) noexcept {
    // We deliberately omit PROT_EXEC. The host CPU never executes the
    // guest bytes directly — the JIT translates them into host code
    // that lives in a separate `JitBuffer`. So PROT_READ is enough to
    // catch SMC writes via SIGSEGV, and dropping PROT_EXEC keeps us
    // out of macOS Apple Silicon's MAP_JIT W^X enforcement.
    int prot = PROT_READ;
    if (writable) {
        prot |= PROT_WRITE;
    }
    // mprotect requires host-page alignment + multiples. Round the
    // start down and the length up to the host page size. If the host
    // page is larger than the guest page (e.g. macOS arm64: 16 KiB
    // host vs. 4 KiB guest) this over-protects, which is correct but
    // marginally less precise — see RFC 0009 "Trade-offs".
    const std::size_t hps = host_page_size();
    const std::uint64_t base = page_addr & ~static_cast<std::uint64_t>(hps - 1);
    const std::size_t len = (kGuestPageSize + (hps - 1)) & ~(hps - 1);
    void* addr = reinterpret_cast<void*>(static_cast<std::uintptr_t>(base));
    if (::mprotect(addr, len, prot) != 0) {
        std::fprintf(stderr,
                     "prisma::SmcGuard: mprotect(0x%llx, %zu, 0x%x) failed; "
                     "continuing without page protection\n",
                     static_cast<unsigned long long>(base),
                     len,
                     static_cast<unsigned>(prot));
        return false;
    }
    return true;
}

void SmcGuard::on_translate(std::uint64_t guest_pc,
                            std::size_t   guest_byte_len,
                            std::uint64_t cache_key) {
    if (guest_byte_len == 0) {
        return;
    }
    const std::uint64_t first = page_base(guest_pc);
    const std::uint64_t last  = page_base(guest_pc + guest_byte_len - 1);

    std::lock_guard<std::mutex> lock(mu_);
    for (std::uint64_t p = first; p <= last; p += kGuestPageSize) {
        auto& keys = protected_pages_[p];
        const bool first_key = keys.empty();
        // Skip duplicate entries — re-translation of the same key is a
        // no-op for tracking purposes.
        if (std::find(keys.begin(), keys.end(), cache_key) == keys.end()) {
            keys.push_back(cache_key);
        }
        if (first_key) {
            // First translation on this page: arm the protection.
            (void)protect_page(p, /*writable=*/false);
        }
    }
}

void SmcGuard::on_invalidate(std::uint64_t cache_key) {
    std::lock_guard<std::mutex> lock(mu_);
    for (auto it = protected_pages_.begin(); it != protected_pages_.end();) {
        auto& keys = it->second;
        keys.erase(std::remove(keys.begin(), keys.end(), cache_key), keys.end());
        if (keys.empty()) {
            const std::uint64_t page = it->first;
            (void)protect_page(page, /*writable=*/true);
            it = protected_pages_.erase(it);
        } else {
            ++it;
        }
    }
}

bool SmcGuard::handle_fault(
    void* fault_addr,
    const std::function<void(std::uint64_t)>& invalidate_cb) {
    const std::uint64_t addr =
        static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(fault_addr));
    const std::uint64_t page = page_base(addr);

    std::vector<std::uint64_t> victims;
    {
        std::lock_guard<std::mutex> lock(mu_);
        auto it = protected_pages_.find(page);
        if (it == protected_pages_.end()) {
            return false;
        }
        victims = std::move(it->second);
        protected_pages_.erase(it);
        // Flip back to writable so the guest store can retry.
        (void)protect_page(page, /*writable=*/true);
    }

    // Run callbacks outside the lock — they may call back into the
    // guard via on_invalidate and we want to avoid recursion.
    for (auto key : victims) {
        invalidate_cb(key);
    }
    return true;
}

std::size_t SmcGuard::tracked_page_count() const {
    std::lock_guard<std::mutex> lock(mu_);
    return protected_pages_.size();
}

bool SmcGuard::is_tracked(std::uint64_t addr) const {
    std::lock_guard<std::mutex> lock(mu_);
    return protected_pages_.find(page_base(addr)) != protected_pages_.end();
}

}  // namespace prisma::runtime
