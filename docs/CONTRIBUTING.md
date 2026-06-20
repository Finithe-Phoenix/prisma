# Contributing to Prisma

> **Status: placeholder.** Prisma is private during Fase 1 and Fase 2.
> External contributions open up around Fase 2.5 when the first paper
> drops and the repo goes public on GitHub
> (`github.com/prisma-emu/prisma`, F0-LG-006 / F0-DX-011 prereq).
>
> This file exists so the open-source on-ramp is in place before that
> moment, not after. The rules below apply today to the two AI agents
> (`claude` and `codex`) plus the dev lead (Danny). They generalise to
> human contributors when the time comes.

## Before you contribute

Read these in order:

1. [`README.md`](../README.md) — what Prisma is.
2. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — how the monorepo is
   laid out.
3. [`PROYECTO_PLAN_EJECUCION.md`](../PROYECTO_PLAN_EJECUCION.md) —
   the 48-54 month strategic plan and the six technical pillars.
4. [`CLAUDE.md`](../CLAUDE.md) — coding conventions and the working
   agreement for AI agents (which generalises cleanly to humans:
   small atomic commits, no emojis, no copying code from FEX/Box64,
   etc.).
5. [`docs/COORDINATION.md`](COORDINATION.md) — multi-agent claim
   protocol on `docs/BACKLOG.md`. If you're a human contributor,
   you'd use the same protocol with a per-contributor handle.

## Claiming work

Every meaningful change starts with a claim in
[`docs/BACKLOG.md`](BACKLOG.md):

```markdown
- [~|<your-handle>] F1-XX-NNN: <short description>
```

Then:

1. Commit the claim immediately so other contributors see it.
2. Land your changes in small atomic commits (see commit discipline
   below).
3. Mark the item done in the same commit that resolves it:
   ```markdown
   - [x] (<sha>) F1-XX-NNN: <short description>
   ```
4. If you abandon the work, revert the claim with a `chore(backlog):
   <handle> abandons FX-XX-NNN — <reason>` commit so it returns to
   the unclaimed pool.

If your idea is not in BACKLOG.md, open a discussion (Fase 2.5+ this
will be a GitHub Issue; today, ping Danny). Don't add backlog items
unilaterally — backlog growth is a strategic-plan decision.

## Commit discipline

From [`CLAUDE.md`](../CLAUDE.md):

- **English** (research is global).
- **Format**: `<scope>: <what>`. Example: `core/decoder: add SIB byte
  handling for ModR/M`.
- **Atomic commits.** Prefer many small commits over one monolithic.
  Each commit must compile + pass tests on its own.
- **No commits without tests.** Documented exception: WIP prototypes
  on a feature branch.
- **No emojis** in code, commits, or issue titles.

## Pull request requirements

A PR must:

1. **Build clean** under the default Debug build:
   ```bash
   cmake -S core -B core/build -G Ninja -DCMAKE_BUILD_TYPE=Debug
   cmake --build core/build
   ```
