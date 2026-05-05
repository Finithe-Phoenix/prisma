# CLAUDE.md

Guía para futuras sesiones de AI trabajando en este repositorio.

## Contexto del proyecto

**Prisma** es un Dynamic Binary Translator x86/x64 → ARM64 para emulación Windows en Android. Timeline 48-54 meses (abril 2026 – Q2 2030). Dev lead único: Danny.

El manifiesto técnico y las decisiones estratégicas están en [PROYECTO_PLAN_EJECUCION.md](PROYECTO_PLAN_EJECUCION.md). La investigación previa del ecosistema está en [compass_artifact_wf-*.md](compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md) — esa investigación proponía forkear Winlator; **Danny rechazó conscientemente ese camino el 2026-04-19** a favor del plan épico actual. No revisar esta decisión sin razones nuevas.

## Principios de trabajo en este repo

1. **Ambición técnica sobre pragmatismo de time-to-market.** La meta es impacto académico y salto generacional, no revenue temprano. Sugerencias tipo "mejor usa X que existe" o "forkea Y" violan el manifiesto — salvo en los decision points explícitos del plan.

2. **Research output es entregable de primera clase.** Cada pilar técnico tiene un paper asociado. Blog posts cada 2-3 meses. LaTeX drafts viven en [papers/](papers/).

3. **Correctness > performance, siempre.** El IR se especifica formalmente en Lean 4. Las optimizaciones críticas (especialmente TSO adaptativo) requieren demostración formal antes de merge.

4. **Honest failure mode.** Los decision points del plan son reales. Si un pilar no funciona, publicar resultados negativos honestamente. No esconder ni glosar.

## Convenciones de código

- **C++**: C++20, concepts, std::span, std::atomic_ref. Estilo Google con tabwidth 2 spaces. Clang-format + clang-tidy obligatorio.
- **Rust**: edition 2024 cuando esté estable, 2021 mientras tanto. `cargo clippy -- -D warnings` en CI.
- **Kotlin**: Kotlin 2.0 + Jetpack Compose. Detekt + ktlint.
- **Lean 4**: mathlib como dependencia principal. Lake como build.
- **Python**: 3.12+, black + ruff + mypy strict.

## Testing

- **C++**: Catch2 para unit tests. Cada instrucción x86 decodificada tiene test: bytes → IR → interpreter → comparar con QEMU de referencia.
- **Rust**: `cargo test` + proptest para fuzzing de parsers.
- **Kotlin**: JUnit5 + instrumentation tests para APIs Android específicas.
- **Lean 4**: las proofs son el test.
- **Fuzzing continuo**: AFL++ contra el decoder desde el momento que existe.

## Commit discipline

- Commits en inglés (research is global).
- Formato: `<scope>: <what>` (ej `core/decoder: add SIB byte handling for ModR/M`).
- Nunca commits gigantes — prefer small atomic commits.
- No commits con código sin tests (excepción documentada: WIP prototypes en una rama feature).

## Qué NO hacer

- No añadir dependencias sin justificación documentada en `docs/rfc/`.
- No copiar código de FEX/Box64/QEMU al core. Inspiración sí, copia no (licencias + originalidad técnica).
- No bypassear el fuzzing/tests para "acelerar".
- No escribir docstrings multi-párrafo. Comentarios solo para el WHY no-obvio.
- No usar emojis en código/commits.

## Subsistemas activos

Estado al **2026-05-05** (mediados de Fase 2 — SSE/SSE2 substantialmente
cubierto, ejecutando en hardware ARM64).

