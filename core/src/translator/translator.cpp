// core/src/translator/translator.cpp — Translator facade implementation.

#include "prisma/translator.hpp"

#include <cstring>
#include <memory>
#include <span>
#include <utility>
#include <variant>

#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"
#include "prisma/jit_memory.hpp"
#include "prisma/lowering.hpp"

namespace prisma::translator {

namespace {

// Decode forward until we either hit a terminator (Return) or exhaust
// the input. Returns the collected statements and how many guest bytes
// were consumed.
struct Decoded {
    std::vector<ir::Stmt> stmts;
    std::size_t consumed{0};
};

std::variant<Decoded, decoder::DecodeError>
decode_until_terminator(std::span<const std::uint8_t> bytes) {
    Decoded out;
    ir::Ref next = 0;
    std::size_t cursor = 0;
    bool terminated = false;

    while (cursor < bytes.size()) {
        auto res = decoder::decode_one(bytes.subspan(cursor), next);
        if (std::holds_alternative<decoder::DecodeError>(res)) {
            return std::get<decoder::DecodeError>(res);
        }
        auto& d = std::get<decoder::Decoded>(res);
        for (auto& s : d.stmts) {
            if (std::holds_alternative<ir::Return>(s.op)) {
                terminated = true;
            }
            out.stmts.push_back(std::move(s));
        }
        cursor += d.bytes_consumed;
        if (terminated) break;
    }

    out.consumed = cursor;
    return out;
}

}  // namespace

Translator::Translator() : pipeline_(passes::default_pipeline()) {}

Translator::~Translator() = default;

void Translator::set_pipeline(passes::PassManager pm) {
    pipeline_ = std::move(pm);
}

TranslateResult Translator::translate(
    std::uint64_t guest_addr,
    std::span<const std::uint8_t> guest_bytes) {

    ++stats_.translations_attempted;

    if (guest_bytes.empty()) {
        return TranslateError::EmptyInput;
    }

    // -- 1. In-process lookup by guest_addr.
    //
    // The in-process Record knows which JitBuffer owns the executable
    // entry point. We ALSO consult / update the persistent
    // TranslationCache in parallel — it is the Pilar 4 seed and future
    // P2P / CDN distribution reads from it directly.
    const std::uint64_t hash_of_input = cache::fnv1a_64(guest_bytes);

    if (auto it = by_addr_.find(guest_addr); it != by_addr_.end()) {
        const Record& rec = it->second;
        if (rec.content_hash == hash_of_input) {
            ++stats_.cache_hits;
            const runtime::JitBuffer* buf = buffers_[rec.buffer_idx].get();
            return TranslatedBlock{
                buf->entry(), rec.code_size, rec.guest_size, /*from_cache=*/true};
        }
        // SMC: the guest bytes at this address changed. Drop the record,
        // drop the persistent cache entry, fall through to retranslate.
        by_addr_.erase(it);
    }

    // -- 2. Cache miss: decode, optimise, lower, JIT.
    // cache_misses is accounted for only on successful translation so
    // that {hits, misses, decode_failures, lower_failures} partitions
    // attempts cleanly.

    auto decoded = decode_until_terminator(guest_bytes);
    if (std::holds_alternative<decoder::DecodeError>(decoded)) {
        ++stats_.decode_failures;
        return TranslateError::DecodeFailed;
    }
    const auto& dec = std::get<Decoded>(decoded);

    // Optimise.
    auto [optimised, _stats] = pipeline_.run(dec.stmts);

    // Strip the trailing Return; the lowerer will add an ARM64 ret
    // for us. This keeps the block-end contract consistent whether or
    // not future passes elide the explicit IR Return.
    std::vector<ir::Stmt> body = std::move(optimised);
    if (!body.empty()
        && std::holds_alternative<ir::Return>(body.back().op)) {
        body.pop_back();
    }

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto lr = lw.lower(body);
    if (!lr.success) {
        ++stats_.lower_failures;
        return TranslateError::LowerFailed;
    }
    // Emit the block-terminating ret explicitly.
    em.ret();
    em.finalize();

    const auto emitted = em.code_bytes();
    if (emitted.empty()) {
        return TranslateError::LowerFailed;
    }

    auto jit = std::make_unique<runtime::JitBuffer>(emitted.size());
    if (!jit->write(emitted)) {
        return TranslateError::JitAllocFailed;
    }
    jit->make_executable();

    const std::uint8_t* entry = jit->entry();
    const std::size_t code_size = emitted.size();
    const std::size_t buffer_idx = buffers_.size();
    buffers_.push_back(std::move(jit));

    // Persistent cache: store bytes for SMC verification + future
    // distribution. The actual executable memory lives in our buffer.
    cache::Entry entry_obj;
    entry_obj.code_bytes.assign(entry, entry + code_size);
    entry_obj.guest_size = dec.consumed;
    entry_obj.guest_content_hash = hash_of_input;
    cache_.upsert(cache::Key{guest_addr, hash_of_input}, std::move(entry_obj));

    // In-process record for the next call.
    by_addr_[guest_addr] = Record{
        buffer_idx, code_size, dec.consumed, hash_of_input};

    ++stats_.cache_misses;  // accounted only on success; see comment above.
    return TranslatedBlock{entry, code_size, dec.consumed, /*from_cache=*/false};
}

}  // namespace prisma::translator
