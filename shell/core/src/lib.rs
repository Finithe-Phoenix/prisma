//! Safe Rust API for the Prisma DBT core (RFC 0014).
//!
//! Wraps the raw `prisma-core-sys` bindings with RAII handles and
//! `Result`-based errors. The lifetime story mirrors the C contract:
//! a [`Dispatcher`] borrows its [`Translator`] mutably and its
//! [`GuestImage`] immutably, so neither can be dropped (or mutated)
//! while guest code can still reach them.
//!
//! All `unsafe` in this crate is confined to FFI call sites; every
//! call upholds the capi.h contract documented per function.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::NonNull;

use prisma_core_sys as sys;

/// Errors surfaced by the core across the C boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("invalid argument crossed the FFI boundary")]
    InvalidArgument,
    #[error("decoder rejected the guest bytes")]
    DecodeFailed,
    #[error("lowerer rejected the IR")]
    LowerFailed,
    #[error("empty guest input")]
    EmptyInput,
    #[error("JIT buffer allocation failed")]
    JitAllocFailed,
    #[error("internal core error (exception stopped at the boundary)")]
    Internal,
    #[error("C ABI version mismatch: bindings expect {expected}, core has {actual}")]
    AbiMismatch { expected: u32, actual: u32 },
    #[error("unknown status code {0}")]
    Unknown(i32),
}

const fn check(status: sys::PrismaStatus) -> Result<(), CoreError> {
    match status {
        sys::PRISMA_OK => Ok(()),
        sys::PRISMA_STATUS_INVALID_ARGUMENT => Err(CoreError::InvalidArgument),
        sys::PRISMA_STATUS_DECODE_FAILED => Err(CoreError::DecodeFailed),
        sys::PRISMA_STATUS_LOWER_FAILED => Err(CoreError::LowerFailed),
        sys::PRISMA_STATUS_EMPTY_INPUT => Err(CoreError::EmptyInput),
        sys::PRISMA_STATUS_JIT_ALLOC_FAILED => Err(CoreError::JitAllocFailed),
        sys::PRISMA_STATUS_INTERNAL => Err(CoreError::Internal),
        other => Err(CoreError::Unknown(other)),
    }
}

/// Runtime C ABI version of the loaded core library.
#[must_use]
pub fn capi_version() -> u32 {
    // SAFETY: no arguments, no state; always safe to call.
    unsafe { sys::prisma_capi_version() }
}

/// Guest general-purpose registers in x86 encoding order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Gpr {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

/// How a translated block hands control back to the dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockExitKind {
    None,
    Return,
    JumpRel,
    JumpReg,
    CondJumpRel,
    CallRel,
    CallReg,
    RetAdjusted,
    RepStos,
    RepMovs,
    /// The core reported a kind these bindings do not know yet.
    Unknown(i32),
}

impl BlockExitKind {
    const fn from_raw(raw: i32) -> Self {
        match raw {
            sys::PRISMA_BLOCK_EXIT_NONE => Self::None,
            sys::PRISMA_BLOCK_EXIT_RETURN => Self::Return,
            sys::PRISMA_BLOCK_EXIT_JUMP_REL => Self::JumpRel,
            sys::PRISMA_BLOCK_EXIT_JUMP_REG => Self::JumpReg,
            sys::PRISMA_BLOCK_EXIT_COND_JUMP_REL => Self::CondJumpRel,
            sys::PRISMA_BLOCK_EXIT_CALL_REL => Self::CallRel,
            sys::PRISMA_BLOCK_EXIT_CALL_REG => Self::CallReg,
            sys::PRISMA_BLOCK_EXIT_RET_ADJUSTED => Self::RetAdjusted,
            sys::PRISMA_BLOCK_EXIT_REP_STOS => Self::RepStos,
            sys::PRISMA_BLOCK_EXIT_REP_MOVS => Self::RepMovs,
            other => Self::Unknown(other),
        }
    }
}

/// Metadata for one translated block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockInfo {
    pub code_size: u64,
    pub guest_size: u64,
    pub exit_kind: BlockExitKind,
    pub from_cache: bool,
    pub target_guest_pc: u64,
    pub fallthrough_guest_pc: u64,
    pub return_guest_pc: u64,
}

/// Translator counters (mirrors `Translator::Stats`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TranslatorStats {
    pub translations_attempted: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decode_failures: u64,
    pub lower_failures: u64,
}

/// Why a dispatcher run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchExit {
    Halted,
    StepLimit,
    FetchFailed,
    TranslationFailed,
    /// The core reported an exit these bindings do not know yet.
    Unknown(i32),
}

