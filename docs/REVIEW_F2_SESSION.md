# F2 Session Review — `claude/hopeful-taussig-051239`

> Consolidated review of the Fase 2 feature branch using the
> [`prisma-pr-review`](../.claude/skills/prisma-pr-review/SKILL.md)
> and
> [`prisma-security-review`](../.claude/skills/prisma-security-review/SKILL.md)
> skills landed in `bf10e9d`.

**Range:** `8884efd..f2450e4` (45 commits)
**Reviewed:** 2026-05-12 by a Claude session on Daniel's Windows
coordination machine. No local build possible (toolchain absent on
host) — review is static analysis + protocol checks; the previous
session's reported test status (800 cases / 5004 assertions verde on
Apple Silicon) is taken at face value.

---

## Executive verdict

**HOLD** — branch is technically sound and well tested, but two issues
block merge to `main`:

1. **[Protocol]** Two-eyes rule violation on `core/include/prisma/ir.hpp`
   and `core/src/backend/lowering.cpp`. Five commits (`a2fabde`,
   `02c900f`, `afccedd`, `5448c9b`, `8317648`) authored solo by Claude
   in territory the project policy treats as requiring peer review.
2. **[Security HIGH]** REP STOSB/MOVSB lowering emits an unbounded
   guest-controlled loop. `RCX = 2^63` hangs the host. The dispatcher's
   per-block `max_steps` does not protect against per-instruction side
   effects.

Both must be addressed. Code quality itself is good; this is about
protocol and one missed attacker model.

### Status update (later 2026-05-12 / 2026-05-13 session)

- **Blocker B — Two-eyes:** option 2 (post-hoc RFCs) satisfied via
  [RFC 0011](rfc/0011-f2-avx256-fma-lowering.md) (pair-of-Vec128 + FMA
  IR shape, covers `a2fabde`, `02c900f`, `afccedd`) and
  [RFC 0012](rfc/0012-f2-wide-binops-and-rep-string.md) (wide-form
  BinOps + REP IR ops, covers `5448c9b`, `8317648`). Authorship
  recorded as `claude`; Danny waiver or Codex co-sign is the
  remaining step to flip the status to `accepted` under two-eyes.
- **Blocker A — REP DoS: RESOLVED** in `5756084`. `RepStos` and
  `RepMovs` are now block-terminating IR ops carrying `pc_of_rep` and
  `pc_after_rep`. Lowering caps per-call iterations at
  `kRepMaxBytesPerCall / step` (16 MiB of stores) and selects the
  exit PC from post-loop RCX — when remaining count is non-zero, the
  dispatcher re-enters the same REP with `x0 = pc_of_rep`. Per-call
  host latency is bounded at ~16 ms regardless of guest RCX.
  Validation: 768/767 cases + 4494 assertions verde under Debug AND
  ASan+UBSan on a Linux x86_64 container (mirrors CI). Adversarial
  e2e (`RepStos { rcx = 16 MiB + 7 }` halts in 3 dispatcher hops,
  full buffer filled, RCX=0) is wired but ARM64-only — gets
  `SUCCEED("skipped on non-ARM64 host")` until validated on Apple
  silicon or `linux/arm64` CI.

### Follow-on landings (2026-05-13)

After PR #1 went all-green for the first time, the autonomous run
continued with the F2 hot-spot queue from HANDOFF §5.D and the
broader follow-on items now tracked in `docs/WORK_QUEUE.md`:

- **F2-IR-049 VPTEST ymm** (`67a7336`). New `WriteFlagsPtestYmm` IR
  op + `Emitter::vptest_ymm` primitive (composes per-half AND/BIC/ORR
  before the existing NZCV-build). Decoder branches on `vex.L` in the
  legacy `0F 38 17` handler.
- **F2-IR-050 VBLEND VEX family** (`6a21ba5`). VPBLENDVB / VBLENDVPS /
  VBLENDVPD (`66 0F 3A 4C/4A/4B`) for xmm + ymm. Mask register from
  `imm8[7:4]`, src1 from `vex.vvvv`. Reuses the existing `VecBlend`
  IR op; L=1 emits the parallel high-half.
- **Lean spec catch-up** (`b7a8f31` + `9d1660a`). `BinOp` extended with
  the 6 F2-BK-007 variants; `Op` extended with `repStos` / `repMovs`.
  Unsigned `evalBinOp` cases concrete via `Nat`; signed are placeholders
  (`sorry` for sMulHi/sDiv/sMod). `.sorry-budget` bumped 0 → 3.
  Exhaustive `cases op with` in DCE + ConstProp passes updated.
