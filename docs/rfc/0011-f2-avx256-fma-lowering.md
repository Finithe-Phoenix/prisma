---
id: 0011
title: AVX-256 pair-of-Vec128 representation and FMA IR ops
status: accepted
authors: [claude]
created: 2026-05-12
updated: 2026-05-12
supersedes: []
superseded_by: null
---

# RFC 0011: AVX-256 pair-of-Vec128 representation and FMA IR ops

## Summary

Prisma represents AVX-256 (`VEX.L=1`) state as a *paired* pair of 128-bit
slots — `xmm[16]` for the low halves and a parallel `ymm_hi[16]` for the
high halves — and adds two IR ops (`LoadVecRegHi`, `StoreVecRegHi`) to
move the high lane, plus two FMA-shaped ops (`VecFpFma`, `VecFpScalarFma`)
to lower the FMA3 family without losing the single-rounded semantics
the ISA requires. We do **not** introduce a `Vec256` IR type. Every
AVX-256 instruction materialises as a pair of 128-bit triplets at the IR
level and as two ARM64 NEON sequences at the emitter level.

## Motivation

ARM64 NEON is 128-bit. SVE2 is the only 256-bit-capable ISA on ARM, and
it is neither universally available on the hardware Prisma targets
(Apple silicon, Snapdragon, Dimensity) nor uniform in vector length. A
DBT that wants to run real Windows binaries on ARM phones must lower
AVX-256 into 128-bit primitives no matter what shape the IR carries.

The question is therefore *where* the split happens — at the IR level
(every pass sees pairs) or below the IR (passes see a single Vec256
node, lowering decomposes). Both work; we picked the former.

This RFC is post-hoc. The decision was made and implemented in commits
`a2fabde` (state + LoadVecRegHi/StoreVecRegHi infrastructure), `02c900f`
(VFMADD132/213/231 PS+PD xmm), `afccedd` (scalar FMA), and the dozens
of opcode-extension commits that followed (`bd4e6cc`, `6cf73f9`,
`c5d4a71`, `99d2056`, `b2b7ea9`, `c46554a`, `5745961`, `bdef659`,
`0ddc2dd`, `3402428`, …). Recording the design here so the next agent
inheriting the branch does not re-litigate it.

## Context

### Constraints

- **IR was 128-bit max before F2-IR-005.** Every existing `VecBinOp`,
  `VecCompare`, `VecShuffle`, etc. consumed and produced `Ref`s typed
  as 128-bit values. Existing passes (const_prop, CSE, DCE, redundant
  load, etc.) assume a uniform value-size space.

- **CpuStateFrame offset stability matters.** `xmm[]` lives at offset
  144 of the state frame and is the load/store target for every existing
  legacy SSE handler. Shifting that offset would touch every existing
  emitter call site and every test that pokes at the frame directly.
  See `core/include/prisma/cpu_state.hpp:107-115` for the offset helpers.

- **VEX.128 zeroes the high lane.** When an AVX instruction with
  `VEX.L=0` writes a register, the upper 128 bits of the corresponding
  YMM are zeroed. Legacy SSE writes (no VEX) leave the upper lane
  *undefined* — we choose to preserve it. This means the new high-lane
  store path must run on AVX-128 writes too.

- **FMA semantics require single rounding.** `VFMADD231PS` computes
  `dst = round(a × b + c)` with one rounding step over the full
  intermediate. Decomposing to `mul` + `add` performs two roundings and
  can differ by a ULP. The IEEE 754-2008 `fma()` is the matching
  primitive, and ARM64 has `fmla` / `fmls` that lower directly.

### Prior art

- **FEX** uses 256-bit IR nodes plus a target-specific decomposition
  pass. Their decoder produces `VOp256_*` directly; the ARM64 backend
  rewrites at codegen time. Their IR carries the 256-bit shape through
  every pass.
- **Box64** has no IR; AVX-256 is decomposed inline at decode time.
- **DynamoRIO** uses an opcode-table representation; the question is moot.

### Affected components at decision time

