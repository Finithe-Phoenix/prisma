// core/src/capi/capi.cpp — C ABI implementation (RFC 0014).
//
// Every export follows the same shape: validate arguments, do the
// work inside a try block, stop any exception at the boundary as
// PRISMA_STATUS_INTERNAL. Out-params are written only on PRISMA_OK.

#include "prisma/capi.h"

#include <algorithm>
#include <cstring>
#include <span>
#include <string>
#include <variant>

#include "prisma/cpu_state.hpp"
#include "prisma/dispatcher.hpp"
#include "prisma/translator.hpp"

struct prisma_translator {
    prisma::translator::Translator impl;
};

struct prisma_dispatcher {
    prisma_dispatcher(prisma::translator::Translator& t,
                      prisma::runtime::GuestMemoryReader r)
        : impl(t, std::move(r)) {}
    prisma::runtime::Dispatcher impl;
};

namespace {

prisma_status to_status(prisma::translator::TranslateError e) {
    using prisma::translator::TranslateError;
    switch (e) {
        case TranslateError::DecodeFailed:
            return PRISMA_STATUS_DECODE_FAILED;
        case TranslateError::LowerFailed:
            return PRISMA_STATUS_LOWER_FAILED;
        case TranslateError::EmptyInput:
            return PRISMA_STATUS_EMPTY_INPUT;
        case TranslateError::JitAllocFailed:
            return PRISMA_STATUS_JIT_ALLOC_FAILED;
    }
    return PRISMA_STATUS_INTERNAL;
}

void fill_block_info(const prisma::translator::TranslatedBlock& b,
                     prisma_block_info& out) {
    out.code_size = b.code_size;
    out.guest_size = b.guest_size;
    out.target_guest_pc = b.target_guest_pc;
    out.fallthrough_guest_pc = b.fallthrough_guest_pc;
    out.return_guest_pc = b.return_guest_pc;
    out.exit_kind = static_cast<int32_t>(b.exit_kind);
    out.from_cache = b.from_cache ? 1u : 0u;
    std::fill(std::begin(out.reserved), std::end(out.reserved),
              static_cast<uint8_t>(0));
}

void copy_message(const std::string& src, char (&dst)[128]) {
    const std::size_t n = std::min(src.size(), sizeof(dst) - 1);
    std::memcpy(dst, src.data(), n);
    dst[n] = '\0';
}

}  // namespace

