# Prisma Agent Coordination Protocol

> Cómo tres agentes (CODEX, Claude, Gemini) colaboran en paralelo
> sobre el mismo repositorio sin pisarse. Documento raíz para toda
> coordinación multi-agente.

## Agent Roles

```
                    ┌─────────────────────────┐
                    │      Danny (human)       │
                    │  Dueño del producto      │
                    └────────────┬────────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
              ┌─────┴─────┐           ┌───────┴──────┐
              │   CODEX    │           │    Claude    │
              │ Decoder    │           │ IR types     │
              │ Cache      │           │ Passes       │
              │ Backend    │           │ Runtime      │
              └─────┬─────┘           └───────┬──────┘
                    │                          │
                    └──────────┬──────────────┘
                               │
                      ┌────────┴────────┐
                      │    Gemini       │
                      │  Reviewer       │
                      │  Lean cross-chk │
                      │  QA gate       │
                      └─────────────────┘
```

## Territory

| Área | Dueño | Archivos clave |
|------|-------|---------------|
| IR types (Rust) | **Claude** | `shell/prisma-ir/src/lib.rs` |
| IR types (C++) | **CODEX** | `core/include/prisma/ir.hpp` |
| IR types (Lean) | **Gemini** | `ir-spec/PrismaIR/Syntax.lean` |
| Decoder | **CODEX** | `shell/prisma-decoder/`, `core/src/decoder/` |
| Passes | **Claude** | `shell/prisma-passes/`, `core/src/passes/` |
| Cache | **CODEX** | `shell/prisma-cache/`, `core/src/cache/` |
| Runtime | **Claude** | `shell/prisma-runtime/`, `core/src/runtime/` |
| Backend | **CODEX** | `shell/prisma-backend/`, `core/src/backend/` |
| Spec formal | **Gemini** | `ir-spec/` |

## Workflow

### 1. Claim

Antes de empezar cualquier tarea, el agente debe:

```
a) Buscar en BACKLOG.md o WORK_QUEUE.md un item disponible
b) Marcarlo como [~|<agente>] en BACKLOG.md
c) Hacer commit del claim
```

### 2. Implement

Cada agente trabaja en su territorio de forma independiente.

**Reglas:**
- No modificar archivos que no son de tu territorio sin coordinación
- Si necesitas un cambio en el Op enum (IR), avisar al otro agente implementador + Gemini
- Commits pequeños y atómicos
- `cargo clippy -- -D warnings` antes de commit
- `cargo fmt` antes de commit

### 3. Review (Gemini)

Antes de mergear a main, Gemini revisa:

```
a) ¿Los tipos Rust coinciden con la spec Lean?
b) ¿Los tests differentiales existen y pasan?
c) ¿Los bloques unsafe tienen comentarios // SAFETY:?
d) ¿FFI structs usan #[repr(C)]?
e) ¿El commit message sigue el formato del proyecto?
```

Gemini documenta findings en `docs/REVIEWS/2026-<mes>-rust-<crate>.md`.

### 4. Merge

Solo mergear cuando:
- `cargo test --workspace` verde
- `cargo clippy` verde
- Gemini review approved o documented como "non-blocking"
- Suite C++ tests verde

## Conflict resolution

| Conflicto | Resolución |
|-----------|-----------|
| Dos agentes modifican mismo archivo | El claim más temprano gana; el otro rebasea |
| Op enum cambia durante implementación | Agente que cambia notifica al otro + Gemini re-check |
| Test differential falla | Agente implementador debuggea; Gemini revisa fix |
| Lean spec out of sync | Gemini actualiza spec o documenta discrepancia |

## Communication

Sin chat en tiempo real. Usar commits como mensajes:

```
feat(rust-prisma-ir): add VecFpFma op variant for FMA3 lowering

IR-009 — needed by CODEX for FMA decoder path.
Gemini: please verify Lean cross-check after this commit.
```

Tags en commit messages:
- `IR-NNN` — referencia a issue del IR spec
- `Needs review:` — pide revisión de Gemini
- `Blocks:` — indica qué otro trabajo depende de este commit

## Current SPARK assignments

Snapshot: 2026-06-11, Windows/MSVC local validation.

Shared gate:
- Rust shell is green with explicit Cargo path and C++ bridge environment:
  `cargo fmt --all --check`, `cargo +1.95.0-x86_64-pc-windows-msvc test --workspace`,
  and `cargo +1.95.0-x86_64-pc-windows-msvc clippy --workspace --all-targets -- -D warnings`.
  For workspace-wide FFI tests, set `PRISMA_CORE_LIB_DIR=core/build/Debug`
  and put `core/build/Debug` on `PATH` so `prisma_core_c.dll` is visible.
  On Windows, `scripts/validate-rust-workspace.ps1` applies that environment
  and runs the full gate.
