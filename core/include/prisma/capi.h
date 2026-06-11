/* prisma/capi.h — C ABI for the Prisma DBT core (RFC 0014).
 *
 * This is the only header the Rust shell (and any other non-C++
 * consumer) sees. Rules of the boundary live in RFC 0014; the short
 * version:
 *
 *   - Opaque handles, _create/_destroy pairs, destroy(NULL) is a no-op.
 *   - Fallible calls return prisma_status; out-params are written only
 *     on PRISMA_OK.
 *   - No exception or panic crosses this boundary in either direction.
 *   - Result structs are fixed-layout integers + fixed char arrays and
 *     grow only by appending alongside a PRISMA_CAPI_VERSION bump.
 *   - Handles are not thread-safe.
 */

#ifndef PRISMA_CAPI_H
#define PRISMA_CAPI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Bump on any ABI-visible change (new function, struct growth, enum
 * value). Consumers compare against prisma_capi_version() at startup. */
#define PRISMA_CAPI_VERSION 2u

uint32_t prisma_capi_version(void);

typedef enum prisma_status {
    PRISMA_OK = 0,
    PRISMA_STATUS_INVALID_ARGUMENT = 1,
    /* Mirrors translator::TranslateError. */
    PRISMA_STATUS_DECODE_FAILED = 2,
    PRISMA_STATUS_LOWER_FAILED = 3,
    PRISMA_STATUS_EMPTY_INPUT = 4,
    PRISMA_STATUS_JIT_ALLOC_FAILED = 5,
    /* A C++ exception was stopped at the boundary. */
    PRISMA_STATUS_INTERNAL = 6
} prisma_status;

/* Mirrors translator::BlockExitKind. */
typedef enum prisma_block_exit_kind {
    PRISMA_BLOCK_EXIT_NONE = 0,
    PRISMA_BLOCK_EXIT_RETURN = 1,
    PRISMA_BLOCK_EXIT_JUMP_REL = 2,
    PRISMA_BLOCK_EXIT_JUMP_REG = 3,
    PRISMA_BLOCK_EXIT_COND_JUMP_REL = 4,
    PRISMA_BLOCK_EXIT_CALL_REL = 5,
    PRISMA_BLOCK_EXIT_CALL_REG = 6,
    PRISMA_BLOCK_EXIT_RET_ADJUSTED = 7,
    PRISMA_BLOCK_EXIT_REP_STOS = 8,
    PRISMA_BLOCK_EXIT_REP_MOVS = 9
} prisma_block_exit_kind;

/* Mirrors runtime::DispatchExit. */
typedef enum prisma_dispatch_exit {
    PRISMA_DISPATCH_HALTED = 0,
    PRISMA_DISPATCH_STEP_LIMIT = 1,
    PRISMA_DISPATCH_FETCH_FAILED = 2,
    PRISMA_DISPATCH_TRANSLATION_FAILED = 3
} prisma_dispatch_exit;

/* x86 register-encoding order, matching ir::Gpr. */
typedef enum prisma_gpr {
    PRISMA_GPR_RAX = 0,
    PRISMA_GPR_RCX = 1,
    PRISMA_GPR_RDX = 2,
    PRISMA_GPR_RBX = 3,
    PRISMA_GPR_RSP = 4,
    PRISMA_GPR_RBP = 5,
    PRISMA_GPR_RSI = 6,
    PRISMA_GPR_RDI = 7,
    PRISMA_GPR_R8 = 8,
    PRISMA_GPR_R9 = 9,
    PRISMA_GPR_R10 = 10,
    PRISMA_GPR_R11 = 11,
    PRISMA_GPR_R12 = 12,
    PRISMA_GPR_R13 = 13,
    PRISMA_GPR_R14 = 14,
    PRISMA_GPR_R15 = 15
} prisma_gpr;

#define PRISMA_GPR_COUNT 16u

typedef struct prisma_translator prisma_translator;
typedef struct prisma_dispatcher prisma_dispatcher;

/* ------------------------------------------------------------------ */
/* Translator                                                          */
/* ------------------------------------------------------------------ */

typedef struct prisma_block_info {
    uint64_t code_size;
    uint64_t guest_size;
    uint64_t target_guest_pc;
    uint64_t fallthrough_guest_pc;
    uint64_t return_guest_pc;
    int32_t exit_kind; /* prisma_block_exit_kind */
    uint8_t from_cache;
    uint8_t reserved[3];
} prisma_block_info;