impl DispatchExit {
    const fn from_raw(raw: i32) -> Self {
        match raw {
            sys::PRISMA_DISPATCH_HALTED => Self::Halted,
            sys::PRISMA_DISPATCH_STEP_LIMIT => Self::StepLimit,
            sys::PRISMA_DISPATCH_FETCH_FAILED => Self::FetchFailed,
            sys::PRISMA_DISPATCH_TRANSLATION_FAILED => Self::TranslationFailed,
            other => Self::Unknown(other),
        }
    }
}

/// Dispatcher counters (mirrors `runtime::DispatchStats`).
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchStats {
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

/// Result of one dispatcher run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub exit: DispatchExit,
    pub final_pc: u64,
    pub stats: DispatchStats,
    /// Human-readable context on errors; empty on clean exits.
    pub message: String,
}

/// Owned handle to a core translator.
///
/// Not `Send`/`Sync` by construction (raw pointer member): the C
/// contract makes handles single-threaded.
pub struct Translator {
    raw: NonNull<sys::PrismaTranslator>,
}

impl Translator {
    /// Create a translator, verifying the C ABI version first.
    ///
    /// # Errors
    ///
    /// [`CoreError::AbiMismatch`] when the loaded library does not
    /// match these bindings; otherwise any status the core returns.
    pub fn new() -> Result<Self, CoreError> {
        let actual = capi_version();
        if actual != sys::PRISMA_CAPI_VERSION {
            return Err(CoreError::AbiMismatch {
                expected: sys::PRISMA_CAPI_VERSION,
                actual,
            });
        }
        let mut raw: *mut sys::PrismaTranslator = std::ptr::null_mut();
        // SAFETY: out-pointer is valid; written only on PRISMA_OK.
        check(unsafe { sys::prisma_translator_create(&raw mut raw) })?;
        NonNull::new(raw).map_or(Err(CoreError::Internal), |raw| Ok(Self { raw }))
    }

    /// Translate `bytes` as guest code at `guest_addr`.
    ///
    /// # Errors
    ///
    /// Decode/lower/allocation failures from the core.
    pub fn translate(&mut self, guest_addr: u64, bytes: &[u8]) -> Result<BlockInfo, CoreError> {
        let mut info = sys::PrismaBlockInfo::default();
        // SAFETY: handle is live (owned by self); `bytes` outlives the
        // call; out-pointer is valid.
        check(unsafe {
            sys::prisma_translator_translate(
                self.raw.as_ptr(),
                guest_addr,
                bytes.as_ptr(),
                bytes.len(),
                &raw mut info,
            )
        })?;
        Ok(BlockInfo {
            code_size: info.code_size,
            guest_size: info.guest_size,
            exit_kind: BlockExitKind::from_raw(info.exit_kind),
            from_cache: info.from_cache != 0,
            target_guest_pc: info.target_guest_pc,
            fallthrough_guest_pc: info.fallthrough_guest_pc,
            return_guest_pc: info.return_guest_pc,
        })
    }

    /// Translate `bytes` and return emitted host bytes.
    ///
    /// # Errors
    ///
    /// Decode/lower/allocation failures from the core.
    pub fn translate_with_code(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
    ) -> Result<(BlockInfo, Vec<u8>), CoreError> {
        let mut info = sys::PrismaBlockInfo::default();
        let mut code_len = 0usize;
        // SAFETY: handle is live; all pointers are valid for this call.
        check(unsafe {
            sys::prisma_translator_translate_with_code(
                self.raw.as_ptr(),
                guest_addr,
                bytes.as_ptr(),
                bytes.len(),
                &raw mut info,
                std::ptr::null_mut(),
                0,
                &raw mut code_len,
            )
        })?;

        let mut code = vec![0u8; code_len];
        if code_len > 0 {
            let mut written = 0usize;
            // SAFETY: second call is idempotent for identical input and this
            // translator instance; `code` is sized for `code_len`.
            check(unsafe {
                sys::prisma_translator_translate_with_code(
                    self.raw.as_ptr(),
                    guest_addr,
                    bytes.as_ptr(),
                    bytes.len(),
                    &raw mut info,
                    code.as_mut_ptr(),
                    code.len(),
                    &raw mut written,
                )
            })?;
            code.truncate(written);
        }

        Ok((
            BlockInfo {
                code_size: info.code_size,
                guest_size: info.guest_size,
                exit_kind: BlockExitKind::from_raw(info.exit_kind),
                from_cache: info.from_cache != 0,
                target_guest_pc: info.target_guest_pc,
                fallthrough_guest_pc: info.fallthrough_guest_pc,
                return_guest_pc: info.return_guest_pc,
            },
            code,
        ))
    }

