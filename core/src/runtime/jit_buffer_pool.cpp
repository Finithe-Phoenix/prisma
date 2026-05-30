// core/src/runtime/jit_buffer_pool.cpp — F1-RT-009 implementation.

#include "prisma/jit_buffer_pool.hpp"

#include <cerrno>
#include <cstring>
#include <new>
#include <stdexcept>
#include <string>
#include <sys/mman.h>
#include <unistd.h>

#if defined(__APPLE__)
#include <libkern/OSCacheControl.h>
#include <pthread.h>
#endif

namespace prisma::runtime {

namespace {

#if defined(__APPLE__)
constexpr int kPoolMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_JIT;
constexpr int kPoolMmapProt  = PROT_READ | PROT_WRITE | PROT_EXEC;
#else
constexpr int kPoolMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS;
constexpr int kPoolMmapProt  = PROT_READ | PROT_WRITE | PROT_EXEC;
#endif

std::size_t page_size() {
    static const std::size_t cached = static_cast<std::size_t>(::sysconf(_SC_PAGESIZE));
    return cached;
}

std::size_t round_up(std::size_t v, std::size_t align) noexcept {
    return (v + align - 1) & ~(align - 1);
}

void invalidate_icache(void* base, std::size_t size) {
#if defined(__APPLE__)
    sys_icache_invalidate(base, size);
#else
    auto* begin = static_cast<char*>(base);
    auto* end   = begin + size;
    __builtin___clear_cache(begin, end);
#endif
}

}  // namespace

JitSlabPool::JitSlabPool() = default;

JitSlabPool::~JitSlabPool() {
    std::lock_guard<std::mutex> lk{mu_};
    for (auto& s : slabs_) free_slab(s);
}

JitSlabPool::Slab JitSlabPool::allocate_slab(std::size_t bytes) {
    const std::size_t ps   = page_size();
    const std::size_t size = round_up(bytes < kSlabBytes ? kSlabBytes : bytes, ps);

    void* p = ::mmap(nullptr,
                     size,
                     kPoolMmapProt,
                     kPoolMmapFlags,
                     /*fd=*/-1,
                     /*offset=*/0);
    if (p == MAP_FAILED) {
        throw std::bad_alloc{};
    }

    Slab s;
    s.base     = static_cast<std::uint8_t*>(p);
    s.capacity = size;
    s.cursor   = 0;
    return s;
}

void JitSlabPool::free_slab(Slab& s) noexcept {
    if (s.base != nullptr) {
        ::munmap(s.base, s.capacity);
        s.base     = nullptr;
        s.capacity = 0;
        s.cursor   = 0;
    }
}

JitBlock JitSlabPool::acquire(std::span<const std::uint8_t> src) {
    const std::size_t need = round_up(src.size(), kBlockAlignment);

    std::lock_guard<std::mutex> lk{mu_};

    // Find a slab that can fit this allocation, else allocate a new
    // one. We always check the most-recent slab first (LIFO) — the
    // common case is sequential acquires of comparable sizes.
    Slab* target = nullptr;
    std::size_t target_idx = 0;
    for (std::size_t i = slabs_.size(); i-- > 0;) {
        if (slabs_[i].cursor + need <= slabs_[i].capacity) {
            target     = &slabs_[i];
            target_idx = i;
            break;
        }
    }

    if (target == nullptr) {
        slabs_.push_back(allocate_slab(need));
        target     = &slabs_.back();
        target_idx = slabs_.size() - 1;
        stats_.total_slabs           = slabs_.size();
        stats_.total_bytes_allocated += target->capacity;
    }

    const std::size_t off = target->cursor;
    std::uint8_t* dst = target->base + off;

#if defined(__APPLE__)
    // Drop write-protect for this thread, copy bytes, re-enable.
    pthread_jit_write_protect_np(/*write_protect=*/0);
    std::memcpy(dst, src.data(), src.size());
    pthread_jit_write_protect_np(/*write_protect=*/1);
#else
    std::memcpy(dst, src.data(), src.size());
#endif

    invalidate_icache(dst, src.size());

    target->cursor          += need;
    stats_.total_bytes_used += need;
    stats_.acquire_count    += 1;

    JitBlock blk;
    blk.entry          = dst;
    blk.size_bytes     = src.size();
    blk.slab_index     = target_idx;
    blk.offset_in_slab = off;
    return blk;
}

void JitSlabPool::release(JitBlock /*blk*/) noexcept {
    // No-op MVP. Future: maintain a per-slab free list keyed on
    // (offset, size) and merge adjacent free spans on release.
}

JitPoolStats JitSlabPool::stats() const {
    std::lock_guard<std::mutex> lk{mu_};
    return stats_;
}

}  // namespace prisma::runtime
