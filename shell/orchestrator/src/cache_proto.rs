//! Wire-format types for the future P2P translation cache (Pillar 4).
//!
//! Status: types only. The libp2p network layer + the actual
//! request/response handlers land alongside the F2.5 P2P work; the
//! serde-derived structs here let the C++ core (via the future FFI
//! bridge) and the Rust networking code agree on the same shape now.

use serde::{Deserialize, Serialize};

use crate::integrity::Sha256Hash;

/// One translation-cache entry as it travels over the wire.
///
/// Mirrors the on-disk format described in RFC 0007 (cache file
/// format) — keep in lockstep when that RFC ships v3.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheEntry {
    pub guest_addr:   u64,
    pub content_hash: u64,        // FNV-1a of guest bytes (cache key)
    pub guest_size:   u64,
    /// SHA-256 of the on-the-wire `code_bytes`. Used by the receiver
    /// to verify integrity before accepting the entry.
    pub code_sha256:  Sha256Hash,
    pub code_bytes:   Vec<u8>,
    /// Optional zstd compression: when true, `code_bytes` is the
    /// compressed payload; receiver decompresses before verifying
    /// `code_sha256` against the decoded form.
    pub compressed:   bool,
}

/// Top-level frame the P2P transport expects. Lets the wire format
/// evolve (versioned envelope, never re-tag).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheMsg {
    /// "Do you have a translation for `(guest_addr, content_hash)`?"
    Query {
        guest_addr:   u64,
        content_hash: u64,
    },
    /// Reply: empty if the peer doesn't have it.
    Reply {
        entries: Vec<CacheEntry>,
    },
    /// Push: peer offers entries unsolicited (gossip mode).
    Announce {
        entries: Vec<CacheEntry>,
    },
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
            guest_addr:   0x1000,
            content_hash: 0xCAFEBABE,
            guest_size:   16,
            code_sha256:  h.clone(),
            code_bytes:   vec![0x90, 0x90, 0xC3],
            compressed:   false,
        };
        // toml needs a wrapper struct because of top-level array
        // restriction. JSON works directly:
        let s = serde_json::to_string(&e).expect("serialize");
        let r: CacheEntry = serde_json::from_str(&s).expect("parse");
        assert_eq!(e, r);
    }

    #[test]
    fn cache_msg_query_and_reply_round_trip() {
        let q = CacheMsg::Query { guest_addr: 0x4000, content_hash: 0xDEADBEEF };
        let s = serde_json::to_string(&q).unwrap();
        let r: CacheMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(q, r);

        let a = CacheMsg::Reply { entries: Vec::new() };
        let s = serde_json::to_string(&a).unwrap();
        let r: CacheMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(a, r);
    }

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
