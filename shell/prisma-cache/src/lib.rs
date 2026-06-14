// Prisma translation cache — Fase 1 del roadmap Rust.
//
// RFC 0015: migración de `core/src/cache/translation_cache.cpp`,
// `sha256.cpp`, `compress.cpp`.
//
// Responsabilidad: CODEX (según territory split).

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::manual_let_else,
    clippy::manual_unwrap_or_default,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    clippy::use_self
)]

pub mod cache;
pub mod compress;
pub mod sha256;

pub use cache::TranslationCache;
pub use sha256::Sha256Hash;

/// Key for cache lookup: (`guest_addr`, `fnv1a_content_hash`).
pub type CacheKey = (u64, u64);

/// Entry stored in the translation cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub guest_addr: u64,
    pub guest_size: u32,
    pub code_size: u32,
    pub code_bytes: Box<[u8]>,
    pub hit_count: u64,
    pub last_used: u64, // tick
}
