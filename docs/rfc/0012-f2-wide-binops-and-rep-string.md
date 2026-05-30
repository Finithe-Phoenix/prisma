---
id: 0012
title: Wide-form integer BinOps and REP string IR ops
status: accepted
authors: [claude]
created: 2026-05-12
updated: 2026-05-12
supersedes: []
superseded_by: null
---

# RFC 0012: Wide-form integer BinOps and REP string IR ops

## Summary

Two thematic extensions to the IR cover x86 instructions whose
semantics do not map cleanly to a single ARM64 op:

1. **Wide-form integer arithmetic.** `MUL` / `IMUL` write 128 bits across
   `RDX:RAX`; `DIV` / `IDIV` write both quotient (`RAX`) and remainder
   (`RDX`). We model these as six new `BinOpKind` variants —
   `UMulHi`, `SMulHi`, `UDiv`, `SDiv`, `UMod`, `SMod` — riding the
   existing `BinOp` machinery rather than introducing a multi-result
   IR construct.
2. **`REP` string ops.** `REP STOSB/STOSW/STOSD/STOSQ` and
   `REP MOVSB/W/D/Q` become native ARM64 loops emitted from two new
   IR ops `RepStos` and `RepMovs`, instead of the previous
   `InlineAsm`-fallback that round-tripped to the host per iteration.

The two are bundled in one RFC because they share the design pattern
*"x86 instruction with implicit multi-register effect, modelled as
separate IR-level effects that the existing pipeline understands"*.

**This RFC also documents a `[Security HIGH]` known issue in the
landed REP lowering and the agreed remediation. See Open questions §1.**

## Motivation

Real Windows binaries lean on three classes of x86 instructions that
both prior fallbacks (`InlineAsm` for MUL/DIV, `InlineAsm` for REP)
served badly:

- **Wide multiplies / divisions** — every `printf` call site, every
  hash-mix sequence, every fixed-point arithmetic block. The `InlineAsm`
  fallback meant a host syscall per instruction; no optimisation could
  reason about the value flowing from a multiply to its consumer.
- **`memset` / `memcpy`** — overwhelmingly emitted by MSVC as `REP STOSB`
  / `REP MOVSB`. Anti-debug runtime checks and game-engine clears
  saturate these. The `InlineAsm` fallback turned each `memset` into a
  host call with marshalling overhead.
- **`memcmp` / `strlen`** — out of scope for this RFC (REP CMPS / SCAS
  still fall back to `InlineAsm`). Tracked for a follow-up.

This RFC is post-hoc. The decision was made and implemented in commits
`8317648` (MUL/DIV), `de1a836` (algebraic identities for the new
BinOpKinds), and `5448c9b` (RepStos / RepMovs). Recording the design so
the next agent inheriting the branch does not re-litigate it, and so
the DoS open question (§1 below) is preserved across sessions.

## Context

### Constraints

- **Existing `BinOp` already provides 1-result arithmetic.** Adding new
  `BinOpKind` variants is mechanical: const-prop, algebraic-simplify,
  CSE, DCE, dead-store, pretty-print, validator, serializer all switch
  on `BinOpKind` and would need a new case anyway, regardless of how
  we represent the multi-result effect.

- **ARM64 split:** `mul` (low 64), `umulh` (high 64 unsigned),
  `smulh` (high 64 signed), `udiv` / `sdiv` (quotient), `msub`
  (remainder via `q*b` then subtract). Each maps to exactly one of the
  six new BinOpKinds, modulo register pinning for RDX:RAX.

- **No multi-result IR construct existed.** `WriteFlags` is a side
  effect (no SSA result). `LoadVec` produces one `Ref`. Adding a
  `Mul128 → {hi, lo}` tuple would be the first multi-result op; that
  decision would propagate to every pass and every `Ref`-walker.

- **REP semantics.** Per the x86 ISA, `REP` is interruptible: an
  interrupt during a long `REP STOSB` leaves `RCX` at the
  not-yet-stored count, `RDI` at the next address, and the instruction
  restarts on resume. The dispatcher's per-block `max_steps` does not
  protect against per-instruction host-side iteration count — `RCX`
  is guest-controlled.

