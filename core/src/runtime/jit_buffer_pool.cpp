// core/src/runtime/jit_buffer_pool.cpp — F1-RT-009 implementation.

#include "prisma/jit_buffer_pool.hpp"

#include <cerrno>
#include <cstring>
#include <limits>
#include <new>
#include <stdexcept>
#include <string>

#if defined(_WIN32)
#  ifndef NOMINMAX
#    define NOMINMAX
#  endif
#  include <windows.h>
#else
#  include <sys/mman.h>
#  include <unistd.h>
#endif

#if defined(__APPLE__)
#include <pthread.h>

#include <libkern/OSCacheControl.h>
#endif

namespace prisma::runtime {

namespace {

#if defined(__APPLE__)
constexpr int kPoolMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_JIT;
constexpr int kPoolMmapProt = PROT_READ | PROT_WRITE | PROT_EXEC;
#elif !defined(_WIN32)
constexpr int kPoolMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS;
constexpr int kPoolMmapProt = PROT_READ | PROT_WRITE | PROT_EXEC;
#endif

std::size_t page_size() {
#if defined(_WIN32)
  SYSTEM_INFO info{};
  ::GetSystemInfo(&info);
  return static_cast<std::size_t>(info.dwPageSize);
#else
  static const std::size_t cached = static_cast<std::size_t>(::sysconf(_SC_PAGESIZE));
  return cached;
#endif
}

std::size_t round_up(std::size_t v, std::size_t align) noexcept {
  return (v + align - 1) & ~(align - 1);
}

bool is_aligned_4(std::uintptr_t value) noexcept {
  return (value & 0x3u) == 0;
}

bool signed_byte_delta(std::uintptr_t from, std::uintptr_t to, std::int64_t& out) noexcept {
  if (to >= from) {
    const std::uintptr_t delta = to - from;
    if (delta > static_cast<std::uintptr_t>(std::numeric_limits<std::int64_t>::max())) {
      return false;
    }
    out = static_cast<std::int64_t>(delta);
    return true;
  }

  const std::uintptr_t delta = from - to;
  if (delta > static_cast<std::uintptr_t>(std::numeric_limits<std::int64_t>::max())) {
    return false;
  }
  out = -static_cast<std::int64_t>(delta);
  return true;
}

void invalidate_icache(void* base, std::size_t size) {
#if defined(__APPLE__)
  sys_icache_invalidate(base, size);
#elif defined(_WIN32)
  ::FlushInstructionCache(::GetCurrentProcess(), base, size);
#else
  auto* begin = static_cast<char*>(base);
  auto* end = begin + size;
  __builtin___clear_cache(begin, end);
#endif
}

}  // namespace

JitSlabPool::JitSlabPool() = default;

JitSlabPool::~JitSlabPool() {
  std::lock_guard<std::mutex> lk{mu_};
  for (auto& s : slabs_)
    free_slab(s);
}

JitSlabPool::Slab JitSlabPool::allocate_slab(std::size_t bytes) {
  const std::size_t ps = page_size();
  const std::size_t size = round_up(bytes < kSlabBytes ? kSlabBytes : bytes, ps);

#if defined(_WIN32)
  void* p = ::VirtualAlloc(nullptr, size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
  if (p == nullptr) {
    throw std::bad_alloc{};
  }
#else
  void* p = ::mmap(nullptr, size, kPoolMmapProt, kPoolMmapFlags,
                   /*fd=*/-1,
                   /*offset=*/0);
  if (p == MAP_FAILED) {
    throw std::bad_alloc{};
  }
#endif

  Slab s;
  s.base = static_cast<std::uint8_t*>(p);
  s.capacity = size;
  s.cursor = 0;
  return s;
}

void JitSlabPool::free_slab(Slab& s) noexcept {
  if (s.base != nullptr) {
#if defined(_WIN32)
    ::VirtualFree(s.base, 0, MEM_RELEASE);
#else
    ::munmap(s.base, s.capacity);
#endif
    s.base = nullptr;
    s.capacity = 0;
    s.cursor = 0;
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
      target = &slabs_[i];
      target_idx = i;
      break;
    }
  }

  if (target == nullptr) {
    slabs_.push_back(allocate_slab(need));
    target = &slabs_.back();
    target_idx = slabs_.size() - 1;
    stats_.total_slabs = slabs_.size();
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

  target->cursor += need;
  stats_.total_bytes_used += need;
  stats_.acquire_count += 1;

  JitBlock blk;
  blk.entry = dst;
  blk.size_bytes = src.size();
  blk.slab_index = target_idx;
  blk.offset_in_slab = off;
  return blk;
}

void JitSlabPool::release(JitBlock /*blk*/) noexcept {
  // No-op MVP. Future: maintain a per-slab free list keyed on
  // (offset, size) and merge adjacent free spans on release.
}

JitPatchResult JitSlabPool::patch_aarch64_branch(const JitBlock& block, std::size_t offset,
                                                 const std::uint8_t* target) {
  if (target == nullptr || offset > block.size_bytes ||
      block.size_bytes - offset < sizeof(std::uint32_t)) {
    return JitPatchResult::SiteNotOwned;
  }

  std::lock_guard<std::mutex> lk{mu_};
  if (block.slab_index >= slabs_.size()) {
    return JitPatchResult::SiteNotOwned;
  }

  const Slab& slab = slabs_[block.slab_index];
  const auto* expected_entry = slab.base + block.offset_in_slab;
  if (block.entry != expected_entry || block.offset_in_slab > slab.cursor ||
      block.size_bytes > slab.cursor - block.offset_in_slab) {
    return JitPatchResult::SiteNotOwned;
  }

  std::uint8_t* site = slab.base + block.offset_in_slab + offset;
  const std::uintptr_t site_addr = reinterpret_cast<std::uintptr_t>(site);
  const std::uintptr_t target_addr = reinterpret_cast<std::uintptr_t>(target);
  if (!is_aligned_4(site_addr) || !is_aligned_4(target_addr)) {
    return JitPatchResult::Unaligned;
  }

  std::int64_t byte_delta = 0;
  if (!signed_byte_delta(site_addr, target_addr, byte_delta) || (byte_delta % 4) != 0) {
    return JitPatchResult::OutOfRange;
  }

  const std::int64_t word_delta = byte_delta / 4;
  constexpr std::int64_t kMinBranchWords = -(1LL << 25);
  constexpr std::int64_t kMaxBranchWords = (1LL << 25) - 1;
  if (word_delta < kMinBranchWords || word_delta > kMaxBranchWords) {
    return JitPatchResult::OutOfRange;
  }

  const std::uint32_t encoded =
      0x14000000u | (static_cast<std::uint32_t>(word_delta) & 0x03ffffffu);

#if defined(__APPLE__)
  pthread_jit_write_protect_np(/*write_protect=*/0);
  std::memcpy(site, &encoded, sizeof(encoded));
  pthread_jit_write_protect_np(/*write_protect=*/1);
#else
  std::memcpy(site, &encoded, sizeof(encoded));
#endif

  invalidate_icache(site, sizeof(encoded));
  return JitPatchResult::Patched;
}

JitPoolStats JitSlabPool::stats() const {
  std::lock_guard<std::mutex> lk{mu_};
  return stats_;
}

}  // namespace prisma::runtime
