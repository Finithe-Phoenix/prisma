// prisma/translator.hpp — top-level Translator facade.
//
// Unifies the five subsystems built so far into one public surface:
//
//   decoder  →  pass manager  →  lowerer  →  jit buffer  →  runtime
//                      ↑                                       │
//                      └────────── translation cache ──────────┘
//
// A Translator owns a cache, a pass pipeline, and whatever JIT buffers
// it has produced. Clients call translate(guest_addr, guest_bytes) to
// receive a callable entry point, with memoisation + SMC detection
// handled for them.
//
// Status: Fase 0 / early Fase 1 MVP. Scope:
//
//   * One block at a time (translate-until-control-flow-instruction).
//   * No thunking or calling convention marshalling — the raw
//     translated body is returned, the caller wraps it if needed.
//   * No flow-of-control linking between blocks (no dispatcher loop).
//   * Passes = default_pipeline() unless the caller overrides.
//
// The thunking / dispatcher / chaining come in subsequent sessions;
// having the facade now gives us a clean boundary to plug them in.

#pragma once

#include <cstdint>
#include <memory>
#include <span>
#include <string>
#include <unordered_map>
#include <variant>
#include <vector>

#include "prisma/decoder.hpp"
#include "prisma/jit_buffer_pool.hpp"
#include "prisma/passes.hpp"
#include "prisma/translation_cache.hpp"

namespace prisma::translator {

enum class TranslateError {
    DecodeFailed,        // decoder returned a DecodeError.
    LowerFailed,         // lowerer returned a LowerError.
    EmptyInput,          // zero guest bytes supplied.
    JitAllocFailed,      // could not allocate a JitBuffer.
};

struct TranslatedBlock {
    // Pointer to the executable code. Owned by the Translator; invalid
    // after Translator destruction or after cache eviction of the key.
    const std::uint8_t* code_entry{nullptr};
    // Size of the emitted code in bytes.
    std::size_t code_size{0};
    // Original guest region length (in bytes). Useful for the caller to
    // advance a PC cursor after this block completes.
    std::size_t guest_size{0};
    // True iff the result came from the cache (not freshly translated).
    bool from_cache{false};
};

using TranslateResult = std::variant<TranslatedBlock, TranslateError>;

class Translator {
public:
    Translator();
    ~Translator();

    Translator(const Translator&) = delete;
    Translator& operator=(const Translator&) = delete;
    Translator(Translator&&) = delete;
    Translator& operator=(Translator&&) = delete;

    // Translate the guest bytes starting at `guest_addr` up to the first
    // terminator (Return / unsupported op that ends a block) and return
    // the cached/fresh translation. On repeat calls with identical guest
    // bytes the cache path runs in O(1).
    //
    // `guest_bytes` MUST cover the full guest region the translation will
    // consume. The translator reads bytes until either a Return is seen
    // or the decoder runs out.
    [[nodiscard]] TranslateResult translate(
        std::uint64_t guest_addr,
        std::span<const std::uint8_t> guest_bytes);

    // Override the pass pipeline. Defaults to passes::default_pipeline().
    void set_pipeline(passes::PassManager pm);

    // Override the function-level (CFG-aware) pass pipeline. Defaults
    // to `passes::default_function_pipeline()` (`global_cse` →
    // `loop_invariant_motion`). The pipeline runs **between** decoding
    // and the stmt-level pipeline, on the CFG built from the decoded
    // stmts. For single-block functions (the common case today) the
    // function pipeline is skipped entirely.
    void set_function_pipeline(passes::FunctionPassManager pm);

    // The underlying cache, for tests and for the eventual runtime to
    // signal page invalidation.
    [[nodiscard]] cache::TranslationCache& cache() noexcept { return cache_; }

    // Simple stats — useful for tests and for the future Pilar 1 ML
    // classifier input.
    struct Stats {
        std::size_t translations_attempted{0};
        std::size_t cache_hits{0};
        std::size_t cache_misses{0};
        std::size_t decode_failures{0};
        std::size_t lower_failures{0};
    };
    [[nodiscard]] const Stats& stats() const noexcept { return stats_; }

private:
    // Internal book-keeping for each translation we own. The entry
    // pointer is owned by the JitBufferPool's slab list; it stays
    // valid for the lifetime of the Translator.
    struct Record {
        const std::uint8_t* entry{nullptr};
        std::size_t code_size{0};
        std::size_t guest_size{0};
        std::uint64_t content_hash{0};
    };

    passes::PassManager           pipeline_;
    passes::FunctionPassManager   function_pipeline_;
    cache::TranslationCache       cache_;
    // Pool that owns every translated region. F1-RT-009: replaces the
    // previous one-mmap-per-translation pattern.
    runtime::JitBufferPool pool_;
    // Lookup by guest_addr → Record. The persistent TranslationCache
    // is updated in parallel so future Fase 2.5 P2P distribution sees
    // entries.
    std::unordered_map<std::uint64_t, Record> by_addr_;
    Stats stats_;
};

}  // namespace prisma::translator
