# Prisma — Active Work Queue

> Externalised state for the autonomous F2 follow-on session.
> Mirrors / supersedes the agent's mental list. When a row flips to
> ✅, the agent updates the `Completed at` column with the landing
> SHA and a one-line note in `Notes`. Multi-commit items list every
> commit in order under `SHAs`.

Last updated: 2026-05-13 (in-flight after `b0b589b`).

## Currently active

Branch: `claude/hopeful-taussig-051239` (== PR #1).
CI on `9d1660a`: lint-docs ✅, ir-spec ✅, core-stub ✅, core-sanitizers ✅, shell-stub ✅.

## Queue (priority order)

| # | Item | Scope | SHAs | Status | Notes |
|---|------|-------|------|--------|-------|
| 1 | F2-PS-004 Global CSE via dominators | 1 commit, `core/src/passes/` | `0396c19` | ✅ done | FunctionPassManager + `global_cse`. Wiring into translator deferred (still single-block today). |
| 2 | F2-PS-003 LICM (loop-invariant code motion) | 1 commit | `ff41e83` | ✅ done | Iterates to fixed point; skips multi-entry loops conservatively. |
| 3 | docs(playbook): consolidate agent flows | 1 commit | `78868aa` | ✅ done | AGENT_PLAYBOOK.md landed. |
| 4 | VPERMQ ymm (lane-crossing qword permute) | 1 commit | `fbd714a` | ✅ done | New `VecTbl2` IR op + `vtbl2_q` emitter primitive. Reusable for VPERMD / VPGATHER followups. |
| 5 | F2-IR-052 VPERMD ymm | 1 commit | `ed505a8` | ✅ done | Runtime byte-index build via And + Shl + Pshufb + Add, then VecTbl2. |
| 6 | feat(translator): wire function pipeline | 1 commit | `a5fc152` | ✅ done | translate() now runs build_cfg → function_pipeline_ → flatten → stmt pipeline. Single-block early-out. |
| 7 | BMI2 shift family (SHLX/SARX/SHRX/RORX) | 2 commits | `1824167`, `eb31777` | ✅ done | Variable-count + imm-count rotates, all flag-preserving. |
| 8 | BMI2 MULX | 1 commit | `b0b589b` | ✅ done | Two-dest unsigned multiply, reuses MUL + UMulHi. |
| 9 | BMI2 BZHI / PDEP / PEXT | 2-3 commits | — | ⏸ queued | BZHI needs count saturation (Select-based). PDEP/PEXT need software emulation (no direct ARM64 equivalent). |
| 10 | F2-BK-010 Call/Ret return-stack predictor | 4-6 commits, `core/src/runtime/` + dispatcher | — | ⏸ blocked-by-real-RET | HANDOFF §5.F. Prereq: real RET semantics (decoder + dispatcher) currently halt-sentinel only. |
| 11 | Real CALL/RET semantics (prereq for #10) | 3-4 commits | — | ⏸ queued | Decoder: emit JumpReg from popped RSP; update e2e tests to expect dispatcher continuation rather than halt-on-RET. |
| 12 | F2-IR-007/008 x87 baseline | 6-8 commits, new domain | — | ⏸ queued | HANDOFF §5.E. Treat x87 stack as doubles; document precision divergence. `X87Slot` in `cpu_state.hpp:43-48`. |
| 13 | VPGATHER {D,Q}{PS,PD,D,Q} family | 6-8 commits | — | ⏸ queued | VSIB encoding + per-element conditional load. |
| 14 | AES / SHA hardware crypto opcodes | 4-6 commits | — | ⏸ queued | Direct ARM64 mappings via Crypto extensions. Niche but cheap. |
| 15 | Direct branch threading | 4-6 commits | — | ⏸ queued | When CondJumpRel target is already translated, branch directly in JIT instead of dispatcher round-trip. Major perf win for hot loops. |
| 5 | VPGATHER {D,Q}{PS,PD,D,Q} family | 6-8 commits, `core/src/decoder/` + new IR op | — | ⏸ queued | Lane-crossing AVX-256. Each variant is its own opcode (`66 0F 38 90/91/92/93`). |
| 6 | F2-IR-007/008 x87 baseline | 6-8 commits, new domain | — | ⏸ queued | HANDOFF §5.E. Treat x87 stack as doubles; document precision divergence. Reuse `X87Slot` in `cpu_state.hpp:43-48`. |

## Completed (this session)

| # | Item | SHA | Note |
|---|------|-----|------|
| – | RFC 0011 — AVX-256 pair-of-Vec128 + FMA | `143a330` | Post-hoc, resolves Blocker B option 2 for AVX/FMA commits. |
| – | RFC 0012 — wide-form BinOps + REP string ops | `9acb5e6` | Surfaces Blocker A in §1 with the canonical clamp sketch. |
| – | REVIEW: Blocker B resolved note | `5829cb7` | — |
| – | `fix(runtime): #include <cstdlib>` | `08c6cf8` | Unblocks scaffolding-check on clang-17/libstdc++. |
| – | `ci(ir-spec)`: tolerate zero sorries in budget check | `03f4d88` | grep+pipefail false-positive. |
| – | **Blocker A — REP DoS bounded + PC-aware re-entry** | `5756084` | RepStos/RepMovs are now block terminators; 16 MiB clamp. |
| – | REVIEW: Blocker A resolved | `553ee46` | — |
| – | `docs(rfc)`: MD004/MD026 markdownlint nits | `e2fc8a7` | — |
| – | `test(zydis-diff)`: SUCCEED instead of !shouldfail+WARN | `115e69b` | — |
| – | `ci(shell-stub)`: split clippy/rustfmt `--component` | `bf91c38` | rustup syntax. |
| – | `fix(shell)`: cargo fmt + clippy-clean orchestrator | `b8d74c6` | 20 warnings → 0, 22/22 tests verde. |
| – | F2-IR-049 — VPTEST ymm + `WriteFlagsPtestYmm` | `67a7336` | New emitter primitive `vptest_ymm`. |
| – | F2-IR-050 — VPBLENDVB / VBLENDVPS / VBLENDVPD VEX | `6a21ba5` | xmm + ymm; reuses `VecBlend`. |
| – | spec(lean): wide-form BinOps + RepStos/RepMovs | `b7a8f31` | `.sorry-budget` 0 → 3 for signed corner cases. |
| – | REVIEW: VPTEST/VBLEND/Lean follow-ons + two-eyes tally | `b9b3e7e` | — |
| – | spec(lean): DCE + ConstProp case-splits for new ops | `9d1660a` | Closes Lean exhaustive-match failure. |
| – | docs: add WORK_QUEUE.md | `59ac4c0` | This file. |
| 1 | feat(passes): F2-PS-004 — FunctionPassManager + global_cse | `0396c19` | 780/780 verde Debug + ASan/UBSan. |
| 2 | feat(passes): F2-PS-003 — loop_invariant_motion | `ff41e83` | 788/788 verde Debug + ASan/UBSan. |
| 3 | docs: AGENT_PLAYBOOK.md | `78868aa` | Container, 13-file IR-op recipe, function-pass authoring, two-eyes, Lean, CI, WORK_QUEUE contract. |
| 4 | feat(ir,decoder,backend): F2-IR-051 — VPERMQ ymm via VecTbl2 | `fbd714a` | 790/790 verde Debug + ASan/UBSan. |
| 5 | feat(decoder): F2-IR-052 — VPERMD ymm | `ed505a8` | 792/792 verde Debug + ASan/UBSan. Runtime byte-index build. |
| 6 | feat(translator): wire function pipeline into translate() | `a5fc152` | 792/792 verde; GCSE/LICM now live on multi-block decoded regions. |
| 7 | feat(decoder): F2-IR-053 — SHLX / SARX / SHRX | `1824167` | 795/795 verde Debug + ASan/UBSan. |
| 8 | feat(decoder): F2-IR-053 followup — RORX | `eb31777` | 796/796 verde. |
| 9 | feat(decoder): F2-IR-053 followup — MULX | `b0b589b` | 797/797 verde Debug + ASan/UBSan. |

## Standing decisions (carry across items)

- **Recipe for new IR ops**: HANDOFF.md §4 lists 11 mandatory file touches.
  Two more files in practice (`core/src/translator/translator.cpp::is_block_terminator`
  if the op terminates blocks; `core/src/ir/cfg.cpp::is_terminator` likewise).
  Plus tests in 3 places (`test_decoder.cpp`, `test_lowering.cpp`, `test_profiler.cpp`).
- **Sanitizer validation is local**: `cmake --build core/build-asan` in the
  `prisma-build` container runs the full suite under ASan+UBSan in ~3 min.
  Use this before every push to catch UB / memory issues that the
  default Debug build misses.
- **`.sorry-budget` is the tripwire**: bump only when a real sorry is added.
  CI fails on > budget. Drop the budget back down once a proof lands.
- **Two-eyes ledger**: every solo Claude commit in IR / decoder / lowering /
  emitter territory needs eventual co-sign. Tally lives in
  `docs/REVIEW_F2_SESSION.md` under the "Two-eyes tally" entry.
- **Container lifecycle**: `prisma-build` container persists across
  ScheduleWakeup-style restarts. `apt-get install` + `wget` of cmake
  has been done once; rebuilds are seconds. `~/.cargo` + rustup also installed.

## Skipped / out-of-scope (this session)

- **F2-CLAUDE-WAIVER.md** authoring — Danny's call to write, not the agent.
- **Apple Silicon E2E adversarial test** of the REP clamp — needs the
  other machine; the wired test SUCCEED-skips on x86_64.
- **Lean signed-arithmetic proofs** for sMulHi/sDiv/sMod — tracked
  under F1-LN-014/015/016 placeholders; not blocking F2 merge.
