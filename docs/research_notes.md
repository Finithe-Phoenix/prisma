# Research Notes — Prisma DBT

> Notas de investigación personales de Danny mientras construye Prisma. Este documento es el entregable central de Fase 0 y crece continuamente después.

**Formato**: por cada fuente leída, una sección con (1) resumen en 3-5 bullets, (2) lecciones aplicables a Prisma, (3) preguntas abiertas que generó. **No copiar y pegar** — síntesis en tus palabras. Si no puedes resumir algo, no lo entendiste.

**Target Fase 0:** 40-60 páginas. Publicar como blog post al fin de Fase 0.

---

## Reading list estructurada

Marcar cada item con su estado: ⬜ pendiente | 🟡 en progreso | ✅ completado. Al completar, añadir enlace a la sección de notas correspondiente abajo.

### Tier 1: Código fuente de producción (obligatorio, profundo)

- 🟡 **FEX-Emu** — `FEXCore/Source/Interface/Core/` (IR, passes, threading). *Scan AI-asistido completo (ver [notas](#fex-emu-scan-inicial-ai-asistido)); lectura personal profunda pendiente.*  
  *Repo:* https://github.com/FEX-Emu/FEX (clonado en `~/Documents/sandbox/prisma-research/FEX`)  
  *Target:* entender IR SSA, multipass optimization, cómo maneja TSO. ~20h.
- 🟡 **Box64** — `src/dynarec/arm64/`. *Scan AI-asistido completo (ver [notas](#box64-scan-inicial-ai-asistido)); lectura personal profunda pendiente.*  
  *Repo:* https://github.com/ptitSeb/box64 (clonado en `~/Documents/sandbox/prisma-research/box64`)  
  *Target:* dynarec de 4 pases, estrategia de wrappers nativos para libc/Mesa. ~15h.
- ⬜ **Wine** — `loader/` + `dlls/ntdll/`.  
  *Repo:* https://gitlab.winehq.org/wine/wine  
  *Target:* cómo carga PE, cómo implementa subset de Win32 API. ~10h.
- ⬜ **DXVK** — arquitectura general, cómo traduce D3D11 state a Vulkan.  
  *Repo:* https://github.com/doitsujin/dxvk  
  *Target:* inspiración para Pilar 6 (graphics translation avanzada). ~8h.

### Tier 2: Papers académicos fundacionales (obligatorios)

- ⬜ **Transmeta Crusoe Code Morphing Software**  
  *Relevancia:* el DBT más ambicioso que ha existido. Modelo de referencia.  
  *Búsqueda:* "Transmeta Crusoe Code Morphing Software" (IEEE Micro 2000, Klaiber).
- ⬜ **IA-32 Execution Layer** (Intel 2003, para Itanium)  
  *Relevancia:* el fracaso más instructivo. Por qué no funcionó técnicamente.
- ⬜ **TOSTING: Investigating Total Store Ordering on ARM** (Springer 2023)  
  *Relevancia:* 8.94% cost de TSO por hardware. Base cuantitativa para el Pilar 3.
- ⬜ **Valgrind VEX IR**  
  *Fuente:* http://www.valgrind.org/docs/manual/ — design document.  
  *Relevancia:* referencia de IR minimalista. Inspiración para `ir-spec/`.
- ⬜ **DynamoRIO architecture papers** (Bruening, MIT PhD thesis 2004)  
  *Relevancia:* referencia clásica de DBT infrastructure.
- ⬜ **Pin: Building Customized Program Analysis Tools** (Luk et al, PLDI 2005)  
  *Relevancia:* dispatch indirecto, side-exit handling.

### Tier 3: Memory models y concurrency (crítico para Pilar 3)

- ⬜ **Sarkar, Sewell et al — "The Semantics of x86-CC Multiprocessor Machine Code"**  
  *Relevancia:* especificación formal del memory model x86. Base para TSO adaptativo.
- ⬜ **Batty et al — "Mathematizing C++ Concurrency" (POPL 2011)**  
  *Relevancia:* cómo especificar formalmente memory models. Inspiración metodológica.
- ⬜ **ARM Architecture Reference Manual, sección B2 (Memory Ordering)**  
  *Relevancia:* entender LDAR/STLR/DMB en detalle.

### Tier 4: Formal methods para compilers/DBT (para Pilar 2)

- ⬜ **CompCert: A Formally Verified C Compiler** (Leroy, CACM 2009)  
  *Relevancia:* el precedente más exitoso de verificación formal en compilers.
- ⬜ **CakeML: A Verified Implementation of ML** (Kumar et al, POPL 2014)  
  *Relevancia:* verified backend, referencia directa para Prisma IR.
- ⬜ **Lean 4 tutorial + mathlib intro**  
  *Fuente:* https://leanprover-community.github.io/  
  *Target:* calibrar tu curva de aprendizaje. Critical para decision point de semana 20.

### Tier 5: Rosetta 2 + Apple silicon (para el techo teórico)

- ⬜ **Rosetta 2 internals** — charla Koh M. Nakagawa, BSides 2021.  
  *Búsqueda:* "Rosetta 2 internals Nakagawa BSides".
- ⬜ **Apple silicon TSO bit** — documentación y análisis independiente.  
  *Relevancia:* entender exactamente qué es lo que no podemos replicar en Android.

### Tier 6: NPU / ML acceleration en móvil (para Pilar 1)

- ⬜ **NNAPI architecture** — Android docs.  
  *Fuente:* https://developer.android.com/ndk/guides/neuralnetworks.
- ⬜ **ONNX Runtime Mobile + NNAPI delegate**  
  *Fuente:* https://onnxruntime.ai/docs/execution-providers/NNAPI-ExecutionProvider.html.
- ⬜ **MediaTek NeuroPilot SDK docs**  
  *Relevancia:* Dimensity 8300-Ultra NPU.
- ⬜ **Qualcomm Hexagon SDK docs**  
  *Relevancia:* Snapdragon NPUs.
- ⬜ **Papers sobre ML-guided compilation** (buscar: "ML for compiler optimization survey").

### Tier 7: Graphics translation (para Pilar 6)

- ⬜ **DXVK architecture blog posts de doitsujin**.
- ⬜ **VKD3D-Proton source tour** — cómo traducen DX12 a Vulkan.
- ⬜ **Mesa Turnip driver architecture**.
- ⬜ **Vortek CPU-assisted driver** (Winlator Cmod fork) — entender limitaciones.

### Tier 8: Virtualización Android (para Pilar 5)

- ⬜ **Android Virtualization Framework (AVF) docs**  
  *Fuente:* https://source.android.com/docs/core/virtualization.
- ⬜ **Danny Lin Windows 11 on Pixel 6 writeup** (abril 2022).
- ⬜ **pKVM design paper** (Google/LSE).

### Tier 9: Ecosistema (conocer al competidor)

- ⬜ **Winlator Cmod source** — entender por qué es el fork más exitoso.
- ⬜ **GameNative (Pluvia fork)** — integración Steam/Epic/GOG.
- ⬜ **Microsoft Prism (lo que es público)** — XTA Cache, ARM64EC, WowBox64.
- ⬜ **ExaGear post-mortem** — por qué Huawei solo compró engine, no marca.

---

## Notas de lectura

Plantilla para cada fuente:

```
## [Título de la fuente]

**Fecha lectura:** YYYY-MM-DD  
**Tiempo invertido:** Xh  
**Link/Ref:** ...

### Resumen (3-5 bullets, en tus palabras)
- ...

### Lecciones aplicables a Prisma
- ...

### Preguntas abiertas
- ...

### Fragmentos de código que vale referenciar más tarde
(si aplica, con path exacto y número de línea del source leído)
```

---

## FEX-Emu: scan inicial AI-asistido

> **Disclaimer honesto:** esto es un scan preliminar generado con ayuda de un agente AI que leyó el código fuente de FEX en `~/Documents/sandbox/prisma-research/FEX`. Los números de línea pueden no ser exactos (FEX evoluciona rápido). **Para el paper académico y decisiones arquitectónicas, validar personalmente** con lectura directa. Estas notas son el punto de partida, no el destino.

**Fecha scan:** 2026-04-19
**Tamaño codebase FEX:** ~83k LOC en FEXCore.

### Overview

FEX es DBT de producción con pipeline basado en **IR SSA-like con nodos ordenados y offsets comprimidos a 32 bits** (límite 4M nodos/traducción). Optimizaciones especializadas para DBT, no passes genéricos de compilador. Componente crítico: soporte TSO adaptativo según capacidades detectadas en runtime del host ARM64 (LRCPC2 > RCPC > fallback con DMB).

### IR architecture

- **Forma:** SSA-like. Estructura `OrderedNode` en `FEXCore/Source/Interface/IR/IR.h` ~línea 226.
- Cada nodo: header con Previous/Next, OpNodeWrapper (offset 32-bit a IROp_Header), contador de usos, registro asignado post-RA.
- **Tamaños:** OpSize enum con i8/i16/i32/i64/f80/i128/i256 (para AVX/SVE).
- **Opcodes principales:**
  - Control flow: Jump, CondJump, Call, Return, Syscall, Break
  - Memoria: LoadContext, LoadRegister, **LoadMemTSO**, StoreMemTSO, LoadMemPair
  - Aritmética: Add/Sub/Mul/Div/Rem con variantes signed/unsigned
  - Vector: ~40 ops (VectorAdd, VectorPackedADD, etc.)
  - **x87 specific:** x87StackOp, x87LoadTag, x87UntagStack (ver pass dedicado)
  - Especiales: **Fence**, Extract, Insert, InlineConstant

### Pass pipeline (orden de ejecución)

Archivo: `FEXCore/Source/Interface/IR/PassManager.cpp`.

1. **X87StackOptimizationPass** — DBT-specific. Crítico porque x87 es stateful (stack-based FPU).
2. **DeadFlagCalculationElimination** — DBT-specific. Elimina cálculos de EFLAGS muertos via data-flow. (Archivo: `Passes/RedundantFlagCalculationElimination.cpp`.)
3. **RegisterAllocationPass** — Clásico (constrained RA con spilling).

**Insight:** faltan pases clásicos de compilador (CSE, LICM, DCE genérico). Intencional — mantener overhead de traducción bajo. Prisma debería considerar esta trade-off.

### TSO handling — lo más interesante

FEX detecta runtime qué extensiones ARM64 soporta el host y elige el camino más barato:

1. **TSO hardware-accelerated** (si `CTX->HostFeatures.SupportsTSOImm9`): usa LRCPC2 con immediate 9-bit (ldapur*). ~50% más rápido que DMB puro.
2. **RCPC fallback** (si `SupportsRCPC`): ldapr* (acquire load sin immediate). ARMv8.1+.
3. **Fallback general:** LDAR + DMB ISH (inner shareable).

Además: `AddForceTSOInformation` permite **desactivar TSO por rango de dirección guest** — primera señal real de que se puede hacer TSO adaptativo por región, validando el Pilar 3 de Prisma.

**Config flags relevantes:** `TSOEnabled`, `VectorTSOEnabled`, `MemcpySetTSOEnabled`. FEX ya tiene granularidad, pero no tiene ML-driven classification.

### Backend ARM64

- **NO usa vixl** — escribieron emitter custom sobre `ARMEmitter::Emitter` en `FEXCore/Source/Interface/Core/ArchHelpers/Arm64Emitter.{h,cpp}`.
- **StaticRegisters:** 28 regs ARM64 (x0-x27) mapeados estáticamente a registros x86_64 frecuentes.
  - x28 = STATE pointer (CpuStateFrame)
  - x25 = REG_CALLRET_SP
  - x26/x27 = REG_PF/REG_AF (parity/adjust flag como registros virtuales)
- GPRs temporales para spilling.
- **LoadConstant optimizado:** movz + movk, max 4 instrucciones para cargar immediate 64-bit sin ir a memoria.

### Translation cache

- **Persistente en disco.** Offline code cache generator serializa bloques.
- **Key:** SHA256(binario) + block offset. File naming: `<executable>-<hash>[.nomb]`.
- **Independiente de configuración FEX** — un cache funciona con diferentes build flags.
- Mapeo guest→host en memoria (GuestToHostMapping). Invalidación SMC via memory protection heurística.
- Config: `EnableCodeCacheValidation`.

### Lecciones aplicables a Prisma

1. **Offsets 32-bit en vez de punteros 64-bit en IR** reduce memoria 50%. Crítico para traducir millones de nodos.
2. **SSA-like con mutabilidad controlada** (liveness, reg assignment como anotaciones mutables post-allocation) es el punto dulce entre rigor y pragmatismo.
3. **Passes DBT-specific ANTES de RA** es la decisión correcta (X87, flag optimization dependen de semántica x86).
4. **TSO detectado en runtime → elegir mejor path disponible** es la base sobre la cual Prisma construye adaptividad ML.
5. **Register allocation constrained post-IR** simplifica backend sin sacrificar calidad.
6. **Code cache como subsistema abstracto** permite múltiples backends (memoria/disco/remoto/P2P — Pilar 4).
7. **AddForceTSOInformation** ya es un precedente: TSO por región funciona. ML classifier es la siguiente iteración natural.

### Preguntas abiertas

- ¿Cómo decide FEX qué bloques traducir? ¿Eager o con profiling dirigido?
- Excepciones x86 (SIGSEGV, div-by-zero) — integración con signals del host no clara.
- No hay LTO cross-block / tail-call elimination — ¿es oportunidad o es inviable sin IR más rico?
- X87 control word dinámico (rounding mode changes mid-exec) — ¿el pass lo asume estático?
- SMC aggressive retraducción — ¿hay throttling?

### Archivos clave a leer personalmente después

- `FEXCore/Source/Interface/IR/IR.h:226` — OrderedNode
- `FEXCore/Source/Interface/Core/JIT/MemoryOps.cpp:60-92` — TSO load paths
- `FEXCore/Source/Interface/IR/PassManager.cpp:10-24` — pipeline
- `FEXCore/Source/Interface/IR/Passes/RedundantFlagCalculationElimination.cpp:20-30` — EFLAGS optimization
- `FEXCore/Source/Interface/Core/ArchHelpers/Arm64Emitter.h:108-150` — register setup

---

## Box64: scan inicial AI-asistido

> **Disclaimer honesto:** igual que FEX, este es un scan preliminar AI-asistido sobre `~/Documents/sandbox/prisma-research/box64`. **Validar personalmente** antes de usar en decisiones de diseño o en el paper.

**Fecha scan:** 2026-04-19
**Tamaño codebase Box64:** ~515k LOC (2120 archivos), con 279 wrappers = ~67k LOC adicionales.

### Overview

Box64 es **fundamentalmente diferente a FEX**: NO usa IR intermedio. Template-based directo en 4 pases. Pragmático, rápido de compilar, pero limita optimizaciones cross-instruction. 15+ años de madurez. Complementa con **native wrapping** (libc, Mesa, SDL2 no se traducen — se reemplazan por llamadas ARM64 nativas).

### Los 4 pases del dynarec

Archivo base: `src/dynarec/dynarec_native.c`.

1. **Pass 0 — análisis estructural** (línea ~553 `native_pass0`): decodifica bloque x86 con Zydis, identifica jump targets, construye CFG, backward dataflow para flags, mapea GPRs fijos (xRAX=r10, ..., xR15=r25).
2. **Pass 1 — optimizaciones FPU + flags** (línea ~705): propagación lazy/eager de flags, análisis x87 stack, decisiones sobre memory barriers.
3. **Pass 2 — dimensionamiento** (línea ~723): dry-run de emisión solo para calcular tamaño exacto. Resuelve offsets que ARM64 LDR requiere en rangos limitados. Macro `EMIT(A)` solo suma 4 a `dyn->native_size`.
4. **Pass 3 — emisión real** (línea ~817): emite ARM64 bytecode a buffer executable, llena table64 con constantes 64-bit, registra instsize para debugging/signals.

**¿Por qué 4 y no menos?** ARM64 offsets requieren tamaños exactos → mínimo 2 pases. Box64 separa Pass 0 (análisis) de Pass 1 (optimización) "por modularidad" — pero en la práctica el overhead se acepta.

### Native wrapping strategy — la gran ventaja pragmática

En `src/wrapped/` hay 279 archivos. Ejemplo: `wrappedlibc.c` (~línea 134) define `my_printf` que:
1. Marshaling x86-64 ABI → ARM64 ABI (RDI→x0, RSI→x1, ...).
2. Llama `printf` nativo ARM64 directamente.
3. Retorna con marshaling inverso.

**Callbacks C → x86:** cuando `qsort` nativa llama un comparator x86, Box64 crea thunk ARM64 que setupea x86 ABI y salta a código traducido. Bidireccional.

**Count:** ~1000+ funciones wrappeadas across libc, libm, glib2, X11, OpenGL, SDL2, etc.

**Debilidad:** mantener sync con evolución de libs (glibc 2.40+) es mantenance-heavy.

### Register allocation: FIJO, no dinámico

`src/dynarec/arm64/arm64_mapping.h` (líneas 40-59):
- 16 GPRs x86-64 → 16 ARM64 regs dedicados (r10-r25)
- r0 = xEmu (puntero a contexto emulador x64emu_t)
- r1-r6 = scratch temporales
- r26 = xFlags, r27 = xRIP, r28 = xSavedSP

**Ventaja:** simplicidad, compilation speed.
**Desventaja:** sin register allocation real, oportunidades de optimización perdidas. Valores temporales siempre van a scratch regs.

SIMD (XMM/YMM) sí usa allocation dinámico via `neoncache`.

### Flags (RFLAGS) handling: LAZY / DEFERRED

Archivo: `src/dynarec/arm64/updateflags_arm64_pass.c` (~líneas 62-103).

Modelo:
1. Instrucción x86 que modifica flags → solo actualiza `x64emu_t.df` (deferred flag code: d_add8, d_imul32, d_dec16, etc.).
2. Instrucción que lee flags (JE, SETNZ) → llama dynablock `UpdateFlags` que computa en runtime según `df`.
3. Reset `df = d_none` después.

**Auxiliary carry (AF):** el peor de los flags. Box64 lo computa solo si detecta DAA/DAS en Pass 0. Raro en binarios modernos.

**Ventaja:** ~30% de instrucciones no leen flags → ahorro real.
**Desventaja:** cadenas largas de instrucciones con flags necesitan re-compute costoso.

### Translation cache + SMC

Struct `dynablock_t` en `src/dynarec/dynablock_private.h` (~líneas 20-56):
- `block`: código ARM64 compilado
- `x64_addr`, `x64_size`: región guest
- `hash`: CRC32 de la región x64 (detección SMC)
- `gone`, `dirty`: estados de invalidación
- `instsize[]`: mapeo instruction-level x64↔ARM offsets
- `sep[]`: secondary entry points (permite saltos dentro de bloque sin recompilar)

**En-memory only, no persistente.** (Debilidad vs FEX, oportunidad para Prisma Pilar 4.)

**SMC:** recomputa CRC antes de ejecutar; si cambió, invalida. Hot pages con "always_test". No es fino — por bloque, no por byte.

### TSO

- `LOCK` prefix → usa LDAXR/STLXR loops vía `arm64_lock.h` helpers.
- `CMPXCHG` → LDAXR + CMP + STLXR con retry.
- `MFENCE` → DMB ISH (inner shareable).
- `DMB` en LOCK, liberaciones explícitas.
- **Modo relajado implícito:** env var `BOX64ENV(dynarec_no_barrier)` desactiva barreras globalmente. **Riesgoso** en multithread pero rápido.

**NO hay clasificación por región.** Prisma puede ganar aquí claramente.

### Lecciones aplicables a Prisma

**Fortalezas a adoptar:**
1. Template-based rápido es válido para prototipo. Pero Prisma NO lo adopta como final — queremos IR para optimización cross-instruction.
2. **Native wrapping es brutal** para performance. Prisma debería wrappear al menos libc, Mesa/Vulkan, SDL2 desde Fase 3. Adoptar, no inventar.
3. **Lazy flags** recupera ~30% perf. Prisma debe implementarlo, idealmente con decisión eager/lazy por instrucción (ML-informed?).
4. **Fixed register mapping** para GPRs es simple y efectivo. Prisma puede empezar así y refinar a RA dinámico en Fase 2.5.
5. **Secondary entry points (`sep`)** para jumps mid-block son esenciales. Adoptar.
6. **CRC32 para SMC** es barato. Empezar con eso, refinar si es hot-spot.

**Debilidades a mejorar:**
1. **Falta IR → sin optimización global.** Prisma tiene IR formal (Pilar 2). Ganancia clara.
2. **Cache no persistente.** Prisma persiste + distribuye (Pilar 4).
3. **TSO sin clasificación ML.** Prisma introduce NPU-assisted classification (Pilares 1+3).
4. **Wrappers manual-maintained.** Prisma debería generar wrappers desde DWARF o LLVM IR automáticamente — research opportunity.
5. **Table64 bloat** (constantes 64-bit in-line). Prisma debería usar PC-relative encoding donde posible.
6. **Pass 1/2/3 re-decodifican mismo código.** Prisma cachea AST/IR entre pases.
7. **SMC detection per-block.** Prisma puede hacer fine-grained (por-byte o por-cacheline) para binarios self-modifying agresivos.

### Preguntas abiertas

- REPZ/LOOP — ¿emite bucles explícitos o fallback interprete?
- Vectorización — ¿qué pass decide NEON mapping, Pass 1 o durante emisión?
- Multithread safety de modificaciones concurrentes a dynablock — ¿race conditions?
- Overhead real de 4 pases vs 2 — ¿alguien lo midió?
- Signal handlers con state consistente (x87, SIMD, flags) — ¿cómo?

### Archivos clave a leer personalmente después

- `src/dynarec/dynarec_native.c:553,705,723,817` — los 4 pases
- `src/dynarec/arm64/arm64_mapping.h:40-59` — register mapping
- `src/dynarec/arm64/updateflags_arm64_pass.c:25-103` — deferred flags
- `src/dynarec/dynablock_private.h:20-56` — cache structure
- `src/dynarec/arm64/arm64_lock.h:1-97` — atomics
- `src/wrapped/wrappedlibc.c:101-277` — wrapping pattern

---

## Comparativa FEX vs Box64 — qué adopta Prisma

| Dimensión | FEX | Box64 | Prisma |
|---|---|---|---|
| **IR intermedio** | SSA-like con 32-bit offsets | Ninguno (template directo) | SSA-like + **semántica formal Lean 4** (Pilar 2) |
| **Passes** | X87 + DeadFlag + RA | 4 pases directos | DCE + ConstProp + TSOAdaptive (formal) + RA |
| **Register allocation** | Constrained post-IR | Fijo 16→16 | Fijo fase 1, constrained fase 2+ |
| **Flags** | Eliminación de calc muerto | Lazy/deferred | **Híbrido**: eager cuando inmediato, lazy cuando no (ML-informed Pilar 1) |
| **TSO** | Adaptive por feature detection + por región | Global on/off, CMPXCHG costoso | **ML-classified por región** (Pilar 3) sobre base FEX |
| **Translation cache** | Persistente en disco, SHA256 key | In-memory, CRC32 | **Persistente + CDN + P2P** (Pilar 4) |
| **SMC** | Memory protection heuristics | CRC por bloque + hot pages | Per-cacheline + ML hot-path prediction |
| **Native wrapping** | No énfasis | Extenso (279 libs) | **Adoptar Box64 approach** + autogeneración desde DWARF |
| **ARM64 emitter** | Custom propio | Custom propio | **vixl** (Apache 2.0, mantenido por ARM) |
| **Backend extras** | x87 optimization pass | x87 minimal | x87 + NPU-assisted optimization hints (Pilar 1) |
| **Virtualización híbrida** | N/A | N/A | **DBT + KVM en Tensor SoCs** (Pilar 5) |
| **Graphics translation** | Delega a DXVK stock | Delega a DXVK stock | **Whole-graph optimization** (Pilar 6) |
| **Verification** | Tests + fuzzing | Tests + fuzzing | Tests + fuzzing + **proofs Lean 4** de passes críticos |

**Conclusión del análisis comparativo:** los 6 pilares de Prisma son genuinamente novedosos — ninguno de los dos DBTs líderes los tiene. El riesgo no es "duplicar FEX/Box64", es "no llegar a su nivel base" en fundamentos (IR robusto, RA, TSO básico). Fase 1-2 debe cerrar ese gap antes de activar los pilares diferenciadores en Fase 2.5.

---

## Decisiones técnicas tomadas durante Fase 0

Documentar aquí cada decisión arquitectónica con:
- **Decisión:**
- **Fecha:**
- **Alternativas consideradas:**
- **Razón:**
- **Reversible?:** (y cómo, si aplica)

### Decisión #1 — Usar vixl como ARM64 emitter (no custom)

- **Decisión:** adoptar `vixl` (ARM, Apache 2.0) como biblioteca de emisión ARM64.
- **Fecha:** 2026-04-19.
- **Alternativas consideradas:**
  - Custom emitter (lo que hacen FEX y Box64) — años de trabajo, sin ventaja técnica real.
  - `asmjit` (MIT) — también válido, pero vixl tiene mejor soporte ARM64 específico + mantenido por ARM directamente.
  - `LLVM JIT` — overkill, compile times lentos, dependencia enorme.
- **Razón:** el emitter no es diferenciador de Prisma. Los 6 pilares están en IR formal, NPU, TSO adaptativo, cache distribuida, virtualización híbrida, graphics. Gastar 12+ meses en emitter custom es perder tiempo en commodity. vixl nos da ARM64 completo en semanas.
- **Reversible?:** sí, la capa de emisión puede reemplazarse más adelante si vixl limita algo crítico. No es load-bearing en el diseño.

### Decisión #2 — IR SSA-like, no template-based

- **Decisión:** Prisma usa IR SSA-like (al estilo FEX), no template-based directo (al estilo Box64).
- **Fecha:** 2026-04-19.
- **Alternativas consideradas:**
  - Template-based directo (Box64) — más rápido de compilar, pero sin optimización cross-instruction ni verificación formal.
  - Tree-based IR — menos potente que SSA para análisis de dataflow.
  - CLIF (Cranelift IR) — diseñado para WebAssembly, no apto para x86 DBT (ver manifiesto v1).
- **Razón:** Pilar 2 (semántica formal en Lean 4) REQUIERE IR puro con estructura bien definida. Pilar 3 (TSO adaptativo ML-classified) REQUIERE análisis de dataflow cross-instruction que template-based no permite. IR SSA es prerequisito de 4 de los 6 pilares.
- **Reversible?:** caro. Cambiar IR es un refactor grande. Por eso Fase 1 dedica 24 semanas a diseñarlo bien, incluyendo 6 semanas de spec formal Lean 4 antes de tocar C++.

### Decisión #3 — Register allocation: fijo en Fase 1, dinámico en Fase 2+

- **Decisión:** arrancamos con mapping fijo 16 GPR x86_64 → 16 ARM64 regs dedicados (al estilo Box64), migramos a constrained RA (estilo FEX) cuando el IR esté maduro.
- **Fecha:** 2026-04-19.
- **Razón:** RA constrained requiere liveness analysis y estructuras grafo de interferencia. Diseñarlo BIEN toma 6-8 semanas. En Fase 1 (correctness-first) queremos algo simple que funcione. En Fase 2.5 (frontier research) añadimos RA real + NPU hints (Pilar 1).
- **Reversible?:** sí, el RA es un pass independiente. Sustituirlo no requiere refactor del IR.

### Decisión #4 — Cache persistente desde día uno (con P2P en Fase 2.5)

- **Decisión:** Fase 1 ya implementa translation cache persistente en disco con hash SHA256(binario) como key (estilo FEX). CDN + P2P es Fase 2.5.
- **Fecha:** 2026-04-19.
- **Razón:** cache in-memory (Box64) pierde todo entre launches — UX terrible. FEX ya demostró que persistente funciona. No repetir el error de Box64.
- **Reversible?:** sí, el cache layer es abstracto.

### Decisión #5 — Adoptar native wrapping strategy de Box64 en Fase 3

- **Decisión:** cuando llegamos a Wine integration, wrapear libc/Mesa/Vulkan/SDL2 nativamente en ARM64 en vez de traducir.
- **Fecha:** 2026-04-19.
- **Razón:** Box64 demostró 30-50% mejora en juegos CPU-bound. Esto NO es diferenciador nuestro pero es mandatory baseline. Sin esto, Prisma pierde contra Box64 en performance básico.
- **Reversible?:** sí, los wrappers son capa opcional encima del DBT.
- **Riesgo conocido:** mantenance de wrappers cuando libs evolucionan. Mitigación: investigar autogeneración desde DWARF debug info o LLVM IR de las libs — research opportunity para un blog post.

_(más decisiones se añadirán conforme avanza Fase 0)_

---

## Preguntas abiertas globales

Lista viva de preguntas que emergen de la investigación y aún no tienen respuesta clara:

- ¿Es viable expresar en Lean 4 una spec completa del IR con todas las instrucciones x86 relevantes, o es demasiado grande para un solo-dev?
- ¿Cuál es el overhead real de NNAPI inference desde C++? ¿Es sub-microsegundo o ya no sirve para hot path prediction?
- ¿Hay alguna forma de detectar en runtime si el SoC tiene FEAT_LSE2 + LRCPC2 + FlagM y explotar las instrucciones acquire/release cheap? **(Respuesta parcial del scan FEX: sí, `HostFeatures.SupportsTSOImm9` y `SupportsRCPC` son patrón a imitar.)**
- ¿El protocolo P2P del Pilar 4 debe ser custom o podemos piggyback en libp2p existente?
- **(Nueva, de scan FEX):** ¿cómo maneja FEX excepciones x86 sincrónicas como SIGSEGV del guest? No quedó claro del scan. Crítico para correctness en Fase 2.
- **(Nueva, de scan Box64):** ¿es viable autogenerar wrappers de libc/Mesa/SDL2 desde DWARF debug info? Sería paper/blog post independiente y resolvería el maintenance nightmare que Box64 tiene.
- **(Nueva, de scan comparativo):** Box64 tiene secondary entry points (`sep`) para jumps mid-block. FEX parece no tenerlos. ¿Son esenciales o pueden emularse con block splitting en Prisma?
- **(Nueva):** table64 bloat en Box64 (constantes 64-bit in-line) — ¿PC-relative addressing en ARM64 reduce esto a near-zero?

_(actualizar conforme aparezcan nuevas preguntas)_
