---
id: 0009
title: IR binary serialization format — version 1
status: accepted
authors: [Claude]
created: 2026-05-03
updated: 2026-05-03
supersedes: []
superseded_by: null
---

# RFC 0009: IR binary serialization format — version 1

## Summary

A compact, self-contained, little-endian binary encoding of
`prisma::ir::Stmt`, `prisma::ir::BasicBlock`, and `prisma::ir::Function`.
Magic `'PIRB'`, version `0x0001`, CRC32C trailer. Tag-length-value
layout where every `Op` variant gets a stable `OpKind` byte and a
fixed-shape payload. Implementation: `core/src/ir/serialize.cpp`,
public surface: `core/include/prisma/ir_serialize.hpp`.

## Motivation

The persistent translation cache (RFC 0007) currently caches generated
machine code keyed by `(guest_addr, content_hash)`. We expect to also
cache *post-passes IR* so we can re-lower without re-decoding when a
host-CPU feature flag changes (FEAT_LSE / LSE2 / LRCPC / FlagM /
DotProd toggling between runs flips lowering decisions but leaves IR
intact). That requires a stable on-disk IR encoding.

A second motivation is reproducibility for the Lean spec workflow: a
serialized IR snapshot is something we can pin in a regression test
and decode deterministically across compiler / OS combinations.

## Context

- `prisma::ir::Op` is a `std::variant` of 24 alternatives today
  (`core/include/prisma/ir.hpp`). The set has grown as decoder coverage
  expanded; future additions are expected.
- The cache layer already uses an explicit little-endian byte writer
  (`core/src/cache/translation_cache.cpp`) for deterministic on-disk
  bytes. The same convention is reused here.
- We deliberately do not pull in protobuf / flatbuffers / cap'n'proto —
  the codec is ~50 LOC of `put_u8/u16/u32/u64` plus per-variant
  payload writers, and external dependencies for binary marshalling
  trip RFC 0001's "minimalistic surface" principle.
- CRC32C (Castagnoli, polynomial `0x1EDC6F41`) is chosen for the
  trailer. CRC32C is what the SHA-256 trust envelope path
  (Fase 2.5, P2P cache sharing) will eventually pair with for fast
  per-chunk integrity; using it here keeps a single CRC implementation
  in the tree.

## Considered alternatives

### 1. Stringified pretty-print

Reuse `pretty_print(Function)` and re-parse on load.
- **+** Already implemented.
- **−** Pretty-print is for humans; whitespace and field order are not
  contractual. A change to debug output would silently break cached IR
  on disk.
- **Verdict:** rejected.

### 2. JSON / MsgPack

- **+** Schema-friendly, easy to inspect.
- **−** Quote-escaping for binary fields (Refs, opcodes) bloats the
  stream. Pulls in an extra dep.
- **Verdict:** rejected; same arguments as RFC 0007.

### 3. Protocol Buffers / FlatBuffers

- **+** Forward-compat via tagged fields.
- **−** Heavy build dependency. Schema language is overkill for 24
  variants with fixed shapes.
- **Verdict:** rejected for now. Revisit when P2P needs cross-language
  decoders.

### 4. Tag-length-value with stable OpKind byte (accepted)

- Each `Stmt` is `(has_result, [result], op_kind, payload)`.
- Each `Op` variant has a permanent `OpKind` byte tag (1-based).
- Payload is fixed-shape per OpKind, no padding, all little-endian.
- Single CRC32C over the full stream as a corruption guard.

This is the simplest scheme that survives compiler / STL upgrades and
keeps the on-disk bytes a pure function of the IR contents.

## Decision

### Stream layout

```
IRStream :=
  Magic[4]         = 'PIRB'  (0x42 0x52 0x49 0x50 LE → u32 0x42524950)
  Version[2]       = 0x0001 little-endian
  Reserved[2]      = 0x0000 (must be written 0; readers must ignore)
  Body             // variant: stmt-stream OR function-stream
  Crc32C[4]        // little-endian, CRC32C over Magic..end-of-Body

Body (stmt stream, from `serialize(const std::vector<Stmt>&)`):
  StmtCount[u32]
  Stmt*                       // exactly StmtCount entries

Body (function stream, from `serialize(const Function&)`):
  Entry[u32]                  // entry block id
  BlockCount[u32]
  Block*                      // exactly BlockCount entries

Block :=
  Id[u32]
  StmtCount[u32]
  Stmt*

Stmt :=
  HasResult[u8]               // 0 or 1; any other value → BadSize
  if HasResult == 1: Result[u32]
  OpKind[u8]                  // see table below; 0 reserved
  Payload                     // op-specific, fixed shape
```

