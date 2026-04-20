---
id: 0006
title: Register allocator design — linear scan + stack spilling for intra-block SSA
status: accepted
authors: [Danny]
created: 2026-04-20
updated: 2026-04-20
supersedes: []
superseded_by: null
---

# RFC 0006: Register allocator design

## Summary

Prisma's Lowerer uses a two-pass **linear-scan** allocator over a
fixed scratch pool of x0-x9, backed by a **stack-slot spill** path
for blocks whose simultaneously-live ref set exceeds the pool. Guest
GPRs (rax..r15) stay pinned to dedicated host registers
(x10..x17 / x19..x26); those are not part of the pool. The design
is intentionally intra-block only — CFG-aware allocation arrives
with block-level lowering (F1-BK-006) and is out of scope for this
RFC.

## Motivation

A dynamic binary translator emits large volumes of throwaway code.
Register allocation at translation time must be:

1. **Fast.** Allocation is on the critical path for every block we
   JIT; a quadratic algorithm is unacceptable once blocks start
   hitting dozens of instructions.
2. **Correct.** A miscompile in the allocator surfaces as a hard-to-
   diagnose guest-state corruption, usually weeks after it was
   introduced.
3. **Small scratch footprint.** ARM64 has 31 GPRs but our ABI
   already pins x10..x26 to guest GPRs and x27 to the
   `CpuStateFrame*`. The scratch pool is therefore capped at ten
   registers (x0..x9); the allocator must handle that as a
   hard wall.
4. **Deterministic output.** The content-hash cache key depends on
   byte-stable translation output, so two translations of the same
   guest bytes must emit identical host code. Random-ordered
   allocators (or allocators whose heuristic depends on hash-map
   iteration order) break the cache.

Earlier revisions of the Lowerer used a bump-pointer allocator: every
`Constant`, `LoadReg`, or `BinOp` result consumed the next
unallocated scratch, and `LowerError::OutOfScratchRegs` fired the
moment we reached the eleventh ref. That made trivial test blocks
fail (11 `Constants` in a row) even when their live intervals were
trivially disjoint. A proper allocator fixes this without losing
speed.

## Context

- **SSA invariant (RFC 0001).** Every IR ref has exactly one def that
  precedes all uses inside the block. Live intervals are therefore
  contiguous; the classical linear-scan algorithm (Poletto & Sarkar,
  POPL 1999) fits without modification.
- **Single basic block per call to `lower()`.** The current translator
  emits one block at a time. Cross-block allocation is handled by
  the Translator's prologue/epilogue writing guest GPRs through the
  pinned x10..x26 registers; in-block scratches never cross block
  boundaries. RFC 0003 / F1-BK-006 will reconsider this for proper
  multi-block regions.
- **Pool = x0..x9 only.** x18 is reserved on Apple platforms, x19..x26
  are callee-saved and used for guest GPR pinning, x27 for the state
  pointer, x28 spare, x29/x30 are FP/LR. The 10-register pool is a
  hard constraint.
- **Stack space is cheap.** The Translator's prologue already reserves
  a stack frame (AAPCS64-compliant 16-byte-aligned) for callee-saved
  spills. Extending it with N spill slots is a one-line change.
- **Belady's rule is near-optimal for straight-line code.** Without
  branches inside a block, the oracle "evict the ref whose next use
  is furthest in the future" is provably optimal up to a constant.
  We already compute `last_use_[ref]` for the expiry pass, so Belady
  is free.

## Considered alternatives

### 1. Graph colouring
Build a full interference graph, colour with 10 colours.
- **+** Classical minimum register count; can produce very dense code.
- **−** O(n²) interference construction + ChaitinBriggs spill iteration.
- **−** Overkill for straight-line blocks where the SSA lifetime order
  already tells us everything.
- **Verdict:** rejected. The complexity cost buys zero return until we
  have cross-block CFG allocation, at which point we'd reconsider (and
  likely still lean linear-scan-style rather than graph colouring —
  see Wimmer & Franz 2010 for SSA-based linear-scan that matches GC
  output on straight-line code).