- C++ Debug runner now builds and loads on Windows. Root cause was missing
  `__declspec(dllexport)` on the C API DLL. `prisma_core_c.dll` now exports
  the public C API symbols.
- Direct Catch2 smoke filters are green:
  `smc_guard:*`, `capi:*`, and `JitBuffer*`.
- Do not spend another SPARK cycle on `0xC000007B` unless it reproduces after
  a clean build; that issue is closed by F25-RS-008 and ready for review.

Windows CTest status:
- F25-RS-009: `catch_discover_tests` produces mojibake filters for Unicode
  Catch2 test names on Windows, so many CTest cases fail with
  `No test cases matched`. This is a test-discovery/encoding problem, not a
  DLL-loader problem.
- Windows CTest now bypasses per-case discovery and registers ASCII smoke
  tests for `smc_guard`, `capi`, and `JitBuffer`. `ctest -C Debug` is green
  for that smoke path.
- Avoid claiming "full C++ suite green" on Windows until full per-case Catch2
  discovery is repaired or replaced.

Next parallel work:
- CODEX: keep `prisma-cache`, `prisma-decoder`, and `prisma-backend` moving.
  If Windows needs granular reporting, replace the temporary smoke CTest path
  with a Unicode-safe Catch2 discovery strategy.
- CODEX backend: the current Rust backend slice includes exact ARM64 encoders
  and lowering tests for sized unsigned-offset `LoadMem`/`StoreMem`
  (`I8`/`I16`/`I32`/`I64`) plus assembler label fixups for forward/backward
  `B`, `B.cond`, `CBZ/CBNZ X`, direct IR `Jump`, boolean `CondJump`,
  `CondJumpFlags`, `BR/BLR X`, and `JumpReg` lowering. `Compare` materializes
  booleans via `CMP` + `CSET`; `CmpFlags` records an NZCV-backed flag ref and
  `CondJumpFlags` branches directly with `B.cond`. Logical `And/Or/Xor` now
  lowers to `AND/ORR/EOR Xd,Xn,Xm`, and variable `Shl/Shr/Sar/Ror` lowers to
  `LSLV/LSRV/ASRV/RORV`. `CmpFlags` now matches the C++ sub-64-bit operand
  alignment path for `I8/I16/I32` before `CMP`, with exact `CondJumpFlags`
  tests. Scalar `Mul/UMulHi/SMulHi/UDiv/SDiv` now lowers directly to ARM64
  multiply/divide instructions, and `UMod/SMod` lowers through `UDIV/SDIV`
  plus `MSUB`. Next backend work should avoid duplicating that and should
  target differential tests against the C++ backend for flags/ALU slices.
- Claude: continue `prisma-passes` and `prisma-runtime` parity work; avoid
  changing C API struct layouts without updating `PRISMA_CAPI_VERSION` and
  notifying CODEX/Gemini. The dispatcher-level cache probe/adapter over
  `prisma_cache::TranslationCache::lookup` and `SmcGuard` page/key invalidation
  state are now covered by runtime unit tests. Drained `SmcGuard` invalidation
  pages are also applied to `TranslationCache::invalidate_page` through
  `dispatcher::apply_smc_invalidations`. `dispatcher::install_translation` ties
  cache insertion, SMC registration, probing, and invalidation together before
  real execution. `Dispatcher::run_with_callbacks` now adds a no-execute
  fetch/cache/translate/install state machine with typed outcomes, and
  `GuestFetcher`/`GuestTranslator` promote that boundary into stable traits.
  `RustSmokeTranslator` wires the Rust decoder into the Rust backend for NOP,
  `mov rax, imm64`, `mov rax, rcx`, `add/sub/and/or/xor rax, rcx`,
  full `83 /0..7` `rax` imm8 coverage, plus `add rbx, imm8` and REX.B
  `cmp r11, imm8`, base-memory `add/cmp/or/and/xor [rbx], imm8`, SIB+disp8
  `add [rax + rcx*4 + disp8], imm8`, disp32 `add [rbx + disp32],
  imm8`, REX.X/B SIB `add [r8 + r9*4 + disp8], imm8`, and
  RIP-relative `add/or/and/xor [rip + disp32], imm8` without JIT execution.
  The backend now has minimal `LoadReg`/`StoreReg` lowering over
  `CpuStateFrame::gpr[]` plus `Add/Sub` register-register lowering.
  `shell/prisma-runtime/tests/smoke_differential.rs` pins Rust backend bytes
  for those one-instruction paths. `shell/core/tests/smoke_differential.rs`
  now drives the live C++ FFI translator for the same smoke subset and checks
  guest byte consumption, emitted code presence, and cache hits. The C++ decoder
  gap for `83 /0..7` is closed for the register-direct `rax` smoke set plus
  RBX/R11 REX.B, `[rbx]` ADD/CMP/logical memory, SIB+disp8 ADD/CMP, disp32,
  REX.X/B SIB, RIP-relative ADD/OR/AND/XOR smoke probes, RIP-relative CMP
  decoder coverage, and negative disp8/imm8 sign-extension; ADC/SBB are parity
  placeholders matching existing C++ behavior. The next bounded decoder/runtime
  task is designing a versioned C
  ABI extension if byte-for-byte C++ backend emission is needed.
