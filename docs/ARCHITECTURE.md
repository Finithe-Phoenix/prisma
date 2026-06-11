# Prisma architecture — one-page tour

Prisma is a dynamic binary translator (DBT) that runs x86_64 Windows
binaries on ARM64 Android devices. This document is the short-form
orientation for someone who just opened the repo. Depth lives in the
RFCs and the blog drafts; this page points you at them.

For the strategic plan (48-54 month timeline, six technical pillars,
decision points) see
[`PROYECTO_PLAN_EJECUCION.md`](../PROYECTO_PLAN_EJECUCION.md).

## The emulation stack at a glance

```
┌──────────────────────────────────────────────────────────────┐
│  Android app (Kotlin / Jetpack Compose)           android/   │
│    ↓  (JNI)                                                  │
│  Rust shell  —  PE loader, FS layer, syscall dispatch        │
│                                                      shell/  │
│    ↓  (C++ FFI)                                              │
│  Prisma core  —  x86→IR decoder, passes, ARM64 emitter,      │
│                  translation cache, runtime dispatcher       │
│                                                      core/   │
│    ↓                                                         │
│  Host ARM64 CPU  +  MAP_JIT pages  +  vixl assembler         │
└──────────────────────────────────────────────────────────────┘
            ·                                       ·
            ·  Pillar 4 · P2P translation cache     ·
            ·────────────────────────────────────────
                                ↓
                         server/ (Python)
```

## Top-level layout

```
Prisma/
├── core/              C++20 DBT engine — see "The core" below
├── ir-spec/           Lean 4 formal spec of the IR and its passes
├── shell/             Rust PE loader + syscall shims (Fase 2+)
├── android/           Kotlin / Compose app (Fase 3+)
├── server/            Python cache + telemetry backend (Fase 2.5+)
├── npu-models/        ML models for NPU-assisted optimisation
│                      (Pillar 1; Fase 2.5+)
├── papers/            LaTeX drafts of the academic outputs
├── docs/              This file, RFCs, BACKLOG, blog drafts,
│                      research notes, coordination protocol
├── fuzz/              AFL++ harness + seed corpus (F1-TC-004)
├── tools/             Scripts (benchmarks, corpus generation)
├── cmake/             Shared CMake helpers (BuildVixl.cmake)
├── third_party/       Out-of-tree deps (checked-in mirrors)
├── CLAUDE.md          Working agreement for AI sessions
└── README.md
```

Fase 0 / Fase 1 have populated `core/`, `ir-spec/`, `docs/`, and
`fuzz/`. `shell/`, `android/`, `server/`, `npu-models/`, and
`papers/` are reserved placeholders. The strategic plan says when
each opens up.

## The core

This is where the interesting code lives in April 2026.

```
core/
├── include/prisma/        Public C++ headers
├── src/
│   ├── ir/                IR data structures, pretty-print, validator
│   ├── decoder/           x86_64 bytes → Prisma IR
│   ├── passes/            IR optimisation passes (10 today)
│   ├── backend/           Emitter (vixl-backed) + Lowerer (IR → ARM64)
│   ├── cache/             Translation cache + SHA-256 + zstd
│   ├── runtime/           JIT memory, signal handlers, dispatcher,
│   │                      host feature detection
│   └── translator/        Top-level facade combining everything
├── tests/                 Catch2 tests + benchmarks
├── build/                 (git-ignored) CMake output
└── CMakeLists.txt
```

Each subdirectory becomes its own `prisma_*` static library. They
compose cleanly: `prisma_decoder` depends on `prisma_ir`,
`prisma_passes` depends on `prisma_ir`, `prisma_emitter` depends on
`prisma_ir` and a vendored `prisma_vixl`, and the `prisma_translator`
facade pulls all of them plus `prisma_cache` and `prisma_runtime`
into one public API.

### The pipeline

A guest block goes through five stages:

1. **Decoder** (`core/src/decoder/x86_decoder.cpp`) parses x86_64
   bytes into a flat `std::vector<ir::Stmt>`. Handles REX prefixes,
   SIB, RIP-relative, operand-size (0x66) and address-size (0x67)
   overrides, segment-override no-ops, and an opt-in `Cpuid` pseudo-
   op. Every recognised instruction has a test in
   `test_decoder.cpp`. The AFL++ harness (`fuzz/decoder/`) pounds
   random bytes at it continuously.

