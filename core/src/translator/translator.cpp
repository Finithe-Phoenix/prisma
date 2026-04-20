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

// Terminator check: an IR Stmt that ends the block.
bool is_block_terminator(const ir::Op& op) noexcept {
    return std::holds_alternative<ir::Return>(op)
        || std::holds_alternative<ir::JumpRel>(op)
        || std::holds_alternative<ir::CondJumpRel>(op);
}

std::variant<Decoded, decoder::DecodeError>
decode_until_terminator(std::span<const std::uint8_t> bytes,
                        std::uint64_t guest_addr) {
    Decoded out;
    ir::Ref next = 0;
    std::size_t cursor = 0;
    bool terminated = false;

    while (cursor < bytes.size()) {
        const std::uint64_t instr_pc = guest_addr + cursor;
        auto res = decoder::decode_one(bytes.subspan(cursor), next, instr_pc);
        if (std::holds_alternative<decoder::DecodeError>(res)) {
            return std::get<decoder::DecodeError>(res);
        }
        auto& d = std::get<decoder::Decoded>(res);
        for (auto& s : d.stmts) {
            if (is_block_terminator(s.op)) {
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

    auto decoded = decode_until_terminator(guest_bytes, guest_addr);
    if (std::holds_alternative<decoder::DecodeError>(decoded)) {
        ++stats_.decode_failures;
        return TranslateError::DecodeFailed;
    }
    const auto& dec = std::get<Decoded>(decoded);

    // Optimise.
    auto [optimised, _stats] = pipeline_.run(dec.stmts);

    // Determine the block terminator (if any). The Lowerer emits its
    // own ret for Return / JumpRel / CondJumpRel, so we only need to
    // append a ret ourselves when the block ended without a terminator
    // (ran off the end of guest_bytes — unusual but possible).
    std::vector<ir::Stmt> body = std::move(optimised);
    bool body_has_terminator = false;
    if (!body.empty()) {
        if (std::holds_alternative<ir::Return>(body.back().op)) {
            // Pure Return: let Lowerer handle it; it already emits ret.
            body_has_terminator = true;
        } else if (std::holds_alternative<ir::JumpRel>(body.back().op)
                || std::holds_alternative<ir::CondJumpRel>(body.back().op)) {
            body_has_terminator = true;
        }
    }

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto lr = lw.lower(body);
    if (!lr.success) {
        ++stats_.lower_failures;
        return TranslateError::LowerFailed;
    }
    if (!body_has_terminator) {
        em.ret();
    }
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
