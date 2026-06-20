//! Property-based robustness fuzzing for the translation cache.
//!
//! The persistent cache file (RFC 0007 port) is a host<->guest trust
//! boundary: a cache on disk can be corrupted or crafted by an attacker, and
//! its header carries an attacker-controlled entry `count`. These properties
//! assert the loader is robust — it never panics and never pre-allocates on a
//! malicious `count` — that a clean round-trip preserves the live entry set,
//! and that eviction always honours the configured budget (the
//! resource-discipline invariant: a restart must not inherit an unbounded
//! cache). Complements the hand-written unit tests in `cache.rs` and is the
//! fifth proptest surface alongside decoder/passes/backend/translator.

use proptest::prelude::*;

use prisma_cache::{CacheEntry, CacheKey, TranslationCache};

/// `guest_addr`/`last_used`/`hit_count` are overwritten by insert/upsert, so
/// the caller only supplies the persisted payload fields.
fn entry(guest_size: u32, code: Vec<u8>) -> CacheEntry {
    CacheEntry {
        guest_addr: 0,
        guest_size,
        code_size: code.len() as u32,
        code_bytes: code.into_boxed_slice(),
        hit_count: 0,
        last_used: 0,
    }
}

/// (`guest_addr`, `content_hash`, `guest_size`, `code_bytes`).
fn entry_spec() -> impl Strategy<Value = (u64, u64, u32, Vec<u8>)> {
    (
        any::<u64>(),
        any::<u64>(),
        0u32..4096,
        prop::collection::vec(any::<u8>(), 0..64),
    )
}

fn build_cache(specs: &[(u64, u64, u32, Vec<u8>)]) -> TranslationCache {
    let mut c = TranslationCache::new();
    for (addr, hash, gsize, code) in specs {
        c.upsert((*addr, *hash), entry(*gsize, code.clone()));
    }
    c
}

fn temp_path() -> tempfile::TempPath {
    tempfile::NamedTempFile::new()
        .expect("create temp file")
        .into_temp_path()
}

proptest! {
    /// A clean save -> load round-trip preserves the live entry set: same live
    /// count, same total code bytes, every live key present.
    #[test]
    fn roundtrip_preserves_live_entries(specs in prop::collection::vec(entry_spec(), 0..32)) {
        let src = build_cache(&specs);
        let path = temp_path();
        prop_assert!(src.save_to_file(&path).is_none());

        let mut dst = TranslationCache::new();
        prop_assert!(dst.load_from_file(&path).is_none());

        prop_assert_eq!(dst.entry_count(), src.live_entry_count());
        prop_assert_eq!(dst.total_code_bytes(), src.total_code_bytes());
        for (addr, hash, _, _) in &specs {
            let key: CacheKey = (*addr, *hash);
            if src.contains_key(&key) {
                prop_assert!(dst.contains_key(&key));
            }
        }
    }

    /// Loading a bit-flipped / truncated cache file never panics and never
    /// allocates unboundedly; it returns cleanly (Ok or an IoError).
    #[test]
    fn corrupted_file_never_panics(
        specs in prop::collection::vec(entry_spec(), 0..16),
        flips in prop::collection::vec((any::<usize>(), any::<u8>()), 0..32),
        truncate in any::<usize>(),
    ) {
        let src = build_cache(&specs);
        let path = temp_path();
        prop_assert!(src.save_to_file(&path).is_none());

        let mut bytes = std::fs::read(&path).expect("read back saved cache");
        if !bytes.is_empty() {
            for (idx, xor) in &flips {
                let n = bytes.len();
                bytes[*idx % n] ^= *xor;
            }
            bytes.truncate(truncate % (bytes.len() + 1));
        }
        std::fs::write(&path, &bytes).expect("write mutated cache");

        let mut dst = TranslationCache::new();
        // Must not panic. Result intentionally ignored.
        let _ = dst.load_from_file(&path);
    }

    /// Loading arbitrary bytes (no valid structure) never panics.
    #[test]
    fn arbitrary_bytes_never_panic(raw in prop::collection::vec(any::<u8>(), 0..4096)) {
        let path = temp_path();
        std::fs::write(&path, &raw).expect("write raw bytes");
        let mut dst = TranslationCache::new();
        let _ = dst.load_from_file(&path);
    }

    /// After every insert the cache honours its configured budget: never more
    /// than `max_entries` entries, never more than `max_bytes` of code.
    #[test]
    fn eviction_respects_limits(
        specs in prop::collection::vec(entry_spec(), 0..64),
        max_entries in 1usize..16,
        max_bytes in 1usize..512,
    ) {
        let mut c = TranslationCache::new();
        c.set_limits(max_entries, max_bytes);
        for (addr, hash, gsize, code) in &specs {
            c.upsert((*addr, *hash), entry(*gsize, code.clone()));
            prop_assert!(c.entry_count() <= max_entries);
            prop_assert!(c.total_code_bytes() <= max_bytes);
        }
    }
}