2. **Passes** (`core/src/passes/*.cpp`) rewrite the IR. Default
   pipeline in order: `constant_propagate`, `algebraic_simplify`,
   `strength_reduce`, `constant_propagate_2`, `redundant_load_
   eliminate`, `common_subexpression_eliminate`, `copy_propagate`,
   `dead_store_eliminate`, `branch_fold`, `dead_code_eliminate`.
   Each pass is a pure function `stmts → stmts`; the `PassManager`
   times every run and optionally fires dump hooks. See RFC 0006
   for the register allocator that ends up picking up after these.

3. **Lowerer** (`core/src/backend/lowering.cpp`) walks the IR and
   drives the Emitter. Uses a two-pass linear-scan register
   allocator over a 10-register scratch pool (x0..x9), with
   Belady-optimal stack spilling when the pool exhausts (F1-BK-008,
   RFC 0006).

4. **Emitter** (`core/src/backend/emitter.cpp`) wraps vixl's
   MacroAssembler. Exposes ~40 operations covering the full ALU,
   mul/div (including umulh/smulh for 128-bit products),
   clz/cls/rbit, shifts including emulated rol, atomic RMW
   (ldxr/stxr pairs + LSE cas/ldadd), load/store with size
   dispatch, labels + branches, sp-relative load/store for spills,
   and literal-pool management.

5. **Cache** (`core/src/cache/translation_cache.cpp`) keys entries
   by `(guest_addr, content_hash)` — SMC-safe by construction. LRU
   and byte-budget eviction, per-entry hit-count telemetry,
   persistent binary format v2 (RFC 0007), optional zstd compression
   (RFC 0008), async save to a writer thread, SHA-256 reserved for
   the Fase 2.5 P2P trust envelope.

   The **runtime** (`core/src/runtime/`) holds this together:
   `JitBuffer` manages MAP_JIT pages, the dispatcher loop calls
   into translated code, the signal handler catches SIGSEGV / SIGILL
   from buggy translations and unwinds via setjmp/longjmp, and
   `HostFeatures` detects FEAT_LSE/LSE2/LRCPC/FlagM/DotProd/CRC32
   so the lowerer picks the right encodings.

### Quick map: "I want to look at X"

| You want to understand…       | Start here                                      |
|-------------------------------|-------------------------------------------------|
| How a guest byte becomes IR   | `core/src/decoder/x86_decoder.cpp` + test_decoder  |
| The IR itself                 | `core/include/prisma/ir.hpp` + RFC 0001         |
| Why SSA                       | RFC 0001 + blog 002                             |
| How a pass is structured      | `core/src/passes/constant_prop.cpp` (simplest)  |
| The pass manager              | `core/src/passes/pass_manager.cpp`              |
| How IR → ARM64 happens        | `core/src/backend/lowering.cpp` + RFC 0006      |
| The emitter's API             | `core/include/prisma/emitter.hpp`               |
| vixl integration              | `cmake/BuildVixl.cmake` + RFC 0002              |
| The translation cache         | `core/include/prisma/translation_cache.hpp` + RFC 0007 |
| zstd cache compression        | RFC 0008 + `core/src/cache/compress.cpp`        |
| JIT memory on macOS           | `core/src/runtime/jit_memory.cpp` + blog 003    |
| Signal-based fault recovery   | `core/src/runtime/signal_handler.cpp` + `test_signal_handler.cpp` |
| Formal IR spec                | `ir-spec/PrismaIR/` + blog 002                  |
| CI pipeline                   | `.github/workflows/*.yml`                       |
| Multi-agent coordination      | `docs/COORDINATION.md`                          |

## The six pillars

Prisma has six concurrent technical bets, spread across the 48-54
month timeline. Three live in `core/` today:

- **Pillar 2 — Formally verified IR.** The IR is specified in Lean
  (`ir-spec/`). Passes will ship with soundness proofs. Today:
  syntax, semantics, and three base lemmas land. Proofs for
  `constant_propagate` and `dead_code_eliminate` are next
  (F1-LN-010, F1-LN-011).
- **Pillar 3 — TSO-adaptive pass.** Downgrades acquire/release
  loads to plain loads in provably single-threaded regions. Lives
  behind Pillar 2; starts landing in Fase 2.5.
- **Pillar 4 — Distributed translation cache.** Local cache is
  here (RFC 0007). Fase 2.5 wraps it in a signed-chunk P2P
  protocol (server lives in `server/`, not yet opened).

The other three pillars (NPU-assisted translation, DBT+KVM hybrid,
advanced graphics translation) are Fase 2.5+ concerns; their
placeholder directories exist but aren't populated.

## Coordination