- Gemini: review the Rust migration diffs with focus on temporary clippy
  allowances, FFI layout stability, and whether C++/Rust differential tests are
  blocked only by F25-RS-009 or by missing Rust behavior too.

## 2026-06-12 — Claude SPARK update (branch claude/rust-passes-pipeline)

Claude completó territorio **passes** + un slice de **runtime**. Commits en
`claude/rust-passes-pipeline` (sobre main, sin push/PR — el workspace completo
sigue dependiendo de los slices de codex para `--workspace`):

- `prisma-passes` **100% completo (16/16 pases)**: los 13 del default_pipeline
  con paridad C++ (orden exacto de pass_manager.cpp) + global_cse, licm,
  tail_call. Nuevo `cfg.rs` (successors/postorder/dominators CHK/natural_loops)
  vive en prisma-passes para NO tocar el prisma-ir compartido. PipelineStats
  (run_with_stats). 78 tests, clippy limpio. Revisado por codex+gemini.
- `prisma-runtime/host_features.rs`: modelo ARM64 real (11 FEAT_*) + detección
  HWCAP Linux/aarch64 + override de test. Reemplaza el shim sse2/avx2.

**Flag para CODEX (no es mi territorio, no lo toco):** `cargo clippy -D warnings`
falla en `shell/prisma-decoder/src/decode.rs:138` con `if_same_then_else`
("this `if` has identical blocks", ×2). Es WIP de codex sin commitear; bloquea
el gate de clippy del workspace. Por favor resolver en el slice del decoder.

**Pregunta para CODEX/Gemini:** ¿quieren que las primitivas CFG (dominators,
natural_loops, successors) se muevan a `prisma-ir` (territorio IR-CORE) para que
backend/decoder también las usen? Por ahora son privadas a passes. Coordinar el
move si hace falta.

## 2026-06-13 — Claude: ARM64 e2e validated + LZCNT carry correctness fix

Hito: el DBT C++ **ejecuta código traducido x86->ARM64 en la arquitectura
objetivo**. Cross-compilado a aarch64 (clang, velocidad nativa) y corrido bajo
`qemu-aarch64-static` (`scripts/docker-test-core.sh arm64-cross`): **1118/1118
test cases, 7006 assertions** verdes, incluidos los e2e que hacen JIT y EJECUTAN
el código ARM64 generado.

**FIXME(correctness) resuelto en territorio CODEX (backend/emitter):** el flag
carry de LZCNT/TZCNT estaba INVERTIDO. `count_zero_flags` emitía `cset ..., ne`
(C = src!=0) pero Intel SDM define CF = (src==0). Un `JC` tras `LZCNT/TZCNT`
tomaba la rama equivocada en ARM64 — respuesta silenciosamente incorrecta.
Invisible en x86_64 porque el test que lo ejercita
(`test_e2e_dispatcher.cpp:443`) es `if constexpr (!is_arm64)`-skip; nunca había
corrido hasta esta sesión. Fix en `emitter.cpp:373` (`ne`->`eq`). Origen:
156a664. CODEX: revisar por si hay más flags con polaridad invertida en el
mismo batch de modelado de flags.

Lección de proceso: la validación cross-arch (cross-compile + qemu-user)
encuentra bugs de corrección ARM64 que los tests x86-only saltan. Recomendado
correr `docker-test-core.sh arm64-cross` en CI o antes de merges que toquen
emitter/lowering/decoder de flags.

## Emergency stop

Si un agente detecta un bug de corrección (silent wrong answer) en
C++ o Rust, debe:

1. Commit del fix o del test que expone el bug
2. Mensaje con `FIXME(correctness):` en el commit
3. Notificar a Danny via issue/PR
4. Pausar trabajo en esa área hasta confirmación
