// Prisma optimization passes — Fase 2 del roadmap Rust.
//
// RFC 0015: migración de los 16 archivos en `core/src/passes/`.
//
// Responsabilidad: Claude (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    unused_assignments,
    unused_imports,
    unused_variables,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::manual_checked_ops,
    clippy::manual_rotate,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::option_if_let_else
)]

pub mod algebraic;
pub mod branch_fold;
pub mod const_prop;
pub mod copy_prop;
pub mod cse;
pub mod dce;
pub mod dead_store;
pub mod flag_write_elim;
pub mod global_cse;
pub mod licm;
pub mod peephole;
pub mod pipeline;
pub mod redundant_load;
pub mod strength_reduce;
pub mod tail_call;
pub mod x87_stack;

pub use pipeline::{default_pipeline, PassPipeline};

use prisma_ir::Function;

/// Each pass transforms a `Function` into another `Function`.
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, func: Function) -> Function;
}