### Prior art

- **FEX** has `_UMulH` / `_SMulH` IR opcodes shaped exactly like our
  new BinOpKinds, and a dedicated `_DivU` / `_DivS` / `_RemU` / `_RemS`
  set. They also keep `DIV` / `IDIV` as a single ARM64 sequence rather
  than separate quotient / remainder IR ops; we choose to split for
  optimiser visibility.
- **Box64** decodes MUL/DIV into a hand-rolled ARM64 sequence directly,
  no IR.
- **QEMU TCG** has `tcg_gen_muls2_*` (single op writing both halves).
  That works because TCG ops are intrinsically multi-result; ours are
  not.

For REP: FEX has `_RepMovs` / `_RepStos` IR ops that lower to a JIT
loop. They also clamp by `RCX_max` (we should — see §1).

### Affected components

- `core/include/prisma/ir.hpp` — `BinOpKind` enum + `RepStos` /
  `RepMovs` op declarations.
- `core/src/passes/const_prop.cpp` — `__int128` arithmetic for the
  high-half multiply folding; ARM64 semantics for div/mod corner cases.
- `core/src/passes/algebraic.cpp` — identity rules for the new kinds.
- `core/src/passes/dce.cpp` — purity / operand-collect for `RepStos`
  / `RepMovs` (impure — side effect on memory).
- `core/src/backend/lowering.cpp` — pin RDX:RAX as needed; emit
  `umulh` / `smulh` / `udiv` / `sdiv` / `msub`; emit native REP loops.
- `core/src/decoder/x86_decoder.cpp` — decode `F6 /4..7`, `F7 /4..7`,
  and the `F3 AA / AB / A4 / A5` REP-prefixed forms.

## Considered alternatives

### Alt A — Single multi-result IR op (Mul128, Div128)

Introduce `struct Mul128 { Ref a; Ref b; }` producing a tuple `{hi, lo}`,
and `struct Div128 { Ref dividend_hi; Ref dividend_lo; Ref divisor; }`
producing `{quot, rem}`. Use a new SSA construct — say,
`MultiRef { stmt_id; lane }` — to address the components.

**Pros:**

- Closer to x86's mental model; one IR op per x86 instruction.
- Multi-result construct is reusable for other split-effect ops
  (`PUSHF` / `LAHF` could share it for flag bundles).
- Cross-component DCE is trivial: if both halves are dead, drop the op.

**Cons:**

- Invasive. Every pass that walks `Ref`s needs to handle `MultiRef`.
  `Ref`'s data model is a single `uint32_t` today; widening it to
  include a lane index either grows every SSA carrier (memory bloat
  in every IR) or forces a side table.
- No other instruction in the IR needs this today. YAGNI applies.
- Lean spec would grow a new type former. Single-result SSA proofs
  no longer compose without case splits on tuple vs. scalar.

### Alt B — Six new `BinOpKind` variants (chosen)

Add `UMulHi`, `SMulHi`, `UDiv`, `SDiv`, `UMod`, `SMod` to
`BinOpKind`. Each is a single-result `BinOp` like every other; the
decoder emits two separate IR ops to model MUL (one for low via
existing `Mul`, one for high via new `UMulHi`/`SMulHi`) and three for
DIV (quotient, remainder, plus the optional flag-write).

**Pros:**

- Zero new IR machinery. The 10-pass pipeline runs unchanged.
- const-prop can fold each half independently. If only the low half
  is consumed, the high-half computation is DCE-eliminable. Same for
  remainder-only DIV.
- Lean spec extension is straightforward — six new arithmetic ops
  with closed-form semantics over `BitVec n` (assuming wrap on
  overflow / ARM64 semantics on div-by-zero).

**Cons:**

- The decoder emits two or three IR ops where x86 has one; the IR
  pipeline pays for it.
