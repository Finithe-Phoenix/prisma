// Prisma x86_64 → IR decoder — Fase 3 del roadmap Rust.
//
// RFC 0015: migración de `core/src/decoder/x86_decoder.cpp` (7,318 líneas).
// El archivo C++ es el más grande del core; la migración se hace por
// grupos de opcodes, no de una vez.
//
// Responsabilidad: CODEX (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    dead_code,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::branches_sharing_code,
    clippy::manual_let_else,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::needless_borrow,
    clippy::needless_pass_by_ref_mut,
    clippy::ptr_arg,
    clippy::struct_excessive_bools,
    clippy::trivially_copy_pass_by_ref,
    clippy::wildcard_imports
)]

pub mod decode;
mod modrm;
mod prefixes;
pub mod tables;

/// Errors that can occur during decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("insufficient bytes")]
    Truncated,
    #[error("unsupported opcode (0x{0:02X})")]
    UnsupportedOpcode(u8),
    #[error("invalid ModR/M at byte {0}")]
    InvalidModRm(usize),
    #[error("invalid SIB at byte {0}")]
    InvalidSib(usize),
    #[error("lock prefix not allowed on this opcode")]
    InvalidLock,
    #[error("invalid VEX encoding")]
    InvalidVex,
}

pub use decode::{decode_one, decode_one_at, Decoded};
