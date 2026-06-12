// core/tests/test_dispatcher.cpp — run-loop tests.
//
// The dispatcher alternates translate + execute until a halt condition.
// These tests exercise the loop on small hand-crafted guest programs.
// They run on ARM64 hosts only because the executed code is ARM64.

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <span>
#include <unordered_map>
#include <vector>

#include "prisma/dispatcher.hpp"
#include "prisma/translator.hpp"

using namespace prisma;

namespace {

constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif

// A pageable memory map: PC → bytes. Lookup returns the span of bytes
// starting at the PC the guest is trying to fetch from. If no segment
// contains that PC the returned span is empty (dispatcher treats as
// fetch failure).
struct GuestMemory {
    std::unordered_map<std::uint64_t, std::vector<std::uint8_t>> segments;

    std::span<const std::uint8_t> read(std::uint64_t pc) const {
        // Find the segment whose base ≤ pc and covers the PC.
        for (const auto& [base, bytes] : segments) {
            if (pc >= base && pc < base + bytes.size()) {
                const std::size_t off = static_cast<std::size_t>(pc - base);
                return std::span<const std::uint8_t>{
                    bytes.data() + off, bytes.size() - off};
            }
        }
        return {};
    }
};

}  // namespace

TEST_CASE("Dispatcher: halts cleanly on RET (x0 = 0 sentinel)",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // Guest at 0x1000:  RET (0xC3).
    GuestMemory mem;
    mem.segments[0x1000] = {0xC3};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    // With real CALL/RET semantics on by default (F2-IR-054), the RET
    // pops the halt sentinel off the guest stack — which must exist.
    // Latent SEGFAULT before core-build-arm64 ran this in CI.
    d.install_halt_return_stack();

    auto r = d.run(0x1000, /*max_steps=*/10);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.final_pc == runtime::CpuStateFrame::kHaltSentinel);
    REQUIRE(r.stats.blocks_executed == 1);
}

TEST_CASE("Dispatcher: JMP chain reaches a RET and halts",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x1000:  EB 0E             jmp +14 -> 0x1010
    // 0x1002..0x100F: padding bytes that would trap if fetched
    // 0x1010:  EB 0E             jmp +14 -> 0x1020
    // 0x1012..0x101F: padding
    // 0x1020:  C3                ret
    GuestMemory mem;
    mem.segments[0x1000] = {0xEB, 0x0E};
    mem.segments[0x1010] = {0xEB, 0x0E};
    mem.segments[0x1020] = {0xC3};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.install_halt_return_stack();  // the final RET pops the sentinel

    auto r = d.run(0x1000, /*max_steps=*/10);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.final_pc == 0u);  // kHaltSentinel
    REQUIRE(r.stats.blocks_executed == 3);  // JMP, JMP, RET
    REQUIRE(r.stats.direct_thread_misses == 2);
    REQUIRE(r.stats.direct_thread_installs == 2);
}

TEST_CASE("Dispatcher: return-stack predictor hits on direct CALL/RET",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x4100: E8 01 00 00 00  call 0x4106
    // 0x4105: C3              outer RET pops the halt sentinel
    // 0x4106: C3              callee RET returns to 0x4105
    GuestMemory mem;
    mem.segments[0x4100] = {
        0xE8, 0x01, 0x00, 0x00, 0x00,
        0xC3,
        0xC3,
    };

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.install_halt_return_stack();

    auto r = d.run(0x4100, /*max_steps=*/10);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.stats.blocks_executed == 3);
    REQUIRE(r.stats.ras_pushes == 1);
    REQUIRE(r.stats.ras_pops == 2);
    REQUIRE(r.stats.ras_hits == 1);
    REQUIRE(r.stats.ras_misses == 0);
    REQUIRE(r.stats.ras_underflows == 1);
    REQUIRE(r.stats.direct_thread_misses == 1);
    REQUIRE(r.stats.direct_thread_installs == 1);
}

