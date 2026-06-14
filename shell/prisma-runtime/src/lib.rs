// Prisma JIT runtime — Fase 4 del roadmap Rust.
//
// RFC 0015: migración de los 8 archivos en `core/src/runtime/`.
// Incluye dispatcher, signal handler, SMC guard, syscall handler,
// host features, JIT memory allocation.
//
// Responsabilidad: Claude (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod dispatcher;
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
