// core/src/translator/translator.cpp — Translator facade implementation.

#include "prisma/translator.hpp"

#include <cstring>
#include <memory>
#include <span>
#include <type_traits>
#include <utility>
#include <variant>

#include "prisma/abi.hpp"
#include "prisma/cfg.hpp"
#include "prisma/cpu_state.hpp"
#include "prisma/emitter.hpp"
#include "prisma/host_features.hpp"
#include "prisma/ir.hpp"
#include "prisma/jit_buffer_pool.hpp"
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
        || std::holds_alternative<ir::CondJumpRel>(op)
        || std::holds_alternative<ir::CallRel>(op)
        || std::holds_alternative<ir::CallReg>(op)
        || std::holds_alternative<ir::RetAdjusted>(op)
        || std::holds_alternative<ir::RepStos>(op)
        || std::holds_alternative<ir::RepMovs>(op);
}

struct ExitMetadata {
    BlockExitKind kind{BlockExitKind::None};
    std::uint64_t target_guest_pc{0};
    std::uint64_t fallthrough_guest_pc{0};
    std::uint64_t return_guest_pc{0};
};

ExitMetadata exit_metadata(const std::vector<ir::Stmt>& body) noexcept {
    if (body.empty()) return {};
    return std::visit([](const auto& op) -> ExitMetadata {
        using T = std::decay_t<decltype(op)>;
        if constexpr (std::is_same_v<T, ir::Return>) {
            return {BlockExitKind::Return, 0, 0, 0};
        } else if constexpr (std::is_same_v<T, ir::JumpRel>) {
            return {BlockExitKind::JumpRel, op.target_guest_pc, 0, 0};
        } else if constexpr (std::is_same_v<T, ir::JumpReg>) {
            return {BlockExitKind::JumpReg, 0, 0, 0};
        } else if constexpr (std::is_same_v<T, ir::CondJumpRel>) {
            return {BlockExitKind::CondJumpRel,
                    op.target_guest_pc,
                    op.fallthrough_guest_pc,
                    0};
        } else if constexpr (std::is_same_v<T, ir::CallRel>) {
            return {BlockExitKind::CallRel,
                    op.target_guest_pc,
                    0,
                    op.return_guest_pc};
        } else if constexpr (std::is_same_v<T, ir::CallReg>) {
            return {BlockExitKind::CallReg, 0, 0, op.return_guest_pc};
        } else if constexpr (std::is_same_v<T, ir::RetAdjusted>) {
            return {BlockExitKind::RetAdjusted, 0, 0, 0};
        } else if constexpr (std::is_same_v<T, ir::RepStos>) {
            return {BlockExitKind::RepStos, op.pc_of_rep, op.pc_after_rep, 0};
        } else if constexpr (std::is_same_v<T, ir::RepMovs>) {
            return {BlockExitKind::RepMovs, op.pc_of_rep, op.pc_after_rep, 0};
        } else {
            return {};
        }
    }, body.back().op);
}