- Pairing the two halves of a multiply for the purpose of "is this a
  full 128-bit operation" requires reconstruction at the lowering
  level (the lowerer recognises adjacent `Mul` + `UMulHi` over the
  same operands and emits a single ARM64 `mul` + `umulh` against the
  same source pair).

### Alt C — Keep `InlineAsm` fallback

Leave MUL/DIV/REP unchanged. Pay the per-instruction host round-trip.

**Pros:** zero IR / pass work.

**Cons:** unacceptable performance — `memset`-heavy code (game
engine clears) and arithmetic-heavy code (every fixed-point routine)
would never reach JIT-grade speed. Was the status quo through F1; F2
requires the upgrade.

### Alt D — Two separate IR ops for REP (decode + iterate)

For REP specifically, an alternative is a "load-bounded-N" iterator
that the pass framework would unroll if N is small. Too clever; defers
the security question (still need a bound). Rejected.

## Decision

### Wide-form integer (commit `8317648`)

Adopt **Alt B**: six new `BinOpKind`s. Declaration order in
`core/include/prisma/ir.hpp:62-72`:

```cpp
enum class BinOpKind : std::uint8_t {
    Add = 0, Sub, Mul,
    And,     Or,  Xor,
    Shl,     Shr, Sar,
    Rol,     Ror,
    Rcl,     Rcr,
    UMulHi,  SMulHi,    // F2-BK-007
    UDiv,    SDiv,      // F2-BK-007
    UMod,    SMod,      // F2-BK-007
};
```

Decoder mapping for `F6 /4..7` and `F7 /4..7`:

- `MUL r/m` → `BinOp{Mul, rax, src}` writes RAX; `BinOp{UMulHi, rax, src}`
  writes RDX. Both have the same operands so const-prop / CSE can
  share computation.
- `IMUL r/m` → `BinOp{Mul, rax, src}` + `BinOp{SMulHi, rax, src}`.
- `DIV r/m` → `BinOp{UDiv, rdx:rax-pair, src}` writes RAX; `BinOp{UMod,
  rdx:rax-pair, src}` writes RDX. The 128-bit dividend is currently
  represented by passing RAX only and accepting that the high half (RDX)
  contributes through register pinning at the emitter level. A future
  refinement would carry a 128-bit `Ref` shape; not needed today.
- `IDIV r/m` → analogous with `SDiv` + `SMod`.

`const_prop` (commit `de1a836`) folds the new kinds using `__int128`
for `UMulHi` / `SMulHi` to capture the high 64 bits without overflow.
For `UDiv` / `SDiv` / `UMod` / `SMod`, the folder mirrors ARM64 semantics
on the corner cases:

- `b == 0` → return `0` (ARM64 wraps; x86 traps `#DE`).
- `INT_MIN / -1` → return `INT_MIN` (ARM64 wraps; x86 traps `#DE`).

Both divergences are intentional and documented under Open questions §2.

`algebraic_simplify` covers `x*0/0*x/0/x/0%x → 0` and `x%1 → 0`.

### REP string ops (commit `5448c9b`)

Two new IR ops:

```cpp
struct RepStos { OpSize size; bool reverse; };  // ir.hpp:614
struct RepMovs { OpSize size; bool reverse; };  // ir.hpp:622
```

`size ∈ {I8, I16, I32, I64}` selects the element width.
`reverse` mirrors the guest `DF` flag; the decoder currently always
emits `reverse=false` because DF tracking is not yet wired. Once DF
joins the flag set, the decoder reads it at decode time and passes the
boolean through.

Operands are pinned at the lowering level:

- `RepStos`: `RCX` (count), `RDI` (destination), `RAX` (value).
- `RepMovs`: `RCX` (count), `RDI` (destination), `RSI` (source).

Lowering emits a native ARM64 loop:

```asm
        cbz     rcx, end
loop:   store   <al/ax/eax/rax>, [rdi]    ; or load+store for MOVS
        add/sub rdi, rdi, #step           ; step = bit_width(size) / 8
        sub     rcx, rcx, 1
        cbnz    rcx, loop
end:
```

