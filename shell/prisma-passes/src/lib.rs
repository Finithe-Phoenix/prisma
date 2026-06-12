// Prisma optimization passes — Fase 2 del roadmap Rust.
//
// RFC 0015: migración de los 16 archivos en `core/src/passes/`.
//
// Responsabilidad: Claude (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod pipeline;
pub mod const_prop;
pub mod algebraic;
pub mod strength_reduce;
pub mod redundant_load;
pub mod cse;
pub mod global_cse;
pub mod copy_prop;
pub mod dead_store;
pub mod branch_fold;
pub mod dce;
pub mod flag_write_elim;
pub mod tail_call;
pub mod x87_stack;
pub mod licm;
pub mod peephole;

pub use pipeline::{PassPipeline, default_pipeline};

use prisma_ir::Function;

/// Each pass transforms a `Function` into another `Function`.
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, func: Function) -> Function;
}
