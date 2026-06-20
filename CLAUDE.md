# CLAUDE.md

Guía para futuras sesiones de AI trabajando en este repositorio.

## Snapshot operativo actual

Ultima actualizacion: 2026-06-19 America/Mexico_City.

- **Estado del proyecto:** ver [docs/STATUS.md](docs/STATUS.md) (mapa con
  evidencia archivo:linea) y [docs/ROADMAP.md](docs/ROADMAP.md) (plan completo
  por fases + pila de tareas). Tecnicamente estamos a **mitad de Fase 2**: el
  core C++ es un DBT x86->ARM64 funcional que **JIT-ejecuta en ARM64 real**;
  hay una migracion a Rust en marcha (`shell/`, RFC 0015) con el primer motor
  de traduccion integrado `prisma-translator` (decode -> optimizar -> lower ->
  cache + fusion de bloques con renumeracion SSA).
- **Toolchain en la maquina Windows de Danny:** ya hay `cargo` nativo (1.96+).
  El workspace `shell/` completo compila, testea, fmt y clippea en local. El
  build script de los crates FFI tolera la ausencia del DLL, asi que
  `cargo clippy --workspace --all-targets -D warnings` corre el gate completo
  sin el DLL C++ (solo el *link* de los tests FFI lo necesita). Los paths
  `#[cfg(unix)]` no compilan en Windows: esos lints solo afloran en `ffi-link`
  (Linux) de CI. No hay cmake/ninja/clang/lake nativos: para C++/Lean usar CI o
  Docker.
- `docs/BACKLOG.md` ya no debe contener marcadores `pending commit`; si aparece
  uno nuevo, auditarlo antes de tomar mas trabajo.
- Los workflows deben correr en todo PR para poder ser required checks sin
  quedarse en `Pending` por path filters. Los contexts esperados son
  `core-build`, `core-build-arm64`, `asan-ubsan`, `tsan`, `ir-spec-build`,
  `ffi-link`, `ffi-link-arm64`, `ffi-link-windows`, `markdownlint`,
  `check-rfc-frontmatter`, `shell-check` y `benchmarks-smoke`. **`ffi-link`
  (ubuntu) es el gate real cross-language** (clippy --workspace + test
  --workspace contra el DLL C++ via RFC 0014). `ffi-link-windows` esta fijado a
  `windows-2022` (el runner `windows-latest` perdio Visual Studio 2022).
- Antes de empujar a `main`, validar en una rama/PR de integracion y esperar
  GitHub Actions verde.

## Trabajo con agentes en paralelo

El trabajo de programacion se organiza con **agentes en paralelo** sobre
fronteras de archivos cerradas (ver [docs/ROADMAP.md](docs/ROADMAP.md) §1 y
[docs/COORDINATION.md](docs/COORDINATION.md)):

- Fan-out tipico para una familia de instrucciones nueva: 1 agente decoder, 1
  backend/lowering, 1 tests/differential — sincronizados por el IR. Ningun
  agente toca el modulo de otro.
- Fan-out para revision/auditoria: N agentes por dimension (correctness,
  seguridad, perf) + verificacion adversarial + sintesis.
- Cada agente declara ownership en `docs/BACKLOG.md` (`[~|<agente>]`) y commitea
  el claim antes de tocar codigo; al terminar marca `[x] (<sha>)`.
- Split de territorio: Claude dirige emitter + passes + lowerer + cache +
  runtime + infra CI; Codex dirige decoder + IR variants + dispatcher + backend.

## Contexto del proyecto

**Prisma** es un Dynamic Binary Translator x86/x64 → ARM64 para emulación Windows en Android. Timeline 48-54 meses (abril 2026 – Q2 2030). Dev lead único: Danny.

El manifiesto técnico y las decisiones estratégicas están en [PROYECTO_PLAN_EJECUCION.md](PROYECTO_PLAN_EJECUCION.md). La investigación previa del ecosistema está en [compass_artifact_wf-*.md](compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md) — esa investigación proponía forkear Winlator; **Danny rechazó conscientemente ese camino el 2026-04-19** a favor del plan épico actual. No revisar esta decisión sin razones nuevas.

## Principios de trabajo en este repo

1. **Ambición técnica sobre pragmatismo de time-to-market.** La meta es impacto académico y salto generacional, no revenue temprano. Sugerencias tipo "mejor usa X que existe" o "forkea Y" violan el manifiesto — salvo en los decision points explícitos del plan.

