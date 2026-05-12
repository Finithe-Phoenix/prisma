---
name: prisma-backlog-claim
description: Automate the docs/BACKLOG.md claim → implement → done ritual per docs/COORDINATION.md. Use when starting work on a backlog item — handles the claim commit, ensures baseline tests are green, and locks the completion commit format. Saves the recurring boilerplate around every meaningful change.
---

# Prisma backlog claim

The claim/complete protocol on `docs/BACKLOG.md` is the most-repeated ritual on the project. This skill standardizes it so you can't forget a step.

## Background (COORDINATION.md)

`docs/BACKLOG.md` is the shared work queue. Every task has `F<phase>-<component>-<num>`. Status markers:

| Marker | Meaning |
|---|---|
| `[ ]` | TODO, unclaimed |
| `[~|claude]` or `[~|codex]` | IN PROGRESS, claimed |
| `[x] (<sha>)` | DONE — `<sha>` is the first commit that landed it |
| `[!] <note>` | BLOCKED |
| `[?]` | DEFERRED |

## Before claiming

Run these in parallel:

```bash
git log --oneline -20                  # what did the other agent just do?
ctest --test-dir core/build            # baseline green?
grep "\[~|" docs/BACKLOG.md            # what's currently claimed?
```

Then pick a `[ ]` item:
- Whose component is NOT under an active `[~|other-agent]` claim (reduces merge conflict risk).
- Respecting territory split: codex owns decoder/IR/dispatcher, claude owns emitter/passes/lowerer/cache/runtime/infra-CI.

## Claim step

1. Edit `docs/BACKLOG.md` — flip the chosen line:
   ```
   - [ ] F1-XX-NNN: <title>
   ```
   to
   ```
   - [~|claude] F1-XX-NNN: <title>
   ```
2. Commit **only the backlog change**:
   ```
   chore(backlog): claude claims F1-XX-NNN: <title>
   ```
   Body optional; keep it small. This commit is the claim signal.
3. Start implementing.

## Implementation phase

Per CLAUDE.md commit discipline:
- English commits.
- Format: `<scope>: <what>`, e.g. `core/passes: F1-PS-NNN tail-call fusion`.
- Atomic — each commit builds + passes tests standalone.
- No `--no-verify`, no emojis, no `--amend` of pushed commits.
- Tests required (no code without tests, except WIP on feature branches).
- For runtime/cache/allocator: also pass under `-DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON`.

## Completion step

1. Verify tests green:
   ```bash
   core/build/prisma_core_tests --reporter compact ~"signal_handler*"
   ```
   (Signal-handler test is a known macOS flake — exclude locally; CI runs it on Linux where it's stable.)

2. **Separate commit**: flip `[~|claude]` to `[x] (<sha>)` in `docs/BACKLOG.md`, where `<sha>` is the short hash of the implementation commit (or the first commit in a multi-commit series).

3. Commit message: `chore(backlog): F1-XX-NNN done in <sha>` or similar terse format.

## Abandoning

If you can't finish (blocker, scope change, handoff):

1. Flip line back to `[ ]` (or `[!] <reason>` if truly blocked).
2. Commit: `chore(backlog): claude unclaims F1-XX-NNN — <reason>`.
3. Add a short note to `docs/HANDOFFS.md` (create the file if missing) explaining where you got to.

## Merge-conflict on BACKLOG.md

Will happen. Resolution rules from COORDINATION.md:

- Both tried to claim the same item: **earlier commit wins**. Later agent pulls, reverts their claim commit, picks another item.
- Unrelated items: standard 3-way merge. Re-run ctest to confirm nothing broke.

## Cross-territory courtesy

Don't touch a component where the other agent has an active claim, unless:
- Obvious typo fix.
- Regression test that pins existing behaviour (defensive, helpful).
- Explicit RFC under `docs/rfc/` agreeing on shape change.

If you find a bug in the other agent's code: add a regression test in a separate commit. Do NOT refactor their subsystem without claiming it first.

## Output format when invoked

When asked to claim an item:

```
Pre-flight:
- Baseline tests: PASS/FAIL
- Recent log (last 20): <summary, any other-agent activity in component>
- Active claims: <list of [~|*] items>

Claiming F1-XX-NNN: <title>
Component: <decoder/passes/cache/etc>
Territory: claude / codex / shared
Conflict risk: low/med/high (and why)

[Edits docs/BACKLOG.md, commits the claim, reports SHA]

Next: implement, then call back for completion step.
```

When asked to close:

```
Closing F1-XX-NNN.
Implementation commit: <sha> <subject>
Tests: PASS
[Flips marker to [x] (<sha>), commits]
```