Two agents (`claude` and `codex`) collaborate on this repo. The
protocol lives in [`docs/COORDINATION.md`](COORDINATION.md):

- Claim a backlog item by marking it `[~|<agent>]` in
  [`docs/BACKLOG.md`](BACKLOG.md) and committing the claim.
- On completion, flip it to `[x] (<sha>)`.
- Territory split today: Codex owns decoder + IR variants +
  dispatcher; Claude owns emitter + passes + lowerer + cache +
  runtime + infra CI.
- Conflicts: the earlier claim wins; the other agent reverts and
  re-claims.

[`CLAUDE.md`](../CLAUDE.md) has the agent-facing working agreement
(coding conventions, build commands, what NOT to do).

## How to build things

```bash
# Core (Debug by default)
cmake -S core -B core/build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build core/build
core/build/prisma_core_tests

# Lean proofs
cd ir-spec && lake build

# Sanitizer build (F1-TC-008)
cmake -S core -B /tmp/prisma-asan \
  -DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON -G Ninja
cmake --build /tmp/prisma-asan
```

More recipes in [`CLAUDE.md`](../CLAUDE.md).

## Reading order

If you have an hour, in order:

1. Skim [`README.md`](../README.md) for the elevator pitch.
2. This file.
3. [`docs/rfc/0001-ir-ssa-over-template-based.md`](rfc/0001-ir-ssa-over-template-based.md)
   for the IR decision that shapes everything else.
4. [`docs/blog-drafts/001-x86-to-arm64-one-instruction.md`](blog-drafts/001-x86-to-arm64-one-instruction.md)
   for a walkthrough of a single instruction end-to-end.
5. [`docs/BACKLOG.md`](BACKLOG.md) for where we are and what's next.

If you have a day, add the other RFCs (0002-0008) and the blog
drafts.

If you have a month, start implementing the items in BACKLOG marked
`[ ]` — or pick one of the open questions at the end of the RFCs and
write a follow-up RFC.

## Late-F2 update (2026-05-13)

A few moving parts have landed beyond what the original architecture
overview above describes. This subsection is the catch-up patch; the
single source of truth for "what does the agent do next" is
[`WORK_QUEUE.md`](WORK_QUEUE.md), and the operational how-to is
[`AGENT_PLAYBOOK.md`](AGENT_PLAYBOOK.md).

### Function-level pass infrastructure (F2-PS-003 / F2-PS-004)

The stmt-level `PassManager` has been joined by a parallel
`FunctionPassManager` (`core/src/passes/pass_manager.cpp` +
`global_cse.cpp` + `licm.cpp`). The Translator now runs CFG
construction (`ir::build_cfg`) between the decoder and the stmt-level
pipeline:

```
decode_until_terminator → flat stmts
build_cfg(stmts)         → ir::Function (multi-block when the stream
                            contains Cpuid / Syscall / Trap / RepStos /
                            InlineAsm / CondJumpFlags split points)
function_pipeline.run    → global_cse → loop_invariant_motion
flatten back             → stmts
stmt pipeline (the 12)   → optimised stmts
lower + JIT
```

The default function pipeline carries the dominator-tree-forwarded
GCSE (single-predecessor / unique-idom gate; conservative for
join blocks) and LICM (unique non-loop predecessor of the header;
multi-entry loops skipped). Both are no-ops when the translator
produces a single-block function (the common case today), which
makes the wiring safe for back-compat.

### Real CALL/RET semantics (F2-IR-054 + F2-BK-010)

The translator exposes `set_real_call_ret(bool)` and defaults it on.
When enabled, the decoder emits first-class `CallRel`, `CallReg`, and
`RetAdjusted` terminators for CALL rel32, CALL r/m64, RET, and RET
imm16. Lowering performs the guest-stack push/pop directly and returns
the callee or popped return target in `x0`.

The Dispatcher has `install_halt_return_stack()` which allocates an
internal zero-initialised stack and points `state.gpr[Rsp]` at it; the
outermost RET pops a 0 (= `kHaltSentinel`) and the dispatcher exits
cleanly. Translator block metadata also exposes call/ret terminator
shape, and the dispatcher keeps RAS-style push/pop/hit/miss counters
for return prediction work.

The first direct-threading stages are live in the dispatcher. For
direct `JumpRel`, `CondJumpRel`, `CallRel`, and bounded `RepStos` /
`RepMovs` exits, block metadata records the valid target and
fallthrough guest PCs. If the selected successor is already translated
and the current guest bytes still match the cached content hash, the
dispatcher executes that cached block immediately. If the successor is
not cached yet, the dispatcher translates it in-place and continues the
direct chain without bouncing through the outer dispatch loop.

