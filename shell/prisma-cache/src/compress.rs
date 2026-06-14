//! Compression helpers used by the Rust cache persistence path.
//!
//! The C++ implementation in this migration path uses Zstandard one-shot
//! operations for short payloads. For Rust we mirror that behavior with a
//! minimal wrapper over `zstd::stream`.

use std::io::{self, Read, Write};

/// Compresses `bytes` with Zstandard at the requested level.
///
/// Returns an empty vector when the input is empty or compression fails,
/// following the permissive behavior of the C++ reference implementation.
pub fn compress(bytes: &[u8], level: i32) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }

    let mut encoder = match zstd::Encoder::new(Vec::new(), level) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    if encoder.write_all(bytes).is_err() {
        return Vec::new();
    }

    match encoder.finish() {
        Ok(compressed) => compressed,
        Err(_) => Vec::new(),
    }
}

/// Decompresses a Zstandard frame.
///
/// Returns `None` on malformed input or decoder errors.
pub fn decompress(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.is_empty() {
        return Some(Vec::new());
    }

    let mut decoder = zstd::Decoder::new(io::Cursor::new(frame)).ok()?;
    let mut out = Vec::new();
    if decoder.read_to_end(&mut out).is_err() {
        return None;
    }

    Some(out)
}