2. **Research output es entregable de primera clase.** Cada pilar técnico tiene un paper asociado. Blog posts cada 2-3 meses. LaTeX drafts viven en [papers/](papers/).

3. **Correctness > performance, siempre.** El IR se especifica formalmente en Lean 4. Las optimizaciones críticas (especialmente TSO adaptativo) requieren demostración formal antes de merge.

4. **Honest failure mode.** Los decision points del plan son reales. Si un pilar no funciona, publicar resultados negativos honestamente. No esconder ni glosar.

## Cláusula obligatoria: disciplina de memoria y recursos

**SIEMPRE liberar la memoria y los recursos del SO de forma determinista.** No es
opcional. El servidor (runtime host / backend P2P en `server/`) se reinicia, y un
recurso filtrado **no debe sobrevivir un reinicio, filtrarse entre reinicios, ni
corromper el estado**.

Aplica a todos los recursos sensibles a reinicio que Prisma maneja:

- **Memoria ejecutable JIT** (MAP_JIT / mmap / `ExecBuffer` / `JitSlabPool`): los
  buffers W^X se desmapean en su `Drop` / al evictar.
- **Mapeos del guest** (PE loader, espacio de direcciones invitado).
- **File handles y archivos de la translation cache** (RFC 0007): flush + close
  limpio.
- **Entradas en memoria de la cache**: liberadas en eviction LRU/byte-budget y en
  `clear_cache` / invalidación SMC.

Reglas:

- Cada allocación tiene un dueño cuyo `Drop` (Rust) / destructor RAII (C++) la
  libera. Preferir RAII sobre free manual. **Nada de leaks.**
- Las rutas de shutdown/restart liberan **explícitamente** (flush → close →
  unmap); no confiar en que la salida del proceso reclame (un reinicio puede no
  ser una salida limpia).
- Nada de `mem::forget` (Rust) ni ownership-leak sobre un recurso del SO sin
  justificación documentada en `docs/rfc/`.
- Código nuevo que asigna memoria ejecutable / mmaps / fds debe añadir el `Drop` +
  un test (o chequeo ASan/leak) de que el recurso se libera.

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

Estado al **2026-05-30** (linea avanzada de Fase 2 — SSE/SSE2/AVX/FMA
substantialmente cubiertos, ejecutando en hardware ARM64).