bool can_use_patchable_tail(BlockExitKind kind) noexcept {
    return kind == BlockExitKind::JumpRel || kind == BlockExitKind::CallRel;
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

// Block prologue / epilogue live in `prisma::backend::abi` (F1-BK-009)
// so future inline guest-CALL sites can reuse the same callee-saved
// discipline. These thin forwarders keep the call sites readable.

inline void emit_prologue(backend::Emitter& em) {
    backend::abi::emit_block_prologue(em);
}

inline void emit_epilogue_and_ret(backend::Emitter& em) {
    backend::abi::emit_block_epilogue_and_ret(em);
}

std::variant<Decoded, decoder::DecodeError>
decode_until_terminator(std::span<const std::uint8_t> bytes,
                        std::uint64_t guest_addr,
                        bool real_call_ret) {
    Decoded out;
    ir::Ref next = 0;
    std::size_t cursor = 0;
    bool terminated = false;

    while (cursor < bytes.size()) {
        const std::uint64_t instr_pc = guest_addr + cursor;
        auto res = decoder::decode_one(bytes.subspan(cursor), next, instr_pc,
                                       real_call_ret);
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

Translator::Translator()
    : pipeline_(passes::default_pipeline()),
      function_pipeline_(passes::default_function_pipeline()),
      pool_(std::make_unique<runtime::JitSlabPool>()) {}

Translator::~Translator() = default;

void Translator::set_pipeline(passes::PassManager pm) {
    pipeline_ = std::move(pm);
}

void Translator::set_function_pipeline(passes::FunctionPassManager pm) {
    function_pipeline_ = std::move(pm);
}

void Translator::set_real_call_ret(bool enabled) noexcept {
    real_call_ret_ = enabled;
}

std::optional<TranslatedBlock> Translator::lookup_cached(
    std::uint64_t guest_addr,
    std::span<const std::uint8_t> guest_bytes) const {
    if (guest_bytes.empty()) return std::nullopt;
    const std::uint64_t hash_of_input = cache::fnv1a_64(guest_bytes);
    auto it = by_addr_.find(guest_addr);
    if (it == by_addr_.end()) return std::nullopt;
    const Record& rec = it->second;
    if (rec.content_hash != hash_of_input) return std::nullopt;
    return TranslatedBlock{
        rec.entry,
        rec.code_size,
        rec.guest_size,
        /*from_cache=*/true,
        rec.exit_kind,
        rec.target_guest_pc,
        rec.fallthrough_guest_pc,
        rec.return_guest_pc,
        rec.direct_patch};
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
            return TranslatedBlock{
                rec.entry,
                rec.code_size,
                rec.guest_size,
                /*from_cache=*/true,
                rec.exit_kind,
                rec.target_guest_pc,
                rec.fallthrough_guest_pc,
                rec.return_guest_pc,
                rec.direct_patch};
        }
        // SMC: the guest bytes at this address changed. Drop the record,
        // drop the persistent cache entry, fall through to retranslate.
        by_addr_.erase(it);
    }

    // -- 2. Cache miss: decode, optimise, lower, JIT.
    // cache_misses is accounted for only on successful translation so
    // that {hits, misses, decode_failures, lower_failures} partitions
    // attempts cleanly.

    auto decoded = decode_until_terminator(guest_bytes, guest_addr,
                                           real_call_ret_);
    if (std::holds_alternative<decoder::DecodeError>(decoded)) {
        ++stats_.decode_failures;
        return TranslateError::DecodeFailed;
    }
    const auto& dec = std::get<Decoded>(decoded);

    // CFG-aware passes (F2-PS-004 + F2-PS-003). The decoder produces a
    // flat stmt list; build_cfg splits at every internal terminator
    // (Trap / Cpuid / Syscall / InlineAsm / RepStos / RepMovs / ...)
    // so multi-x86-instruction regions can naturally yield multi-block
    // ir::Function. For single-block functions (the common case today)
    // we skip the function pipeline — it would be a no-op.
    std::vector<ir::Stmt> pre_stmt_pipeline_input;
    {
        ir::Function fn = ir::build_cfg(dec.stmts);
        if (fn.blocks.size() > 1) {
            auto [opt_fn, _fstats] = function_pipeline_.run(fn);
            pre_stmt_pipeline_input.reserve(dec.stmts.size());
            for (auto& blk : opt_fn.blocks) {
                for (auto& st : blk.stmts) {
                    pre_stmt_pipeline_input.push_back(std::move(st));
                }
            }
        } else {
            pre_stmt_pipeline_input = dec.stmts;
        }
    }

    // Optimise (stmt-level pipeline).
    auto [optimised, _stats] = pipeline_.run(pre_stmt_pipeline_input);

    // Determine the block terminator (if any). The Lowerer emits its
    // own ret for Return / JumpRel / JumpReg / CondJumpRel, so we only
    // need to append a ret ourselves when the block ended without a
    // terminator
    // (ran off the end of guest_bytes — unusual but possible).
    std::vector<ir::Stmt> body = std::move(optimised);
    const ExitMetadata exit = exit_metadata(body);
    const bool body_has_terminator =
        !body.empty() && is_block_terminator(body.back().op);

    backend::Emitter em;

    // Prologue: load guest state from the CpuStateFrame* passed in x0
    // into pinned host regs x10..x25 and save the state ptr in x19.
    emit_prologue(em);

    // Body: lower with `emit_ret_on_terminator = false` so terminators
    // (Return / JumpRel / JumpReg / CondJumpRel) put the next-PC in x0 but
    // ret yet — the epilogue needs to run between the terminator's
    // "set x0" and the final `ret`.
    backend::LowerOptions lopts{/*emit_ret_on_terminator=*/false};
    // Guest CPUID / XGETBV values are baked into the generated code at
    // translation time. Crypto bits are advertised only when the host
    // implements the ARMv8 extensions the lowering relies on, so a
    // guest that honours CPUID never reaches those instructions on a
    // crypto-less core. Everything else advertised below is lowered
    // unconditionally on any ARM64 host.
    // Fase 2.5 note: baked values make cached blocks host-feature-
    // dependent; the P2P trust envelope must carry the feature set
    // alongside the code bytes (RFC 0007 follow-up).
    lopts.cpuid_max_leaf = 7;
    // Vendor "GenuineIntel" (EBX,EDX,ECX order, ASCII little-endian) —
    // the Rosetta 2 precedent: runtime dispatchers take their tuned
    // x86 paths instead of generic fallbacks. Feature BITS, not the
    // vendor/signature, are the compatibility contract.
    lopts.cpuid_vendor_ebx = 0x756E6547u;  // "Genu"
    lopts.cpuid_vendor_edx = 0x49656E69u;  // "ineI"
    lopts.cpuid_vendor_ecx = 0x6C65746Eu;  // "ntel"
    // Family 6 model 42 stepping 7 (Sandy-Bridge-era signature: AVX
    // without AVX2 matches our surface best). EBX: CLFLUSH line size
    // field = 8 chunks (64 bytes) so alignment probes read sane data.
    lopts.cpuid_leaf1_eax = 0x000206A7u;
    lopts.cpuid_leaf1_ebx = 0x00000800u;
    // Leaf 1 EDX: FPU, TSC (RDTSC reads CNTVCT_EL0), CX8 (CMPXCHG8B
    // decoded), CMOV, SSE, SSE2. MMX / FXSR deliberately clear (not
    // decoded).
    lopts.cpuid_leaf1_edx = (1u << 0) | (1u << 4) | (1u << 8) |
                            (1u << 15) | (1u << 25) | (1u << 26);
    // Leaf 1 ECX: SSE3 (ADDSUB/HSUB now decoded), SSSE3, FMA (96/96
    // forms), CMPXCHG16B, SSE4.1, MOVBE, POPCNT (32/64-bit + memory
    // forms + ZF), OSXSAVE, AVX. Deliberately clear:
    //   SSE4.2 — its canonical use-case PCMPISTRI/PCMPESTRI (string
    //            functions) is not decoded; only PCMPGTQ/CRC32 are.
    //   XSAVE  — instructions + CPUID leaf 0xD unmodelled; the
    //            canonical AVX gate (Intel's sequence, MSVC's
    //            __isa_available, glibc) checks OSXSAVE+XGETBV only.
    //   PCLMULQDQ / F16C / RDRAND — not decoded.
    // Known thin spots inside advertised families (loud decode
    // failures by design, queued): SSSE3 PMADDUBSW/PMULHRSW/PSIGN,
    // SSE4.1 INSERTPS/BLENDPS-imm/DPPS/PACKUSDW.
    lopts.cpuid_leaf1_ecx = (1u << 0) | (1u << 9) | (1u << 12) |
                            (1u << 13) | (1u << 19) | (1u << 22) |
                            (1u << 23) | (1u << 27) | (1u << 28);
    // Leaf 7 EBX: BMI2 (bit 8) — SHLX/SARX/SHRX, RORX, MULX, BZHI,
    // PDEP, PEXT are all decoded; that is the complete BMI2 set. BMI1
    // deliberately clear (ANDN/BEXTR/BLSI/BLSMSK/BLSR not decoded);
    // AVX2 deliberately clear (VPBROADCAST*/VINSERTI128/variable
    // shifts missing).
    lopts.cpuid_leaf7_ebx = 1u << 8;
    {
        const auto& hf = runtime::host_features();
        if (hf.feat_sha1 && hf.feat_sha256) {
            lopts.cpuid_leaf7_ebx |= 1u << 29;  // CPUID.7.0:EBX.SHA
        }
        if (hf.feat_aes) {
            lopts.cpuid_leaf1_ecx |= 1u << 25;  // CPUID.1:ECX.AESNI
        }
    }
    // XCR0: x87 + SSE + AVX state enabled — what XGETBV(0) reports,
    // matching the OSXSAVE + AVX bits above.
    lopts.xgetbv_xcr0 = 0x7u;
    backend::Lowerer lw(em, lopts);
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
    // Direct exits with exactly one static successor use a patchable tail
    // slot. It still returns normally until the runtime decides SMC policy
    // allows patching the branch word to a successor entry point.
    TranslatedBlock::DirectPatchSite direct_patch{};
    if (body_has_terminator && can_use_patchable_tail(exit.kind)) {
        const auto patch = backend::abi::emit_block_epilogue_patchable_tail(em);
        direct_patch = TranslatedBlock::DirectPatchSite{
            /*available=*/true,
            patch.branch_offset,
            patch.fallback_offset,
            exit.target_guest_pc};
    } else {
        emit_epilogue_and_ret(em);
    }
    em.finalize();

    const auto emitted = em.code_bytes();
    if (emitted.empty()) {
        return TranslateError::LowerFailed;
    }

    runtime::JitBlock blk;
    try {
        blk = pool_->acquire(emitted);
    } catch (const std::bad_alloc&) {
        return TranslateError::JitAllocFailed;
    }

    const std::uint8_t* entry = blk.entry;
    const std::size_t code_size = emitted.size();

    // Persistent cache: store bytes for SMC verification + future
    // distribution. The actual executable memory lives in our buffer.
    cache::Entry entry_obj;
    entry_obj.code_bytes.assign(entry, entry + code_size);
    entry_obj.guest_size = dec.consumed;
    entry_obj.guest_content_hash = hash_of_input;
    cache_.upsert(cache::Key{guest_addr, hash_of_input}, std::move(entry_obj));

    // In-process record for the next call.
    by_addr_[guest_addr] = Record{
        entry,
        code_size,
        dec.consumed,
        hash_of_input,
        exit.kind,
        exit.target_guest_pc,
        exit.fallthrough_guest_pc,
        exit.return_guest_pc,
        direct_patch};

    ++stats_.cache_misses;  // accounted only on success; see comment above.
    return TranslatedBlock{
        entry,
        code_size,
        dec.consumed,
        /*from_cache=*/false,
        exit.kind,
        exit.target_guest_pc,
        exit.fallthrough_guest_pc,
        exit.return_guest_pc,
        direct_patch};
}

}  // namespace prisma::translator
