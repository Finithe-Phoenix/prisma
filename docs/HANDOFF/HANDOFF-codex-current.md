# HANDOFF: Codex current checkpoint

> Snapshot for Codex/Claude coordination. This file does not claim new
> work; it records the validated local state and the next safe Codex
> lanes for the active Rust migration swarm.

Last checked: 2026-06-12, Docker + Windows/MSVC validation pass.

## Local state validated

- Branch: `main`, ahead of `origin/main` by 11 commits at the time of
  inspection.
- Working tree already contains a large active Rust migration diff across
  `shell/`, `core/`, `.github/workflows/`, `docs/`, and `scripts/`.
- `core/build/Debug` exists and contains `prisma_core_c.dll`,
  `prisma_core_tests.exe`, and the Debug import libraries needed by the
  Rust bridge.
- `scripts/validate-rust-workspace.ps1` passed when invoked with a
  process-local PowerShell execution-policy bypass:

  ```powershell
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\validate-rust-workspace.ps1
  ```

  This covered `cargo fmt --all --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings` with
  `PRISMA_CORE_LIB_DIR=core/build/Debug`.
- Windows CTest smoke path passed:

  ```powershell
  ctest --test-dir core\build -C Debug --output-on-failure
  ```

  Result: 3/3 passed (`smc_guard`, `capi`, `JitBuffer`).

- Dockerized Linux core gate passed:

  ```bash
  docker exec prisma-dev bash -lc "cmake --build /work/core/build-linux --target prisma_core_tests --parallel 2 && ctest --test-dir /work/core/build-linux --output-on-failure"
  ```

  Result: build of `prisma_core_tests` succeeded and `1156` Linux tests passed.

  ```bash
  docker exec prisma-dev bash -lc '/work/core/build-linux/prisma_core_tests --reporter compact "~signal_handler*"'
  ```

  Result: `1156` assertions in `1156` test cases, all passed.

  ```bash
  docker exec prisma-dev bash -lc "/work/core/build-linux/prisma_core_tests --reporter compact '~signal_handler*'"
  ```

  Result: filtered smoke run passed with the same pass rate; no failures.

## 2026-06-12 validation refresh (same tree, new full-run)

- `ctest --test-dir /work/core/build-linux --output-on-failure`
  and `scripts/validate-rust-workspace.ps1` re-run locally in this turn.
  - Linux: `1156` tests, `0` failed.
  - Rust workspace: `cargo fmt --check`, `cargo test --workspace`, and
    `cargo clippy --workspace --all-targets -- -D warnings`, all green.
  - Windows CTest smoke (`core/build`): `3/3` passed.
  - Note: `core/build-msvc-release-clean` still does not currently contain the
    `Release/prisma_core_tests.exe` binary for `ctest -C Release`.

## Codex territory for the active swarm

Use `docs/WORK_QUEUE.md` as the live execution overlay and
`docs/BACKLOG.md` as the claim ledger.

- `F25-RS-002` / `codex-decoder`: `shell/prisma-decoder`,
  C++ decoder parity tests, and differential fixtures.
- `F25-RS-003` / `codex-cache`: `shell/prisma-cache`, cache envelope,
  persistence, and C++ cache parity tests.
- `F25-RS-006` / `codex-backend`: `shell/prisma-backend`, ARM64
  assembler/lowerer, ABI shim, and backend differential tests.
- `F25-RS-007..009` / `codex-integracion`: Rust workspace gate,
  C++/Rust bridge validation, and Windows CTest reporting.

Avoid changing `shell/prisma-passes`, `shell/prisma-runtime`, or the
public `shell/prisma-ir` API unless the change is explicitly coordinated
with Claude and, for IR shape changes, Gemini.

## Next safe Codex work

1. Backend: start the differential harness from
   `docs/REVIEW_TEMPLATES/rust-backend-differential-plan.md`, beginning
   with `CmpFlags + CondJumpFlags` for `I8/I16/I32/I64` and scalar
   `And/Or/Xor/Shl/Shr/Sar/Ror/Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`
   `I64` snippets. Completed this turn: `Compare` now includes explicit I8/I16/I32
   operand-alignment unit coverage; next add C++/Rust byte comparison harness.
2. Decoder: broaden beyond Group 1 memory smoke coverage only after the
   current backend/runtime smoke boundary remains green.
3. Cache: continue from the current Rust implementation and add parity
   tests against the documented C++ save/load semantics before changing
   the binary format.
4. Integration: keep Windows CTest language precise. Current Windows
   CTest is green for ASCII smoke tests only; full per-case Catch2
   Unicode discovery is still deferred.

## Validation commands to rerun before Codex commits

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\validate-rust-workspace.ps1
ctest --test-dir core\build -C Debug --output-on-failure
```

If a future task changes C++ code outside the current Debug artifacts,
rebuild `core/build` first, then rerun both commands.