The two body shapes are distinguished by the **API entry point**
chosen by the caller: `serialize(stmts)` ↔ `deserialize_stmts`,
`serialize(fn)` ↔ `deserialize_function`. There is no in-band marker;
mixing them is a programmer error and out of scope for this RFC.

### OpKind tag table

OpKind tag values are **1-based, dense, and PERMANENT**. Once an
OpKind number is assigned, it is never reused. To retire a variant we
keep its tag and reject the stream as `UnknownOpKind` only at the
reader's discretion (e.g. the variant is removed from `prisma::ir::Op`
in source but the tag stays reserved). To add a variant we append a
new tag at the end and bump the reader's `kMaxOpKind`. **Reserved
zero** means "invalid"; a stream containing OpKind 0 is rejected with
`UnknownOpKind`.

| Tag | Op variant      | Payload                                                   | Bytes |
|----:|-----------------|-----------------------------------------------------------|------:|
|  0  | *(reserved)*    | —                                                         |   —   |
|  1  | `Constant`      | `u64 value`, `u8 size`                                    |  9    |
|  2  | `LoadReg`       | `u8 reg`, `u8 size`                                       |  2    |
|  3  | `StoreReg`      | `u8 reg`, `u32 value_ref`, `u8 size`                      |  6    |
|  4  | `LoadSegBase`   | `u8 seg`                                                  |  1    |
|  5  | `BinOp`         | `u8 op`, `u32 lhs`, `u32 rhs`, `u8 size`                  | 10    |
|  6  | `Compare`       | `u8 cc`, `u32 lhs`, `u32 rhs`, `u8 size`                  | 10    |
|  7  | `Select`        | `u8 cc`, `u32 true`, `u32 false`, `u8 size`               | 10    |
|  8  | `LoadMem`       | `u32 addr`, `u8 size`                                     |  5    |
|  9  | `StoreMem`      | `u32 addr`, `u32 value`, `u8 size`                        |  9    |
| 10  | `LoadMemTSO`    | `u32 addr`, `u8 size`                                     |  5    |
| 11  | `StoreMemTSO`   | `u32 addr`, `u32 value`, `u8 size`                        |  9    |
| 12  | `Jump`          | `u32 target_block`                                        |  4    |
| 13  | `CondJump`      | `u32 cond`, `u32 if_true`, `u32 if_false`                 | 12    |
| 14  | `Return`        | (empty)                                                   |  0    |
| 15  | `JumpReg`       | `u32 target`                                              |  4    |
| 16  | `CmpFlags`      | `u32 lhs`, `u32 rhs`, `u8 size`                           |  9    |
| 17  | `JumpRel`       | `u64 target_guest_pc`                                     |  8    |
| 18  | `CondJumpRel`   | `u8 cc`, `u64 target`, `u64 fallthrough`                  | 17    |
| 19  | `CallRel`       | `u64 target_guest_pc`, `u64 return_guest_pc`              | 16    |
| 20  | `CallReg`       | `u32 target_ref`, `u64 return_guest_pc`                   | 12    |
| 21  | `RetAdjusted`   | `u64 pop_bytes`                                           |  8    |
| 22  | `Cpuid`         | (empty)                                                   |  0    |
| 23  | `Syscall`       | (empty)                                                   |  0    |
| 24  | `Trap`          | `u8 kind`                                                 |  1    |

`size` payloads encode `OpSize` (`I8=0, I16=1, I32=2, I64=3`); any
value > 3 yields `BadSize`. `reg` payloads are `0..15` for
`Gpr::Rax..Gpr::R15`; out-of-range values yield `BadSize`. `seg`
covers `Es..Gs` (`0..5`). `cc` covers the full `CondCode` enum
(`0..15`). `kind` for `Trap` is `Sigtrap=0, Sigill=1, Sigfpe=2`.

### Version policy

- The format version is `kSerializeVersion = 0x0001` today. Any reader
  must reject a stream whose version is greater than the version it
  was compiled against, with `BadVersion`.
