// core/src/runtime/jit_memory.cpp — platform-specific JIT memory impl.

#include "prisma/jit_memory.hpp"

#include <algorithm>
#include <cerrno>
#include <cstdint>
#include <cstring>
#include <new>
#include <span>
#include <stdexcept>
#include <string>

#include <sys/mman.h>
#include <unistd.h>

#if defined(__APPLE__)
#  include <libkern/OSCacheControl.h>
#  include <pthread.h>
#  include <TargetConditionals.h>
#endif

namespace prisma::runtime {

namespace {

std::size_t page_size() {
    const long ps = ::sysconf(_SC_PAGESIZE);
    return (ps > 0) ? static_cast<std::size_t>(ps) : std::size_t{4096};
}

std::size_t round_up_to_page(std::size_t n, std::size_t ps) {
    return (n + ps - 1) & ~(ps - 1);
}

#if defined(__APPLE__)
// Apple silicon requires the MAP_JIT flag for W^X-enabled regions, plus
// per-thread write/execute toggle via pthread_jit_write_protect_np.
// On Intel macOS MAP_JIT is tolerated but not required. We always use it
// on Apple targets for consistency.
constexpr int kMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_JIT;
#else
constexpr int kMmapFlags = MAP_PRIVATE | MAP_ANONYMOUS;
#endif

// Invalidate the I-cache over [base, base + size) so newly-written code
// is seen by the fetcher. Different hosts, different primitives.
void invalidate_icache(void* base, std::size_t size) {
#if defined(__APPLE__)
    sys_icache_invalidate(base, size);
#else
    // __builtin___clear_cache takes a half-open range [begin, end).
    auto* begin = static_cast<char*>(base);
    auto* end   = begin + size;
    __builtin___clear_cache(begin, end);
#endif
}

}  // namespace

JitBuffer::JitBuffer(std::size_t min_bytes) {
    const std::size_t ps = page_size();
    const std::size_t size = round_up_to_page(min_bytes == 0 ? 1 : min_bytes, ps);

    void* p = ::mmap(nullptr,
                     size,
                     PROT_READ | PROT_WRITE,
                     kMmapFlags,
                     /*fd=*/-1,
                     /*offset=*/0);
    if (p == MAP_FAILED) {
        const int e = errno;
        throw std::bad_alloc{};
        (void)e;
    }

    base_ = static_cast<std::uint8_t*>(p);
    capacity_ = size;

#if defined(__APPLE__)
    // We want to write first, then execute. On Apple silicon that means
    // disabling write-protect on this thread before we touch the memory.
    pthread_jit_write_protect_np(/*write_protect=*/0);
#endif
}

JitBuffer::~JitBuffer() {
    if (base_ != nullptr) {
#if defined(__APPLE__)
        // Leave the thread in write-protected state regardless of state
        // on destruction, to avoid leaking the toggle.
        pthread_jit_write_protect_np(/*write_protect=*/1);
#endif
        ::munmap(base_, capacity_);
    }
}

bool JitBuffer::write(std::span<const std::uint8_t> src) {
    if (executable_) return false;
    if (src.size() > capacity_) return false;
    std::memcpy(base_, src.data(), src.size());
    written_size_ = src.size();
    return true;
}

void JitBuffer::make_executable() {
    if (executable_) return;

#if defined(__APPLE__)
    // Re-enable write-protect on this thread, then flip the mapping to
    // R+X. On Apple silicon the combination is what actually gives us
    // executable fetches.
    pthread_jit_write_protect_np(/*write_protect=*/1);
#endif

    if (::mprotect(base_, capacity_, PROT_READ | PROT_EXEC) != 0) {
        // Best-effort error handling — JIT failures at this stage are
        // catastrophic; escalate to a runtime exception with errno for
        // diagnostics. This is test-only code for now.
        const int e = errno;
        throw std::runtime_error(
            std::string{"JitBuffer::make_executable mprotect failed: "} +
            std::strerror(e));
    }

    invalidate_icache(base_, written_size_);
    executable_ = true;
}

JitBufferPool::JitBufferPool(std::size_t min_buffer_bytes)
    : min_buffer_bytes_(min_buffer_bytes) {}

JitBufferPool::~JitBufferPool() = default;

std::optional<JitBufferAllocation>
JitBufferPool::add(std::span<const std::uint8_t> code) {
    if (code.empty()) return std::nullopt;

    try {
        auto buffer = std::make_unique<JitBuffer>(
            std::max(min_buffer_bytes_, code.size()));
        if (!buffer->write(code)) return std::nullopt;
        buffer->make_executable();

        JitBufferAllocation allocation{
            /*index=*/0,
            /*entry=*/buffer->entry(),
            /*code_size=*/code.size(),
            /*capacity=*/buffer->capacity(),
        };

        std::lock_guard<std::mutex> lock(mutex_);
        allocation.index = buffers_.size();
        buffers_.push_back(std::move(buffer));
        return allocation;
    } catch (...) {
        return std::nullopt;
    }
}

const JitBuffer* JitBufferPool::get(std::size_t index) const {
    std::lock_guard<std::mutex> lock(mutex_);
    if (index >= buffers_.size()) return nullptr;
    return buffers_[index].get();
}

std::size_t JitBufferPool::size() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return buffers_.size();
}

}  // namespace prisma::runtime
