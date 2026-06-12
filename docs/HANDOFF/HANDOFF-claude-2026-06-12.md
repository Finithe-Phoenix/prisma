# Handoff — Claude → Codex, 2026-06-12

Status note for the parallel session. Claude is driving the broader
"finish the project" push; this records what Claude touched so Codex's
in-flight working tree does not conflict.

## What Claude committed (on `main` → `claude/integration-main`, PR #23)

- `4e5d182` `fix(ir,backend): WriteFlagsCountZero is impure + track its
  operand liveness` — touches **only** `core/src/ir/validate.cpp` and
  `core/src/backend/lowering.cpp`. Fixes the two red core-build tests
  (`validate: WriteFlagsCountZero ...` #554, `Lowerer: WriteFlagsCountZero
  ...` #1077) introduced with the LZCNT/TZCNT flag work in `156a664`.
  Root cause: `op_is_pure()` wrongly listed the op (it is impure — nullopt
  result, materialises NZCV like CmpFlags/AluFlags) and `compute_liveness()`
  had no case for it, so its operands expired before the flag write.
- `1f89d2d` `ci(markdownlint): disable cosmetic MD060 after cli2 v23 bump`
  — touches **only** `.markdownlint.jsonc` and `.markdownlint.yaml`.

Neither commit touches any file in your uncommitted working set.

## PR #23 remaining red — resolved by YOUR uncommitted WIP, please commit

Verified in the `prisma-build-env` container (clang-18) that the **working
tree as a whole** (your uncommitted WIP + Claude's two fixes above) builds
clean and passes **1118/1118** core tests. The two still-red CI checks on
the committed branch are closed by your uncommitted changes:

- `core-build` test #943 `smc_guard: on_translate with zero guest_byte_len`
  — passes on the working tree (your uncommitted `core/src/runtime/smc_guard.cpp`
  +43 already handles the zero-length case). The test was committed red in
  the integration; your impl is uncommitted.
- `shell-check` `workspace.lints was not defined` — the committed scaffolding
  members carry a bare `[lints]` (e.g. `shell/prisma-backend/Cargo.toml:14`)
  with no `[workspace.lints]` in the root manifest. Your working tree removes
  those bare `[lints]` stanzas (the `-[lints]` lines in `git diff`). Commit
  that and `cargo fmt`/shell-check goes green.

Once you commit your WIP on top of `claude/integration-main`, PR #23 should
be fully green and `main` can be fast-forwarded; dependabot PRs #18–#22 then
close as superseded.

## Container build/test recipe (reuse this)

```bash
# persistent dev container, repo mounted at /work
docker run -d --name prisma-dev -v "<repo>:/work" prisma-build-env:latest sleep infinity
docker exec prisma-dev bash -lc \
  "cd /work && cmake -S core -B /build-clang -G Ninja -DCMAKE_BUILD_TYPE=Debug \
   -DCMAKE_CXX_COMPILER=clang++ -DCMAKE_C_COMPILER=clang && cmake --build /build-clang"
docker exec prisma-dev bash -lc "cd /build-clang && ctest"
```

Note: a default `g++` configure fails `-Werror=pedantic` on the `__int128`
const-prop folds — use clang to match CI.

## Coordination

Claude is taking new work strictly in its territory (passes / runtime /
emitter / lowerer / IR-side / infra-CI) and will claim items in BACKLOG.md
once your uncommitted edit to it lands (cannot edit it now without clobbering
your working copy). Decoder / cache / backend / Rust-migration crates remain
yours.
