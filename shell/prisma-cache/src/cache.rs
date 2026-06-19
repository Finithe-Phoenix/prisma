use crate::compress;
use crate::{CacheEntry, CacheKey};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread::{self, JoinHandle};

/// FNV-1a 64-bit hash used as cache key for fast in-process SMC detection.
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const K_FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const K_FNV_PRIME: u64 = 0x1_0000_0000_01b3;
    let mut h = K_FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(K_FNV_PRIME);
    }
    h
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissReason {
    UnknownAddress,
    StaleContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupResult {
    Hit(CacheEntry),
    Miss(MissReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryStats {
    pub hit_count: u64,
    pub last_used_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    OpenFailed,
    WriteFailed,
    ReadFailed,
    BadMagic,
    UnsupportedVersion,
    Truncated,
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::OpenFailed => write!(f, "open failed"),
            IoError::WriteFailed => write!(f, "write failed"),
            IoError::ReadFailed => write!(f, "read failed"),
            IoError::BadMagic => write!(f, "bad magic"),
            IoError::UnsupportedVersion => write!(f, "unsupported version"),
            IoError::Truncated => write!(f, "truncated"),
        }
    }
}

impl std::error::Error for IoError {}

const MAGIC: u64 = 0x50_52_53_4d_43_41_43_48;
const VERSION: u32 = 2;
const FLAG_COMPRESSED: u32 = 1;

fn write_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn read_u32(input: &[u8], offset: &mut usize) -> Option<u32> {
    if input.len().saturating_sub(*offset) < 4 {
        return None;
    }
    let mut b = [0u8; 4];
    b.copy_from_slice(&input[*offset..*offset + 4]);
    *offset += 4;
    Some(u32::from_le_bytes(b))
}

fn read_u64(input: &[u8], offset: &mut usize) -> Option<u64> {
    if input.len().saturating_sub(*offset) < 8 {
        return None;
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&input[*offset..*offset + 8]);
    *offset += 8;
    Some(u64::from_le_bytes(b))
}

#[derive(Debug)]
pub struct TranslationCache {
    entries: HashMap<CacheKey, CacheEntry>,
    addr_to_hash: HashMap<u64, u64>,
    max_entries: usize,
    max_bytes: usize,
    next_tick: u64,
    compress_on_save: bool,
    async_handle: Option<JoinHandle<Option<IoError>>>,
}

impl Default for TranslationCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            addr_to_hash: HashMap::new(),
            max_entries: 0,
            max_bytes: 0,
            next_tick: 1,
            compress_on_save: false,
            async_handle: None,
        }
    }
}

