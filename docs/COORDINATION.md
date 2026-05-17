# Coordination protocol — Claude + Codex (+ future agents)

> Two or more AI agents work on this repo in parallel. This file is the
> rulebook that lets them avoid stepping on each other and lets Danny
> track who did what.

## Single source of truth

[`docs/BACKLOG.md`](BACKLOG.md) is the shared work queue. Every task has a
unique ID (`F<phase>-<component>-<num>`). Status markers:

For autonomous follow-on sessions, [`docs/WORK_QUEUE.md`](WORK_QUEUE.md)
is the active-session overlay. Use it for the exact current execution
order, partial runtime stages, and per-session completion ledger; keep
`BACKLOG.md` as the lifetime phase map and cross-agent claim protocol.

| Marker | Meaning | Owner visible? |
|---|---|---|
| `[ ]` | TODO, unclaimed. Either agent may take. | no |
| `[~|<agent>]` | IN PROGRESS, claimed by the named agent. | yes |
| `[x] (<sha>)` | DONE. `<sha>` is the first commit that landed it. | log |
| `[!] <note>` | BLOCKED. Note explains on what. | reason |
| `[?]` | DEFERRED. | — |

`<agent>` is `claude` or `codex` today. New agents extend the set.

## Before claiming

1. `git pull` (once we have a remote) or `git log --oneline | head -20`
   locally. Look for recent commits from the other agent that might
   affect your work.
2. Run `ctest --test-dir core/build` to confirm baseline is green.
3. `grep "\[~|" docs/BACKLOG.md` to see what is currently claimed.
4. Pick a `[ ]` item whose ID is NOT in the same component as an active
   `[~|other-agent]` claim (reduces merge conflict risk).

## Claiming

1. Edit `docs/BACKLOG.md`: flip `[ ]` to `[~|claude]` (or `[~|codex]`)
   on the line you're taking.
2. Commit **just the backlog change**:

   ```
   chore(backlog): <agent> claims F1-XX-NNN: <title>
   ```

   This commit is a claim signal to the other agent. Keep it small.
3. Start implementing.

## Completing

1. Make sure tests are green locally.
2. Commit the code with a normal message. No special tag needed.
3. **Separate commit**: flip `[~|<agent>]` to `[x] (<sha>)` in the
   backlog, where `<sha>` is the short hash of the code commit.
4. If your change produced multiple commits, cite the first one or the
   merge/squash commit.

## Abandoning

If you can't finish a claim (blocker, scope change, handed off):

1. Flip the line back to `[ ]` (or `[!] <reason>` if truly blocked).
2. Commit with `chore(backlog): <agent> unclaims F1-XX-NNN: <reason>`.
3. Add a short note to [`docs/HANDOFFS.md`](HANDOFFS.md) (create if
   missing) explaining where you got to.

## Merge conflicts on `BACKLOG.md`

They will happen. When they do:

- If both agents tried to claim the same item: the earlier commit wins.
  The later agent pulls, reverts its claim, picks another item.
- If both agents changed unrelated items: standard 3-way merge resolves
  it. Re-run `ctest` to confirm nothing broke.

## Don't step on each other

**Soft rules** (not enforced by anything but common sense):

- Don't touch a component where the other agent has an active claim
  unless it's an obvious typo fix or a test that pins the existing
  behaviour (the CmpFlags DCE regression test in `a9327b1` is a good
  example — Codex touched `core/tests/test_passes.cpp` defensively
  after Claude's core change in `a52cbe0`).
- If you find a bug in the other agent's code, add a regression test
  under `tests/` and ship it as a separate commit. Do **not** refactor
  their subsystem without claiming it first.
- If you need an API you see the other agent building, coordinate via
  a new RFC under `docs/rfc/` rather than assuming shape.

## Attribution

Git author is the agent that wrote the commit. Commit messages may
optionally add a note about cross-agent collaboration (e.g. "Fixes a
bug introduced in `<sha>` by <other-agent>").

When Danny reviews `git log --oneline`, a healthy two-agent stream
looks like alternating authorship with occasional test-only commits
from the non-owner agent.

## Current claims (live)

See [`BACKLOG.md`](BACKLOG.md). Grep for `[~|`.
