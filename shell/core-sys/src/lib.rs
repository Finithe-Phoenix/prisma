//! Raw FFI bindings to the Prisma DBT core C ABI (RFC 0014).
//!
//! Mirrors `core/include/prisma/capi.h` one-to-one. Everything here is
//! `unsafe` by nature; the safe surface lives in the `prisma-core`
//! crate. Keep declarations in header order so drift reviews are a
//! side-by-side diff.

use core::ffi::{c_char, c_void};

/// Mirror of `PRISMA_CAPI_VERSION`. Compare against
/// [`prisma_capi_version`] before any other call.
pub const PRISMA_CAPI_VERSION: u32 = 2;

/// `prisma_status` values. The C enum is ABI-`int`; we keep plain
/// `i32` constants so unknown future values stay representable.
pub type PrismaStatus = i32;
pub const PRISMA_OK: PrismaStatus = 0;
pub const PRISMA_STATUS_INVALID_ARGUMENT: PrismaStatus = 1;
pub const PRISMA_STATUS_DECODE_FAILED: PrismaStatus = 2;
pub const PRISMA_STATUS_LOWER_FAILED: PrismaStatus = 3;
pub const PRISMA_STATUS_EMPTY_INPUT: PrismaStatus = 4;
pub const PRISMA_STATUS_JIT_ALLOC_FAILED: PrismaStatus = 5;
pub const PRISMA_STATUS_INTERNAL: PrismaStatus = 6;

/// `prisma_block_exit_kind` values.
pub type PrismaBlockExitKind = i32;
pub const PRISMA_BLOCK_EXIT_NONE: PrismaBlockExitKind = 0;
pub const PRISMA_BLOCK_EXIT_RETURN: PrismaBlockExitKind = 1;
pub const PRISMA_BLOCK_EXIT_JUMP_REL: PrismaBlockExitKind = 2;
pub const PRISMA_BLOCK_EXIT_JUMP_REG: PrismaBlockExitKind = 3;
pub const PRISMA_BLOCK_EXIT_COND_JUMP_REL: PrismaBlockExitKind = 4;
pub const PRISMA_BLOCK_EXIT_CALL_REL: PrismaBlockExitKind = 5;
pub const PRISMA_BLOCK_EXIT_CALL_REG: PrismaBlockExitKind = 6;
pub const PRISMA_BLOCK_EXIT_RET_ADJUSTED: PrismaBlockExitKind = 7;
pub const PRISMA_BLOCK_EXIT_REP_STOS: PrismaBlockExitKind = 8;
pub const PRISMA_BLOCK_EXIT_REP_MOVS: PrismaBlockExitKind = 9;

/// `prisma_dispatch_exit` values.
pub type PrismaDispatchExit = i32;
pub const PRISMA_DISPATCH_HALTED: PrismaDispatchExit = 0;
pub const PRISMA_DISPATCH_STEP_LIMIT: PrismaDispatchExit = 1;
pub const PRISMA_DISPATCH_FETCH_FAILED: PrismaDispatchExit = 2;
pub const PRISMA_DISPATCH_TRANSLATION_FAILED: PrismaDispatchExit = 3;

/// Number of guest GPRs (`PRISMA_GPR_COUNT`).
pub const PRISMA_GPR_COUNT: u32 = 16;

/// Opaque handle to a `prisma::translator::Translator`.
#[repr(C)]
pub struct PrismaTranslator {
    _private: [u8; 0],
}

/// Opaque handle to a `prisma::runtime::Dispatcher`.
#[repr(C)]
pub struct PrismaDispatcher {
    _private: [u8; 0],
}

/// Mirror of `prisma_block_info`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PrismaBlockInfo {
    pub code_size: u64,
    pub guest_size: u64,
    pub target_guest_pc: u64,
    pub fallthrough_guest_pc: u64,
    pub return_guest_pc: u64,
    pub exit_kind: PrismaBlockExitKind,
    pub from_cache: u8,
    pub reserved: [u8; 3],
}

