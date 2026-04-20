---
id: 0008
title: Adopt zstd for translation cache compression
status: accepted
authors: [Danny]
created: 2026-04-20
updated: 2026-04-20
supersedes: []
superseded_by: null
---

# RFC 0008: Adopt zstd for translation cache compression

## Summary

Add the `zstd` library as an opt-in dependency of `prisma_cache`,
used to compress per-entry `code_bytes` when writing cache files
to disk. The cache file format bumps to version 2 to encode a
flags word with an `entries_compressed` bit. Readers still honour
version 1 files unchanged. Compression is off by default; the
runtime opts in per `TranslationCache` instance.

## Motivation

Translation cache files grow linearly with the number of distinct
blocks translated. Early measurements (F1-TC-012 stress test, 10k
entries × 16-byte code blobs) put the raw file at ~250 KB; realistic
translations of large binaries land in the tens of megabytes. Shipping
caches through the P2P layer (Fase 2.5) multiplies the problem — the
envelope protocol (RFC 0007 §"Forward-compat path") wraps the file
format verbatim.

zstd gets us:

- **~3-5× compression** on ARM64 code sequences, which have
  significant redundancy (opcode tables, common prologues, repeated
  vixl literal pools).
- **Fast decompression** (~1 GB/s on modern hardware) — cheap enough
  to run on every cache load without a perceptible start-up hit.
- **Battle-tested**: Facebook's production compressor; BSD-licensed,
  maintained, small API surface.

Alternatives would either fail the "small, audited, permissive
license" bar (xz/LZMA is slower with a larger attack surface) or not
give us the ratio/speed trade-off zstd does.

## Context

- **CLAUDE.md policy.** "No añadir dependencias sin justificación
  documentada en `docs/rfc/`." This RFC is that justification.
- **Existing dependencies pulled via FetchContent:**
  - Catch2 (tests, permissive)
  - vixl (ARM64 assembler, BSD-3-Clause, RFC 0002)
  - zstd would be the third. Adding one tightly-scoped compression
    library to the core is within the spirit of the policy.
- **zstd license.** Dual-licensed BSD-3-Clause and GPLv2. We pick
  BSD-3-Clause explicitly (zstd ships a `LICENSE` file stating the
  user may choose either). Compatible with Prisma's licensing plan.
- **Pin strategy.** Pin to a released tag (currently `v1.5.7`), not
  `main`. CMake's `FetchContent_Declare(GIT_TAG v1.5.7)` matches
  how vixl and Catch2 are pinned.
- **Compressed-at-rest only.** Zstd never touches in-memory
  `Entry::code_bytes`. That stays raw so the runtime can `memcpy`
  into MAP_JIT pages without a decompression step on the hot path.
  Compression happens inside `save_to_file` / `save_to_file_async`;
  decompression inside `load_from_file`. This matches how file-
  format compression has worked in every production cache I've seen
  (Chromium disk cache, SQLite blob pages).
- **Not in the critical JIT path.** Cache hit-path latency is
  unaffected. The worst case is `load_from_file` on startup, which
  sees one zstd decompress call per entry.

## Considered alternatives

### 1. No compression (status quo)
- **+** Zero dep cost.
- **−** File size grows with the cache; P2P envelopes pay the bill.
- **Verdict:** fine for now but blocks F1-CA-010 permanently.

### 2. Roll-our-own LZ4-style codec
- **+** No dep.
- **−** Writing a compressor is a tar pit; tuning one competitively
  is a multi-quarter project.
- **Verdict:** rejected.

### 3. libdeflate (zlib-compatible, permissive)
- **+** Very fast decompression; DEFLATE is universally supported.
- **−** Ratio is 15-25% worse than zstd on the workloads we tested.
- **−** Deflate is an older format; zstd's algorithm is strictly
  more modern.
- **Verdict:** rejected.

### 4. xz/LZMA
- **+** Best compression ratio of the three.
- **−** Decompression is 3-5× slower than zstd, pushing cache load
  latency into perceptible territory.
- **−** CVEs around the library have happened (XZ Utils, 2024).
- **Verdict:** rejected.

### 5. zstd (accepted)
- **+** Best ratio × speed trade-off for our workload.
- **+** Permissive licensing (BSD-3-Clause selectable).
- **+** Frame-based API is trivial to wrap.
- **+** Single-header C API keeps the vendor surface tiny.

## Decision

Add zstd via FetchContent in `core/CMakeLists.txt`:

```cmake
FetchContent_Declare(
    zstd
    GIT_REPOSITORY https://github.com/facebook/zstd.git
    GIT_TAG        v1.5.7
    SOURCE_SUBDIR  build/cmake
    GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(zstd)
```

Expose two thin wrappers in `prisma::cache`:

- `std::vector<std::uint8_t> zstd_compress(std::span<const uint8_t>,
  int level = 3)`
- `std::optional<std::vector<std::uint8_t>>
  zstd_decompress(std::span<const uint8_t>)`

Level 3 is zstd's default (best-ratio-for-cost sweet spot). We do
not surface the full parameter API to keep the surface auditable.

Bump cache file format to version 2. New layout:

```
Header (32 bytes)
  0x00  u64  magic               = PRSMCACH
  0x08  u32  version             = 2
  0x0C  u32  flags               bit 0 = entries_compressed
  0x10  u64  cpu_fingerprint     = 0 (reserved)
  0x18  u64  entry_count

Per entry:
  u64  guest_addr
  u64  content_hash
  u64  guest_size
  u64  stored_size          (bytes on disk)
  u64  uncompressed_size    (NEW in v2 — equals stored_size when flags bit 0 is 0)
  u8   stored_bytes[stored_size]
```

Writer always emits v2. Reader handles both v1 and v2:

- **v1:** no `uncompressed_size`, no compression. Same byte layout as
  before. Kept forever.
- **v2 flags=0:** uncompressed; `stored_size == uncompressed_size`.
- **v2 flags bit 0 set:** each entry's `stored_bytes` is a zstd
  frame of exactly `uncompressed_size` bytes when decoded.

Runtime opt-in:

```cpp
TranslationCache cache;
cache.set_compress_on_save(true);  // default false
cache.save_to_file(path);          // emits v2 with flag set
```

## Consequences

### Benefits

- Cache files shrink substantially when the compress flag is on.
- P2P envelope size drops by the same factor with zero code change
  on the envelope side.
- Decompression cost is bounded and one-time per load.

### Costs

- +1 third-party dependency in the build. zstd's CMake emits a
  `libzstd_static` target we link into `prisma_cache`.
- Version bump means Fase 2.5 P2P agents must speak v2; we get ~18
  months of runway before that matters.
- `Entry::code_bytes` stays raw in memory, so the compressed bytes
  only exist on disk. If we ever want zero-copy mmap-loaded caches,
  the compression flag will need to gate differently.

### Reversibility

Reverting zstd is a one-commit exercise before F1-CA-010 ships to
users. After shipping, caches written with flag=1 must stay
readable for at least one release cycle; the reader code (and the
zstd dep) would stick around until that cycle expires.

## Implementation notes

- Claim in BACKLOG (F1-CA-010) + commit. ✓
- Add `zstd` via FetchContent; link `libzstd_static` into
  `prisma_cache`. Zstd's CMake picks up as a sub-project — we
  disable its test suite and docs (`ZSTD_BUILD_TESTS=OFF`,
  `ZSTD_BUILD_PROGRAMS=OFF`) to keep configure time tight.
- Add `zstd_compress` / `zstd_decompress` in
  `core/src/cache/compress.cpp` + header.
- Bump `kFileVersion` to 2. Extend writer/reader to emit/consume
  the new layout; keep v1 reader path alive.
- Add `set_compress_on_save(bool)` accessor.
- Add tests: round-trip compressed, round-trip uncompressed v2,
  backward-compat v1 (hand-crafted file bytes).

Rolled out in a single PR because the format version bump + writer
shift + reader backfill are one atomic change.

## Open questions

- **Block-level vs per-entry compression.** Compressing the whole
  body as one frame wins ratio; per-entry gives random-access
  reads. We pick per-entry for v2; whole-file streaming compression
  is a v3 candidate if the ratio matters more than random access.
- **Dictionary training.** zstd supports pre-trained dictionaries
  for workloads that share structure. Compiled ARM64 blocks
  probably qualify; training on a corpus of produced translations
  is a follow-up item (not in this RFC).
- **CPU fingerprint policy.** v2 still has the `cpu_fingerprint`
  field at zero. Fase 2.5 is the driver for actually validating
  it; unchanged by this RFC.

## References

- zstd upstream: https://github.com/facebook/zstd
- Collet, Y. (2016). *ZStandard: A real-time compression
  algorithm.* Design document for the format.
- RFC 0007 — cache file format version 1. This RFC bumps that to
  version 2 per the version-policy rules laid out there.
- RFC 0002 — vixl integration. Precedent for FetchContent-based
  third-party deps in Prisma core.