For `JumpRel` and `CallRel`, the translator emits a patchable AArch64
tail slot (`b fallback`) after restoring the block ABI. The dispatcher
may patch that slot to a translated successor, but only as a one-hop
chain: before entering a patched source it re-reads and hash-checks the
target bytes, and it unpatches if the target is stale, a halt PC, or
would consume more steps than remain. This keeps SMC safety, halt-PC
checks, return-stack bookkeeping, and step limits visible while still
cutting the C++ dispatch round trip on hot direct edges.

### Blocker A — REP STOS / MOVSB DoS clamp

`RepStos` and `RepMovs` (commit 5756084) are now **block terminators**
that clamp per-call iteration count at
`kRepMaxBytesPerCall` (= 16 MiB worth of stores). When the count
hasn't drained, the emitted block exits with `x0 = pc_of_rep` so the
dispatcher re-enters the same REP for another bounded slice. This
matches x86's REP-is-interruptible semantics exactly and removes the
unbounded-loop DoS risk.

### Lane-crossing AVX-256 (VecTbl2)

A new general-purpose IR op `VecTbl2 { src_lo, src_hi, idx }` lowers
to NEON `tbl` over a two-vector source (with V30/V31 fixed scratch
for the adjacency requirement). VPERMQ ymm synthesises a compile-
time byte-index `VecConstant`; VPERMD ymm builds the byte-index at
runtime via `VecBinOp{And, …} → VecShiftImm{Shl, …} →
VecPshufb{…} → VecBinOp{Add, …}` before invoking VecTbl2. Same
primitive is the natural lowering target for future VPGATHER work.

### Two-eyes ledger

CONTRIBUTING.md's two-eyes rule applies to IR / decoder / lowering /
emitter changes. When Codex is unavailable and Claude proceeds
single-author under Danny's blanket authority, the running tally of
solo SHAs lives in [`REVIEW_F2_SESSION.md`](REVIEW_F2_SESSION.md)
under "Two-eyes tally". Each waiver pass either lands as a Codex
audit or a single `docs/REVIEWS/F2-CLAUDE-WAIVER.md` co-signed by
Danny.

### Container-based development for non-ARM64 hosts

The development workflow now supports `linux/amd64` Docker
containers as a mirror of the GH Actions `ubuntu-latest` runner.
Setup recipe in [`AGENT_PLAYBOOK.md`](AGENT_PLAYBOOK.md) §1. E2E
JIT tests still need real ARM64 (Apple silicon or a future
`linux/arm64` runner); decoder / IR / lowering shape tests + the
full sanitizer suite run fine on x86_64.

## FFI bridge update (2026-06-09)

### The C++/Rust hybrid boundary (RFC 0014)

The Rust shell now drives the C++ core through a C ABI:

```
shell/orchestrator (pe_loader, config, integrity)
        |  safe Rust API
shell/core (prisma-core: RAII handles, Result errors, lifetimes)
        |  hand-written extern "C" decls
shell/core-sys (prisma-core-sys)
        |  C ABI - opaque handles, status codes, version gate
core/include/prisma/capi.h  ->  libprisma_core_c.so
        |  C++20
prisma_translator + prisma_runtime (Dispatcher)
```

Boundary rules live in [RFC 0014](rfc/0014-ffi-boundary-core-shell.md):
no exception or panic crosses in either direction (the Rust guest
memory reader trampoline converts panics into fetch faults), out
params are written only on `PRISMA_OK`, and `PRISMA_CAPI_VERSION`
gates startup. No cxx / bindgen / Corrosion - the surface is small
enough to maintain by hand, and the cross-language integration tests
(`shell/core/tests/`) catch drift as a link or behaviour failure.

The milestone test is `shell/core/tests/pe_e2e.rs`: a synthetic
x86-64 PE32+ is parsed and mapped by the orchestrator loader in safe
Rust, then translated (every host) and executed (ARM64 hosts) by the
DBT through the bridge. The `ffi-link-arm64` CI job runs this on a
real ARM64 runner, so guest execution is now continuously verified
in CI - not just on a developer Apple Silicon machine.

Build workflow (two commands, builds stay separate by design):

```bash
cmake --build core/build --target prisma_core_c
PRISMA_CORE_LIB_DIR=$PWD/core/build \
LD_LIBRARY_PATH=$PWD/core/build \
  cargo test --manifest-path shell/Cargo.toml --workspace
```