- **`core/`** (C++20) — DBT engine. Ya existe y compila con
  `cmake --build core/build`. Subdivisiones:
  - `prisma_ir` — IR SSA + pretty-print + validator (F1-IR-016).
  - `prisma_decoder` — x86_64 → IR. SIB, RIP-relative, REX.R/X/B,
    prefijo 0x67 address-size override, segment overrides aceptados,
    `Cpuid` pseudo-op. SSE/SSE2 cobertura amplia (F2-IR-001..022):
    PADDx/PSUBx/PAND/POR/PXOR/PMULLW (reg+mem),
    MOVDQA/MOVDQU/MOVAPS/MOVUPS/MOVAPD/MOVUPD,
    MOVD/MOVQ (GPR↔XMM), MOVSS/MOVSD,
    ADDPS/SUBPS/MULPS/DIVPS/MAXPS/MINPS/SQRTPS + PD variants,
    ADDSS/SUBSS/MULSS/DIVSS/MAXSS/MINSS/SQRTSS + SD variants,
    PCMPEQB/W/D + PCMPGTB/W/D, PSHUFD, SHUFPS/SHUFPD,
    PUNPCKL/H{BW,WD,DQ,QDQ} + UNPCKL/HPS/PD,
    PSLLW/D/Q + PSRLW/D/Q + PSRAW/D + PSLLDQ/PSRLDQ,
    CVTSI2SS/SD + CVTTSS/SD2SI + CVTSS2SD/CVTSD2SS,
    PINSRW/PEXTRW, ANDPS/ORPS/XORPS + PD variants.
  - `prisma_passes` — 10 pases en el pipeline por defecto:
    const_prop → algebraic → strength_reduce → const_prop_2 →
    redundant_load → CSE → copy_propagate → dead_store →
    branch_fold → dead_code_eliminate. Pass timing + dump hooks
    (F1-PS-016/017).
  - `prisma_emitter` — vixl-backed ARM64 emitter. ALU 3-reg,
    mul/umulh/smulh/sdiv/udiv/msub, clz/cls/rbit, rol/ror,
    ldxr/stxr/ldaxr/stlxr, casal/ldaddal (LSE), labels + branches,
    literal pool flush, sp-relative load/store.
  - `prisma_cache` — translation cache. Key = (guest_addr,
    content_hash FNV-1a). SMC-safe, LRU + byte-budget eviction,
    persistent binary format (RFC 0007), async save
    (`save_to_file_async`), SHA-256 disponible para trust envelope
    Fase 2.5, per-entry stats (hit_count, last_used_tick).
  - `prisma_runtime` — JIT memory (MAP_JIT en macOS), signal
    handlers, dispatcher loop, `HostFeatures` (FEAT_LSE/LSE2/LRCPC/
    FlagM/DotProd/CRC32 detection).
  - `prisma_translator` — facade que combina decoder + passes +
    lowerer + cache + runtime en un API público.
  - `prisma_core_tests` — 690+ Catch2 tests / 4500+ assertions.
    E2E tests verifican SSE2 ejecutando en ARM64 JIT real (Apple silicon).
    Benchmarks opt-in vía
    `[.benchmark]` tag (F1-TC-007).
- **`ir-spec/`** (Lean 4) — Spec formal del IR. Sintaxis,
  semánticas puras, 3 lemmas base. Un único `sorry` pendiente (budget
  enforced en CI, F1-TC-009).
- **`docs/rfc/`** — 0001 IR-SSA, 0002 vixl integration, 0005 CFG
  design, 0006 register allocator, 0007 cache format.
- **`fuzz/`** — AFL++ harness para el decoder (F1-TC-004).

Territorios todavía vacíos: `shell/` (Rust loader), `android/` (app
Kotlin), `server/` (Python backend P2P), `tools/benchmarks/`.

## Coordinación multi-agente

Hay dos agentes activos sobre el repo: `claude` y `codex`. Protocolo
en [docs/COORDINATION.md](docs/COORDINATION.md). Reglas clave:

- Antes de tocar un item del backlog, marcarlo `[~|<agente>]` y
  hacer commit del claim.
- Al completar, marcar `[x] (<sha>)` en el commit que lo resuelve.
- Codex dirige decoder + IR variants + dispatcher; Claude dirige
  emitter + passes + lowerer + cache + runtime + infra CI.
- Si hay conflicto de merge, resolver en favor del agente cuyo
  claim fue primero; revertir el otro y re-claim.

## Comandos frecuentes

```bash
# Build del core (Debug por defecto)
cmake -S core -B core/build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build core/build

# Tests del core
core/build/prisma_core_tests
# o con filtros:
core/build/prisma_core_tests --reporter compact ~"signal_handler*"

# Benchmarks (opt-in, tag [.benchmark])
core/build/prisma_core_tests --run-benchmarks

# Sanitizers (F1-TC-008)
cmake -S core -B /tmp/prisma-asan \
  -DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON -G Ninja
cmake --build /tmp/prisma-asan
/tmp/prisma-asan/prisma_core_tests

# Coverage (Clang-only, F1-TC-005)
cmake -S core -B /tmp/prisma-cov \
  -DPRISMA_ENABLE_COVERAGE=ON -DCMAKE_CXX_COMPILER=clang++ -G Ninja

# Verificar proofs Lean 4
cd ir-spec && lake build

# Commands que aún no existen (subproyectos no arrancados):
#   cargo build --workspace          (shell/)
#   ./gradlew assembleDebug          (android/)
```

## Memoria persistente

Este repo tiene memoria AI en `~/.claude/projects/-Users-...Prisma/memory/`. Si se te pide "recordar" algo estable del proyecto, guardarlo ahí siguiendo las reglas del sistema de memoria.