2. **Pass the full test suite**:
   ```bash
   core/build/prisma_core_tests --reporter compact ~"signal_handler*"
   ```
   (The signal-handler test is a known macOS flake — exclude it
   locally; CI runs it on Linux where it's stable.)
3. **Pass under sanitizers** for changes touching the runtime,
   cache, or anything that allocates:
   ```bash
   cmake -S core -B /tmp/prisma-asan \
     -DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON -G Ninja
   cmake --build /tmp/prisma-asan
   /tmp/prisma-asan/prisma_core_tests
   ```
4. **Not increase the Lean sorry budget** (see
   `ir-spec/.sorry-budget`). If you legitimately need a sorry,
   bump the budget and explain in the PR.
5. **Cite an RFC** when introducing architectural decisions or
   third-party dependencies (per
   [`docs/rfc/README.md`](rfc/README.md)).

## Code reviews

Two-eyes rule applies to anything affecting:

- The IR shape (`core/include/prisma/ir.hpp`).
- The cache file format (`core/include/prisma/translation_cache.hpp`).
- The Lowerer's allocator (`core/src/backend/lowering.cpp`).
- The Lean spec (`ir-spec/`).
- Public API surfaces.

Self-merge is fine for tests, docs, RFCs, and contained refactors
inside one subsystem.

### External review (Codex / Gemini)

For substantive diffs (decoder/IR/lowerer/runtime), run the external reviewers
as the second set of eyes before merge:

```pwsh
pwsh -File scripts/review-with-codex.ps1            # codex on origin/main...HEAD
pwsh -File scripts/review-with-codex.ps1 -WithGemini  # once Gemini is re-authed
```

Triage every finding against the C++ reference and the existing test suite
before acting — reviewers produce occasional false positives. Record the
outcome under `docs/REVIEWS/` for anything in two-eyes territory.

## Automation & merge flow

The repo is configured so routine steps are not done by hand:

- **Auto-merge is enabled.** Open a PR, then `gh pr merge <n> --squash --auto
  --delete-branch`. GitHub merges it the moment all required checks pass and the
  branch is up to date — no manual polling.
- **Head branches auto-delete on merge.** No manual branch cleanup.
- **Local pre-push validation:** `pwsh -File scripts/validate-rust-workspace.ps1`
  runs fmt + clippy (`-D warnings`) + tests across the `shell/` workspace.
- **The real cross-language gate** is the `ffi-link` family (`ffi-link`,
  `ffi-link-arm64`, `ffi-link-windows`) plus `core-build-arm64`: these run the
  C++/Rust differential and execute translated blocks on real ARM64. Treat them
  as blocking even though they are not yet in the branch-protection required set
  (adding them is a pending governance change for the repo owner).

## Style

Languages and tools per [`CLAUDE.md`](../CLAUDE.md):

| Language | Standard          | Tools                          |
|----------|-------------------|--------------------------------|
| C++      | C++20             | clang-format, clang-tidy       |
| Rust     | edition 2024/2021 | rustfmt, `cargo clippy -D warnings` |
| Kotlin   | Kotlin 2.0        | detekt, ktlint                 |
| Lean 4   | mathlib + Lake    | (proofs are the test)          |
| Python   | 3.12+             | black, ruff, mypy strict       |

Formatter and linter configs live at the repo root (or per-subproject
where they make sense).

## What NOT to do

From [`CLAUDE.md`](../CLAUDE.md):

- Don't add dependencies without a documented RFC.
- Don't copy code from FEX, Box64, or QEMU into Prisma's core.
  Inspiration yes, copy no — both for licence reasons and for
  technical originality.
- Don't bypass fuzzing or tests "to ship faster".
- Don't write multi-paragraph docstrings; comments are for the
  non-obvious WHY only.
- Don't use emojis in code or commits.

## Reporting bugs

Today: ping Danny directly (this repo is private). Once public:
GitHub Issues at `github.com/prisma-emu/prisma/issues`.

Crash reports should include:

- Host (macOS / Linux distro / Android version), CPU
  (`sysctl machdep.cpu.brand_string` or `/proc/cpuinfo`).
- Build flags (`PRISMA_ENABLE_ASAN`, etc.).
- Whether the crash reproduces under `~"signal_handler*"` exclusion.
- Test name (or guest binary + entry-point) that triggers it.

A test case that reproduces the bug is the gold-standard report.

## Security

Don't open public issues for security-sensitive bugs. Use the
process from [`docs/SECURITY.md`](SECURITY.md) once that file lands
(F2-DX-* item, not yet claimed).

For now: email Danny.

## Licence

Prisma's licence is a Fase 2.5 decision (it interacts with the
Wine/emulator legal review, F0-LG-006). Until that ships, treat the
repo as "all rights reserved" for the purposes of redistribution.
Internal contributions assign authorship to the contributor under
the eventual Prisma OSS licence.