### 2. Pure bump-pointer (status quo before this RFC)
Next ref gets the next scratch; fail hard on exhaustion.
- **+** Trivial to implement.
- **−** Can't handle blocks with >10 concurrent live refs even when
  they have plenty of dead intervals in the mix.
- **Verdict:** kept the speed, lost the ability to compile nontrivial
  blocks. Fails the "correctness" bar.

### 3. Linear scan (accepted)
Pre-pass computes `last_use_` per ref. Main forward scan emits stmts
in IR order; after each stmt, expired refs return their host regs to
a free-list pool. Allocation pops from the pool (prefer lowest-number
for deterministic output).
- **+** O(n) per block, small constants.
- **+** Straightforward extension to spilling.
- **+** Deterministic output: the free-list is a stack; iteration
  order is fixed.
- **−** Can still fail on blocks whose peak simultaneous live refs
  exceeds the pool size — addressed by the spill path below.

### 4. Spill-to-register (no stack)
When pool exhausts, pick two near-death refs and share a register
via timing.
- **+** Avoids stack traffic.
- **−** Doesn't actually help when the live set is legitimately
  bigger than the pool — the two refs are still simultaneously
  live.
- **Verdict:** rejected. The real win comes from stack spills.

## Decision

We adopt **linear-scan + optional stack spilling** with the following
concrete design:

### Phase 1: liveness pre-pass

Walk the IR statement list once. For each ref `r`:
- `last_use_[r]` := max statement index that reads `r`, or its def
  index if `r` is never used.

Seeding `last_use_` with the def index ensures a never-read def
expires immediately after its own statement instead of getting
pinned forever.

### Phase 2: forward scan

Maintain:
- `free_regs_` — stack of available scratch regs. Seeded in reverse
  order so `x0` pops first.
- `ref_to_scratch_` — live ref → host reg map.
- `stmt_temporaries_` — per-stmt temporaries (e.g. Rol's neg helper),
  freed at end of stmt.
- `spilled_to_slot_` — evicted ref → stack slot index.
- `free_slots_` — stack of unused slot indices (0..spill_slots-1).

For each stmt at index `i`:

1. Lower the stmt. When it needs a scratch for its `result`:
   1. If `free_regs_` is empty: try `spill_one_ref()`. If that also
      fails, return `OutOfScratchRegs`.
   2. Pop a reg, bind it to the result ref.

2. When an operand ref is read:
   1. If it's in `ref_to_scratch_`, return the mapped reg.
   2. If it's in `spilled_to_slot_`, allocate a fresh scratch (which
      may itself cause eviction), emit `ldr X_reg, [sp, #slot]`,
      remove from `spilled_to_slot_`, return the reloaded reg.
   3. Otherwise the ref is undefined (bug in the passes or IR).

3. After emitting the stmt:
   1. Return `stmt_temporaries_` to `free_regs_`.
   2. Expire every ref whose `last_use_` has passed: move its reg
      from `ref_to_scratch_` back to `free_regs_`.

### Spill victim selection (Belady)

`spill_one_ref()` scans `ref_to_scratch_` for the ref whose
`last_use_` is furthest in the future. Refs about to expire this
stmt (`last_use_ <= stmt_index_`) are skipped — spilling a zombie is
wasteful. If no candidate exists or `free_slots_` is empty, return
false and the caller falls back to `OutOfScratchRegs`.

The victim's contents are stored at
`[sp, #spill_slot_base_offset + slot*8]`.

### Reload-driven recursion

A reload emits `ldr` and needs a fresh scratch — which itself may
trigger eviction. Because SSA guarantees the operand being reloaded
is distinct from the operand being evicted (distinct refs by
definition), the recursion terminates in at most one extra spill
step. The combined limit is `pool + spill_slots` simultaneously-live
refs.

### LowerOptions knobs

