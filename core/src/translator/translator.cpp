// core/src/translator/translator.cpp — Translator facade implementation.

#include "prisma/translator.hpp"

#include <cstring>
#include <span>
#include <utility>
#include <variant>

#include "prisma/cpu_state.hpp"
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
        || std::holds_alternative<ir::JumpReg>(op)
        || std::holds_alternative<ir::CondJumpRel>(op);
}

// ---------------------------------------------------------------------
// Block prologue / epilogue (F1-RT-004/005/006 groundwork).
//
// Calling convention for every Translator-produced block:
//
//   uint64_t block(CpuStateFrame* state);
//      entry: x0 = state pointer
//      exit:  x0 = next guest PC
//
// Register allocation within a block:
//
//   x0..x9    scratch pool (caller-saved in AAPCS64; free to clobber).
//   x10..x17  pinned guest GPRs rax..rdi (caller-saved; free to clobber).
//   x18       NEVER touched — platform-reserved on Apple silicon.
//   x19..x26  pinned guest GPRs r8..r15 (AAPCS64 callee-saved → must
//             save/restore).
//   x27       state pointer (callee-saved → save/restore).
//   x28       unused but also callee-saved; spared.
//   x29       frame pointer (callee-saved) — preserved.
//   x30       link register — block `ret` uses it; not otherwise touched.
//
// Prologue / epilogue save the 10 callee-saved regs we either write to
// (x19..x26, x27) or want to keep consistent (x29, x30 for safety) via
// 5 stp pairs (80 bytes of stack frame; 16-byte aligned as required).
//
// After save, the state pointer is stashed in x27 and guest GPRs
// loaded from the frame into their pinned hosts. On exit the epilogue
// stores the pinned hosts back and restores the callee-saved regs from
// the stack before `ret`.
// ---------------------------------------------------------------------

constexpr arm64::Reg kStatePtrReg = arm64::Reg::X27;

void emit_prologue(backend::Emitter& em) {
    // Save callee-saved regs we will clobber (pushed as pairs so SP
    // stays 16-byte aligned). The `push_pair` helper is stp+pre-index.
    em.push_pair(arm64::Reg::X29, arm64::Reg::X30);  // FP + LR
    em.push_pair(arm64::Reg::X27, arm64::Reg::X28);  // state ptr holder + spare
    em.push_pair(arm64::Reg::X25, arm64::Reg::X26);  // guest r14, r15
    em.push_pair(arm64::Reg::X23, arm64::Reg::X24);  // guest r12, r13
    em.push_pair(arm64::Reg::X21, arm64::Reg::X22);  // guest r10, r11
    em.push_pair(arm64::Reg::X19, arm64::Reg::X20);  // guest r8,  r9

    // x27 = x0  (preserve the state pointer across the body)
    em.mov_reg_reg(kStatePtrReg, arm64::Reg::X0);

    // Load 16 guest GPRs from state->gpr[] into their pinned host regs.
    // (No host reg overlaps with x27 here because host_reg_for skips x18
    // and lands in x10..x17, x19..x26.)
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        const ir::Gpr g = static_cast<ir::Gpr>(i);
        const arm64::Reg host = arm64::host_reg_for(g);
        const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
        em.load_offset(host, kStatePtrReg, off);
    }
}

void emit_epilogue_and_ret(backend::Emitter& em) {
    // Store pinned host regs back to the state frame. None of these
    // writes touch x0 (our next-PC return value) or x27 (state ptr,
    // still valid until we finish using it).
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        const ir::Gpr g = static_cast<ir::Gpr>(i);
        const arm64::Reg host = arm64::host_reg_for(g);
        const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
        em.store_offset(host, kStatePtrReg, off);
    }

    // Restore callee-saved regs in reverse push order, then ret.
    em.pop_pair(arm64::Reg::X19, arm64::Reg::X20);
    em.pop_pair(arm64::Reg::X21, arm64::Reg::X22);
    em.pop_pair(arm64::Reg::X23, arm64::Reg::X24);
    em.pop_pair(arm64::Reg::X25, arm64::Reg::X26);
    em.pop_pair(arm64::Reg::X27, arm64::Reg::X28);
    em.pop_pair(arm64::Reg::X29, arm64::Reg::X30);

    em.ret();
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
            const runtime::JitBuffer* buf = buffers_.get(rec.buffer_idx);
            if (buf != nullptr) {
                ++stats_.cache_hits;
                return TranslatedBlock{
                    buf->entry(), rec.code_size, rec.guest_size, /*from_cache=*/true};
            }
        }
        // SMC (or a stale internal record): the guest bytes at this address
        // changed. Drop the in-process record and fall through to retranslate.
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
    // own ret for Return / JumpRel / JumpReg / CondJumpRel, so we only
    // need to append a ret ourselves when the block ended without a
    // terminator
    // (ran off the end of guest_bytes — unusual but possible).
    std::vector<ir::Stmt> body = std::move(optimised);
    bool body_has_terminator = false;
    if (!body.empty()) {
        if (std::holds_alternative<ir::Return>(body.back().op)) {
            // Pure Return: let Lowerer handle it; it already emits ret.
            body_has_terminator = true;
        } else if (std::holds_alternative<ir::JumpRel>(body.back().op)
                || std::holds_alternative<ir::JumpReg>(body.back().op)
                || std::holds_alternative<ir::CondJumpRel>(body.back().op)) {
            body_has_terminator = true;
        }
    }

    backend::Emitter em;

    // Prologue: load guest state from the CpuStateFrame* passed in x0
    // into pinned host regs x10..x25 and save the state ptr in x27.
    emit_prologue(em);

    // Body: lower with `emit_ret_on_terminator = false` so terminators
    // (Return / JumpRel / JumpReg / CondJumpRel) put the next-PC in x0 but
    // ret yet — the epilogue needs to run between the terminator's
    // "set x0" and the final `ret`.
    backend::Lowerer lw(em, backend::LowerOptions{/*emit_ret_on_terminator=*/false});
    auto lr = lw.lower(body);
    if (!lr.success) {
        ++stats_.lower_failures;
        return TranslateError::LowerFailed;
    }
    // If the body didn't end in a terminator (ran off the guest region),
    // default x0 to the halt sentinel so the dispatcher stops cleanly.
    if (!body_has_terminator) {
        em.mov_imm64(arm64::Reg::X0, runtime::CpuStateFrame::kHaltSentinel);
    }

    // Epilogue: store pinned host regs back to state[], then ret.
    emit_epilogue_and_ret(em);
    em.finalize();

    const auto emitted = em.code_bytes();
    if (emitted.empty()) {
        return TranslateError::LowerFailed;
    }

    const auto allocation = buffers_.add(emitted);
    if (!allocation) {
        return TranslateError::JitAllocFailed;
    }

    const std::uint8_t* entry = allocation->entry;
    const std::size_t code_size = allocation->code_size;
    const std::size_t buffer_idx = allocation->index;

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