TEST_CASE("Dispatcher: CMP + JE branches to the taken leg on equal operands",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x2000:  48 B8 2A 00 00 00 00 00 00 00   mov rax, 42
    // 0x200A:  48 BB 2A 00 00 00 00 00 00 00   mov rbx, 42
    // 0x2014:  48 39 D8                        cmp rax, rbx
    // 0x2017:  74 02                           je +2  -> 0x201B
    // 0x2019:  EB 00                           jmp +0 -> 0x201B (fallthrough
    //                                                  path collapses to same target)
    // 0x201B:  C3                              ret
    //
    // rax == rbx, so the JE is taken; the block at 0x2000 ends at the JE
    // with next_pc=0x201B. The dispatcher then fetches 0x201B and runs
    // RET, which halts.
    GuestMemory mem;
    mem.segments[0x2000] = {
        0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0xBB, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0x39, 0xD8,
        0x74, 0x02,
        0xEB, 0x00,
        0xC3,
    };

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.install_halt_return_stack();  // the final RET pops the sentinel

    auto r = d.run(0x2000, /*max_steps=*/20);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.stats.blocks_executed >= 2);  // at minimum the CMP+JE block and the RET.
    REQUIRE(r.stats.direct_thread_installs >= 1);
}

TEST_CASE("Dispatcher: step limit trips when the program loops forever",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x3000:  EB FE    jmp -2  (loops back to itself)
    GuestMemory mem;
    mem.segments[0x3000] = {0xEB, 0xFE};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });

    auto r = d.run(0x3000, /*max_steps=*/5);
    REQUIRE(r.exit == runtime::DispatchExit::StepLimit);
    REQUIRE(r.stats.blocks_executed == 5);
    REQUIRE(r.final_pc == 0x3000u);

    // The direct-thread cache keeps hot direct loops inside the dispatcher
    // once the first translation is installed.
    REQUIRE(r.stats.direct_thread_hits >= 4);  // initial translate, then 4 cache hits.
    REQUIRE(r.stats.direct_thread_misses == 0);
    REQUIRE(r.stats.direct_thread_installs == 0);
}

TEST_CASE("Dispatcher: one-hop JIT patches preserve the block step budget",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x3100: EB 0E  jmp +14 -> 0x3110
    // 0x3110: EB EE  jmp -18 -> 0x3100
    //
    // The dispatcher may patch 0x3100 -> 0x3110, but it must still
    // account both blocks and stop exactly at max_steps.
    GuestMemory mem;
    mem.segments[0x3100] = {0xEB, 0x0E};
    mem.segments[0x3110] = {0xEB, 0xEE};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });

    auto r = d.run(0x3100, /*max_steps=*/6);
    REQUIRE(r.exit == runtime::DispatchExit::StepLimit);
    REQUIRE(r.stats.blocks_executed == 6);
    REQUIRE(r.stats.steps_taken == 6);
    REQUIRE(r.final_pc == 0x3100u);
    REQUIRE(r.stats.direct_jit_patch_attempts == 3);
    REQUIRE(r.stats.direct_jit_patch_applied == 1);
    REQUIRE(r.stats.direct_jit_patch_rejected == 2);
    REQUIRE(r.stats.direct_jit_patch_unpatches == 0);
    REQUIRE(r.stats.direct_jit_patch_executes == 2);
    REQUIRE(t.direct_exit_is_patched(0x3100));
    REQUIRE_FALSE(t.direct_exit_is_patched(0x3110));
}

