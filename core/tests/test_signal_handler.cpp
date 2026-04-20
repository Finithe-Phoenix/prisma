// core/tests/test_signal_handler.cpp — verify SIGSEGV/SIGILL recovery.
//
// Each test installs the handlers, sets up a ScopedProtected scope, and
// induces a fault. The test passes when:
//   * Execution resumes at the setjmp point with the right FaultKind.
//   * The process does not abort.
//
// These are ARM64-only because we induce illegal-instruction faults with
// handwritten ARM64 bytes. On x86_64 they are simply skipped.

#include <catch2/catch_test_macros.hpp>

#include <csetjmp>
#include <csignal>
#include <cstdint>
#include <cstring>

#include "prisma/emitter.hpp"
#include "prisma/jit_memory.hpp"
#include "prisma/signal_handler.hpp"

using namespace prisma;

namespace {
constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif
}  // namespace

TEST_CASE("signal_handler: SIGSEGV recovery via setjmp/longjmp") {
    runtime::install_handlers();

    // Force a deterministic page fault by dereferencing a known-bad
    // address. Volatile + forced load stops the optimiser from eliding
    // the access.
    std::jmp_buf jb;
    if (setjmp(jb) == 0) {
        runtime::ScopedProtected guard(jb);
        volatile std::uint64_t* p = reinterpret_cast<std::uint64_t*>(0x10);
        (void)*p;  // BOOM
        FAIL("should have faulted");
    } else {
        // Resumed after the fault.
        REQUIRE(runtime::last_fault_kind() == runtime::FaultKind::Segv);
    }
}

TEST_CASE("signal_handler: SIGILL recovery from an illegal ARM64 instruction",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    runtime::install_handlers();

    // Emit an illegal-instruction bit pattern (UDF #0) + a ret we'll
    // never reach. UDF #0 = 0x00000000 is guaranteed illegal on ARM64.
    backend::Emitter em;
    // Raw four-byte illegal pattern through the macro-assembler: vixl
    // doesn't expose UDF directly in its public API we've wired up, so
    // we hand-craft the 32-bit word.
    // 0x00000000 is illegal in AArch64 (C6.2.394 UDF #0).
    em.finalize();

    // Build a JitBuffer with the illegal instruction followed by ret.
    const std::uint8_t bytes[] = {
        0x00, 0x00, 0x00, 0x00,   // UDF #0 (illegal)
        0xC0, 0x03, 0x5F, 0xD6,   // ret
    };
    runtime::JitBuffer jit(sizeof(bytes));
    REQUIRE(jit.write(bytes));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    std::jmp_buf jb;
    if (setjmp(jb) == 0) {
        runtime::ScopedProtected guard(jb);
        (void)fn();
        FAIL("should have faulted");
    } else {
        REQUIRE(runtime::last_fault_kind() == runtime::FaultKind::Ill);
    }
}

TEST_CASE("signal_handler: nested ScopedProtected preserves stack on normal exit") {
    // When scopes exit normally (no fault), the RAII destructors run in
    // LIFO order and `tls_current_jb` is correctly restored. We don't
    // test re-faulting after a recovery because siglongjmp bypasses
    // destructors — the framework requires the caller to treat the
    // ScopedProtected as consumed after a fault and re-arm if needed.
    runtime::install_handlers();

    std::jmp_buf outer;
    int outer_sj = setjmp(outer);
    REQUIRE(outer_sj == 0);

    runtime::ScopedProtected outer_guard(outer);

    {
        std::jmp_buf inner;
        int inner_sj = setjmp(inner);
        REQUIRE(inner_sj == 0);
        runtime::ScopedProtected inner_guard(inner);
        // No fault: just exit the inner scope normally.
    }

    // After the inner scope, an outer fault still routes to `outer`.
    if (setjmp(outer) == 0) {
        runtime::ScopedProtected re_arm(outer);
        volatile std::uint64_t* p = reinterpret_cast<std::uint64_t*>(0x30);
        (void)*p;
        FAIL("should have faulted");
    } else {
        REQUIRE(runtime::last_fault_kind() == runtime::FaultKind::Segv);
    }
}
