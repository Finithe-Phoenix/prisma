// Prisma x86_64 → IR decoder — Fase 3 del roadmap Rust.
//
// RFC 0015: migración de `core/src/decoder/x86_decoder.cpp` (7,318 líneas).
// El archivo C++ es el más grande del core; la migración se hace por
// grupos de opcodes, no de una vez.
//
// Responsabilidad: CODEX (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod decode;
pub mod tables;

use prisma_ir::{Op, Stmt};

/// Result of decoding a single x86 instruction.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub stmts: Vec<Stmt>,
    pub bytes_consumed: usize,
}

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

/// Decode one x86 instruction at the given offset.
/// Returns the decoded statements and the number of bytes consumed.
pub fn decode_one(bytes: &[u8], offset: usize) -> Result<Decoded, DecodeError> {
    // TODO: Phase 3 implementation
    Err(DecodeError::UnsupportedOpcode(bytes.get(offset).copied().unwrap_or(0)))
}