TEST_CASE("Dispatcher: one-hop JIT patches unpatch stale targets",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // First run installs 0x3200 -> 0x3210. Then the target bytes change
    // to RET; the next run must reject the stale cached target before
    // entering the patched source.
    GuestMemory mem;
    mem.segments[0x3200] = {0xEB, 0x0E};
    mem.segments[0x3210] = {0xEB, 0xEE};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.install_halt_return_stack();

    auto warm = d.run(0x3200, /*max_steps=*/2);
    REQUIRE(warm.exit == runtime::DispatchExit::StepLimit);
    REQUIRE(warm.stats.direct_jit_patch_attempts == 1);
    REQUIRE(warm.stats.direct_jit_patch_applied == 1);
    REQUIRE(t.direct_exit_is_patched(0x3200));

    mem.segments[0x3210] = {0xC3};
    auto stale = d.run(0x3200, /*max_steps=*/4);
    REQUIRE(stale.exit == runtime::DispatchExit::Halted);
    REQUIRE(stale.final_pc == runtime::CpuStateFrame::kHaltSentinel);
    REQUIRE(stale.stats.direct_jit_patch_unpatches == 1);
    REQUIRE(stale.stats.direct_jit_patch_executes == 0);
}

TEST_CASE("Dispatcher: custom halt PC stops even without a guest RET",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    //  0x4000: EB 0E  jmp +14 -> 0x4010
    //  0x4010: EB 0E  jmp +14 -> 0x4020
    //  0x4020: EB 0E  jmp +14 -> 0x4030 (never reached — we halt at 0x4020)
    GuestMemory mem;
    mem.segments[0x4000] = {0xEB, 0x0E};
    mem.segments[0x4010] = {0xEB, 0x0E};
    mem.segments[0x4020] = {0xEB, 0x0E};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.add_halt_pc(0x4020);

    auto r = d.run(0x4000, 10);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.final_pc == 0x4020u);
    REQUIRE(r.stats.blocks_executed == 2);  // 0x4000 and 0x4010; halt before 0x4020 runs.
    REQUIRE(r.stats.direct_thread_installs == 1);
}

TEST_CASE("Dispatcher: halt PC wins over exact step-limit exhaustion",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x4050: EB 0E  jmp +14 -> 0x4060
    GuestMemory mem;
    mem.segments[0x4050] = {0xEB, 0x0E};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });
    d.add_halt_pc(0x4060);

    auto r = d.run(0x4050, 1);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.final_pc == 0x4060u);
    REQUIRE(r.stats.blocks_executed == 1);
}

TEST_CASE("Dispatcher: fetch failure when PC leaves known memory",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x5000: EB 0E  jmp +14 -> 0x5010 (no segment at 0x5010).
    GuestMemory mem;
    mem.segments[0x5000] = {0xEB, 0x0E};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });

    auto r = d.run(0x5000, 10);
    REQUIRE(r.exit == runtime::DispatchExit::FetchFailed);
    REQUIRE(r.final_pc == 0x5010u);
    REQUIRE(r.stats.blocks_executed == 1);
    REQUIRE_FALSE(r.message.empty());
}

TEST_CASE("Dispatcher: translation failure on undecodable bytes",
          "[arm64-only]") {
    if constexpr (!is_arm64) { SUCCEED("skipped"); return; }

    // 0x6000: FF (no ModR/M after; decoder rejects).
    GuestMemory mem;
    mem.segments[0x6000] = {0xFF};

    translator::Translator t;
    runtime::Dispatcher d(t, [&](std::uint64_t pc) { return mem.read(pc); });

    auto r = d.run(0x6000, 10);
    REQUIRE(r.exit == runtime::DispatchExit::TranslationFailed);
    REQUIRE(r.final_pc == 0x6000u);
}

TEST_CASE("Dispatcher: CpuStateFrame.guest_pc tracks the final PC") {
    // Non-ARM-specific — we verify state bookkeeping even without
    // actually executing, by hitting the halt sentinel immediately.
    translator::Translator t;
    runtime::Dispatcher d(t, [](std::uint64_t) {
        return std::span<const std::uint8_t>{};
    });

    auto r = d.run(runtime::CpuStateFrame::kHaltSentinel, 10);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(d.state().guest_pc == runtime::CpuStateFrame::kHaltSentinel);
}