- **F2-PS-004 Global CSE** (`0396c19`). `FunctionPassManager` plumbing
  + dominator-tree-forwarded CSE. Single-block translator caveat
  carries.
- **F2-PS-003 LICM** (`ff41e83`). Loop-invariant code motion via
  natural-loops. Pre-header is unique non-loop predecessor; multi-entry
  loops skipped conservatively.
- **F2-IR-051 VPERMQ ymm** (`fbd714a`). New `VecTbl2` IR op + emitter
  `vtbl2_q` (copy through V30/V31 fixed scratches, then NEON TBL).
  Decoder synthesises imm8-controlled byte-index VecConstants at decode
  time. Reusable for future VPERMD / VPGATHER.

Also a pile of CI cleanup that moved PR #1 from "5/9 failing" at
session start to all-green:

- `08c6cf8` `signal_handler.cpp` missing `#include <cstdlib>`
- `03f4d88` ir-spec sorry-budget grep + pipefail false-positive
- `115e69b` zydis stub `!shouldfail+WARN` → `SUCCEED`
- `bf91c38` shell-stub workflow rustup `--component` syntax
- `b8d74c6` shell orchestrator `cargo fmt` + clippy cleanup

Plus playbook docs: `docs/WORK_QUEUE.md` (`59ac4c0`) and
`docs/AGENT_PLAYBOOK.md` (`78868aa`) consolidating the patterns the
session has standardised.

All landings validated under Debug + ASan + UBSan in the Linux
x86_64 container that mirrors CI's `ubuntu-latest` runner.

**Two-eyes tally:** the session's solo IR/decoder/lowering/emitter
commits total 13. A batched Danny waiver in
`docs/REVIEWS/F2-CLAUDE-WAIVER.md` or a Codex audit pass over
`8884efd..HEAD` clears the merge gate:

- F2 review's original 5: `a2fabde`, `02c900f`, `afccedd`, `5448c9b`, `8317648`
- Blocker A reshape: `5756084`
- F2-IR-049 VPTEST ymm: `67a7336`
- F2-IR-050 VBLEND VEX: `6a21ba5`
- Lean spec extension: `b7a8f31`
- DCE/CP case-splits: `9d1660a`
- F2-PS-004 GCSE: `0396c19`
- F2-PS-003 LICM: `ff41e83`
- F2-IR-051 VPERMQ ymm: `fbd714a`

---

## prisma-pr-review — 8-point check

| # | Check | Status | Detail |
|---|---|---|---|
| 1 | Backlog claim trail | PASS | F2-IR-005, F2-IR-006, F2-BK-006, F2-BK-007, F2-BK-008/009, F2-PS-002 all claimed and closed in `docs/BACKLOG.md`. Umbrella F2-IR-002..004 + F2-BK-001..004 closed at `905f122`. No stale `[~|claude]` markers. |
| 2 | Commit discipline | PASS | All 45 commits follow `<scope>: <what>`, English, atomic, no emoji/WIP/FIXME. |
| 3 | Tests must exist | PASS | New IR ops covered: VecFpFma/VecFpScalarFma via 6 e2e at `test_e2e_dispatcher.cpp:1618,2352,2434,2510,2593`; RepStos/RepMovs via `test_decoder.cpp:3440-3474` + e2e; BinOpKind extensions via `test_decoder.cpp:243-385` + const-prop. F2-PS-002 has regression tests at `test_passes.cpp:464-497`. 797 → 800 cases / 4836 → 5004 assertions. |
| 4 | **Two-eyes territory** | **FAIL** | `core/include/prisma/ir.hpp` (IR shape) and `core/src/backend/lowering.cpp` (register allocator) touched solo by Claude across `a2fabde`, `02c900f`, `afccedd`, `5448c9b`, `8317648`. `CONTRIBUTING.md` requires a second reviewer on these surfaces. No reviewer commit present. |
| 5 | Cross-agent territory | PASS (soft rule) | Claude touched decoder (Codex territory) across 26 commits, but no active `[~|codex]` claims existed on F2-IR-005/006 at the branch base `8884efd`. Soft rule satisfied. If this becomes a pattern, an explicit RFC re-allocating decoder ownership is warranted. |
| 6 | RFC requirement | PASS | No new `FetchContent_Declare` entries, no `translation_cache.hpp` byte-layout changes, no new build options. Architectural decisions (pair-of-Vec128, VecFpFma shape, BinOpKind extension) are documented inline in commits + `SESSION_TRACE.md` but no formal RFC. Recommended (not blocking): write `docs/rfc/0011-f2-avx256-fma-lowering.md` post-hoc to capture the design. |
| 7 | License hygiene | PASS | No verbatim/near-verbatim FEX/Box64/QEMU. Deferred items label FEX as *inspiration* only (e.g. F2-PS-001, F2-IR-008). CLAUDE.md no-copy rule respected. |
| 8 | Lean sorry budget | N/A | `ir-spec/**/*.lean` untouched. Budget stays at 1. *Note for future: the new IR ops (LoadVecRegHi, StoreVecRegHi, VecFpFma, VecFpScalarFma, 6 new BinOpKinds) are not yet formalized in Lean — adding them will increase the budget by N (one `sorry` per pending soundness lemma).* |

