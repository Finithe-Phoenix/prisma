//! Wire-format types for the future P2P translation cache (Pillar 4).
//!
//! Status: types only. The libp2p network layer + the actual
//! request/response handlers land alongside the F2.5 P2P work; the
//! serde-derived structs here let the C++ core (via the future FFI
//! bridge) and the Rust networking code agree on the same shape now.

use std::io::Read;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::integrity::Sha256Hash;

/// One translation-cache entry as it travels over the wire.
///
/// Mirrors the on-disk format described in RFC 0007 (cache file
/// format) — keep in lockstep when that RFC ships v3.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheEntry {
    pub guest_addr: u64,
    pub content_hash: u64, // FNV-1a of guest bytes (cache key)
    pub guest_size: u64,
    /// SHA-256 of the on-the-wire `code_bytes`. Used by the receiver
    /// to verify integrity before accepting the entry.
    pub code_sha256: Sha256Hash,
    pub code_bytes: Vec<u8>,
    /// Optional zstd compression: when true, `code_bytes` is the
    /// compressed payload; receiver decompresses before verifying
    /// `code_sha256` against the decoded form.
    pub compressed: bool,
}

/// Hard ceiling on a single decompressed entry. A peer-supplied entry must
/// not expand past this, so a crafted zstd "bomb" (tiny compressed, gigabytes
/// decompressed) is rejected before it can exhaust memory. One translated
/// block is far smaller than 16 MiB.
const MAX_DECOMPRESSED: usize = 16 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CacheVerifyError {
    #[error("decompressed payload exceeds the {0}-byte ceiling")]
    DecompressTooLarge(usize),
    #[error("zstd decompression failed")]
    DecompressFailed,
    #[error("sha-256 mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
}

impl CacheEntry {
    /// Verify a received entry before it is trusted. Decompresses when needed
    /// (bounded, so a malicious peer cannot zip-bomb us) and checks the
    /// declared SHA-256 against the actual payload. **The receiver must call
    /// this before mapping or executing peer-supplied code** — it is the trust
    /// gate of the P2P cache (Pillar 4).
    pub fn verify(&self) -> Result<(), CacheVerifyError> {
        self.verify_bounded(MAX_DECOMPRESSED)
    }

    fn verify_bounded(&self, limit: usize) -> Result<(), CacheVerifyError> {
        if self.compressed {
            let decoder = zstd::Decoder::new(std::io::Cursor::new(&self.code_bytes))
                .map_err(|_| CacheVerifyError::DecompressFailed)?;
            // Read at most `limit + 1` bytes: an over-large stream is caught
            // (and rejected) before the full output is ever materialised.
            let mut out = Vec::new();
            decoder
                .take(limit as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|_| CacheVerifyError::DecompressFailed)?;
            if out.len() > limit {
                return Err(CacheVerifyError::DecompressTooLarge(limit));
            }
            self.check_sha(&out)
        } else {
            self.check_sha(&self.code_bytes)
        }
    }

    fn check_sha(&self, payload: &[u8]) -> Result<(), CacheVerifyError> {
        let actual = Sha256Hash::from_bytes(payload);
        if actual == self.code_sha256 {
            Ok(())
        } else {
            Err(CacheVerifyError::Mismatch {
                expected: self.code_sha256.to_hex(),
                actual: actual.to_hex(),
            })
        }
    }
}

/// Top-level frame the P2P transport expects. Lets the wire format
/// evolve (versioned envelope, never re-tag).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheMsg {
    /// "Do you have a translation for `(guest_addr, content_hash)`?"
    Query { guest_addr: u64, content_hash: u64 },
    /// Reply: empty if the peer doesn't have it.
    Reply { entries: Vec<CacheEntry> },
    /// Push: peer offers entries unsolicited (gossip mode).
    Announce { entries: Vec<CacheEntry> },
}

/// Protocol version; bump on incompatible shape changes.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_entry_round_trips_through_toml() {
        // toml is what the on-disk persistent cache uses; bincode /
        // serde_json would also pass.
        let h = Sha256Hash::from_bytes(b"some-entry");
        let e = CacheEntry {
            guest_addr: 0x1000,
            content_hash: 0xCAFE_BABE,
            guest_size: 16,
            code_sha256: h,
            code_bytes: vec![0x90, 0x90, 0xC3],
            compressed: false,
        };
        // toml needs a wrapper struct because of top-level array
        // restriction. JSON works directly:
        let s = serde_json::to_string(&e).expect("serialize");
        let r: CacheEntry = serde_json::from_str(&s).expect("parse");
        assert_eq!(e, r);
    }

    #[test]
    fn cache_msg_query_and_reply_round_trip() {
        let q = CacheMsg::Query {
            guest_addr: 0x4000,
            content_hash: 0xDEAD_BEEF,
        };
        let s = serde_json::to_string(&q).unwrap();
        let r: CacheMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(q, r);

        let a = CacheMsg::Reply {
            entries: Vec::new(),
        };
        let s = serde_json::to_string(&a).unwrap();
        let r: CacheMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(a, r);
    }

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    fn entry_for(payload: &[u8], compressed: bool) -> CacheEntry {
        let code_bytes = if compressed {
            zstd::encode_all(payload, 3).unwrap()
        } else {
            payload.to_vec()
        };
        CacheEntry {
            guest_addr: 0x1000,
            content_hash: 1,
            guest_size: payload.len() as u64,
            code_sha256: Sha256Hash::from_bytes(payload),
            code_bytes,
            compressed,
        }
    }

    #[test]
    fn verify_accepts_uncompressed_and_compressed() {
        assert!(entry_for(b"arm64-block", false).verify().is_ok());
        assert!(entry_for(b"arm64-block-via-zstd-payload", true)
            .verify()
            .is_ok());
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        // A peer flips the code but keeps the old (or a bogus) digest.
        let mut e = entry_for(b"good-block", false);
        e.code_sha256 = Sha256Hash::from_bytes(b"attacker-chosen");
        assert!(matches!(e.verify(), Err(CacheVerifyError::Mismatch { .. })));
    }

    #[test]
    fn verify_rejects_zip_bomb() {
        // 4 KiB of zeros compresses to a tiny blob; against a 64-byte ceiling
        // it is rejected before the full output is materialised.
        let e = entry_for(&vec![0u8; 4096], true);
        assert!(matches!(
            e.verify_bounded(64),
            Err(CacheVerifyError::DecompressTooLarge(64))
        ));
    }
}