- **Adding a new OpKind**: append a new tag at the end of the table,
  bump `kSerializeVersion` is **not** required — older readers will
  reject the new tag with `UnknownOpKind` cleanly, which is the
  correct behaviour (they can't decode it).
- **Changing an existing payload shape**: bump `kSerializeVersion`.
  This is a one-way door. Old streams must still be readable by a
  bumped reader for at least one minor release.
- **Removing a variant** (variant deleted from `prisma::ir::Op` in
  source): keep the tag reserved forever. Decoder may reject with
  `UnknownOpKind`.

### Error taxonomy

`DeserializeError` values:

- `Ok`            — success.
- `BadMagic`      — first four bytes are not `'PIRB'`.
- `BadVersion`    — version field exceeds reader's `kSerializeVersion`.
- `Truncated`     — stream ends mid-stmt or stmt-count claims more
  data than available (also returned for any stream shorter than the
  16-byte minimum envelope).
- `UnknownOpKind` — `OpKind` byte is `0` or `> kMaxOpKind`.
- `BadCrc`        — trailing CRC32C does not match.
- `BadSize`       — `OpSize`, `Gpr`, `SegmentReg`, `BinOpKind`,
  `CondCode`, or `TrapKind` enum value out of range; or the
  `HasResult` byte is neither `0` nor `1`.

CRC failure takes precedence over `Truncated` whenever the stream is
at least the minimum envelope length, because a CRC mismatch indicates
the bytes are *corrupt*, not merely *short*.

### Endianness

Little-endian everywhere. We do not support running on big-endian
hosts (no ARM64 / x86_64 / RISC-V Linux profile we target is BE).

### CRC32C

Polynomial `0x1EDC6F41` (Castagnoli, RFC 3720 §B.4). The
implementation in `core/src/ir/serialize_crc.cpp` uses the bit-reversed
form `0x82F63B78` and a 256-entry lookup table built at compile time.
Output is the standard final-XOR'd value. This matches iSCSI / Btrfs /
SCTP usage so external test vectors apply.

## Consequences

### Benefits

- Self-contained: no external dependency, no versioned schema file.
- Deterministic: same `Function` always produces same bytes, on any
  platform we target.
- Forward-compatible at the OpKind level: new variants don't require
  a version bump.
- Cheap to validate: a corrupt stream is caught at the CRC trailer in
  one pass.

### Costs

- Additions to the `Op` variant in source require a manual update to
  the OpKind table here, the `op_kind_for` switch in `serialize.cpp`,
  the read-payload dispatch, and a new round-trip test. We accept the
  manual coupling because it's a code-review checkbox once per new
  opcode.
- The CRC is per-stream, not per-statement. A corrupt single statement
  invalidates the whole stream. That's acceptable for the cache use
  case (we'd re-translate from scratch anyway).

### Reversibility

- Bytes are read-only on disk; reverting the format means bumping
  version and shipping a new reader. Same trade-off as RFC 0007.

## Future work

- **Translation-cache integration** — RFC 0007 documents the
  translation cache's existing on-disk format (machine-code blobs).
  Once this serializer lands and is fuzzed against malformed inputs,
  the cache can grow an *optional* IR side-channel: each entry
  acquires a second blob containing the post-passes IR, and on load
  the cache can choose to re-lower from IR (if host CPU features
  changed) or replay the stored machine code as today. This is left
  unimplemented in F1-IR-017/018; the next backlog item to pick this
  up would extend `core/src/cache/translation_cache.cpp` to write /
  read an IR slot alongside `code_bytes` and bump the cache file
  version.
- **Compression** — a serialized IR stream typically has good entropy
  but lots of `0` bytes in the upper half of `u64` fields. Wrapping
  the body in zstd (RFC 0008) would shrink the cache. Defer until we
  measure.
- **Lean cross-check** — once the Lean spec exposes a JSON or
  s-expression dump of `Op`, we can write a property test that
  serializes a Lean-generated `Op`, deserializes it in C++, and
  asserts structural equality. Out of scope for F1-IR-017/018.

## References

- RFC 0001 — IR SSA design (the `Op` variant whose layout this RFC
  encodes).
- RFC 0007 — translation-cache file format. The IR serializer is the
  building block for the planned IR side-channel.
- RFC 0008 — zstd compression library, candidate for body
  compression in a future version.
- `core/include/prisma/ir.hpp` — canonical `Op` definition.
- `core/include/prisma/ir_serialize.hpp` — public C++ surface.
- `core/src/ir/serialize.cpp` — implementation, including the
  `OpKind` enum that mirrors the table above.
- `core/tests/test_ir_serialize.cpp` — round-trip + corruption +
  truncation + Function tests.
- RFC 3720 §B.4 — CRC32C specification.
