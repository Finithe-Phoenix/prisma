---
id: 0003
title: Decoder dispatch — table-of-handlers vs giant switch
status: accepted
authors: [Danny]
created: 2026-05-03
updated: 2026-05-03
supersedes: []
superseded_by: null
---

# RFC 0003: Decoder dispatch — table-of-handlers vs giant switch

## Summary

The x86_64 decoder needs to dispatch on opcode byte (and prefix /
escape sequences) to a per-opcode decode handler. Two natural
designs exist: a sparse `std::array<DecodeFn, 256>` table, or a
giant `switch` statement. **We will use a hybrid table-of-handlers
keyed on the primary opcode byte, with hand-rolled prefix handling
above it.**

## Background

x86_64 has roughly 256 primary opcodes plus a 2-byte (`0F xx`)
escape, plus `0F 38 xx` and `0F 3A xx` 3-byte escapes for SSE/AVX.
Each can take prefixes (REX, F2/F3, 66, 67, segment overrides) that
modify the operand-size or address-size, plus the ModR/M and SIB
bytes that select operands. A complete dispatcher is therefore not
"256 entries" but closer to "256 × 4 escape spaces × prefix combos."

Two dispatch styles dominate the literature:

- **Table-of-handlers**: a flat array indexed by opcode. Each entry
  is a function pointer (or std::function) to the matching decode
  handler. Sparse entries point at a shared "unimplemented" stub.
  Box64 uses this; FEX uses a richer table with metadata.

- **Giant switch**: one switch per escape level. Each case body
  is the inline decode of that opcode. CompCert / QEMU use a
  switch dispatch in many places.

## Decision

The Prisma decoder will use a **hybrid**:

1. **Top-level switch** for prefix recognition (REX, F2/F3, 66, 67,
   segment overrides). Prefixes are stateful — we accumulate them
   into a `Prefixes` struct before consulting the opcode table.
2. **Function-pointer tables** for the primary opcode space and
   each escape space:
   - `kDispatch1[256]`     — primary opcode (no escape).
   - `kDispatch0F[256]`    — `0F xx` escape.
   - `kDispatch0F38[256]`  — `0F 38 xx` SSE4.1+.
   - `kDispatch0F3A[256]`  — `0F 3A xx` SSE4.1+ immediate-form.
3. Each handler returns a `std::variant<Decoded, DecodeError>` and
   advances the cursor by the bytes it consumed. Handlers are
   declared `constexpr`-friendly so the table can live in `.rodata`.

## Why hybrid, not pure table

A pure table forces every prefix to be a separate "instruction" in
the table, exploding the space. A pure switch makes adding a new
opcode require editing a 1000-line switch — a merge-conflict magnet
for two-agent development (see `docs/COORDINATION.md`).

The hybrid lets each agent add new opcodes as standalone handler
functions in `core/src/decoder/op_*.cpp` and register them by writing
one entry in the table. Per-opcode files are easy to review and to
attribute (git blame stays clean per opcode).

## Why function pointers, not std::variant tags

Tag-based dispatch (e.g. `enum Opcode { Mov, Add, … }; switch (op)`)
would let the optimiser see the dispatch and inline. Function
pointers lose that. We accept the indirection because:

- Decoder hot paths are dominated by the actual decode work
  (ModR/M parsing, displacement / immediate fetch), not the
  dispatch jump. Profiling on FEX / Box64 confirms this.
- Tag-based dispatch couples the table to a single `enum`, killing
  the per-opcode-file modularity we want for review and merging.
- vixl-style metadata tables (FEX) bring decoder *and* recogniser
  closer; we can adopt that idea later by adding a flat-data row
  next to each function pointer without changing the dispatch
  shape.

## Trade-offs

- **Cold cache miss** on the first call to each handler. We accept
  this; the cache line gets warm after the first basic block.
- **Indirect call** is not branch-predictable on the very first
  encounter. Modern OoO cores (Apple silicon, Cortex-X4) handle
  this well, but we should re-measure when targeting low-power
  Android cores in Fase 3.

## Future work

- A code-generation step (Python) that emits the four dispatch
  tables from a single `opcodes.json` to keep them in sync. Drops
  human-error risk in the table boundaries.
- Hot-opcode inlining: identify the top-N opcodes by execution
  frequency (Pillar 1 ML feature feed) and inline their decode
  bodies into the dispatch switch as a partial-evaluation pass.

## References

- FEX `arm64/InstructionTables.h` — metadata-rich opcode table.
- Box64 `dynarec_arm64_pass*.c` — function-pointer table style.
- Intel SDM Vol 2 Chapter 2 — opcode encoding ground truth.