---

## prisma-security-review — 6-surface threat model

| # | Surface | Touched? | Findings |
|---|---|---|---|
| 1 | JIT memory (W^X, MAP_JIT) | No | No regressions. |
| 2 | Signal handlers | No | No regressions. |
| 3 | Cache binary deserialization (RFC 0007) | No | No regressions. |
| 4 | Translation cache key collisions (FNV-1a) | No | No regressions. |
| 5 | P2P signed envelopes | N/A | Not yet implemented (Fase 2.5). |
| 6 | Syscall translation | N/A | Not yet implemented (Fase 2). |
| 7 | **New IR/Backend** (not in skill's original 6 — emergent) | YES | See findings below. |

### Findings

| # | Sev | Location | Issue | Remediation |
|---|---|---|---|---|
| 1 | **HIGH** | `core/src/backend/lowering.cpp:777-821` (RepStos/RepMovs lowering) | Unbounded loop on guest-controlled RCX. `cbz rcx, end; loop: ... ; cbnz rcx, loop` has no cap. `RCX = 2^63 - 1` hangs the host; adjacent unmapped pages → host out-of-bounds write. The dispatcher's `max_steps` limits per-block execution, not per-instruction side effects. Tests cover `RCX ∈ {0, 24, 32}` only. | Clamp `rcx` to a sane bound (e.g. `min(rcx, kRepMaxIterations)` where the cap is something like 16 MiB / step_size) before entering the loop. Optionally: implement a per-instruction step budget at the dispatcher level. Add adversarial test `RepStos with RCX=large_value` verifying bounded completion + no host corruption. |
| 2 | MED | `core/src/backend/lowering.cpp:469-483` + `core/src/passes/const_prop.cpp:76-91` | x86 `#DE` (divide error) trap on `b == 0` or `INT_MIN / -1` is not emitted — both lowering and const-prop silently produce `0` (ARM64 native behavior). Guest code written for x86 trap semantics will get wrong results, not a fault. | Documented divergence; not a regression. Track as deferred F2-BK item. Decoder spec already carries a caveat. Surface in user-visible docs once a Windows guest hits it. |
| 3 | LOW | `core/src/backend/emitter.cpp:793-808` (`vec_const_128`) | Drive-by fix in commit `f340201` — VecConstant lowering now loads full 128 bits via two `fmov` + `INS`. The prior implementation truncated to low 64 bits. The fix is **necessary** for FMADDSUB lane masks. | PASS. No remediation needed. Confirmed by e2e tests. |
| 4 | LOW | `core/src/backend/lowering.cpp:1025-1062` | `LoadVecRegHi` / `StoreVecRegHi` validate `op.ymm_index < ir::kXmmCount` (= 16). Decoder always casts from `ModRM.reg` (max 7) or `vex.vvvv` (max 15), so the assertion is defense-in-depth. | PASS. |
| 5 | LOW | `core/src/passes/const_prop.cpp:76-91` | Const-prop correctly mirrors ARM64 sdiv/smod semantics on `INT_MIN / -1` (wraps instead of trapping). | PASS. Consistent with finding #2's design choice. |

---

## Required actions before merge

### Blocker A — Fix the REP DoS (finding #1)

**Pick one:**

1. *Preferred:* Add an RCX clamp before the loop. Sketch:
   ```cpp
   // In RepStos / RepMovs lowering, before emitting the loop body:
   constexpr std::uint64_t kRepMaxIter = 16ull << 20;  // 16M iterations
   // emit: cmp rcx, kRepMaxIter ; csel rcx, rcx, kRepMaxIter, lo
   ```
   Then add a host-side check after the loop that surfaces "REP truncated"
   if `rcx_in > kRepMaxIter`, so guest can re-enter for the remainder
   (matches x86 interruptible-instruction semantics — REP is restartable
   on a host-side preemption).
2. Alternative: Implement a per-instruction step budget in the dispatcher.
   Heavier change but solves a class of similar issues.
3. Minimal documentation-only fix: do nothing in code, document the
   limitation in `docs/SECURITY.md` (which is an F2-DX item not yet
   claimed). **Not recommended** — this is a real DoS vector.

**Test requirement:** Adversarial test `RepStos with RCX = (1 << 32) + 1`
that completes in bounded time and verifies host integrity. If option 1
is taken: also verify the "restart-by-guest" path matches x86 REP-with-
interrupt semantics.

### Blocker B — Resolve two-eyes on IR + lowering (check #4)

**Pick one:**

1. *Preferred:* Codex audit pass. The five touched commits (`a2fabde`,
   `02c900f`, `afccedd`, `5448c9b`, `8317648`) are AVX-256 pair representation,
   FMA IR shape, scalar FMA, REP IR ops, and MUL/DIV BinOpKind extensions —
   all decisions that affect IR shape Codex inherits. A 200-line review
   from Codex with explicit signoff in `docs/REVIEWS/` would clear this.
2. Async RFC: write `docs/rfc/0011-f2-avx256-fma-lowering.md` and
   `docs/rfc/0012-f2-binop-rdx-rax.md` documenting the design, link the
   commits, mark RFCs `accepted` with both Danny and Codex (or just
   Danny) as approvers. This is the durable path; preserves design intent
   for future agents.
3. Danny explicit waiver: a single commit to `docs/REVIEWS/F2-AVX-FMA-WAIVER.md`
   recording that Danny reviewed the commits manually and waives the
   two-eyes rule for this specific branch. Acceptable given Codex's
   inactivity, but doesn't scale.

---

## Recommended (non-blocking) follow-ups

- **Lean spec extension:** the 10 new IR ops (`LoadVecRegHi`, `StoreVecRegHi`,
  `VecFpFma`, `VecFpScalarFma`, `RepStos`, `RepMovs`, and the 6 new
  `BinOpKind`s) need entries in `ir-spec/PrismaIR/Syntax.lean` +
  semantics in `Semantics.lean`. Adding them with `sorry` placeholders
  is fine for now; bump `.sorry-budget` by N (one per pending lemma)
  in the same commit and reference the F1-LN-NNN backlog item that will
  retire each.
- **F2-PS-004 Global CSE** (HANDOFF.md Hot Spot B) is the recommended
  next item, but its actual perf gain is small until the translator
  produces multi-block `ir::Function`s. Consider scheduling translator
  work (multi-block fusion) before Global CSE so the latter has
  something to optimize over.
- **Adversarial test sweep on REP family:** the same DoS class applies
  to any future `RepCmps` / `RepScas` ops. A general "rep-with-large-rcx"
  property test would future-proof.

---

## Files cited

- `core/include/prisma/ir.hpp` — IR shape changes (BinOpKind extensions, new ops).
- `core/src/backend/lowering.cpp` — lines 469-483 (div/mod lowering), 777-821 (REP lowering), 1025-1062 (ymm hi bounds).
- `core/src/backend/emitter.cpp` — lines 793-808 (vec_const_128 fix).
- `core/src/passes/const_prop.cpp` — lines 76-91 (div/mod const folding).
- `core/tests/test_e2e_dispatcher.cpp` — lines 1618, 2352, 2434, 2510, 2593 (FMA e2e).
- `core/tests/test_decoder.cpp` — lines 243-385 (MUL/DIV decoding), 3440-3474 (REP decoding).
- `core/tests/test_passes.cpp` — lines 464-497 (F2-PS-002 regression).
- `docs/BACKLOG.md` — closed F2 umbrella items.
- `docs/SESSION_TRACE.md` — commit grouping.
- `docs/HANDOFF.md` — branch handoff notes.

## Commits flagged for two-eyes

- `a2fabde` — IR + backend: ymm state, LoadVecRegHi/StoreVecRegHi infrastructure.
- `02c900f` — IR + decoder + backend: VFMADD132/213/231 PS+PD xmm.
- `afccedd` — IR + decoder + backend: scalar FMA (VFMADDxxxSS/SD families).
- `5448c9b` — IR + decoder + backend: REP STOSB/MOVSB native loops. **Also carries the HIGH severity DoS.**
- `8317648` — IR + decoder + backend: MUL/DIV proper rdx:rax / rax:rdx lowering.

---

## Verdict reiterated

**HOLD on merge.** Address Blocker A (DoS clamp) and Blocker B
(two-eyes resolution) in any combination of the options above. Once
both are cleared, this branch is mergeable — the tests are green, the
protocol is otherwise clean, and the code quality is high.
