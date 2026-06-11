# Prisma Agent Playbook

> Operational guide for AI agents (`claude`, `codex`, future ones) and
> human contributors. Consolidates the patterns this codebase has
> standardised on, with pointers into `HANDOFF.md` (the high-level
> orientation) and `docs/COORDINATION.md` (the multi-agent protocol).
>
> If you can answer the question "how do I add an IR op?" or "how do
> I validate a backend change without an ARM64 host?" from this file
> alone, the playbook works. If you can't, fix the playbook.

## Table of contents

1. [Container-based build & test workflow](#1-container-based-build--test-workflow)
2. [Adding a new IR op (the 13-file recipe)](#2-adding-a-new-ir-op-the-13-file-recipe)
3. [Adding a function-level pass](#3-adding-a-function-level-pass)
4. [Two-eyes territory & solo-author bookkeeping](#4-two-eyes-territory--solo-author-bookkeeping)
5. [Lean spec extension etiquette](#5-lean-spec-extension-etiquette)
6. [CI surface & how to read failures](#6-ci-surface--how-to-read-failures)
7. [When in doubt: the WORK_QUEUE.md contract](#7-when-in-doubt-the-work_queuemd-contract)
8. [Runtime direct-threading workflow](#8-runtime-direct-threading-workflow)

---

## 1. Container-based build & test workflow

The reference development environment for non-ARM64 hosts is a
persistent Docker container that mirrors GH Actions `ubuntu-latest`
exactly:

```bash
docker run -d --name prisma-build ubuntu:24.04 sleep infinity
docker exec prisma-build bash -c "
  apt-get update -qq &&
  apt-get install -y -qq cmake ninja-build clang git wget
"
# CMake 3.30 isn't in apt yet — install from Kitware:
docker exec prisma-build bash -c "
  wget -qO /tmp/cmake.sh \
    https://github.com/Kitware/CMake/releases/download/v3.30.5/cmake-3.30.5-linux-x86_64.sh &&
  chmod +x /tmp/cmake.sh && /tmp/cmake.sh --skip-license --prefix=/usr/local
"
# Clone + checkout the branch you intend to work on:
docker exec prisma-build bash -c "
  git clone --depth 50 --branch <BRANCH> \
    https://github.com/Finithe-Phoenix/prisma.git /workspace
"
# Configure with C++20 modules scanning disabled (we don't use modules):
docker exec prisma-build bash -c "
  cd /workspace &&
  cmake -S core -B core/build -G Ninja \
    -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    -DCMAKE_CXX_SCAN_FOR_MODULES=OFF
"
```

The current Codex container image exposes Clang 18 as
`/usr/bin/clang++`. Older persistent containers may still have
`clang++-17`; use `command -v clang++ clang++-17` and point CMake at
the compiler that actually exists.

For the mounted-workspace Docker flow used by Codex, the equivalent
fast loop is:

```bash
docker run --rm \
  -v ${PWD}:/work \
  -v prisma-build-cache:/build \
  -w /work/core \
  prisma-build-env \
  bash -lc "set -euo pipefail; cmake --build /build --parallel 2; ctest --test-dir /build --output-on-failure"
```

After that, every edit cycle is:

```bash
# Sync changed file(s) into the container:
docker cp <host-path> prisma-build:<container-path>
# Incremental build:
docker exec prisma-build bash -c "cd /workspace && cmake --build core/build"
# Targeted test run:
docker exec prisma-build bash -c "
  cd /workspace &&
  core/build/prisma_core_tests --reporter compact 'YourFilter*'
"
```

### Sanitizers (mandatory before pushing backend changes)

```bash
# One-time configure (or after CMakeLists changes):
docker exec prisma-build bash -c "
  cd /workspace &&
  cmake -S core -B core/build-asan -G Ninja \
    -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    -DCMAKE_CXX_SCAN_FOR_MODULES=OFF \
    -DPRISMA_ENABLE_ASAN=ON \
    -DPRISMA_ENABLE_UBSAN=ON
"
# Build + run under sanitizers:
docker exec prisma-build bash -c "
  cd /workspace && cmake --build core/build-asan --parallel 1 &&
  ASAN_OPTIONS=detect_leaks=1:halt_on_error=1:abort_on_error=1 \
  UBSAN_OPTIONS=print_stacktrace=1:halt_on_error=1 \
    core/build-asan/prisma_core_tests --reporter compact '~signal_handler*'
"
```

Use `--parallel 1` for sanitizer links when memory is tight; a killed
linker is an infrastructure/OOM signal, not a sanitizer failure. Re-run
the sanitizer build by itself before trusting the result.

### Zydis differential build

```bash
docker run --rm \
  -v ${PWD}:/work \
  -v prisma-build-zydis-cache:/build-zydis \
  -w /work \
  prisma-build-env \
  bash -lc "set -euo pipefail; cmake -S core -B /build-zydis -G Ninja -DCMAKE_BUILD_TYPE=Debug -DCMAKE_CXX_SCAN_FOR_MODULES=OFF -DPRISMA_ENABLE_ZYDIS=ON; cmake --build /build-zydis --parallel 2; ctest --test-dir /build-zydis --output-on-failure"
```

If ASan or UBSan reports anything other than `0 errors`, fix it
*before* pushing — the CI `core sanitizers` job is the same code
path and will block PR merge.

### What the container CANNOT do

E2E tests that JIT-compile and execute ARM64 (`test_e2e_dispatcher.cpp`
and `test_jit_execution.cpp` in particular) only run real ARM64 code
on an ARM64 host. On x86_64 hosts they take the
`if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }`
branch — they pass but don't exercise the JIT. This is fine for
catching compile-time / decoder / IR / lowering shape regressions;
for actual runtime validation route the branch through Apple Silicon
or a `linux/arm64` GitHub runner.

## 2. Adding a new IR op (the 13-file recipe)

The IR is statically dispatched via `std::variant`, and every switch
over IR variants is `-Wswitch`-exhaustive by design. Adding a new op
without touching all the right files yields a compile error at the
first missing case. The canonical order:

| # | File | What to do |
|---|------|------------|
| 1 | `core/include/prisma/ir.hpp` | Declare the `struct`, add to the `Op` variant, add `operator==` declaration. |
| 2 | `core/src/ir/operation.cpp` | `operator==` definition. |
| 3 | `core/include/prisma/profiler.hpp` | Add to `OpCounter::Kind` enum. |
| 4 | `core/src/ir/profiler.cpp` | Map your op type to its `Kind` in `kind_for(op)`. |
| 5 | `core/src/ir/pretty_print.cpp` | Add a print branch. |
| 6 | `core/src/ir/validate.cpp` | `for_each_operand_ref` (yields every `Ref` operand) **and** `op_is_pure` if applicable. |
| 7 | `core/src/ir/serialize.cpp` | New `OpKind` tag, `kind_for` mapping, `write_payload(op)`, `read_payload_<name>`, dispatch case in the read switch. Don't forget `kMaxOpKind`. |
| 8 | `core/src/passes/dce.cpp` | `is_pure_for_dce` returns `true` iff your op is side-effect free. `collect_operand_refs` enumerates every operand ref. |
| 9 | `core/src/backend/lowering.cpp` | Add a branch to `compute_liveness` (bumps operand refs) and to `lower_stmt` (emits ARM64). |
| 10 | `core/include/prisma/emitter.hpp` + `core/src/backend/emitter.cpp` | If the lowering needs a new primitive, declare + implement. |
| 11 | `core/src/decoder/x86_decoder.cpp` | Decoder branch that emits the new op. Add to the AVX-256 allowlist (decoder:2421) if the op admits `VEX.L=1`. |
| 12 | `core/tests/test_profiler.cpp` | Visit a `Stmt` of the new op in the `kCount` test. |
| 13 | `core/tests/test_decoder.cpp` + `core/tests/test_lowering.cpp` | Unit tests for both the decoder and the lowering's emitted ARM64 shape. |

**Block terminators**: if your op acts as a block terminator (must end
a block; controls `x0` on exit), additionally add it to:

- `core/src/ir/cfg.cpp::is_terminator`
- `core/src/translator/translator.cpp::is_block_terminator`

Today's terminators: `Return`, `Jump`, `CondJump`, `JumpReg`, `JumpRel`,
`CondJumpRel`, `CallRel`, `CallReg`, `RetAdjusted`, `Trap`, `Cpuid`,
`Syscall`, `InlineAsm`, `CondJumpFlags`, `RepStos`, `RepMovs`.

**Reference recipe**: commit `5756084` (Blocker A — REP STOSB/MOVSB
bounded) is the canonical example. It touched all 13 + the two
terminator files in a single coherent commit with rich rationale.

## 3. Adding a function-level pass

Two pipelines coexist:

- `PassManager` runs `std::vector<Stmt>` → `std::vector<Stmt>` passes
  (the "intra-block" pipeline; 13 passes today, including the x87
  stack-forwarding pass).
- `FunctionPassManager` (F2-PS-004) runs `ir::Function` → `ir::Function`
  passes (`global_cse`, `loop_invariant_motion` today).

Today's translator commonly emits single-block functions, so the
`FunctionPassManager` is usually cheap on translator output. It runs
correctly on hand-constructed multi-block functions (see the GCSE and
LICM tests for examples), and unlocks real wins as the translator gains
larger decoded regions.

To add a new function-level pass:

1. Declare in `core/include/prisma/passes.hpp` near the existing
   `global_cse` / `loop_invariant_motion` declarations.
2. Implement in `core/src/passes/<name>.cpp`. Use `dominators(fn)`,
   `postorder(fn)`, `natural_loops(fn)` from `prisma/dominators.hpp`
   when you need CFG analyses.
3. Add the file to the `prisma_passes` sources list in
   `core/CMakeLists.txt`.
4. Append to `default_function_pipeline()` in `pass_manager.cpp` in
   the order you want it to run. **Order matters**: GCSE before LICM,
   for instance, so that the copy idioms GCSE emits don't fool LICM's
   variant-vs-invariant classification.
5. Write tests in `core/tests/test_<name>.cpp`. Add to the
   `prisma_core_tests` sources list in `core/CMakeLists.txt`.

## 4. Two-eyes territory & solo-author bookkeeping

`CONTRIBUTING.md` requires a second reviewer on changes touching:

- `core/include/prisma/ir.hpp` (IR shape)
- `core/src/backend/lowering.cpp` (register allocator + emit logic)
- `core/src/decoder/x86_decoder.cpp` (x86 → IR decode)
- `core/src/backend/emitter.cpp` (ARM64 emit primitives)

Under the multi-agent protocol (`docs/COORDINATION.md`), Codex
normally reviews Claude's work in this territory and vice versa.
**When the other agent is unavailable**, Claude (or Codex) can take
their territory under a Danny blanket-authority waiver, but each
solo-author commit accumulates onto a *running tally* that must be
discharged before merge to `main`.

Two acceptable ways to discharge:

- **Post-hoc RFCs** (option 2 of the F2 review). One RFC per
  thematic group; e.g. `docs/rfc/0011-…` covers the AVX-256 / FMA
  commits, `docs/rfc/0012-…` covers wide-form BinOps + REP.
- **Explicit Danny waiver**: `docs/REVIEWS/<branch>-<agent>-WAIVER.md`
  listing the solo SHAs, scope, and Danny's approval. One file per
  branch; one commit by Danny.

The ledger is the "Two-eyes tally" entry in
`docs/REVIEW_F2_SESSION.md`. When new solo commits land in
two-eyes territory, append the SHA and short description to that
section. Don't pre-write a waiver document — that's Danny's call.

## 5. Lean spec extension etiquette

The Lean spec at `ir-spec/PrismaIR/` is authoritative for IR
semantics (per RFC 0001 "the implementation refines the spec"). When
you add a new IR op or BinOp variant:

1. Mirror the constructor in `PrismaIR/Syntax.lean` (`Op` or `BinOp`).
2. Extend any *exhaustive* match on the inductive in:
   - `PrismaIR/Semantics.lean::evalBinOp` (BinOp only)
   - `PrismaIR/Passes/DeadCodeElimination.lean` (`cases op with`)
   - `PrismaIR/Passes/ConstantPropagation.lean` (`cases op with`)
   Other matches (`MachineState.lean::Flags.fromBinop`, the
   `evalPure` match in Semantics, `cp_fold_op` in ConstProp,
   `Op.reads_ref` in DCE) have catch-alls — they typecheck without
   new arms.
3. Provide *real* semantics in `evalBinOp` where you can. Where the
   concrete semantics require non-trivial Int / Int64 plumbing (e.g.
   x86 signed division corner cases that diverge from ARM64 native),
   use a `sorry` placeholder and bump `ir-spec/.sorry-budget` by the
   number of new sorries. Reference an `F1-LN-XXX` backlog item in
   the comment so the placeholder is tracked.
4. CI's `ir-spec` job validates `lake build` *and* enforces
   `actual_sorries ≤ budget`. Reductions in the actual count surface
   as warnings prompting you to lower the budget. **Lower the budget
   in the same commit that retires the proof** — that's how the
   tripwire ratchets monotonically.

Canonical example: commit `b7a8f31` (wide-form BinOps + RepStos/RepMovs)
extended the spec with sorries; commit `9d1660a` closed the
exhaustive-match holes that landed simultaneously.

## 6. CI surface & how to read failures

GH Actions runs five workflows on every push to a PR branch:

| Workflow | Job(s) | Purpose | Fast feedback? |
|----------|--------|---------|----------------|
| `lint-docs` | `markdownlint`, `check-rfc-frontmatter` | Style + RFC schema. | ✅ ~30s |
| `ir-spec (Lean 4)` | `build` | `lake build` + sorry budget check. | ✅ ~30s (cached) |
| `core (C++20) — stub` | `scaffolding-check` | apt cmake/clang/ninja, build, ctest. | ⏳ ~2-3 min |
| `core sanitizers` | `asan-ubsan`, `tsan` | Two parallel sanitizer builds + tests. | ⏳ ~3-5 min |
| `shell (Rust) — stub` | `scaffolding-check` | rustup install, fmt --check, clippy -D warnings, cargo test. | ⏳ ~2 min |

**Common failure shapes:**

- `MD004` (unordered list style) — a `+` at column 1-3 of a continuation
  is read as a list marker. Reword the prose (use "and" / "paired with").
- `MD026` (trailing punctuation in heading) — drop the period.
- `clippy -D warnings` on shell-stub — `cargo fmt` first, then
  `cargo clippy --fix --allow-dirty`, then add crate-level
  `#![allow(...)]` for pedantic lints that don't apply to a skeleton.
- ir-spec `Alternative X has not been provided` — exhaustive match on
  an inductive variant; see §5.
- Sanitizer fail → fix the UB or leak. Don't silence.

For each failure, run `gh run view <runId> --log-failed | grep -iE 'error|fail'`
to get a tight digest.

## 7. When in doubt: the WORK_QUEUE.md contract

`docs/WORK_QUEUE.md` is the externalised state for the active session.
Conventions:

- The agent **picks** an item by setting its row to `🟡 in_progress`.
- The agent **completes** an item by flipping the row to ✅ and
  appending it to the "Completed (this session)" table with the
  landing SHA(s).
- The agent **adds** new items to the bottom of the queue table as
  they surface (mid-implementation discoveries, follow-up scope).
- **Don't delete rows.** Use `✅ done` or `❌ rejected` with a brief
  rationale for posterity.

Danny's review surface for "what is the agent doing right now": the
top of the queue. If it ever feels misaligned with what's actually
in the editor / commits, the agent has drifted — re-anchor.

## 8. Runtime direct-threading workflow

Direct-threading is being staged deliberately:

1. `Translator::TranslatedBlock` carries terminator metadata:
   `exit_kind`, direct target/fallthrough PCs, and call return PC.
2. `Translator::lookup_cached()` probes the executable cache without
   decoding and without mutating translation hit/miss stats. Callers
   must pass current guest bytes so the content hash rejects stale SMC
   entries.
3. `Dispatcher` first tries a hash-checked cached successor for direct
   `JumpRel`, `CondJumpRel`, `CallRel`, and bounded `RepStos` /
   `RepMovs` exits.
4. If the successor is not cached but is fetchable, `Dispatcher`
   translates it in-place and continues the direct chain. Translation
   failure and fetch failure are surfaced with the same public exit
   values as the outer loop.
5. Halt-PC checks and `max_steps` must be evaluated before every
   successor execute. Never bypass them for a faster loop.
6. `JumpRel` and `CallRel` blocks may also expose a patchable AArch64
   tail branch. The dispatcher can auto-patch only a single hop and
   only when the source block has no guest-memory writes. It verifies
   the patched target with `lookup_cached(target, current_bytes)` before
   entering the patched source, and unpatches if the target is stale, a
   halt PC, or would overrun the remaining step budget.

Useful counters:

- `direct_thread_hits`: successor was already cached and executed.
- `direct_thread_misses`: cached lookup missed for a direct successor.
- `direct_thread_installs`: direct successor was translated in-place.
- `direct_jit_patch_attempts`: dispatcher asked the translator to patch
  a one-hop tail branch after a hash-checked successor was available.
- `direct_jit_patch_applied`: the request changed an unpatched source
  into an active JIT branch.
- `direct_jit_patch_rejected`: the translator refused the request
  (for example, because it would create a chain).
- `direct_jit_patch_unpatches`: dispatcher removed an active branch
  before entry because the target was stale, halted, or over budget.
- `direct_jit_patch_executes`: dispatcher entered a source whose
  physical branch executed the one-hop target before returning.

In-JIT direct-exit patching must stay behind the same SMC hash
discipline: no branch may jump to stale code after guest bytes change.
The patch counters are exposed through both C++ `DispatchStats` and the
C/Rust ABI (`prisma_dispatch_stats`, `PRISMA_CAPI_VERSION` 2). If that
public struct grows again, bump the version and update `shell/core-sys`
plus the safe `shell/core` wrapper in the same code commit.
Do not permit multi-hop JIT chains until the dispatcher can account an
arbitrary chain while preserving halt-PC, max-step, and RAS visibility.
Do not auto-patch `CallRel` or any source containing `StoreMem*`,
`StoreVec`, `Rep*`, or `InlineAsm` until SmcGuard/page invalidation is
wired into the generic dispatcher memory model.