The implementation is `core/src/backend/lowering.cpp:759-829`.

## Consequences

### Benefits

- **Real performance on memcpy / memset / wide multiply.** A
  `memset(p, 0, 4096)` is now ~4 KiB worth of `strb` instructions
  through the JIT instead of 4096 host syscalls. Same magnitude of
  improvement on MUL / DIV in arithmetic-heavy code.
- **Optimiser sees through the arithmetic.** const-prop discovers
  compile-time-known multiplies; algebraic identities short-circuit
  `x * 0` and `0 / x`; DCE drops dead high-halves or dead remainders.
- **No new IR machinery.** `Ref` stays single-`uint32_t`; the SSA
  walker stays unchanged; the Lean spec gains six closed-form
  arithmetic ops and two side-effecting loop ops, which is the
  minimum possible disturbance.

### Costs

- Decoder cost: 2-3 IR ops per x86 wide-arithmetic instruction. Bounded
  by the size of the BinOpKind switch in the decoder (one new case per
  kind, total ~30 lines).
- Pairing for the lowerer: the backend recognises adjacent
  `BinOp{Mul}` + `BinOp{UMulHi}` over the same operands and emits a
  single `mul` + `umulh` rather than two independent multiplies.
  Adds a small state machine in `lower_stmt`.
- The REP lowering currently has a known DoS, see §1.

### Reversibility

**Moderate.** Removing BinOpKinds is symmetric to adding them: each
removed kind requires deleting one switch case in each of ~10 files.
Removing `RepStos` / `RepMovs` would require reverting to `InlineAsm`
fallback, which is also symmetric. No data is persisted to disk in
RFC-0007 cache form for either op; the cache is keyed on guest bytes,
not IR.

## Implementation notes

- The pairing recognition in the lowerer is best-effort, not
  guaranteed. If a pass reorders the two halves so they no longer sit
  adjacent in the statement list, the lowerer emits two independent
  multiplies. Not a correctness issue. const-prop / CSE can also fold
  one half away entirely; the lowerer treats that as "single multiply,
  no high half" and just emits `mul`.
- For REP, the per-iteration step is computed at lowering time from
  `op.size` via `ir::bit_width(op.size) / 8u`. The step is materialised
  in a scratch via `mov_imm64`. Allocating two scratches (`step_reg`
  and `one_reg`) keeps the loop body free of immediate operands that
  would otherwise require literal pool flushes.

## Open questions

### 1. **[Security HIGH] REP unbounded iteration is a host DoS.**

The current lowering (lines 777-788 for `RepStos`, 815-827 for
`RepMovs` in `core/src/backend/lowering.cpp`) decrements `RCX` per
iteration with no upper bound. `RCX = 2^63 - 1` hangs the host; adjacent
unmapped pages produce host out-of-bounds writes via the JIT loop.

**Tests cover only RCX ∈ {0, 24, 32}** (see `core/tests/test_decoder.cpp:3440-3474`
and the e2e `RepStos` cases). Surprised by what's not tested:
adversarial values were never wired into the corpus.

**Proposed remediation (preferred):** clamp `RCX` to a sane bound
before entering the loop, and surface a "REP truncated, restart" signal
to the dispatcher so the guest can re-enter for the remaining count.
This matches x86 REP-is-interruptible semantics exactly — guest code
written for REP-with-IRQ already handles the restart path.

```cpp
constexpr std::uint64_t kRepMaxIter = 16ull << 20;  // 16 MiB / step
// Before the loop:
//   cmp  rcx, kRepMaxIter
//   csel rcx, rcx, kRepMaxIter_reg, lo
// After the loop, expose pre-clamp count via thread-local for the
// dispatcher to read and re-enter if (original_rcx > kRepMaxIter).
```

The 16 MiB bound is empirically generous (no real `memset` exceeds it
in a single call without an outer loop already) and bounds the worst
case at "tens of milliseconds of host time per dispatch".

