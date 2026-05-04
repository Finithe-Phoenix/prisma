---
id: 0004
title: Flags model — eager NZCV vs lazy snapshot vs SSA-typed Flags
status: accepted
authors: [Danny]
created: 2026-05-03
updated: 2026-05-03
supersedes: []
superseded_by: null
---

# RFC 0004: Flags model — eager NZCV vs lazy snapshot vs SSA-typed Flags

## Summary

Every x86 ALU instruction sets EFLAGS as a side effect. A DBT can
materialise these flags eagerly, lazily (snapshot the operands and
the operation; compute on read), or treat them as first-class SSA
values typed `Flags`. **Prisma uses a hybrid: implicit NZCV for the
adjacent CmpFlags+CondJumpRel pattern, and explicit SSA-typed
`WriteFlags`/`ReadFlag`/`CondJumpFlags` (F1-IR-003..007) for
non-adjacent or set-once-read-many cases.**

## Background

The cost of "every ALU sets six flags" is real:

- **Eager NZCV**: emit the flag-setting variant of every ARM64 ALU
  op (`adds` / `subs` / `ands` / etc.). Cheap when the next op
  reads NZCV. Wasteful when nothing reads them — a guest `ADD r1,
  r2` followed by an unrelated `MOV r3, r4` recomputes flags that
  no one uses.

- **Lazy snapshot** (Box64 / FEX style): on every ALU op, snapshot
  `(op, lhs, rhs, size)` into a per-thread "pending flags" struct.
  When something actually reads a flag (a Jcc, SETcc, ADC, SBB),
  recompute on demand. Killer optimisation when most flag writes
  are dead. Adds a constant snapshot cost on every ALU.

- **SSA-typed Flags**: model EFLAGS like any other value. A
  `WriteFlags{Sub, lhs, rhs}` op produces a Ref of pseudo-type
  Flags. A `ReadFlag{flags, Zero}` reads it back as I8. The
  validator + DCE pass naturally drop dead flag writes; the
  lowerer emits NZCV only for surviving ones.

## Decision

**Hybrid:**

1. **Same-statement-list, adjacent**: keep the existing implicit-
   NZCV pattern (`CmpFlags` + `CondJumpRel`). The lowerer pins
   the invariant that nothing flag-clobbering may sit between
   producer and consumer; the decoder produces this pattern by
   construction for the common Jcc-after-CMP case. Cheap.

2. **Anything more complex**: use the SSA-typed Flags pillar
   (F1-IR-003 .. F1-IR-007). `WriteFlags` is a pure op that
   produces a Flags Ref; `ReadFlag` and `CondJumpFlags` consume
   it. The lowerer tracks which Refs are valid Flags and uses
   that to decide whether NZCV is current.

3. **DCE on dead Flags Refs is automatic** because Flags Refs
   participate in the standard SSA validator + DCE pass — a
   `WriteFlags` whose result Ref has no consumer is dropped.

## Why not pure lazy snapshot

We tried this on paper:

- The snapshot adds a per-ALU memory write to a thread-local
  struct. On Apple silicon this isn't free (thread-local access
  via TPIDRRO_EL0 is several cycles).
- Reading a flag becomes a software computation that can't fold
  into a `b.cc` directly — we'd emit `cmp+b.cc` even when the
  source ALU already set NZCV.
- The interactions with our SSA-based optimiser get messy: every
  optimisation needs to know about pending-flags state.

The SSA-typed approach gives us most of the lazy-snapshot benefit
(dead flag writes vanish) without paying snapshot cost on every
op.

## Why not pure eager NZCV

We tried this too. It works for the common adjacent CMP-then-Jcc
case (which is why we keep the implicit-NZCV path for that), but:

- It can't represent non-adjacent flag use (e.g. an ALU op,
  several non-flag-clobbering ops, then a SETcc reading the
  earlier flags). The SSA-typed pillar handles this naturally.
- It collides with our register allocator: if NZCV is "in flight"
  across an ALU op that doesn't set it, the allocator has no
  way to express that constraint.

## Lowering rules (current MVP)

- `WriteFlags{Sub, lhs, rhs, sz}` → `cmp lhs, rhs` (no destination
  needed).
- `WriteFlags{Add, lhs, rhs, sz}` → `adds tmp, lhs, rhs` (allocates
  a single-stmt temporary; the result is discarded but NZCV is
  set).
- `WriteFlags{And, lhs, rhs, sz}` → `ands tmp, lhs, rhs`.
- `ReadFlag{flags_ref, Carry}` → `cset rd, hs` (carry-set).
- `ReadFlag{flags_ref, Zero}`  → `cset rd, eq`.
- `ReadFlag{flags_ref, Sign}`  → `cset rd, mi`.
- `ReadFlag{flags_ref, Overflow}` → `cset rd, vs`.
- `ReadFlag{flags_ref, Parity/Aux}` → currently `UnsupportedOp`
  (no direct ARM64 NZCV mapping; software emulation via popcount /
  XOR-low-bits lands later).
- `CondJumpFlags{flags_ref, cc, t, f}` → `b.cc label_t; b label_f`.

## Trade-offs

- **Two paths to maintain**. The implicit-NZCV path stays for the
  hot adjacency case; the SSA-typed path handles everything else.
  Keeping both is a small ongoing cost, not a recurring one.
- **Parity / Aux flags are deferred**. Real x86 code uses them
  rarely (mostly debugger / BCD code); a software fallback in the
  ReadFlag lowerer is sufficient until profiling shows otherwise.
- **Re-encoding flag-setting variants** doubles emitter API
  surface. We added `cmp / adds / ands` only for the operations
  the validator currently allows in WriteFlags; expanding to
  every BinOpKind is incremental.

## Future work

- A pass that *promotes* the implicit-NZCV path to the SSA-typed
  one when the producer and consumer are non-adjacent. Today the
  decoder picks one or the other up-front.
- Lazy-snapshot fallback for guest code that reads flags after a
  long chain of mutations: a third lowering mode for "snapshot,
  defer, recompute on read." Only worth the complexity if
  profiling shows it pays.
- Parity / Aux ReadFlag support via a software-emulated path.

## References

- Box64 dynarec flags handling — `dynarec/arm64/flags.c`.
- FEX `Interface/IR/Passes/FlagOptimization.cpp` — peephole that
  rewrites flag-producing ops with no consumer.
- Intel SDM Vol 1 §3.4.3 — EFLAGS register definition.
- F1-IR-003 / F1-IR-004 / F1-IR-005 / F1-IR-007 in
  `docs/BACKLOG.md` — the SSA Flags pillar this RFC ratifies.