```cpp
struct LowerOptions {
    bool          emit_ret_on_terminator{true};
    unsigned      spill_slots{0};           // 0 disables spilling
    std::int32_t  spill_slot_base_offset{0};
};
```

The Translator owns reserving stack space and picking the offset;
the Lowerer only emits `str/ldr` against the pre-agreed slots.

## Consequences

### Benefits

- Blocks with up to `pool_size + spill_slots` simultaneously-live
  refs now compile. With pool=10 and modest `spill_slots=16`, we
  handle 26 — comfortably above the 12-15 concurrent live set we've
  seen from realistic x86 decodings.
- Dead refs release their reg on the statement after their last use;
  short-lived temporaries no longer eat the pool.
- Belady's heuristic means the reload cost distribution is as close
  to optimal as single-block analysis allows.
- The whole algorithm is O(n) in stmts — benchmarks (F1-TC-007) put
  it at microseconds per 100-stmt block.

### Costs

- Extra state on the `Lowerer`: three maps + one vector for spill
  tracking. ~200 bytes per block. Negligible.
- Spill code is not yet wired through the Translator — callers must
  opt in via `LowerOptions::spill_slots`. That wiring is F1-BK-020
  (Translator frame layout extension); until then the production
  path behaves like it always did.
- Victim scan is O(live) per spill, which is fine for pool=10 but
  worth revisiting if we ever grow the pool.

### Reversibility

Reverting to bump-pointer is a ~40-line change but would regress
every multi-ref test and the Translator's existing guest code paths
that exceed 10 refs in the clear. Realistic reversal would require
picking a different allocator, not going backwards.

## Implementation notes

Landed across four commits (see git log):

- `029997c` — claim in BACKLOG.
- `3f29499` — linear scan + expiry. Peak-live tracking via
  `scratch_used()`.
- `55777a4` — claim spill work in BACKLOG.
- `7fb57c0` — spill path + `Emitter::sp_load/sp_store` + 4 new
  `test_lowering.cpp` cases.

Tests in `core/tests/test_lowering.cpp` cover:

- Two-interval reg reuse (peak == 1).
- 11 dead consts succeed (peak == 1).
- 11 live LoadRegs without spill → `OutOfScratchRegs`.
- 11 live LoadRegs with `spill_slots=4` → success, peak_spills ≥ 1.
- 12 live LoadRegs with `spill_slots=1` → fails cleanly (11 max).
- Non-zero `spill_slot_base_offset` appears in emitted `[sp, #N]`.
- Round-trip spill+reload preserves Add semantics.

## Open questions

- **CFG-aware allocation.** When F1-BK-006 introduces real block-level
  lowering with conditional jumps, cross-block lifetimes will appear.
  Strategy: still linear-scan, but compute liveness over the whole
  function CFG (Wimmer-style intervals with holes at branches). Graph
  colouring gets re-evaluated at that point.
- **Two-operand rewrite for x86 memory forms.** x86 is two-address;
  after legalising to three-operand ARM64 some BinOps want the
  destination to coincide with one operand (saves a mov). A small
  peephole over SSA can detect this and hint the allocator. Not
  urgent — vixl already elides redundant movs on canonical patterns.
- **Rematerialisation.** For `Constant` refs whose spill cost exceeds
  re-emitting the mov-imm, skip the spill and emit the constant
  again on reload. Deferred — realistic workloads don't hit this.

## References

- Poletto, M. & Sarkar, V. (1999). *Linear scan register allocation.*
  ACM TOPLAS. Original linear-scan paper.
- Wimmer, C. & Franz, M. (2010). *Linear scan register allocation on
  SSA form.* CGO 2010. Adaptation of linear-scan to SSA IRs, which
  matches our setup.
- Belady, L. A. (1966). *A study of replacement algorithms for a
  virtual-storage computer.* IBM Systems Journal. Origin of the
  optimal-replacement heuristic used for victim selection.
- RFC 0001 — `docs/rfc/0001-ir-ssa-over-template-based.md`. SSA
  invariant used by this allocator.