**Required test:** `RepStos { size = I8, rcx = (1 << 32) + 1 }`
completes in bounded host time, returns correct guest state with
RCX adjusted to count-remaining, and the dispatcher re-enters cleanly
for the remainder. Same shape for `RepMovs`.

**Alternative remediation:** per-instruction step budget at the
dispatcher level. Heavier change but solves a class of similar issues
(any future `RepCmps` / `RepScas` instance). Discussed in
`docs/REVIEW_F2_SESSION.md` finding §1; deferred unless the clamp
proves insufficient.

**Status:** open. **Must be addressed before merging the F2 branch to
`main`.** Tracked as Blocker A in the F2 review.

### 2. x86 `#DE` (divide-error) trap is not emitted

Both lowering (`core/src/backend/lowering.cpp:469-483`) and const-prop
(`core/src/passes/const_prop.cpp:76-91`) silently produce `0` for
`b == 0` and wrap for `INT_MIN / -1`, matching ARM64 native semantics.

Guest code that relies on x86 trap semantics gets wrong results,
not a fault. This is a known divergence; tracked for a deferred F2-BK
item that introduces an optional pre-check + `Trap` IR op. Decoder spec
already carries a caveat. Not blocking; not a regression.

### 3. DF (direction flag) tracking for REP

`reverse = true` is reachable only once the decoder reads DF at decode
time. Today DF is not part of the modelled flag set; we always emit
`reverse = false`. Guests that explicitly `STD` (set DF) before a
`REP MOVSB` get incorrect behaviour today. Tracked for the F2 flag
infrastructure work.

### 4. Pairing pass should be promoted to an explicit recogniser

The lowerer's ad-hoc adjacent-pair recognition is fragile (depends on
statement ordering surviving the 10-pass pipeline). Promoting it to an
explicit pass — "if two adjacent BinOps are `{Mul, UMulHi}` or `{Mul,
SMulHi}` or `{UDiv, UMod}` or `{SDiv, SMod}` over the same operands,
tag the pair so the lowerer can emit a single ARM64 sequence" — would
de-couple the recogniser from emission and make the optimisation
visible to the IR dumper. Tracked for a follow-up.

### 5. Lean specification

The six new `BinOpKind`s have closed-form arithmetic semantics over
`BitVec n` and should add to `ir-spec/PrismaIR/Semantics.lean` without
introducing `sorry`s, except for div/mod-by-zero where the choice of
ARM64 wrap vs. x86 trap needs a documented divergence.

`RepStos` and `RepMovs` have iteration semantics (`Step` is a small
relation over `(rcx, rdi, [rsi,] memory)`). Adding them with placeholder
`sorry`s for the loop soundness lemma bumps `.sorry-budget`. Tracked.

## References

- Commit `8317648` — IR + decoder + backend: MUL/DIV proper rdx:rax / rax:rdx lowering.
- Commit `de1a836` — algebraic identities for the new BinOpKinds.
- Commit `5448c9b` — IR + decoder + backend: REP STOSB / MOVSB native ARM64 loops.
- Commit `bdd4f01` — earlier `InlineAsm` fallback for REP that this RFC supersedes.
- `core/include/prisma/ir.hpp:62-72` — `BinOpKind` enum.
- `core/include/prisma/ir.hpp:614-625` — `RepStos` / `RepMovs` op declarations.
- `core/src/backend/lowering.cpp:759-829` — REP lowering (carries the §1 DoS).
- `core/src/passes/const_prop.cpp:76-91` — div/mod corner-case handling.
- `docs/REVIEW_F2_SESSION.md` — F2 session HOLD verdict (this RFC
  resolves Blocker B option 2 for the wide-form + REP commits, and
  documents Blocker A's proposed remediation).
- Intel SDM Vol. 2, §3 — MUL / IMUL / DIV / IDIV / REP STOS / REP MOVS.
- ARM ARM, §C6 — `umulh` / `smulh` / `udiv` / `sdiv` semantics.
- FEX `_UMulH`, `_SMulH`, `_RepMovs`, `_RepStos` — prior art reference
  (inspiration only; no code copied per CLAUDE.md no-copy policy).
