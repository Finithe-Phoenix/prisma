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
#include <span>

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
    // returning false. I-cache is invalidated across the written range.
    void make_executable();

    // Entry point: pointer to the first instruction. Only valid to call
    // through this pointer *after* make_executable().
    [[nodiscard]] const std::uint8_t* entry() const noexcept { return base_; }

    [[nodiscard]] std::size_t capacity() const noexcept { return capacity_; }

    [[nodiscard]] bool is_executable() const noexcept { return executable_; }

private:
    std::uint8_t* base_{nullptr};
    std::size_t   capacity_{0};
    bool          executable_{false};
};

}  // namespace prisma::runtime