- **`core/`** (C++20) — DBT engine. Ya existe y compila con
  `cmake --build core/build`. Subdivisiones:
  - `prisma_ir` — IR SSA + pretty-print + validator (F1-IR-016).
  - `prisma_decoder` — x86_64 → IR. SIB, RIP-relative, REX.R/X/B,
    prefijo 0x67 address-size override, segment overrides aceptados,
    `Cpuid` pseudo-op. SSE..SSE4.2 + BMI1 cobertura amplia
    (F2-IR-001..047): toda la aritmética packed + scalar integer y FP
    (incluido sat, min/max, mul-high, PMULUDQ, PSADBW, PMULLD), movs
    (DQA/U, APS/UPS, APD/UPD, DDUP/SLDUP/SHDUP, MOVD/Q GPR↔XMM,
    MOVSS/SD, LDDQU, MOVNT*), ADDPS/PD/SS/SD + SUB/MUL/DIV/MAX/MIN/
    SQRT, HADDPS/PD, CMPxxPS/PD/SS/SD (8 predicates) + UCOMISS/UCOMISD
    con FP-source flag plumbing, PCMPEQ/GT B/W/D + SSE4.1 PCMPEQQ +
    SSE4.2 PCMPGTQ, PSHUFD, PSHUFLW/HW, SHUFPS/PD, PSHUFB, PUNPCKL/H +
    UNPCKL/HPS/PD, PSLLW/D/Q + PSRL + PSRA + PSLLDQ/PSRLDQ, PALIGNR,
    PHADDW/D + PHSUBW/D, CVT* int↔FP + single↔double, PINSR/PEXTR
    B/W/D/Q, PMOVMSKB, MOVMSKPS/PD, PMOVZX/SX × 12, ROUNDPS/PD/SS/SD,
    PABSB/W/D, ANDPS/ORPS/XORPS + PD, PMINUB/PMAXUB/PMINSW/PMAXSW +
    SSE4.1 PMINS B/D + PMINU W/D + PMAXS B/D + PMAXU W/D,
    PBLENDVB/BLENDVPS/BLENDVPD, PTEST, POPCNT, LZCNT/TZCNT (BMI1).
    F2-IR-048 — AVX-128 (VEX C4/C5 prefixes parsed; vvvv como tercer
    operando no-destructivo wired en PADDx/PSUBx/POR/PXOR/PAND/+ sat
    /min/max/mul-high/PMULUDQ/PSADBW/PHADDx/PMULLD/ANDPS family,
    ADDPS/SUBPS/MULPS/DIVPS + PD + SS/SD, PCMPEQ/GT, UNPCKL/H,
    SHUFPS/PD, HADDPS/PD, CMPxxPS/PD/SS/SD, PSHUFB, PALIGNR).
    F2-IR-005 — AVX-256 first batch (VEX.L=1 admitted via opcode
    allowlist; pair-of-Vec128 representation via new `ymm_hi[16]`
    in `CpuStateFrame` + `LoadVecRegHi`/`StoreVecRegHi` IR ops).
    Active: VADDPS/PD, VSUBPS/PD, VMULPS/PD, VDIVPS/PD, VSQRTPS/PD,
    VMINPS/PD, VMAXPS/PD, VANDPS/PD, VORPS/PD, VXORPS/PD,
    VPADDB/W/D/Q, VPSUBB/W/D/Q, VPAND, VPOR, VPXOR, VPCMPEQB/W/D,
    VPCMPGTB/W/D, VUNPCKL/HPS/PD, VPUNPCKL/H BW/WD/DQ/QDQ,
    VSHUFPS/PD, VHADDPS/PD, VCMPxxPS/PD ymm. Lane-crossing ops
    (VBROADCAST*, VINSERTF128, VEXTRACTF128, VPERM2F128) deferred.
    F2-IR-006 — FMA3 completo (VEX C4-only): MADD/SUB/NMADD/NMSUB
    × 132/213/231 × {PS,PD}×{xmm,ymm} + scalar SS/SD + MADDSUB/
    MSUBADD packed (96 instrucciones). VecFpFma (packed) +
    VecFpScalarFma (con scalar_upper). MADDSUB/MSUBADD lower a dos
    VecFpFma (add+sub) + VecBlend con máscara alterna construida via
    VecConstant 128-bit completo. Lane-crossing AVX-256:
    VBROADCASTSS/SD/F128, VINSERTF128, VEXTRACTF128, VPERM2F128 —
    todos sin nuevos IR ops. F2-BK-007 — MUL/IMUL escriben rdx:rax
    completo (UMulHi/SMulHi); DIV/IDIV escriben quotient + remainder
    (UDiv/SDiv + UMod/SMod). const_prop folds con __int128;
    algebraic_simplify maneja x*0/0*x/0/x/0%x/x%1 → 0 para los
    nuevos kinds.
    F2-IR-005 ymm follow-up — toda la familia integer SIMD packed
    extendida a L=1: PADDS/PSUBS sat, PADDUS/PSUBUS, PMINU/PMAXU,
    PMINS/PMAXS, PMULL{W,D}, PMULH{W,UW}, PMULUDQ, PSADBW, PHADDW/D,
    PHSUBW/D, PCMPEQQ, PCMPGTQ, PMIN/PMAXSB/D + PMINUW/D + PMAXUW/D
    (SSE4.1), PSHUFB, PABSB/W/D, PSHUFD, PSHUFLW/HW, PALIGNR ymm.
    Plus VMOVDQA/U + VMOVAPS/UPS + VMOVAPD/UPD ymm (memory and
    register forms), VMOVDDUP / VMOVSLDUP / VMOVSHDUP ymm,
    VLDDQU / VMOVNT{DQ,PS,PD} ymm, VPMOVMSKB ymm + VMOVMSKPS/PD ymm
    (combinados via shift+OR de las dos máscaras de 128-bit),
    VROUNDPS/PD ymm, VPMOVZX/SX BW/BD/BQ/WD/WQ/DQ ymm (lane-crossing
    via VecShiftBytes para sintetizar el high half source).
    VecConstant lowering ahora carga 128 bits completos (lo + INS
    hi) — antes silenciosamente truncaba.
  - `prisma_passes` — 13 pases en el pipeline por defecto
    (`pass_manager.cpp:106-145`): const_prop → algebraic →
    strength_reduce → peephole → const_prop_2 → redundant_load →
    CSE → x87_stack_eliminate → copy_propagate → dead_store →
    branch_fold → flag_write_elim → dead_code_eliminate. Pass
    timing + dump hooks (F1-PS-016/017).
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
  - `prisma_core_tests` — 800+ Catch2 tests / 5004+ assertions.
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
