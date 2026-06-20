// Prisma JIT runtime — Fase 4 del roadmap Rust.
//
// RFC 0015: migración de los 8 archivos en `core/src/runtime/`.
// Incluye dispatcher, signal handler, SMC guard, syscall handler,
// host features, JIT memory allocation.
//
// Responsabilidad: Claude (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
// Mirror the stylistic-lint allow set used across the sibling shell crates
// (decoder/cache/backend): these fire heavily on doc prose, const-eligible
// helpers, and the long dispatcher/JIT routines without flagging real issues.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    // Pervasive `lock().unwrap()` panics on mutex poisoning; documenting it on
    // every accessor is noise.
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

pub mod dispatcher;
pub mod exception_record;
pub mod executor;
pub mod guest_exception;
pub mod guest_signal;
pub mod host_features;
pub mod jit_buffer_pool;
pub mod jit_memory;
pub mod signal_handler;
pub mod smc_guard;
pub mod syscall_handler;

pub use dispatcher::Dispatcher;
pub use host_features::HostFeatures;
pub use jit_memory::JitBuffer;
pub use signal_handler::SignalHandler;
pub use smc_guard::SmcGuard;

/// Reason a translation block exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockExitKind {
    Return,
    JumpRel,
    JumpReg,
    CondJumpRel,
    CallRel,
    CallReg,
    RetAdjusted,
    RepStos,
    RepMovs,
    None,
}