impl TranslationCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_max_entries(&mut self, n: usize) {
        self.max_entries = n;
        self.maybe_evict();
    }

    #[must_use]
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn set_max_bytes(&mut self, n: usize) {
        self.max_bytes = n;
        self.maybe_evict();
    }

    #[must_use]
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn set_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.max_entries = max_entries;
        self.max_bytes = max_bytes;
        self.maybe_evict();
    }

    #[must_use]
    pub fn limits(&self) -> (usize, usize) {
        (self.max_entries, self.max_bytes)
    }

    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.addr_to_hash.len()
    }

    #[must_use]
    pub fn live_entry_count(&self) -> usize {
        self.entries
            .keys()
            .filter(|key| self.is_live_key(key))
            .count()
    }

    #[must_use]
    pub fn stale_count(&self) -> usize {
        self.entries.len().saturating_sub(self.live_entry_count())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.addr_to_hash.clear();
        self.next_tick = 1;
    }

    pub fn contains_key(&self, key: &CacheKey) -> bool {
        self.entries.contains_key(key)
    }

    pub fn lookup(&mut self, guest_addr: u64, guest_bytes: &[u8]) -> LookupResult {
        let current_hash = fnv1a_64(guest_bytes);
        let known = match self.addr_to_hash.get(&guest_addr) {
            Some(hash) => *hash,
            None => return LookupResult::Miss(MissReason::UnknownAddress),
        };

        if known != current_hash {
            return LookupResult::Miss(MissReason::StaleContent);
        }

        let key = (guest_addr, current_hash);
        if let Some(mut e) = self.entries.get_mut(&key).map(|entry| entry.clone()) {
            e.hit_count = e.hit_count.saturating_add(1);
            let tick = self.next_tick;
            self.next_tick = self.next_tick.saturating_add(1);
            if let Some(slot) = self.entries.get_mut(&key) {
                slot.hit_count = e.hit_count;
                slot.last_used = tick;
            }
            e.last_used = tick;
            return LookupResult::Hit(e);
        }

        LookupResult::Miss(MissReason::StaleContent)
    }

    pub fn insert(&mut self, key: CacheKey, mut entry: CacheEntry) -> bool {
        if self.entries.contains_key(&key) {
            return false;
        }
        entry.guest_addr = key.0;
        let tick = self.next_tick;
        self.next_tick = self.next_tick.saturating_add(1);
        entry.last_used = tick;
        entry.hit_count = 0;
        self.addr_to_hash.insert(key.0, key.1);
        self.entries.insert(key, entry);
        self.maybe_evict();
        true
    }

    pub fn upsert(&mut self, key: CacheKey, mut entry: CacheEntry) {
        if let Some(previous_hash) = self.addr_to_hash.get(&key.0).copied() {
            if previous_hash != key.1 {
                self.entries.remove(&(key.0, previous_hash));
            }
        }

        entry.guest_addr = key.0;
        let tick = self.next_tick;
        self.next_tick = self.next_tick.saturating_add(1);
        entry.last_used = tick;
        entry.hit_count = 0;
        self.addr_to_hash.insert(key.0, key.1);
        self.entries.insert(key, entry);
        self.maybe_evict();
    }

    pub fn invalidate(&mut self, key: &CacheKey) {
        if let Some(current_hash) = self.addr_to_hash.remove(&key.0) {
            if current_hash != key.1 {
                self.entries.remove(&(key.0, current_hash));
            }
        }
        self.entries.remove(key);
    }

    pub fn touch(&mut self, key: &CacheKey) -> bool {
        if let Some(entry) = self.entries.get_mut(key) {
            let tick = self.next_tick;
            self.next_tick = self.next_tick.saturating_add(1);
            entry.hit_count = entry.hit_count.saturating_add(1);
            entry.last_used = tick;
            true
        } else {
            false
        }
    }

    pub fn invalidate_page(&mut self, page_addr: u64, page_size: usize) {
        let end = page_addr.saturating_add(page_size as u64);
        let to_drop: Vec<u64> = self
            .addr_to_hash
            .keys()
            .copied()
            .filter(|&addr| addr >= page_addr && addr < end)
            .collect();
        for addr in to_drop {
            if let Some(h) = self.addr_to_hash.remove(&addr) {
                self.entries.remove(&(addr, h));
            }
        }
    }

    #[must_use]
    pub fn total_code_bytes(&self) -> usize {
        self.entries
            .values()
            .map(|entry| entry.code_bytes.len())
            .sum()
    }

    #[must_use]
    pub fn stats_for(&self, key: &CacheKey) -> Option<EntryStats> {
        self.entries.get(key).map(|entry| EntryStats {
            hit_count: entry.hit_count,
            last_used_tick: entry.last_used,
        })
    }

    pub fn reset_hit_counts(&mut self) {
        for entry in self.entries.values_mut() {
            entry.hit_count = 0;
        }
    }

    pub fn compact(&mut self) -> usize {
        let mut evicted = 0usize;
        let keys: Vec<CacheKey> = self.entries.keys().copied().collect();
        for key in keys {
            match self.addr_to_hash.get(&key.0) {
                Some(h) if *h == key.1 => {}
                _ => {
                    self.entries.remove(&key);
                    evicted += 1;
                }
            }
        }
        evicted
    }

    fn is_live_key(&self, key: &CacheKey) -> bool {
        matches!(self.addr_to_hash.get(&key.0), Some(hash) if *hash == key.1)
    }

    pub fn set_compress_on_save(&mut self, on: bool) {
        self.compress_on_save = on;
    }

    #[must_use]
    pub fn compress_on_save(&self) -> bool {
        self.compress_on_save
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Option<IoError> {
        let mut body = Vec::new();
        write_u64(&mut body, MAGIC);
        write_u32(&mut body, VERSION);
        let mut flags = 0u32;
        if self.compress_on_save {
            flags |= FLAG_COMPRESSED;
        }
        write_u32(&mut body, flags);
        write_u64(&mut body, 0); // cpu fingerprint reserved
        write_u64(&mut body, self.live_entry_count() as u64);

        for (key, entry) in self
            .entries
            .iter()
            .filter(|(key, _entry)| self.is_live_key(key))
        {
            write_u64(&mut body, key.0);
            write_u64(&mut body, key.1);
            write_u64(&mut body, u64::from(entry.guest_size));

            let payload = if self.compress_on_save {
                let compressed = compress::compress(&entry.code_bytes, 3);
                if compressed.is_empty() && !entry.code_bytes.is_empty() {
                    return Some(IoError::WriteFailed);
                }
                if compressed.is_empty() {
                    Vec::new()
                } else {
                    compressed
                }
            } else {
                entry.code_bytes.to_vec()
            };
            write_u64(&mut body, payload.len() as u64);
            write_u64(&mut body, entry.code_bytes.len() as u64);
            body.extend_from_slice(&payload);
        }

        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Some(match e.kind() {
                        io::ErrorKind::NotFound => IoError::WriteFailed,
                        _ => IoError::OpenFailed,
                    });
                }
            }
        }
        let mut f = match File::create(path.as_ref()) {
            Ok(f) => f,
            Err(_) => return Some(IoError::OpenFailed),
        };
        if f.write_all(&body).is_err() {
            return Some(IoError::WriteFailed);
        }
        if f.flush().is_err() {
            return Some(IoError::WriteFailed);
        }
        None
    }

    pub fn save_to_file_async<P: AsRef<Path> + Send + 'static>(&mut self, path: P) {
        if let Some(handle) = self.async_handle.take() {
            let _ = handle.join();
        }
        let data = self.entries.clone();
        let addr_to_hash = self.addr_to_hash.clone();
        let compress_on_save = self.compress_on_save;
        let path = path.as_ref().to_path_buf();
        self.async_handle = Some(thread::spawn(move || {
            let mut cache = TranslationCache::new();
            cache.entries = data;
            cache.addr_to_hash = addr_to_hash;
            cache.compress_on_save = compress_on_save;
            cache.save_to_file(path).or(None)
        }));
    }

    pub fn wait_for_async_save(&mut self) -> Option<IoError> {
        self.async_handle
            .take()
            .and_then(|h| h.join().ok().flatten())
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Option<IoError> {
        let mut f = match File::open(path.as_ref()) {
            Ok(f) => f,
            Err(_) => return Some(IoError::OpenFailed),
        };
        let mut input = Vec::new();
        if f.read_to_end(&mut input).is_err() {
            return Some(IoError::ReadFailed);
        }

        let mut i = 0usize;
        let magic = match read_u64(&input, &mut i) {
            Some(v) => v,
            None => return Some(IoError::ReadFailed),
        };
        if magic != MAGIC {
            return Some(IoError::BadMagic);
        }
        let version = match read_u32(&input, &mut i) {
            Some(v) => v,
            None => return Some(IoError::ReadFailed),
        };
        if version != 1 && version != VERSION {
            return Some(IoError::UnsupportedVersion);
        }
        let flags = match read_u32(&input, &mut i) {
            Some(v) => v,
            None => return Some(IoError::ReadFailed),
        };
        let _cpu_fp = match read_u64(&input, &mut i) {
            Some(v) => v,
            None => return Some(IoError::ReadFailed),
        };
        let count = match read_u64(&input, &mut i) {
            Some(v) => v as usize,
            None => return Some(IoError::ReadFailed),
        };
        let compressed = version >= VERSION && (flags & FLAG_COMPRESSED) != 0;

        let mut entries = HashMap::with_capacity(count);
        let mut addr_to_hash = HashMap::with_capacity(count);

        for _ in 0..count {
            let guest_addr = match read_u64(&input, &mut i) {
                Some(v) => v,
                None => return Some(IoError::Truncated),
            };
            let content_hash = match read_u64(&input, &mut i) {
                Some(v) => v,
                None => return Some(IoError::Truncated),
            };
            let guest_size = match read_u64(&input, &mut i) {
                Some(v) => v as u32,
                None => return Some(IoError::Truncated),
            };
            let stored_size = match read_u64(&input, &mut i) {
                Some(v) => v as usize,
                None => return Some(IoError::Truncated),
            };
            let uncompressed_size = if version == VERSION {
                match read_u64(&input, &mut i) {
                    Some(v) => v as usize,
                    None => return Some(IoError::Truncated),
                }
            } else {
                stored_size
            };
            if i + stored_size > input.len() {
                return Some(IoError::Truncated);
            }
            let frame = &input[i..i + stored_size];
            i += stored_size;

            let code_bytes = if compressed {
                let decoded = match compress::decompress(frame) {
                    Some(bytes) => bytes,
                    None => return Some(IoError::ReadFailed),
                };
                if decoded.len() != uncompressed_size {
                    return Some(IoError::Truncated);
                }
                decoded
            } else {
                frame.to_vec()
            };
            let code_size = match u32::try_from(code_bytes.len()) {
                Ok(v) => v,
                Err(_) => return Some(IoError::Truncated),
            };

            let entry = CacheEntry {
                guest_addr,
                guest_size,
                code_size,
                code_bytes: code_bytes.into_boxed_slice(),
                hit_count: 0,
                last_used: 0,
            };
            entries.insert((guest_addr, content_hash), entry);
            addr_to_hash.insert(guest_addr, content_hash);
        }

        if i != input.len() {
            // Lenient: ignore trailing junk after parsed entries.
        }

        self.entries = entries;
        self.addr_to_hash = addr_to_hash;
        self.next_tick = 1;
        for (idx, entry) in self.entries.values_mut().enumerate() {
            entry.last_used = idx as u64;
        }
        None
    }

    fn maybe_evict(&mut self) {
        while (self.max_entries != 0 && self.entries.len() > self.max_entries)
            || (self.max_bytes != 0 && self.total_code_bytes() > self.max_bytes)
        {
            let stale_evicted = self.compact();
            if stale_evicted > 0
                && (self.max_entries == 0 || self.entries.len() <= self.max_entries)
                && (self.max_bytes == 0 || self.total_code_bytes() <= self.max_bytes)
            {
                break;
            }
            if let Some((lru, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, v)| (*k, v.last_used))
            {
                if let Some(entry) = self.entries.remove(&lru) {
                    if let Some(stored) = self.addr_to_hash.get(&entry.guest_addr).copied() {
                        if stored == lru.1 {
                            self.addr_to_hash.remove(&entry.guest_addr);
                        }
                    }
                }
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256;

    fn entry(bytes: &[u8], guest_size: usize, _hash: u64) -> CacheEntry {
        CacheEntry {
            guest_addr: 0,
            guest_size: guest_size as u32,
            code_size: bytes.len() as u32,
            code_bytes: bytes.to_vec().into_boxed_slice(),
            hit_count: 0,
            last_used: 0,
        }
    }

    #[test]
    fn fnv1a_is_stable() {
        let a = b"prisma";
        let b = b"prisma";
        let c = b"prismb";
        assert_eq!(fnv1a_64(a), fnv1a_64(b));
        assert_ne!(fnv1a_64(a), fnv1a_64(c));
        assert_eq!(fnv1a_64(&[]), 0xcbf29ce484222325);
    }

    #[test]
    fn cache_lookup_and_miss() {
        let mut c = TranslationCache::new();
        let guest = vec![0x48, 0xC3];
        let h = fnv1a_64(&guest);
        c.insert((0x1000, h), entry(&[1, 2, 3], guest.len(), h));
        assert!(matches!(c.lookup(0x1000, &guest), LookupResult::Hit(_)));
        assert!(matches!(
            c.lookup(0x2000, &guest),
            LookupResult::Miss(MissReason::UnknownAddress)
        ));
        let stale = vec![0x90, 0xC3];
        assert!(matches!(
            c.lookup(0x1000, &stale),
            LookupResult::Miss(MissReason::StaleContent)
        ));
    }

    #[test]
    fn cache_upsert_replaces() {
        let mut c = TranslationCache::new();
        let guest = vec![0xC3];
        let h1 = fnv1a_64(&guest);
        let guest2 = vec![0x90, 0xC3];
        let h2 = fnv1a_64(&guest2);
        c.upsert((0x2000, h1), entry(&[0xAA], guest.len(), h1));
        c.upsert((0x2000, h2), entry(&[0xBB], guest2.len(), h2));
        assert!(matches!(c.lookup(0x2000, &guest2), LookupResult::Hit(_)));
        assert!(matches!(
            c.lookup(0x2000, &guest),
            LookupResult::Miss(MissReason::StaleContent)
        ));
    }

    #[test]
    fn cache_saves_and_loads_roundtrip() {
        let mut c = TranslationCache::new();
        let guest = vec![0x90];
        let h = fnv1a_64(&guest);
        c.insert((0x1000, h), entry(&[0x11, 0x22], guest.len(), h));

        let path = std::env::temp_dir().join("prisma-cache-roundtrip.bin");
        assert!(c.save_to_file(&path).is_none());
        c.invalidate_page(0x1000, 1);
        assert_eq!(c.entry_count(), 0);
        assert!(c.load_from_file(&path).is_none());
        assert_eq!(c.entry_count(), 1);
        assert!(matches!(c.lookup(0x1000, &guest), LookupResult::Hit(_)));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_skips_superseded_insert_entries() {
        let mut cache = TranslationCache::new();
        let old_guest = [0x90];
        let new_guest = [0x90, 0xC3];
        let old_hash = fnv1a_64(&old_guest);
        let new_hash = fnv1a_64(&new_guest);
        assert!(cache.insert(
            (0x4000, old_hash),
            entry(&[0xAA], old_guest.len(), old_hash)
        ));
        assert!(cache.insert(
            (0x4000, new_hash),
            entry(&[0xBB], new_guest.len(), new_hash)
        ));
        assert_eq!(cache.entry_count(), 2);
        assert_eq!(cache.size(), 1);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.bin");
        assert!(cache.save_to_file(&path).is_none());

        let mut loaded = TranslationCache::new();
        assert!(loaded.load_from_file(&path).is_none());
        assert_eq!(loaded.entry_count(), 1);
        assert!(matches!(
            loaded.lookup(0x4000, &old_guest),
            LookupResult::Miss(MissReason::StaleContent)
        ));
        assert!(matches!(
            loaded.lookup(0x4000, &new_guest),
            LookupResult::Hit(_)
        ));
    }

    #[test]
    fn load_missing_file_reports_open_failed_and_keeps_entries() {
        let mut cache = TranslationCache::new();
        let guest = [0xC3];
        let hash = fnv1a_64(&guest);
        assert!(cache.insert((0x5000, hash), entry(&[0x11], guest.len(), hash)));

        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.bin");
        assert_eq!(cache.load_from_file(&missing), Some(IoError::OpenFailed));
        assert_eq!(cache.entry_count(), 1);
        assert!(matches!(cache.lookup(0x5000, &guest), LookupResult::Hit(_)));
    }

    #[test]
    fn limits_are_observable_and_enforced() {
        let mut c = TranslationCache::new();
        let h1 = fnv1a_64(&[0x90]);
        let h2 = fnv1a_64(&[0x91]);
        c.set_limits(1, 32);
        assert_eq!(c.limits(), (1, 32));

        assert!(c.insert((0x1000, h1), entry(&[0xAA], 1, h1)));
        assert!(c.insert((0x2000, h2), entry(&[0xBB], 1, h2)));

        assert_eq!(c.entry_count(), 1);
        assert!(matches!(c.lookup(0x1000, &[0x90]), LookupResult::Miss(_)));
        assert!(matches!(c.lookup(0x2000, &[0x91]), LookupResult::Hit(_)));
    }

    #[test]
    fn set_limits_reclaim_oldest_lru_first() {
        let mut c = TranslationCache::new();
        let h1 = fnv1a_64(&[0x90]);
        let h2 = fnv1a_64(&[0x91]);
        let h3 = fnv1a_64(&[0x92]);

        c.set_limits(0, 0);
        assert!(c.insert((0x1000, h1), entry(&[0x11], 1, h1)));
        assert!(c.insert((0x1100, h2), entry(&[0x22], 1, h2)));
        assert!(c.insert((0x1200, h3), entry(&[0x33], 1, h3)));
        assert_eq!(c.entry_count(), 3);

        c.set_limits(1, 3);
        assert_eq!(c.entry_count(), 1);
        assert!(matches!(c.lookup(0x1000, &[0x90]), LookupResult::Miss(_)));
        assert!(matches!(c.lookup(0x1100, &[0x91]), LookupResult::Miss(_)));
        assert!(matches!(c.lookup(0x1200, &[0x92]), LookupResult::Hit(_)));
    }

    #[test]
    fn max_bytes_eviction_uses_lru() {
        let mut c = TranslationCache::new();
        c.set_max_bytes(5);
        let h1 = fnv1a_64(&[0x90]);
        let h2 = fnv1a_64(&[0x91]);
        let h3 = fnv1a_64(&[0x92]);

        assert!(c.insert((0x1000, h1), entry(&[1, 2], 2, h1)));
        assert!(c.insert((0x2000, h2), entry(&[3, 4], 2, h2)));
        assert!(c.insert((0x3000, h3), entry(&[5, 6], 2, h3)));

        assert_eq!(c.entry_count(), 2);
        assert!(matches!(c.lookup(0x1000, &[0x90]), LookupResult::Miss(_)));
        assert!(matches!(c.lookup(0x3000, &[0x92]), LookupResult::Hit(_)));
    }

    #[test]
    fn compact_removes_stale_entries_only() {
        let mut c = TranslationCache::new();
        let addr = 0x5000u64;
        let old = b"\x90";
        let new = b"\x90\x90";
        let old_hash = fnv1a_64(old);
        let new_hash = fnv1a_64(new);

        assert!(c.insert((addr, old_hash), entry(&[0xAA], old.len(), old_hash)));
        assert!(c.insert((addr, new_hash), entry(&[0xBB, 0xCC], new.len(), new_hash)));

        assert_eq!(c.stale_count(), 1);
        assert_eq!(c.compact(), 1);
        assert_eq!(c.stale_count(), 0);
        assert_eq!(c.entry_count(), 1);
        assert!(matches!(
            c.lookup(addr, old),
            LookupResult::Miss(MissReason::StaleContent)
        ));
        assert!(matches!(c.lookup(addr, new), LookupResult::Hit(_)));
    }

    #[test]
    fn touch_tracks_stats_and_returns_status() {
        let mut c = TranslationCache::new();
        let hash = fnv1a_64(&[0x90]);
        let key = (0x7000, hash);
        assert!(c.insert(key, entry(&[0x11], 1, hash)));

        assert!(c.touch(&key));
        let first = c.stats_for(&key).unwrap();
        assert_eq!(first.hit_count, 1);
        assert!(c.touch(&key));
        let second = c.stats_for(&key).unwrap();
        assert_eq!(second.hit_count, 2);
        assert!(second.last_used_tick > first.last_used_tick);
        assert!(!c.touch(&(0x7001, hash)));
    }

    #[test]
    fn async_save_and_wait_reports_errors_and_success() {
        let mut c = TranslationCache::new();
        let hash = fnv1a_64(&[0xC3]);
        assert!(c.insert((0x8000, hash), entry(&[0x11], 1, hash)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async-save.bin");
        c.save_to_file_async(path.clone());
        assert!(c.wait_for_async_save().is_none());
        assert!(path.exists());

        let mut reloaded = TranslationCache::new();
        assert!(reloaded.load_from_file(&path).is_none());
        assert_eq!(reloaded.entry_count(), 1);
    }

    #[test]
    fn cache_invalidate_erases_key_and_address_map() {
        let mut c = TranslationCache::new();
        let hash = fnv1a_64(&[0x90]);
        let key = (0x9000, hash);
        assert!(c.insert(key, entry(&[0x11, 0x22], 2, hash)));
        assert_eq!(c.size(), 1);
        assert!(c.contains_key(&key));
        c.invalidate(&key);
        assert_eq!(c.size(), 0);
        assert!(!c.contains_key(&key));
    }

    #[test]
    fn clear_removes_all_state_and_resets_counters() {
        let mut c = TranslationCache::new();
        let h1 = fnv1a_64(&[0x90]);
        let h2 = fnv1a_64(&[0x91]);
        c.set_limits(2, 64);
        c.insert((0xA000, h1), entry(&[0xAA], 1, h1));
        c.insert((0xA100, h2), entry(&[0xBB], 1, h2));
        c.touch(&(0xA000, h1));
        assert!(c.contains_key(&(0xA000, h1)));
        assert_eq!(c.entry_count(), 2);

        c.clear();
        assert_eq!(c.entry_count(), 0);
        assert_eq!(c.size(), 0);
        assert!(!c.contains_key(&(0xA000, h1)));
        assert!(!c.contains_key(&(0xA100, h2)));
    }

    #[test]
    fn unlimited_limits_allow_growth_when_zeroed() {
        let mut c = TranslationCache::new();
        c.set_limits(0, 0);
        assert_eq!(c.limits(), (0, 0));
        let base = 0xB000u64;
        for i in 0u8..16 {
            let hash = fnv1a_64(&[i]);
            assert!(c.insert((base + u64::from(i), hash), entry(&[0xCC; 64], 64, hash)));
        }
        assert_eq!(c.entry_count(), 16);
    }

    #[test]
    fn sha256_hash_type_alias() {
        let digest = sha256::sha256(b"prisma");
        let hex = sha256::to_hex(digest);
        assert_eq!(hex.len(), std::mem::size_of::<sha256::Sha256Hash>() * 2);
        let parsed = sha256::from_hex(&hex).unwrap();
        assert_eq!(digest, parsed);
    }
}
