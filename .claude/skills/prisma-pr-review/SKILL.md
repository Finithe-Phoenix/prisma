---
name: prisma-pr-review
description: Review a PR or commit set on the Prisma DBT against the project's actual protocols — CONTRIBUTING.md two-eyes rule, COORDINATION.md claim protocol, commit-format discipline, no-code-without-tests, claude/codex territory split, RFC requirement, license hygiene, and Lean sorry budget. Use when reviewing incoming commits, your own diff before pushing, or auditing the other agent's work.
---

# Prisma PR review

Apply this when reviewing a PR, an incoming commit range, or your own pending diff before push.

## The 8-point check

### 1. Backlog claim trail
Every meaningful change traces to a `F<phase>-<component>-<num>` claim in `docs/BACKLOG.md`. Verify two commits exist in the series:

- `chore(backlog): <agent> claims F1-XX-NNN: <title>` — the claim, flipping `[ ]` → `[~|<agent>]`.
- A later commit flipping `[~|<agent>]` → `[x] (<sha>)` where `<sha>` is the implementation commit's short hash.

Exempt: typo fixes, regression tests defending the other agent's subsystem, one-line docs.

### 2. Commit discipline (CLAUDE.md)
- English (research is global).
- Format: `<scope>: <what>` — e.g. `core/decoder: add SIB byte handling for ModR/M`.
- Atomic: each commit must build + pass tests on its own.
- No emojis in commits.
- No `--no-verify`, no `--amend` of pushed commits.
- No "WIP" / "fix later" on main.

### 3. Tests must exist
CLAUDE.md: *"No commits con código sin tests (excepción documentada: WIP prototypes en una rama feature)."*

- Any new IR opcode, decoder form, pass, or emitter encoding has a `core/tests/test_*.cpp` case.
- Full suite passes: `core/build/prisma_core_tests --reporter compact ~"signal_handler*"`.
- For runtime/cache/allocator changes: sanitizer build also passes with `-DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON`.

### 4. Two-eyes territory (CONTRIBUTING.md)
These surfaces require a second reviewer even when claude/codex claim and commit themselves:

- `core/include/prisma/ir.hpp` — IR shape
- `core/include/prisma/translation_cache.hpp` — cache file format (RFC 0007 binding)
- `core/src/backend/lowering.cpp` — register allocator
- `ir-spec/**/*.lean` — Lean formal spec
- Public API surfaces in `core/include/prisma/`

If the diff touches any of these and the same agent both claimed and landed it solo: surface explicitly.

### 5. Cross-agent territory (COORDINATION.md)
- **Codex owns**: decoder + IR variants + dispatcher.
- **Claude owns**: emitter + passes + lowerer + cache + runtime + infra CI.

Cross-territory touches require either: a defensive regression test only (acceptable), an explicit RFC agreement, or a typo fix. Otherwise flag.

### 6. RFC requirement
New third-party dependency / architectural decision / file format change requires an RFC under `docs/rfc/`:

- New `FetchContent_Declare(...)` in CMakeLists → RFC needed.
- Changes to `core/include/prisma/translation_cache.hpp` byte layout → RFC 0007 needs an update + version bump.
- New `PRISMA_ENABLE_*` build options → one-line note in `core/README.md` suffices.

### 7. License hygiene
- **Zero copy** of FEX, Box64, QEMU, or any other DBT into Prisma's core. Inspiration documented in `docs/research_notes.md` is fine; verbatim/near-verbatim translation is not.
- Currently-allowed copyleft-adjacent deps: vixl (BSD-3-Clause), zstd (BSD), mathlib (Apache 2.0). New deps must be compatible with the eventual MIT core + commercial app split (Fase 6 open-sourcing).

### 8. Lean sorry budget
If the diff touches `ir-spec/`, the new `sorry` count cannot exceed `ir-spec/.sorry-budget` (currently 1). Verify with:

```bash
grep -rn --include='*.lean' '^[[:space:]]*sorry\b' ir-spec/PrismaIR | wc -l
cat ir-spec/.sorry-budget
```

Bumping the budget upward requires explicit justification in the PR description.

## Process

1. Read every changed file (don't trust the diff alone — context outside the hunk matters).
2. For each of the 8 checks, mark PASS / FAIL / N/A with file:line citations on fails.
3. If everything is green, say so explicitly and recommend merge.
4. If any two-eyes territory item is touched and only the author committed: explicitly recommend a second reviewer.

## Output format

```
PR review — <branch or commit range>

Summary: <one line — pass / hold for X / blocker on Y>

1. Backlog claim trail   — PASS/FAIL: <details>
2. Commit discipline     — ...
3. Tests                 — ...
4. Two-eyes              — ...
5. Cross-agent territory — ...
6. RFC requirement       — ...
7. License hygiene       — ...
8. Lean sorry budget     — ...

Recommendation: merge / hold / block
```

Keep terse. The author reads the diff; you add the protocol signal.
