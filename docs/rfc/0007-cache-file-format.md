---
id: 0007
title: Translation cache on-disk format — version 1, pre-P2P
status: accepted
authors: [Danny]
created: 2026-04-20
updated: 2026-04-20
supersedes: []
superseded_by: null
---

# RFC 0007: Translation cache on-disk format

## Summary

Define a single-process, little-endian binary format for persisting
the in-memory `TranslationCache` across runs. Format version is
encoded in the header so Fase 2.5's P2P cache-sharing protocol can
extend the file without breaking older agents. Content hashing is
FNV-1a today; SHA-256 is reserved for the trust protocol.

## Motivation

Translating x86 blocks is expensive; reusing a translation across
runs skips decode + passes + lower + emit. A local persistent cache
captures this win on a single machine and is also the substrate for
Fase 2.5's distributed cache (Pillar 4). Either way we need a stable
file format that:

1. **Survives re-running the translator** without breakage.
2. **Rejects caches built for a different host** (different CPU
   features, different guest ABI, …).
3. **Is forward-compatible.** A Fase 2.5 agent producing signed
   chunks must still be readable by a plain Fase 1 agent — or at
   least fail cleanly, not corrupt the runtime.
4. **Writes and reads byte-by-byte identically on every platform.**
   Little-endian integers, no implicit padding, no std::hash-derived
   layout that differs between libstdc++ and libc++.

Earlier iterations shipped a raw `std::unordered_map` serialisation
via boost::serialization-style macros. That locked us into a
specific STL implementation and broke as soon as we tried to load a
macOS cache on Linux.

## Context

- **Two maps in the live cache.** `entries_` keyed by `(guest_addr,
  content_hash)`, plus `addr_to_hash_` for staleness detection. The
  on-disk format serialises only `entries_`; `addr_to_hash_` is
  rebuilt at load.
- **Runtime-only state.** LRU access ticks, per-entry hit counts,
  in-flight async-save thread — none of these cross `save_to_file`.
  They reset on load.
- **Host fingerprint slot reserved.** The header carries an 8-byte
  `cpu_fingerprint` field that is `0` today but Fase 2.5 will fill
  with a hash of `HostFeatures` (FEAT_LSE / LSE2 / LRCPC / FlagM /
  …). Mismatching fingerprints on load will reject the cache.
- **FNV-1a vs SHA-256.** FNV-1a is ~20× faster and collision-
  resistant enough within a single process. SHA-256 exists for
  cross-machine trust (RFC to be written for the signing protocol)
  and is emitted separately in the P2P chunk envelope, not in the
  Fase 1 file format.

## Considered alternatives

### 1. Protocol Buffers / FlatBuffers
- **+** Battle-tested, forward-compat story via field numbers.
- **−** Pulls in a large build-time / runtime dependency for a trivial
  binary layout.
- **−** Schema evolution rules (optional fields, unknown-field
  handling) are overkill for a single-process cache.
- **Verdict:** rejected. Return to this once P2P lands and we need
  cross-language codecs.

### 2. JSON / MsgPack
- **+** Human-readable (JSON) or compact-and-typed (MsgPack).
- **−** JSON forces escaping of the 16+ byte binary code blobs.
- **−** Neither format gives us atomic mmap access if we ever want
  zero-copy loads.
- **Verdict:** rejected.

### 3. Raw byte struct with `#pragma pack` + memcpy
- **+** Ultra simple.
- **−** Pack semantics differ between Clang/MSVC/GCC; 64-bit
  alignment surprises on ARM.
- **−** Breaks on host-endian mismatch (we only run on little-endian
  hosts today, but Lean-verified portability is on the roadmap).
- **Verdict:** rejected in favour of explicit little-endian
  marshalling helpers.

### 4. Explicit little-endian writer (accepted)
Emit each integer through a helper that writes byte-by-byte in a
fixed byte order. Compact, deterministic, no external dependency,
no pack-ordering surprises.

## Decision

### On-disk layout (version 1)

All integers are little-endian. Offsets are from the start of the
file. The Fase 1 writer always emits exactly this layout; the reader
validates the header before touching any entry bytes.

```
Header (32 bytes)
  0x00  u64  magic               = 0x4843'4143'4D53'5250
                                    ("PRSMCACH" little-endian)
  0x08  u32  version             = 1
  0x0C  u32  reserved            = 0
  0x10  u64  cpu_fingerprint     = 0 (reserved — Fase 2.5 fills)
  0x18  u64  entry_count

For each entry (variable length):
  u64  guest_addr
  u64  content_hash        (FNV-1a 64 of the guest bytes)
  u64  guest_size          (length of the translated guest region)
  u64  code_size
  u8   code_bytes[code_size]
```

### Version policy

- **Adding fields to the entry:** bump `version`; a reader that
  understands only version N must reject N+1 with
  `IoError::UnsupportedVersion`. Never silently drop unknown fields
  in a single-process cache — divergence between reader and writer
  views is a correctness hazard.