typedef struct prisma_translator_stats {
    uint64_t translations_attempted;
    uint64_t cache_hits;
    uint64_t cache_misses;
    uint64_t decode_failures;
    uint64_t lower_failures;
} prisma_translator_stats;

prisma_status prisma_translator_create(prisma_translator** out);
void prisma_translator_destroy(prisma_translator* t);

/* Translate `len` guest bytes at `guest_addr`. `out_info` may be NULL
 * when the caller only wants the side effect of populating the cache. */
prisma_status prisma_translator_translate(prisma_translator* t,
                                          uint64_t guest_addr,
                                          const uint8_t* bytes,
                                          size_t len,
                                          prisma_block_info* out_info);

/* Real CALL/RET semantics toggle; on by default (F2-IR-054). */
prisma_status prisma_translator_set_real_call_ret(prisma_translator* t,
                                                  int enabled);

prisma_status prisma_translator_get_stats(const prisma_translator* t,
                                          prisma_translator_stats* out);

/* ------------------------------------------------------------------ */
/* Dispatcher                                                          */
/* ------------------------------------------------------------------ */

/* Guest memory reader. Write the base of the readable region holding
 * `pc` into *out_bytes and return how many bytes are readable from it.
 * Return 0 (out_bytes ignored) for "no memory here" — the dispatcher
 * reports a fetch fault. The returned region must stay valid until the
 * enclosing prisma_dispatcher_run call returns. Implementations MUST
 * NOT unwind (Rust trampolines catch panics and return 0). */
typedef size_t (*prisma_mem_reader)(void* ctx,
                                    uint64_t pc,
                                    const uint8_t** out_bytes);

typedef struct prisma_dispatch_stats {
    uint64_t blocks_executed;
    uint64_t steps_taken;
    uint64_t unique_pcs_seen;
    uint64_t ras_pushes;
    uint64_t ras_pops;
    uint64_t ras_hits;
    uint64_t ras_misses;
    uint64_t ras_overflows;
    uint64_t ras_underflows;
    uint64_t direct_thread_hits;
    uint64_t direct_thread_misses;
    uint64_t direct_thread_installs;
    uint64_t direct_jit_patch_attempts;
    uint64_t direct_jit_patch_applied;
    uint64_t direct_jit_patch_rejected;
    uint64_t direct_jit_patch_unpatches;
    uint64_t direct_jit_patch_executes;
} prisma_dispatch_stats;

typedef struct prisma_run_result {
    int32_t exit; /* prisma_dispatch_exit */
    uint8_t reserved[4];
    uint64_t final_pc;
    prisma_dispatch_stats stats;
    /* Truncated, always NUL-terminated human-readable context on
     * errors. Empty string on clean exits. */
    char message[128];
} prisma_run_result;

/* The translator must outlive the dispatcher. `reader` + `ctx` must
 * stay valid for the dispatcher's lifetime. */
prisma_status prisma_dispatcher_create(prisma_translator* t,
                                       prisma_mem_reader reader,
                                       void* ctx,
                                       prisma_dispatcher** out);
void prisma_dispatcher_destroy(prisma_dispatcher* d);

prisma_status prisma_dispatcher_add_halt_pc(prisma_dispatcher* d,
                                            uint64_t pc);

/* See runtime::Dispatcher::install_halt_return_stack(). */
prisma_status prisma_dispatcher_install_halt_return_stack(
    prisma_dispatcher* d);

prisma_status prisma_dispatcher_run(prisma_dispatcher* d,
                                    uint64_t entry_pc,
                                    size_t max_steps,
                                    prisma_run_result* out);

/* Guest CPU state access (between runs). */
prisma_status prisma_dispatcher_gpr_get(const prisma_dispatcher* d,
                                        uint32_t gpr_index,
                                        uint64_t* out);
prisma_status prisma_dispatcher_gpr_set(prisma_dispatcher* d,
                                        uint32_t gpr_index,
                                        uint64_t value);
prisma_status prisma_dispatcher_guest_pc(const prisma_dispatcher* d,
                                         uint64_t* out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PRISMA_CAPI_H */
