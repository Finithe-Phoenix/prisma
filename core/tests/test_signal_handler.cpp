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
#include <cstddef>
#include <cstdint>
#include <cstring>

#include "prisma/emitter.hpp"
#include "prisma/jit_memory.hpp"
#include "prisma/signal_handler.hpp"
#include "prisma/smc_guard.hpp"

#include <sys/mman.h>
#include <unistd.h>

using namespace prisma;

namespace {
constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif

std::size_t host_page() {
    long v = ::sysconf(_SC_PAGESIZE);
    return v > 0 ? static_cast<std::size_t>(v) : 4096;
}

class MappedPage {
public:
    MappedPage() {
        bytes_ = host_page();
        void* p = ::mmap(nullptr, bytes_,
                         PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS,
                         -1, 0);
        REQUIRE(p != MAP_FAILED);
        ptr_ = static_cast<std::uint8_t*>(p);
    }
    ~MappedPage() {
        if (ptr_ != nullptr) {
            ::munmap(ptr_, bytes_);
        }
    }

    [[nodiscard]] std::uint8_t* data() const noexcept { return ptr_; }
    [[nodiscard]] std::uint64_t addr() const noexcept {
        return reinterpret_cast<std::uint64_t>(ptr_);
    }

private:
    std::uint8_t* ptr_{nullptr};
    std::size_t bytes_{0};
};

struct SmcCallbackState {
    volatile std::sig_atomic_t calls{0};
    volatile std::sig_atomic_t key{0};
};

void record_smc_invalidation(std::uint64_t cache_key, void* ctx) noexcept {
    auto* state = static_cast<SmcCallbackState*>(ctx);
    state->key = static_cast<std::sig_atomic_t>(cache_key);
    state->calls = static_cast<std::sig_atomic_t>(state->calls + 1);
}

class SmcSignalScope {
public:
    explicit SmcSignalScope(runtime::SmcGuard& guard) {
        runtime::set_global_smc_guard(&guard);
    }
    ~SmcSignalScope() {
        runtime::clear_smc_invalidate_callback();
        runtime::set_global_smc_guard(nullptr);
    }
    SmcSignalScope(const SmcSignalScope&) = delete;
    SmcSignalScope& operator=(const SmcSignalScope&) = delete;
};
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

TEST_CASE("signal_handler: SmcGuard fault invokes the invalidate callback") {
    runtime::install_handlers();

    MappedPage page;
    runtime::SmcGuard guard;
    SmcSignalScope scope(guard);

    SmcCallbackState state{};
    runtime::set_smc_invalidate_callback(&record_smc_invalidation, &state);

    constexpr std::uint64_t kCacheKey = 77;
    guard.on_translate(page.addr(), /*guest_byte_len=*/16, kCacheKey);
    REQUIRE(guard.is_tracked(page.addr()));

    page.data()[0] = 0x5A;

    REQUIRE(page.data()[0] == 0x5A);
    // The handler only tombstones + queues (a signal handler must not
    // run cache code); the callback fires on the normal-context drain
    // the dispatcher performs between blocks.
    REQUIRE(state.calls == 0);
    REQUIRE_FALSE(guard.is_tracked(page.addr()));
    REQUIRE(runtime::drain_smc_invalidations() == 1);
    REQUIRE(state.calls == 1);
    REQUIRE(state.key == static_cast<std::sig_atomic_t>(kCacheKey));
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