extern "C" {

uint32_t prisma_capi_version(void) { return PRISMA_CAPI_VERSION; }

prisma_status prisma_translator_create(prisma_translator** out) {
    if (out == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    try {
        *out = new prisma_translator();
        return PRISMA_OK;
    } catch (...) {
        return PRISMA_STATUS_INTERNAL;
    }
}

void prisma_translator_destroy(prisma_translator* t) { delete t; }

prisma_status prisma_translator_translate(prisma_translator* t,
                                          uint64_t guest_addr,
                                          const uint8_t* bytes,
                                          size_t len,
                                          prisma_block_info* out_info) {
    if (t == nullptr || (bytes == nullptr && len != 0)) {
        return PRISMA_STATUS_INVALID_ARGUMENT;
    }
    try {
        const auto result =
            t->impl.translate(guest_addr, std::span<const uint8_t>(bytes, len));
        if (const auto* err =
                std::get_if<prisma::translator::TranslateError>(&result)) {
            return to_status(*err);
        }
        if (out_info != nullptr) {
            fill_block_info(
                std::get<prisma::translator::TranslatedBlock>(result),
                *out_info);
        }
        return PRISMA_OK;
    } catch (...) {
        return PRISMA_STATUS_INTERNAL;
    }
}

prisma_status prisma_translator_set_real_call_ret(prisma_translator* t,
                                                  int enabled) {
    if (t == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    t->impl.set_real_call_ret(enabled != 0);
    return PRISMA_OK;
}

prisma_status prisma_translator_get_stats(const prisma_translator* t,
                                          prisma_translator_stats* out) {
    if (t == nullptr || out == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    const auto& s = t->impl.stats();
    out->translations_attempted = s.translations_attempted;
    out->cache_hits = s.cache_hits;
    out->cache_misses = s.cache_misses;
    out->decode_failures = s.decode_failures;
    out->lower_failures = s.lower_failures;
    return PRISMA_OK;
}

prisma_status prisma_dispatcher_create(prisma_translator* t,
                                       prisma_mem_reader reader,
                                       void* ctx,
                                       prisma_dispatcher** out) {
    if (t == nullptr || reader == nullptr || out == nullptr) {
        return PRISMA_STATUS_INVALID_ARGUMENT;
    }
    try {
        prisma::runtime::GuestMemoryReader wrapped =
            [reader, ctx](std::uint64_t pc) -> std::span<const std::uint8_t> {
            const uint8_t* ptr = nullptr;
            const size_t n = reader(ctx, pc, &ptr);
            if (n == 0 || ptr == nullptr) return {};
            return {ptr, n};
        };
        *out = new prisma_dispatcher(t->impl, std::move(wrapped));
        return PRISMA_OK;
    } catch (...) {
        return PRISMA_STATUS_INTERNAL;
    }
}

void prisma_dispatcher_destroy(prisma_dispatcher* d) { delete d; }

prisma_status prisma_dispatcher_add_halt_pc(prisma_dispatcher* d,
                                            uint64_t pc) {
    if (d == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    try {
        d->impl.add_halt_pc(pc);
        return PRISMA_OK;
    } catch (...) {
        return PRISMA_STATUS_INTERNAL;
    }
}

prisma_status prisma_dispatcher_install_halt_return_stack(
    prisma_dispatcher* d) {
    if (d == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    d->impl.install_halt_return_stack();
    return PRISMA_OK;
}

prisma_status prisma_dispatcher_run(prisma_dispatcher* d,
                                    uint64_t entry_pc,
                                    size_t max_steps,
                                    prisma_run_result* out) {
    if (d == nullptr || out == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    try {
        const auto r = d->impl.run(entry_pc, max_steps);
        out->exit = static_cast<int32_t>(r.exit);
        std::fill(std::begin(out->reserved), std::end(out->reserved),
                  static_cast<uint8_t>(0));
        out->final_pc = r.final_pc;
        out->stats.blocks_executed = r.stats.blocks_executed;
        out->stats.steps_taken = r.stats.steps_taken;
        out->stats.unique_pcs_seen = r.stats.unique_pcs_seen;
        out->stats.ras_pushes = r.stats.ras_pushes;
        out->stats.ras_pops = r.stats.ras_pops;
        out->stats.ras_hits = r.stats.ras_hits;
        out->stats.ras_misses = r.stats.ras_misses;
        out->stats.ras_overflows = r.stats.ras_overflows;
        out->stats.ras_underflows = r.stats.ras_underflows;
        out->stats.direct_thread_hits = r.stats.direct_thread_hits;
        out->stats.direct_thread_misses = r.stats.direct_thread_misses;
        out->stats.direct_thread_installs = r.stats.direct_thread_installs;
        copy_message(r.message, out->message);
        return PRISMA_OK;
    } catch (...) {
        return PRISMA_STATUS_INTERNAL;
    }
}

prisma_status prisma_dispatcher_gpr_get(const prisma_dispatcher* d,
                                        uint32_t gpr_index,
                                        uint64_t* out) {
    if (d == nullptr || out == nullptr || gpr_index >= PRISMA_GPR_COUNT) {
        return PRISMA_STATUS_INVALID_ARGUMENT;
    }
    *out = d->impl.state().gpr[gpr_index];
    return PRISMA_OK;
}

prisma_status prisma_dispatcher_gpr_set(prisma_dispatcher* d,
                                        uint32_t gpr_index,
                                        uint64_t value) {
    if (d == nullptr || gpr_index >= PRISMA_GPR_COUNT) {
        return PRISMA_STATUS_INVALID_ARGUMENT;
    }
    d->impl.state().gpr[gpr_index] = value;
    return PRISMA_OK;
}

prisma_status prisma_dispatcher_guest_pc(const prisma_dispatcher* d,
                                         uint64_t* out) {
    if (d == nullptr || out == nullptr) return PRISMA_STATUS_INVALID_ARGUMENT;
    *out = d->impl.state().guest_pc;
    return PRISMA_OK;
}

}  // extern "C"
