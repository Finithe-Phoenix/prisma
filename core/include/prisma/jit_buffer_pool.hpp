// prisma/jit_buffer_pool.hpp — F1-RT-009 thread-safe pooled allocator
// for translated machine code.
//
// Why: today every Translator::translate call allocates a fresh
// JitBuffer (one mmap per translated block). Many small translations
// → many small mmaps → wasted syscalls and address-space
// fragmentation. JitBufferPool amortises the cost by carving out
// "slabs" of `kSlabBytes` bytes (default 1 MiB) and bump-allocating
// translated regions inside them. Reuse of freed regions is deferred
// to a follow-up; today `release` is a no-op.
//
// Thread safety: a single std::mutex guards the slab list and the
// per-slab cursor. `acquire()` is the only mutating method; the
// returned `JitBlock::entry()` is stable for the pool's lifetime.
//
// Apple silicon (MAP_JIT) lifecycle: each slab is allocated with
// MAP_JIT|PROT_READ|PROT_WRITE|PROT_EXEC and lives entirely R+W+X.
// The pthread_jit_write_protect_np dance is performed inside acquire()
// for the byte-copy and again before returning so concurrent threads
// never observe a writable slab they didn't request.

#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <span>
#include <vector>

namespace prisma::runtime {

inline constexpr std::size_t kSlabBytes = 1u << 20;  // 1 MiB
inline constexpr std::size_t kBlockAlignment = 16u;  // 16-byte aligned starts

// Opaque handle to a region inside the pool. Hold for as long as you
// need the translated code to be reachable; pass back to `release` when
// done (no-op today).
struct JitBlock {
    const std::uint8_t* entry{nullptr};   // first byte of the region
    std::size_t         size_bytes{0};    // capacity, not used count
    std::size_t         slab_index{0};    // for `release`
    std::size_t         offset_in_slab{0};
};

struct JitPoolStats {
    std::size_t total_slabs{0};
    std::size_t total_bytes_allocated{0};   // sum of slab sizes
    std::size_t total_bytes_used{0};        // sum of acquired regions
    std::size_t acquire_count{0};
};

class JitSlabPool {
public:
    JitSlabPool();
    ~JitSlabPool();

    JitSlabPool(const JitSlabPool&)            = delete;
    JitSlabPool& operator=(const JitSlabPool&) = delete;
    JitSlabPool(JitSlabPool&&)                 = delete;
    JitSlabPool& operator=(JitSlabPool&&)      = delete;

    // Carve out a region of at least `bytes` and copy `src` into it.
    // Returns a JitBlock pointing at the (now executable) entry.
    // Throws std::bad_alloc on slab-allocation failure. `bytes` is
    // rounded up to kBlockAlignment.
    [[nodiscard]] JitBlock acquire(std::span<const std::uint8_t> src);

    // No-op today. Reserved for the future free-list reuse pass.
    void release(JitBlock blk) noexcept;

    [[nodiscard]] JitPoolStats stats() const;

private:
    struct Slab {
        std::uint8_t* base{nullptr};
        std::size_t   capacity{0};
        std::size_t   cursor{0};   // next free byte offset
    };

    [[nodiscard]] Slab allocate_slab(std::size_t bytes);
    void               free_slab(Slab& s) noexcept;

    mutable std::mutex      mu_;
    std::vector<Slab>       slabs_;
    JitPoolStats            stats_{};
};

}  // namespace prisma::runtime