    /// Toggle real CALL/RET semantics (on by default, F2-IR-054).
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn set_real_call_ret(&mut self, enabled: bool) -> Result<(), CoreError> {
        // SAFETY: handle is live.
        check(unsafe {
            sys::prisma_translator_set_real_call_ret(self.raw.as_ptr(), i32::from(enabled))
        })
    }

    /// Snapshot of the translator's counters.
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn stats(&self) -> Result<TranslatorStats, CoreError> {
        let mut out = sys::PrismaTranslatorStats::default();
        // SAFETY: handle is live; out-pointer is valid.
        check(unsafe { sys::prisma_translator_get_stats(self.raw.as_ptr(), &raw mut out) })?;
        Ok(TranslatorStats {
            translations_attempted: out.translations_attempted,
            cache_hits: out.cache_hits,
            cache_misses: out.cache_misses,
            decode_failures: out.decode_failures,
            lower_failures: out.lower_failures,
        })
    }
}

impl Drop for Translator {
    fn drop(&mut self) {
        // SAFETY: we own the handle; destroy exactly once.
        unsafe { sys::prisma_translator_destroy(self.raw.as_ptr()) }
    }
}

/// A contiguous guest memory image at a fixed base address.
///
/// This is the memory a [`Dispatcher`] fetches guest code from. The
/// dispatcher borrows it immutably, so it cannot move or shrink while
/// guest code can still be fetched.
#[derive(Debug, Clone)]
pub struct GuestImage {
    base: u64,
    bytes: Vec<u8>,
}

impl GuestImage {
    /// # Panics
    ///
    /// When `base + bytes.len()` overflows the guest address space.
    #[must_use]
    pub fn new(base: u64, bytes: Vec<u8>) -> Self {
        let len = u64::try_from(bytes.len()).expect("image length fits u64");
        assert!(
            base.checked_add(len).is_some(),
            "guest image must not wrap the address space"
        );
        Self { base, bytes }
    }

    #[must_use]
    pub const fn base(&self) -> u64 {
        self.base
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// C trampoline for the guest memory reader. Panics must not unwind
/// into C++ (RFC 0014 rule 3): any panic converts into "no memory
/// here", which the dispatcher reports as a fetch fault.
unsafe extern "C" fn image_reader(ctx: *mut c_void, pc: u64, out_bytes: *mut *const u8) -> usize {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ctx is the GuestImage the Dispatcher holds a borrow
        // of for its whole lifetime; valid and unaliased-by-mutation.
        let img = unsafe { &*ctx.cast::<GuestImage>() };
        let Some(off) = pc.checked_sub(img.base) else {
            return 0;
        };
        let Ok(off) = usize::try_from(off) else {
            return 0;
        };
        if off >= img.bytes.len() {
            return 0;
        }
        // SAFETY: out_bytes is a valid out-pointer per the C contract;
        // the offset is in bounds, checked above.
        unsafe {
            *out_bytes = img.bytes.as_ptr().add(off);
        }
        img.bytes.len() - off
    }));
    result.unwrap_or(0)
}

/// Owned handle to a core dispatcher, tied to a translator and a
/// guest image for its whole lifetime.
pub struct Dispatcher<'t, 'g> {
    raw: NonNull<sys::PrismaDispatcher>,
    _translator: PhantomData<&'t mut Translator>,
    _image: PhantomData<&'g GuestImage>,
}

impl<'t, 'g> Dispatcher<'t, 'g> {
    /// Create a dispatcher fetching guest code from `image`.
    ///
    /// # Errors
    ///
    /// Any status the core returns.
    pub fn new(translator: &'t mut Translator, image: &'g GuestImage) -> Result<Self, CoreError> {
        let mut raw: *mut sys::PrismaDispatcher = std::ptr::null_mut();
        // SAFETY: translator handle is live; the reader context is the
        // image borrowed for 'g >= the dispatcher's lifetime; written
        // only on PRISMA_OK.
        check(unsafe {
            sys::prisma_dispatcher_create(
                translator.raw.as_ptr(),
                Some(image_reader),
                std::ptr::from_ref(image).cast_mut().cast::<c_void>(),
                &raw mut raw,
            )
        })?;
        NonNull::new(raw).map_or(Err(CoreError::Internal), |raw| {
            Ok(Self {
                raw,
                _translator: PhantomData,
                _image: PhantomData,
            })
        })
    }

