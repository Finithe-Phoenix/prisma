# HANDOFF: prisma-cache (Fase 1)

> Para CODEX. Cómo implementar el translation cache en Rust.

## Dependencias

- `prisma-ir` (para serializar CacheEntry con serde)
- `sha2` crate (ya en workspace)
- `zstd` crate (ya en workspace)
- `lru` crate (añadir como dependencia)

## Archivos C++ a migrar

| C++ | Rust | Estado |
|-----|------|--------|
| `core/src/cache/translation_cache.cpp` (381 lines) | `shell/prisma-cache/src/cache.rs` | 🚧 Pendiente |
| `core/src/cache/sha256.cpp` (114 lines) | `shell/prisma-cache/src/sha256.rs` | 🚧 Pendiente |
| `core/src/cache/compress.cpp` (37 lines) | `shell/prisma-cache/src/compress.rs` | 🚧 Pendiente |
| `core/tests/test_translation_cache.cpp` (662 lines) | `shell/prisma-cache/tests/` | 🚧 Pendiente |

## API que debe exponer

```rust
pub struct TranslationCache {
    // LRU cache: (guest_addr, content_hash) → CacheEntry
    capacity: usize,           // byte budget
    max_entries: usize,
    entries: LruCache<CacheKey, CacheEntry>,
    stats: CacheStats,
}

pub struct CacheEntry {
    pub guest_addr: u64,
    pub guest_size: u32,
    pub code_size: u32,
    pub code_bytes: Box<[u8]>,
    pub hit_count: u64,
    pub last_used: u64,
}

pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub bytes_used: usize,
}

impl TranslationCache {
    pub fn new(capacity_bytes: usize) -> Self;
    pub fn lookup(&mut self, addr: u64, hash: u64) -> Option<&CacheEntry>;
    pub fn store(&mut self, addr: u64, hash: u64, entry: CacheEntry) -> bool;
    pub fn invalidate(&mut self, addr: u64);
    pub fn stats(&self) -> &CacheStats;
    pub fn save_to_file(&self, path: &Path) -> Result<()>;
    pub fn load_from_file(path: &Path) -> Result<Self>;
    pub fn save_to_file_async(&self, path: &Path) -> impl Future<Output = Result<()>>;
}
```

## C API bridge

```c
// prisma_cache_c.h
typedef struct prisma_cache_t prisma_cache_t;
prisma_status prisma_cache_create(size_t capacity_bytes, prisma_cache_t** out);
void prisma_cache_destroy(prisma_cache_t* cache);
prisma_status prisma_cache_lookup(prisma_cache_t* cache, uint64_t addr, uint64_t hash,
                                   const uint8_t** out_bytes, size_t* out_len);
prisma_status prisma_cache_store(prisma_cache_t* cache, uint64_t addr, uint64_t hash,
                                   const uint8_t* bytes, size_t len);
void prisma_cache_invalidate(prisma_cache_t* cache, uint64_t addr);
```

## Formato binario (RFC 0007)

```
┌──────────────────────────────┐
│ magic: [0x50, 0x52, 0x43, 0x43]  ("PRCC")
│ version: u32 LE              │
│ entry_count: u32 LE          │
│ ┌──────────────────────────┐ │
│ │ entries[]                │ │
│ │   guest_addr: u64 LE     │ │
│ │   guest_size: u32 LE     │ │
│ │   code_size: u32 LE      │ │
│ │   code_bytes: [u8; len]  │ │
│ │   hit_count: u64 LE      │ │
│ │   last_used: u64 LE      │ │
│ └──────────────────────────┘ │
│ sha256_hash: [u8; 32]       │
│ zstd_compressed: bool       │
└──────────────────────────────┘
```

## Tests (orden sugerido)

1. `new()` crea cache vacío con stats en 0
2. `store()` + `lookup()` round-trip
3. `lookup()` miss devuelve None
4. `invalidate()` elimina entrada
5. LRU eviction cuando se excede capacity
6. Save/load round-trip sin compresión
7. Save/load round-trip con zstd
8. SHA-256 trust envelope verify
9. `save_to_file_async` no bloquea
10. Concurrent reads (Arc<RwLock<...>>)
11. **Differential:** C++ y Rust producen mismo binario save

## Checklist

- [ ] `lru` crate añadido a Cargo.toml
- [ ] `sha2` usado correctamente (SHA-256, no SHA-1)
- [ ] `zstd::stream::Encoder/Decoder` para compresión
- [ ] `#[repr(C)]` en structs que cruzan FFI
- [ ] Panic catch_unwind en cada entry point C API
- [ ] `#![deny(unsafe_op_in_unsafe_fn)]`
- [ ] `cargo clippy -- -D warnings` pasa
- [ ] Tests differentiales existen contra `test_translation_cache.cpp`
- [ ] `docs/DIFFERENTIAL_TESTING.md` seguido
