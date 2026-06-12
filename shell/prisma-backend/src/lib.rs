// Prisma ARM64 backend — Fase 5 del roadmap Rust.
//
// RFC 0015: migración de `core/src/backend/emitter.cpp` (2,149 líneas),
// `lowering.cpp` (2,476 líneas), y `abi.cpp` (73 líneas).
// Depende de un assembler ARM64 custom (reemplaza vixl).
//
// Responsabilidad: CODEX (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod assembler;
pub mod lowerer;
pub mod abi;

pub use assembler::Arm64Assembler;
pub use lowerer::Lowerer;