- **Adding fields to the header:** extend the reserved u32 or bump
  the header size, always bumping `version`.
- **Reducing fields:** new major version (version 2+), at which point
  the old layout is simply "version 1 — deprecated". We keep the
  reader code around for one minor release so users can re-save.

### Error handling

`load_from_file` returns `IoError` values without mutating state:

- `OpenFailed` — file couldn't be opened.
- `BadMagic` — first 8 bytes don't match `PRSMCACH`.
- `UnsupportedVersion` — `version > 1` (or `< 1`).
- `Truncated` — `entry_count` claims more data than the file holds.
- `ReadFailed` — any short read mid-file (I/O error).

The live cache is left unchanged on any error. This is verified by
`test_translation_cache.cpp`'s "load leaves cache unchanged on
error" case.

### Save concurrency

`save_to_file` is synchronous and holds the cache for the duration.
`save_to_file_async` (F1-CA-009) takes a deep-copy snapshot on the
caller thread and writes from a worker, so the live cache is free to
mutate immediately after the call returns. The on-disk byte layout
is identical either way.

### Forward-compat path for Fase 2.5

The P2P protocol will wrap this file format in a signed envelope:

```
┌──────────────────┐
│ envelope header  │  sender pubkey, signature over body
├──────────────────┤
│ body = this RFC  │  the cache file exactly as v1 specifies
└──────────────────┘
```

This means the Fase 1 on-disk layout is the canonical body of P2P
chunks — agents can extract and replay a chunk as a local cache
with zero format translation. Signed envelope vs. plain file is a
separate RFC (cache-trust protocol, to be written when the P2P
stack is claimed).

## Consequences

### Benefits

- No dependency churn. The codec is ~100 LOC of little-endian
  helpers, tested against round-trip invariants.
- Backward-reading caches across OS upgrades and standard-library
  changes stays trivial as long as the reader validates the header.
- P2P protocol gets the file format for free; no second codec to
  maintain.

### Costs

- Every format extension costs a version bump. Low operational cost
  today but grows if we iterate rapidly.
- The `cpu_fingerprint` slot is present but unvalidated. Fase 2.5
  must implement the check before we ship cache sharing to real
  users; shipping without it would let a cache built for one CPU
  silently execute wrong code on another.

### Reversibility

File format changes are easy to revert per-version but once a cache
file with version N is written to disk by a user, we own reading
it. In practice we ship a reader that supports N, N-1, and N-2, and
recommend re-saving to get onto the latest version. This mirrors
what mature cache-based systems (Chromium SQLite caches, sqlite
itself) already do.

## Implementation notes

Implementation lives in:

- `core/include/prisma/translation_cache.hpp` — declares the
  `kFileMagic`, `kFileVersion`, `IoError`, `save_to_file`,
  `load_from_file` APIs.
- `core/src/cache/translation_cache.cpp` — binary marshalling via
  anonymous-namespace `write_u32/write_u64/read_u32/read_u64`.
- `core/tests/test_translation_cache.cpp` — round-trip, bad-magic,
  unchanged-on-error, byte-budget + LRU under churn, async-save
  isolation.

Commits of note:

- `ff...` — initial synchronous format (pre-RFC, under F1-CA-003).
- `691d669` — per-entry stats (hit count, last_used tick). Runtime
  only, not on disk.
- `0d4d17a` — SHA-256 (F1-CA-004). Reserved for the future P2P
  trust envelope; not used by the file format yet.
- `58e47a9` — async save (F1-CA-009), same byte layout as the sync
  path via a templated `write_range_to_file` helper.

## Open questions

- **When to bump to version 2.** First triggers will be
  `cpu_fingerprint` filling (Fase 2.5 opens P2P) and per-entry
  stats that outlive a save (e.g. cumulative hit counts that should
  survive a restart so telemetry is accurate).
- **zstd compression (F1-CA-010).** Compression would touch the body
  (entry bytes) but not the header. Likely approach: a new field in
  the header ("compressed" bit) + a per-entry compressed flag for
  entries large enough to benefit. Deferred until we have a real
  benchmark driving the decision.
- **Cache compaction / merging adjacent blocks (F1-CA-008).** This is
  a serialization-time optimisation; semantics of "adjacent" depend
  on CFG-aware translation, which isn't here yet.

## References

- `core/include/prisma/translation_cache.hpp` — canonical format
  comment matches this RFC byte for byte.
- RFC 0001 — IR SSA form, which determines what a translation's
  body content even is.
- FNV-1a 64-bit spec — <http://www.isthe.com/chongo/tech/comp/fnv/>.
- FIPS 180-4 — SHA-256 spec, used by the reserved `cpu_fingerprint`
  design (SHA-256 of host features will populate that slot).