- `core/include/prisma/ir.hpp` — IR op declarations.
- `core/include/prisma/cpu_state.hpp` — frame layout + offset helpers.
- `core/src/decoder/x86_decoder.cpp` — opcode allowlist + handler patches.
- `core/src/backend/lowering.cpp` — `compute_liveness` + `lower_stmt`.
- `core/src/backend/emitter.cpp` — `vec_const_128` (revealed bug, see
  Open questions #3).

## Considered alternatives

### Alt A — `Vec256` IR type (the FEX shape)

Introduce a new value size `VecFpSize::F32x8 / F64x4` and an integer
counterpart. Every existing `VecBinOp` / `VecCompare` / etc. grows a
`size` field that admits 256-bit. Validator, pretty-printer,
serializer, dataflow analysis, DCE, const-prop all gain a 256-bit case.

**Pros:**

- Passes see a uniform AVX-256 view; an optimiser can reason about
  the full 256-bit value as one entity (useful for cross-lane DCE,
  redundant-load elimination across halves).
- Closer to the x86 mental model.
- One IR op per x86 instruction makes the pretty-print and decoder
  unit tests smaller.

**Cons:**

- *Every* pass and analysis grows a 256-bit branch. ~2000 lines of
  diff across `core/src/passes/` and `core/src/ir/`.
- The const-prop folder gains a 256-bit value carrier; arithmetic on
  it requires two operations under the hood anyway.
- Reversal cost is asymmetric: once IR ops carry 256-bit semantics,
  removing them later requires re-deriving the equivalent 128-bit
  pair representation.

### Alt B — Pair-of-Vec128 (chosen)

Keep IR ops 128-bit. Add `LoadVecRegHi` / `StoreVecRegHi` to access
the high-lane file. An AVX-256 instruction emits exactly the same low
half as its SSE cousin, then re-emits the same logic against the high
half if `vex.present && vex.L`. The decoder produces twice as many IR
triplets; passes don't notice.

**Pros:**

- Zero touch to existing passes. Existing const-prop, CSE, DCE etc.
  work unchanged on each half. The full IR optimisation pipeline
  already runs over the doubled instruction stream.
- Lowering is mechanical: each handler grows a `if (vex.present &&
  vex.L) { emit_hi_half(); }` block, no new emitter primitive needed.
- Easy to roll out incrementally — extending a handler to L=1 is
  decoupled from any pass work.
- Lean spec already had 128-bit primitives; no new soundness lemmas
  are needed for the pair representation itself (only for the new IR
  ops, which are trivially pure loads/stores against a different
  CpuStateFrame slot).

**Cons:**

- Per-instruction IR count is doubled. Pretty-printed dumps are
  longer. Profile data sees twice as many ops.
- Cross-lane analysis (e.g. "the whole 256-bit value is dead") is
  awkward; DCE has to drop both halves to fully eliminate. In practice
  the existing per-half DCE catches the common case.
- A future "split" / "extract lane" optimiser that wants to reason
  about whole 256-bit values has to reconstruct the pair from
  matching low/hi pairs at use sites. Not implemented; deferred.

### Alt C — Inline-asm decomposition

Don't bother with IR ops. Decoder emits `InlineAsm` blobs for AVX-256.

**Pros:** zero IR work, fastest path to "decoded".

**Cons:** defeats the entire IR pillar. No optimisation across AVX-256
boundaries, no cross-instruction NPU classification, no Lean spec
coverage. Rejected on first principles.

## Decision

Adopt **Alt B**.

### State representation

`CpuStateFrame` carries two 128-bit arrays of length 16:

```cpp
std::array<XmmReg, kXmmCount> xmm{};      // offset 144
std::array<XmmReg, kYmmCount> ymm_hi{};   // offset 400  (= 144 + 16*16)
```

The offset choice is intentional: legacy SSE handlers already address
`xmm[]` at +144; adding `ymm_hi[]` *after* `xmm[]` keeps every existing
offset table valid. `kYmmCount == kXmmCount == 16`. Two
`static_assert`s in `cpu_state.hpp:130-133` enforce the layout.

### IR ops

Two minimal load/store ops:

```cpp
struct LoadVecRegHi  { std::uint8_t ymm_index; };           // ir.hpp:397
struct StoreVecRegHi { std::uint8_t ymm_index; Ref value; }; // ir.hpp:398
```

Both bounds-check `ymm_index < kXmmCount` in the validator and the
emitter (defense in depth; the decoder cannot produce a value out of
range from `ModRM.reg` or `vex.vvvv`, but the assertion documents the
invariant).

### FMA ops

Two new IR ops cover the entire FMA3 family (24 packed variants ×
ordering 132/213/231 × {PS, PD} × {xmm, ymm} + 12 scalar variants +
MADDSUB / MSUBADD):

```cpp
struct VecFpFma {                                            // ir.hpp:588
    Ref a;
    Ref b;
    Ref c;
    bool neg_addend;   // VFNMADD / VFNMSUB → ±(a*b) + c
    bool neg_mul;      // VFMSUB  / VFNMSUB → (a*b) - c, i.e. negate c
    VecFpSize size;
};

struct VecFpScalarFma {                                      // ir.hpp:760
    Ref a;
    Ref b;
    Ref c;
    Ref scalar_upper;  // carries the upper-lane bits unchanged
                       // (x86 scalar SS/SD semantics)
    bool neg_addend;
    bool neg_mul;
    FpSize size;
};
```

The two booleans encode the four ordering choices that the x86 ISA
enumerates as separate opcodes (MADD / MSUB / NMADD / NMSUB). The
ordering selector (132 / 213 / 231) is *not* a field — the decoder
re-permutes the operands at decode time so that the IR always sees
`fma(a, b, c) = round(a*b + c)`. This keeps the IR op count small and
the lowering trivial.

For `MADDSUB` / `MSUBADD` (packed), the decoder synthesises a single
`VecFpFma` of "add" plus a single `VecFpFma` of "sub" plus a
`VecBlend` over an alternating mask. The mask is materialised as a
full 128-bit `VecConstant` (see Open questions #3); for the ymm variant,
the same pattern is repeated for the high half via `LoadVecRegHi` /
`StoreVecRegHi`.

### AVX-256 dispatch

A single allowlist in `core/src/decoder/x86_decoder.cpp` (search for
`// F2-IR-005 — AVX-256 allowlist`) admits `VEX.L=1` only for the
specific opcodes whose handlers have been extended. Any opcode whose
handler still hard-codes 128-bit-only paths must NOT be on the
allowlist; the global default is `UnsupportedEncoding`.

### Handler pattern for `vex.L=1`

The recipe consolidated across the 40+ extension commits:

1. Hoist the effective-address computation before the first memory
   load (the high half needs to add 16 to it).
2. Emit the low-half sequence (same as the existing SSE/AVX-128 path).
3. If `vex.present && vex.L`:
   - Read the high half via `LoadVecRegHi` for register sources, or
     `LoadVec` with `addr + 16` for memory sources.
   - Re-execute the same IR pattern on the high-half operands.
   - Write the result via `StoreVecRegHi`.
4. Add the opcode to the allowlist.

The decoder unit test for an L=1 form should mirror the existing L=0
test, doubled.

## Consequences

### Benefits

- Optimisation pipeline runs unchanged. The 10-pass default pipeline
  applies to the doubled instruction stream the same way it applies
  to single-lane code; const-prop on the low half is independent of
  the high half, and that is correct.
- Legacy SSE code is byte-for-byte unaffected. The xmm offsets did
  not move; the existing 800+ test corpus stayed green throughout
  the F2-IR-005 ramp.
- Incremental rollout: one opcode at a time, each as its own commit
  paired with a decoder test. No flag-day refactor.
- The Lean spec only grows by the two trivial load/store ops once
  formalised. Pair semantics fall out of repeated application of the
  underlying 128-bit primitives.

### Costs

- Per-instruction IR-op count is 2× for any L=1 form. The 10-pass
  pipeline pays for it. Profiling on representative AVX-256 traces is
  pending; the cost is bounded by `O(passes) × O(extra-ops)` and we
  do not expect it to be material until much later when ymm code
  dominates dispatch.
- Cross-lane DCE / CSE is awkward. A future pass that wants "the
  256-bit value is dead" or "these two pairs are CSE-equivalent" has
  to reconstruct the pair view. Deferred until needed.
- `compute_liveness` in `core/src/backend/lowering.cpp` must have a
  branch for every IR op variant (the switch is `-Wswitch`-exhaustive
  by design). Forgetting `LoadVecRegHi` / `StoreVecRegHi` would let
  FP last-use expiry recycle live refs (DanglingRef). This bit us
  once before via a latent bug fixed in commit `0597402`.

### Reversibility

**Low.** Once decoder + passes + emitter all assume pairs, switching
to a unified `Vec256` IR type would require:

- Adding a 256-bit case to every existing IR op (binop, compare,
  shuffle, etc.) and every pass.
- Recomputing CpuStateFrame layout (probably unify `xmm`/`ymm_hi` into
  one 256-bit array; that breaks every legacy SSE offset).
- Re-deriving the FMA lowering against the unified type.

Estimated at 1-2 weeks of work plus full test re-validation. Acceptable
risk; the project has time-budgeted alternatives if performance data
later forces the change.

## Implementation notes

- `kXmmCount` and `kYmmCount` are both `16` and declared in
  `cpu_state.hpp:27-28`. They stay aliased for now; if a future
  hardware target exposes only 8 YMMs, splitting these is a one-line
  change.
- The high-half offset is computed as `400 + idx * 16` (`cpu_state.hpp:114`)
  and used directly by the emitter via the `vec_high_lo_offset_for`
  helper.
- `LoadVecRegHi` and `StoreVecRegHi` are pure with respect to DCE
  (`core/src/passes/dce.cpp`). `LoadVecRegHi` may be eliminated if its
  result is unused; `StoreVecRegHi` is treated as a side-effecting
  store and cannot be eliminated without proving the slot is
  subsequently overwritten before any reader observes it (we don't
  attempt this yet).
- The full list of files touched per the "new IR op" recipe in
  `docs/HANDOFF.md` section 4 must be respected. Use commit `afccedd`
  (scalar FMA) as the canonical example.

## Open questions

1. **Pair-allocator for ymm scratch registers.** Current backend
   widens the FP scratch pool from `V0..V7` to `V0..V23` (commit
   `05044f8`). This is sufficient for every benchmark we have run,
   but a pair-allocator that reserves matched `Vn` / `Vn+1` slots for
   ymm operations would reduce shuffle counts. Deferred until measured
   demand.
2. **Lean specification.** `LoadVecRegHi`, `StoreVecRegHi`, `VecFpFma`,
   `VecFpScalarFma` are not yet in `ir-spec/PrismaIR/Syntax.lean` or
   `Semantics.lean`. Adding them with placeholder `sorry`s bumps the
   `.sorry-budget` from 0 to 4. Tracked under F1-LN-NNN (TBD); blocks
   no current work but should land before any pass that consumes FMA
   semantics for verified rewriting.
3. **`vec_const_128` latent bug.** Commit `f340201` revealed that the
   prior `VecConstant` lowering loaded only the low 64 bits, silently
   zeroing the high 64. FMA MADDSUB lane masks (alternating add/sub)
   surfaced this — every prior `VecConstant` user happened to use
   values whose high 64 bits were already zero. The fix loads 128 bits
   via `fmov` + `INS`. Documented as a finding, not an open question:
   no regressions known.
4. **Trust-alignment for high-half memory loads.** `LoadVec` with
   `addr + 16` does not fault-check alignment, matching the policy
   for low-half loads (we trust the guest). If a future target enforces
   strict alignment in JIT mode, both halves need a check.
5. **VBROADCASTSS / VBROADCASTSD ymm and friends.** Lane-crossing
   AVX-256 ops (VBROADCAST{SS,SD,F128}, VINSERTF128, VEXTRACTF128,
   VPERM2F128) land without new IR ops, by composing existing
   primitives plus `LoadVecRegHi` / `StoreVecRegHi`. Confirmed in
   commit `e358bcd` and `99ed88c`. Future lane-crossing ops should
   follow this discipline before adding new IR ops.

## References

- Commit `a2fabde` — ymm state, `LoadVecRegHi` / `StoreVecRegHi`
  infrastructure.
- Commit `02c900f` — VFMADD132/213/231 PS+PD xmm (initial FMA shape).
- Commit `afccedd` — scalar FMA (VFMADDxxxSS / SD families).
- Commit `f340201` — VecConstant lowering fix (load full 128 bits).
- Commit `0597402` — `compute_liveness` gaps for FpBinOp / VecFpFma.
- Commit `05044f8` — NEON scratch pool widening (V0..V7 → V0..V23).
- `core/include/prisma/cpu_state.hpp:23-135` — CpuStateFrame layout
  and offset helpers.
- `core/include/prisma/ir.hpp:396-768` — IR op declarations.
- `docs/HANDOFF.md` §4 — "Cuando añadas un IR op nuevo" recipe.
- `docs/REVIEW_F2_SESSION.md` — F2 session HOLD verdict (this RFC
  resolves Blocker B option 2 for the AVX-256 + FMA commits).
- Intel SDM Vol. 2, §3 — VEX prefix encoding and `VEX.L` semantics.
- ARM ARM, §C2.1 — NEON FMLA / FMLS semantics (single-rounded fused
  multiply-accumulate).