/// Mirror of `prisma_translator_stats`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PrismaTranslatorStats {
    pub translations_attempted: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decode_failures: u64,
    pub lower_failures: u64,
}

/// Mirror of `prisma_dispatch_stats`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PrismaDispatchStats {
    pub blocks_executed: u64,
    pub steps_taken: u64,
    pub unique_pcs_seen: u64,
    pub ras_pushes: u64,
    pub ras_pops: u64,
    pub ras_hits: u64,
    pub ras_misses: u64,
    pub ras_overflows: u64,
    pub ras_underflows: u64,
    pub direct_thread_hits: u64,
    pub direct_thread_misses: u64,
    pub direct_thread_installs: u64,
    pub direct_jit_patch_attempts: u64,
    pub direct_jit_patch_applied: u64,
    pub direct_jit_patch_rejected: u64,
    pub direct_jit_patch_unpatches: u64,
    pub direct_jit_patch_executes: u64,
}

/// Mirror of `prisma_run_result`.
#[repr(C)]
pub struct PrismaRunResult {
    pub exit: PrismaDispatchExit,
    pub reserved: [u8; 4],
    pub final_pc: u64,
    pub stats: PrismaDispatchStats,
    /// Truncated, always NUL-terminated context on errors.
    pub message: [c_char; 128],
}

impl Default for PrismaRunResult {
    fn default() -> Self {
        Self {
            exit: 0,
            reserved: [0; 4],
            final_pc: 0,
            stats: PrismaDispatchStats::default(),
            message: [0; 128],
        }
    }
}

/// Mirror of `prisma_mem_reader`. Implementations MUST NOT unwind:
/// catch panics and return 0 ("no memory here" → fetch fault).
pub type PrismaMemReader =
    Option<unsafe extern "C" fn(ctx: *mut c_void, pc: u64, out_bytes: *mut *const u8) -> usize>;

extern "C" {
    pub fn prisma_capi_version() -> u32;

    pub fn prisma_translator_create(out: *mut *mut PrismaTranslator) -> PrismaStatus;
    pub fn prisma_translator_destroy(t: *mut PrismaTranslator);
    pub fn prisma_translator_translate(
        t: *mut PrismaTranslator,
        guest_addr: u64,
        bytes: *const u8,
        len: usize,
        out_info: *mut PrismaBlockInfo,
    ) -> PrismaStatus;
    pub fn prisma_translator_set_real_call_ret(
        t: *mut PrismaTranslator,
        enabled: i32,
    ) -> PrismaStatus;
    pub fn prisma_translator_get_stats(
        t: *const PrismaTranslator,
        out: *mut PrismaTranslatorStats,
    ) -> PrismaStatus;

    pub fn prisma_dispatcher_create(
        t: *mut PrismaTranslator,
        reader: PrismaMemReader,
        ctx: *mut c_void,
        out: *mut *mut PrismaDispatcher,
    ) -> PrismaStatus;
    pub fn prisma_dispatcher_destroy(d: *mut PrismaDispatcher);
    pub fn prisma_dispatcher_add_halt_pc(d: *mut PrismaDispatcher, pc: u64) -> PrismaStatus;
    pub fn prisma_dispatcher_install_halt_return_stack(d: *mut PrismaDispatcher) -> PrismaStatus;
    pub fn prisma_dispatcher_run(
        d: *mut PrismaDispatcher,
        entry_pc: u64,
        max_steps: usize,
        out: *mut PrismaRunResult,
    ) -> PrismaStatus;
    pub fn prisma_dispatcher_gpr_get(
        d: *const PrismaDispatcher,
        gpr_index: u32,
        out: *mut u64,
    ) -> PrismaStatus;
    pub fn prisma_dispatcher_gpr_set(
        d: *mut PrismaDispatcher,
        gpr_index: u32,
        value: u64,
    ) -> PrismaStatus;
    pub fn prisma_dispatcher_guest_pc(d: *const PrismaDispatcher, out: *mut u64) -> PrismaStatus;
}