    /// Treat `pc` as a clean halt in addition to the halt sentinel.
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn add_halt_pc(&mut self, pc: u64) -> Result<(), CoreError> {
        // SAFETY: handle is live.
        check(unsafe { sys::prisma_dispatcher_add_halt_pc(self.raw.as_ptr(), pc) })
    }

    /// Install the 16-slot halt return stack and point guest RSP at
    /// it (see `Dispatcher::install_halt_return_stack` in C++).
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn install_halt_return_stack(&mut self) -> Result<(), CoreError> {
        // SAFETY: handle is live.
        check(unsafe { sys::prisma_dispatcher_install_halt_return_stack(self.raw.as_ptr()) })
    }

    /// Run from `entry_pc` for at most `max_steps` blocks.
    ///
    /// # Errors
    ///
    /// Only on internal core failures â€” guest-level problems (fetch
    /// fault, translation failure) come back inside [`RunOutcome`].
    pub fn run(&mut self, entry_pc: u64, max_steps: usize) -> Result<RunOutcome, CoreError> {
        let mut out = sys::PrismaRunResult::default();
        // SAFETY: handle is live; out-pointer is valid; the guest
        // image borrow is alive for 'g.
        check(unsafe {
            sys::prisma_dispatcher_run(self.raw.as_ptr(), entry_pc, max_steps, &raw mut out)
        })?;
        let msg_end = out
            .message
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(out.message.len());
        // c_char is i8 or u8 depending on the target; go through the
        // byte representation to stay portable.
        let msg_bytes: Vec<u8> = out.message[..msg_end]
            .iter()
            .map(|&c| u8::from_ne_bytes(c.to_ne_bytes()))
            .collect();
        let message = String::from_utf8_lossy(&msg_bytes).into_owned();
        Ok(RunOutcome {
            exit: DispatchExit::from_raw(out.exit),
            final_pc: out.final_pc,
            stats: DispatchStats {
                blocks_executed: out.stats.blocks_executed,
                steps_taken: out.stats.steps_taken,
                unique_pcs_seen: out.stats.unique_pcs_seen,
                ras_pushes: out.stats.ras_pushes,
                ras_pops: out.stats.ras_pops,
                ras_hits: out.stats.ras_hits,
                ras_misses: out.stats.ras_misses,
                ras_overflows: out.stats.ras_overflows,
                ras_underflows: out.stats.ras_underflows,
                direct_thread_hits: out.stats.direct_thread_hits,
                direct_thread_misses: out.stats.direct_thread_misses,
                direct_thread_installs: out.stats.direct_thread_installs,
                direct_jit_patch_attempts: out.stats.direct_jit_patch_attempts,
                direct_jit_patch_applied: out.stats.direct_jit_patch_applied,
                direct_jit_patch_rejected: out.stats.direct_jit_patch_rejected,
                direct_jit_patch_unpatches: out.stats.direct_jit_patch_unpatches,
                direct_jit_patch_executes: out.stats.direct_jit_patch_executes,
            },
            message,
        })
    }

    /// Read a guest GPR.
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn gpr(&self, gpr: Gpr) -> Result<u64, CoreError> {
        let mut out = 0u64;
        // SAFETY: handle is live; out-pointer is valid.
        check(unsafe {
            sys::prisma_dispatcher_gpr_get(self.raw.as_ptr(), gpr as u32, &raw mut out)
        })?;
        Ok(out)
    }

    /// Write a guest GPR.
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn set_gpr(&mut self, gpr: Gpr, value: u64) -> Result<(), CoreError> {
        // SAFETY: handle is live.
        check(unsafe { sys::prisma_dispatcher_gpr_set(self.raw.as_ptr(), gpr as u32, value) })
    }

    /// Read the guest program counter.
    ///
    /// # Errors
    ///
    /// Only on internal core failures.
    pub fn guest_pc(&self) -> Result<u64, CoreError> {
        let mut out = 0u64;
        // SAFETY: handle is live; out-pointer is valid.
        check(unsafe { sys::prisma_dispatcher_guest_pc(self.raw.as_ptr(), &raw mut out) })?;
        Ok(out)
    }
}

impl Drop for Dispatcher<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: we own the handle; destroy exactly once. The
        // translator outlives us by the 't borrow.
        unsafe { sys::prisma_dispatcher_destroy(self.raw.as_ptr()) }
    }
}
