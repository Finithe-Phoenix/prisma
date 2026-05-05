//! Prisma container orchestrator.
//!
//! Status: scaffolding. The real container lifecycle / overlay FS /
//! downloader / P2P cache work lands across Fase 1+ (semanas 93+).
//! This crate exists now so the API shape is fixed early, the Cargo
//! workspace builds clean, and the Android `cdylib` target compiles
//! the moment a host has the Android NDK installed.
//!
//! Module layout:
//!
//! - [`container`]   — `Container { name, prefix_path, ... }` value
//!                     type and its [`ContainerError`].
//! - [`config`]      — TOML-backed per-container configuration.
//! - [`integrity`]   — sha256 verification of downloaded artefacts.
//! - [`cache_proto`] — shared types for the future P2P translation
//!                     cache wire format (Pilar 4).
//!
//! Each module today exports types + Default implementations so the
//! Kotlin side (via the future `jni` bridge) can already round-trip
//! a Container struct in tests.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]

pub mod container;
pub mod config;
pub mod integrity;
pub mod cache_proto;

/// Crate version. Surfaces in JNI and CLI for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_is_non_empty() {
        assert!(!VERSION.is_empty());
    }
}
