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
//! - [`container`]   Ã¢â‚¬â€ `Container { name, prefix_path, ... }` value
//!   type and its [`ContainerError`].
//! - [`registry`]    Ã¢â‚¬â€ directory-backed catalogue of containers under
//!   a root (list / create / remove).
//! - [`config`]      Ã¢â‚¬â€ TOML-backed per-container configuration.
//! - [`integrity`]   Ã¢â‚¬â€ sha256 verification of downloaded artefacts.
//! - [`cache_proto`] Ã¢â‚¬â€ shared types for the future P2P translation
//!   cache wire format (Pilar 4).
//!
//! Each module today exports types + Default implementations so the
//! Kotlin side (via the future `jni` bridge) can already round-trip
//! a Container struct in tests.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
// Skeleton crate (Fase 3 prep). The pedantic-level doc lints below are
// aspirational Ã¢â‚¬â€ they belong on the Fase 3 hardening pass, not on
// scaffolding that exists only to fix the public surface. Re-enable
// once the orchestrator carries real lifecycle / network code.
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::too_many_lines)]

pub mod address_space;
pub mod backed_address_space;
pub mod cache_proto;
pub mod config;
pub mod dxvk_bridge;
pub mod jni_bridge;
pub mod container;
pub mod cpu_features;
pub mod cpuid_leaves;
pub mod guest_layout;
pub mod guest_mem;
pub mod guest_memory;
pub mod guest_stack;
pub mod iat_patch;
pub mod import_resolver;
pub mod init_stack;
pub mod integrity;
pub mod load_pe;
pub mod module_table;
pub mod pe_loader;
pub mod registry;
pub mod vfs;
pub mod win32;
pub mod tso_classifier;
pub mod npu_delegate;

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



