// prisma/jit_memory.hpp — W^X-safe allocator for executable code.
//
// Provides a JitBuffer that gives us a region of memory we can write code
// into and then execute. Three platforms matter (in order of relevance):
//
//   * Apple silicon macOS: requires MAP_JIT + per-thread write/execute
//     toggle via pthread_jit_write_protect_np. W^X is enforced.
//
//   * Linux ARM64 / x86_64: mmap PROT_READ|PROT_WRITE, fill, mprotect to
//     PROT_READ|PROT_EXEC. Classic two-step.
//
//   * Future: Android (same as Linux, but targetSdk 29+ enforces W^X on
//     app-private memory unless we use MAP_JIT-equivalent via
//     anonymous mapping with a specific flag set). Out of scope for
//     Fase 0; handled in Fase 3 when we port to Android.
//
// Status: Fase 0 skeleton. Single-shot buffer; no extend, no shrink. Used
// only from tests right now. The real translation cache (Pillar 4) will
// replace this with an allocator that spans multiple guest basic blocks
// and persists across sessions.

#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <vector>

namespace prisma::runtime {

class JitBuffer {
public:
    // Allocate an executable buffer of at least `min_bytes`. The actual
    // allocation is rounded up to the host page size. Throws std::bad_alloc
    // on failure.
    explicit JitBuffer(std::size_t min_bytes);
    ~JitBuffer();

    JitBuffer(const JitBuffer&) = delete;
    JitBuffer& operator=(const JitBuffer&) = delete;
    JitBuffer(JitBuffer&&) = delete;
    JitBuffer& operator=(JitBuffer&&) = delete;

    // Copy `src` into the buffer at offset 0. The buffer must be in
    // write-enabled state (it starts in this state on construction).
    // Returns true iff `src.size() <= capacity()`.
    bool write(std::span<const std::uint8_t> src);

    // Flip the buffer to executable. After this call `write()` is a no-op
    // returning false. I-cache is invalidated across the bytes most
    // recently written with write().
    void make_executable();

    // Entry point: pointer to the first instruction. Only valid to call
    // through this pointer *after* make_executable().
    [[nodiscard]] const std::uint8_t* entry() const noexcept { return base_; }

    [[nodiscard]] std::size_t capacity() const noexcept { return capacity_; }
    [[nodiscard]] std::size_t written_size() const noexcept { return written_size_; }

    [[nodiscard]] bool is_executable() const noexcept { return executable_; }

private:
    std::uint8_t* base_{nullptr};
    std::size_t   capacity_{0};
    std::size_t   written_size_{0};
    bool          executable_{false};
};

struct JitBufferAllocation {
    std::size_t         index{0};
    const std::uint8_t* entry{nullptr};
    std::size_t         code_size{0};
    std::size_t         capacity{0};
};

class JitBufferPool {
public:
    explicit JitBufferPool(std::size_t min_buffer_bytes = 0);
    ~JitBufferPool();

    JitBufferPool(const JitBufferPool&) = delete;
    JitBufferPool& operator=(const JitBufferPool&) = delete;
    JitBufferPool(JitBufferPool&&) = delete;
    JitBufferPool& operator=(JitBufferPool&&) = delete;

    // Allocate a new executable buffer, copy `code` into it, flip it to
    // executable, then publish it into the pool. Returns nullopt on empty
    // input or allocation/protection failure.
    [[nodiscard]] std::optional<JitBufferAllocation>
    add(std::span<const std::uint8_t> code);

    // Borrow an owned buffer by index. The returned pointer remains stable
    // until the pool is destroyed; pushing more buffers will not move the
    // underlying JitBuffer objects.
    [[nodiscard]] const JitBuffer* get(std::size_t index) const;

    [[nodiscard]] std::size_t size() const;

private:
    std::size_t min_buffer_bytes_{0};
    mutable std::mutex mutex_;
    std::vector<std::unique_ptr<JitBuffer>> buffers_;
};

}  // namespace prisma::runtime
