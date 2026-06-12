---
id: 0015
title: Rust Core — migración incremental del DBT engine
status: draft
authors: [Danny]
created: 2026-06-11
updated: 2026-06-11
supersedes: []
superseded_by: null
---

# RFC 0015: Rust Core — migración incremental del DBT engine

## Summary

Migrar progresivamente el core C++20 de Prisma a Rust, comenzando por
los componentes puros (IR types → passes → cache) y avanzando hacia
los boundarios (decoder → runtime → backend). La migración es
**incremental**: cada componente se reescribe en Rust manteniendo la
ABI C existente (`core-sys`), permitiendo que C++ y Rust coexistan
en el mismo proceso durante toda la transición. El backend (emitter +
lowering, dependiente de vixl) se migra al final, cuando exista un
sustituto Rust para el assembler ARM64.

## Motivation

### Por qué migrar a Rust

Prisma ya tiene una shell en Rust (`shell/orchestrator`, `core-sys`,
`core`). Llevar el DBT engine a Rust consolida todo el stack en un
solo lenguaje de sistemas, eliminando la fricción C↔C++↔Rust en el
FFI y abriendo acceso directo al ecosistema Rust (serde, tokio,
proptest, clippy, cargo-fuzz).

Beneficios concretos:

1.  **Safety en el decoder.** Las tablas de opcodes x86 tienen errores
    de indexación que Rust `enum` + `match` exhaustivo elimina en
    compile-time. El bug `BSF/BSR stored zero` (gap sweep 14j) no
    habría pasado una revisión con tipos algebraicos.

2.  **Property-based testing nativo.** `proptest` + `cargo-fuzz` son
    ciudadanos de primera clase en Rust. Hoy tenemos que mantener un
    generador de IR custom en C++ (`test_property.cpp`) y un harness
    AFL++ aparte. Rust unifica ambas.

3.  **Serialización sin riesgo.** El IR binary format (RFC 0007/0009)
    se serializa con `serde` + `bincode` — sin buffer overruns, sin
    manual CRC, sin `DeserializeError::Truncated` escrito a mano.

4.  **Concurrencia explotable.** El translation cache puede servir
    múltiples workers con `Arc<RwLock<LruCache>>`. Hoy es single-
    threaded con un mutex opaco.

5.  **Ecosistema ARM64.** Crates como `crc32c`, `sha2`, `aes` son
    acelerados por hardware en ARM64 vía `std::arch`. En C++ toca
    mantener intrinsics por plataforma.

### Por qué NO una reescritura total de una vez

El core C++ tiene ~20,119 líneas compiladas, un dependency graph
complejo (`prisma_runtime` ← `prisma_translator` → todos), y depende
de vixl (C++ puro) para el backend. Una reescritura big-bang sería
un riesgo enorme de regresión y consumiría 6-9 meses sin entregar
valor hasta el final.

**La migración incremental es la única estrategia viable para un
proyecto de 48-54 meses.** Cada fase entrega un crate funcional,
probado contra la implementación C++ existente, y desbloquea la
siguiente fase.

## Context

### Stack actual

```
┌─────────────────────────────────────────────────┐
│  Android App (Kotlin)                           │
├─────────────────────────────────────────────────┤
│  JNI                                            │
├─────────────────────────────────────────────────┤
│  shell/orchestrator (Rust)                      │
│  contenedor, PE loader, integridad, cache P2P   │
├─────────────────────────────────────────────────┤
│  shell/core (Rust — safe wrapper)               │
├─────────────────────────────────────────────────┤
│  shell/core-sys (Rust — FFI raw bindings)       │
├═════════════════════════════════════════════════╤══════════╣
│  C API → libprisma_core_c.so  (capi.cpp)       │ ← BOUNDARY
├─────────────────────────────────────────────────┤
│  prisma_translator (facade, 552 lines)          │
├──────────┬──────────┬──────────┬────────────────┤
│ decoder  │ passes   │ backend  │ runtime        │
│ (7,318)  │ (2,192)  │ (4,698)  │ (1,826)        │
│          │          │          │                │
│ IR types │ cache    │ vixl     │ dispatcher     │
│ (3,568)  │ (532)    │ (C++     │ signals        │
│          │          │  only)   │ SMC guard      │
│          │          │          │ syscalls       │
└──────────┴──────────┴──────────┴────────────────┘
```

### C API ABI (RFC 0014)

El contracto existente entre C++ y Rust vía `capi.h` es el pivote
de la migración. Permite:

- Llamar Rust desde C++ (`extern "C" { fn prisma_rust_*() }`)
- Llamar C++ desde Rust (`extern "C" { fn prisma_*() }` vía `core-sys`)
- Reemplazar un componente C++ con su equivalente Rust cambiando
  un solo `#[link]` o un puntero a función

### Estado de los tests

- 800+ Catch2 tests / 5004+ assertions en C++
- 3 propiedades proptest via bridge Rust (1,280+ casos/ejecución)
- E2E en ARM64 real vía CI (core-build-arm64)
- Suite completa pasa en Debug + ASan + UBSan

Cada fase debe mantener la suite verde antes de declararse completa.

## Architecture

### Coexistencia C++/Rust durante la migración

```
┌──────────────────────────────────────┐
│  Rust native crates (nuevos)         │
│  prisma-ir, prisma-passes,           │
│  prisma-cache, prisma-decoder        │
├──────────────────────────────────────┤
│  C FFI shim layer                    │
│  (thin extern "C" wrappers)          │
├──────────────────────────────────────┤
│  C++ core (legacy during transition) │
│  prisma_emitter, prisma_runtime       │
└──────────────────────────────────────┘
```

Cada crate Rust expone una C API mínima que el C++ existente
consume. Cuando el componente C++ se retira, la C API se vuelve
una llamada directa Rust→Rust.

### Principios de diseño

1. **Un crate por componente.** `prisma-ir`, `prisma-passes`,
   `prisma-cache`, `prisma-decoder`, `prisma-runtime`. Cada crate
   es independiente y testeable en aislamiento.

2. **C API como interfaz.** No dependencias directas Rust→C++ ni
   C++→Rust. Solo `extern "C"` con tipos POD. Esto permite swap
   en caliente (cargando la .so correcta).

3. **Tipos algebraicos en el IR.** `Op` como `enum` con variantes
   tipadas, no un `std::variant` genérico. El validator se vuelve
   un `match` exhaustivo.

4. **Serde para serialización.** `bincode` o `postcard` como
   backend, con `serde` derive en todos los tipos IR.

5. **Lean spec como source of truth.** Los tipos IR en Rust se
   generan o verifican contra `ir-spec/`. Idealmente, la spec
   Lean 4 produce bindings Rust vía FFI o generación de código.

6. **proptest como framework de testing.** Cada propiedad que
   exista en C++ se duplica en Rust con más seeds. `cargo-fuzz`
   para el decoder.

## Migration Phases

### Fase 0 — Fundaciones (IR types + Lean cross-check)

**Duración:** 4 semanas
**Crate:** `shell/prisma-ir`
**Dependencias:** Ninguna (Rust puro)
**Líneas C++ a migrar:** ~3,568 (operation.cpp, cfg.cpp,
dominators.cpp, validate.cpp, pretty_print.cpp, profiler.cpp)

**Deliverables:**

- [ ] `src/lib.rs` con tipos IR completos:
  - `OpSize`, `Gpr`, `CondCode`, `FaultKind`
  - `Op` enum (todas las variantes: Constant, BinOp, StoreReg,
    LoadReg, LoadMem, StoreMem, WriteFlags*, etc.)
  - `Ref` newtype
  - `Stmt` = `(Option<Ref>, Op)`
  - `BasicBlock`, `Function`
- [ ] Pretty-printer: `impl Display for Op`
- [ ] Validator: `fn validate(&Function) -> Result<(), Vec<ValidationError>>`
  que replica las checks de `ir::validate()` en C++
- [ ] `cfg::build(&[Stmt]) -> Vec<BasicBlock>` — construcción de CFG
- [ ] `dominators::compute(&[BasicBlock]) -> DominatorTree`
- [ ] `profiler::OpCounter` — conteo de opcodes
- [ ] C API bridge: `extern "C"` para validar IR desde C++
- [ ] Differential tests: mismo IR en C++ y Rust, misma salida de
    validate + pretty-print
- [ ] Serde derives + `bincode` round-trip tests (replica
    `test_property.cpp`)
- [ ] Lean spec cross-check: script que compara tipos IR entre
    `ir-spec/` y `prisma-ir`, reporta discrepancias

**Criterio de éxito:** El Rust IR valida y serializa cualquier
programa que el C++ IR acepta. Suite C++ sigue verde.

---

### Fase 1 — Cache P2P + persistencia

**Duración:** 2 semanas
**Crate:** `shell/prisma-cache`
**Dependencias:** `prisma-ir` (para serializar entries)
**Líneas C++ a migrar:** ~532 (translation_cache.cpp, sha256.cpp,
compress.cpp)

**Deliverables:**

- [ ] `TranslationCache` con:
  - Key = `(guest_addr: u64, content_hash: u64)` — FNV-1a en Rust
    (`fasthash` crate o manual)
  - LRU eviction via `lru` crate
  - Budget byte tracking
  - Per-entry stats (hit_count, last_used_tick)
- [ ] Persistencia: `save()` / `load()` con formato RFC 0007
  - zstd compression via `zstd` crate (ya en Cargo.toml)
  - SHA-256 trust envelope via `sha2` crate (ya en Cargo.toml)
- [ ] `save_to_file_async()` — tokio::spawn + `tokio::fs`
- [ ] SMC-safe invalidation: `invalidate(guest_addr)`
- [ ] C API bridge para que C++ translator pueda usar el cache Rust
  - `prisma_cache_create`, `prisma_cache_lookup`, `prisma_cache_store`
- [ ] Tests:
  - Round-trip save/load (con y sin compresión)
  - LRU eviction ordenada
  - Cache miss / hit contadores
  - SMC invalidation quita la entrada correcta
  - `save_to_file_async` no bloquea

**Criterio de éxito:** Cache Rust funciona como drop-in replacement
del C++ translation_cache con misma semántica y mejor throughput
(async save no bloquea dispatcher).

---

### Fase 2 — Optimization Passes

**Duración:** 6 semanas
**Crate:** `shell/prisma-passes`
**Dependencias:** `prisma-ir`
**Líneas C++ a migrar:** ~2,192 (16 files)

**Deliverables:**

- [ ] Pipeline definition: `struct PassPipeline { passes: Vec<Box<dyn Pass>> }`
  con trait `Pass: fn run(&self, Function) -> Function`
- [ ] Todos los passes migrados:
  - `const_prop` — constant propagation + second pass
  - `algebraic` — x+0→x, x*1→x, x^0→x, etc.
  - `strength_reduce` — mul by power-of-two → shift
  - `redundant_load` — elimina loads redundantes
  - `cse` + `global_cse` — common subexpression elimination
  - `copy_prop` — copy propagation
  - `dead_store` — dead store elimination
  - `branch_fold` — constant conditionals, jmp→jmp coalesce
  - `dce` — dead code elimination
  - `flag_write_elim` — elimina flag writes muertas
  - `tail_call` — tail call detection
  - `x87_stack` — x87 FP stack renaming
  - `licm` — loop invariant code motion (itera a fixed point)
  - `peephole` — patrones locales
- [ ] Pipeline default: mismo orden que C++ (const_prop → algebraic →
    strength_reduce → const_prop_2 → redundant_load → CSE →
    copy_propagate → dead_store → branch_fold → DCE)
- [ ] Pass timing hooks (replicar F1-PS-016/017)
- [ ] C API bridge para que translator C++ use passes Rust
- [ ] Tests:
  - property-based idempotence (replica `test_property.cpp`)
  - property-based never-grows (replica `test_property.cpp`)
  - Differential: para cada programa random, output de C++ passes ==
    output de Rust passes
  - Cada pass tiene test de unidad con casos borde
- [ ] Performance benchmark: Rust passes no son más lentos que C++ en
    el mismo hardware

**Criterio de éxito:** Suite completa pasa con passes Rust en lugar
de C++. Benchmarks muestran paridad o mejora.

---

### Fase 3 — Decoder x86_64 → IR

**Duración:** 10 semanas
**Crate:** `shell/prisma-decoder`
**Dependencias:** `prisma-ir`
**Líneas C++ a migrar:** ~7,318 (x86_decoder.cpp)

**Deliverables:**

- [ ] Arquitectura basada en `enum` + pattern matching:
  ```rust
  enum ModRm { /* mod, reg, rm */ }
  enum Sib { /* scale, index, base */ }
  enum Prefix { Rex, Vex, OperandSize, AddressSize, Lock, Rep, ... }
  enum Opcode { Add, Sub, Mov, Cmp, ... }

  fn decode(bytes: &[u8]) -> Result<Vec<Stmt>, DecodeError>
  ```
- [ ] Tablas de opcodes como `match` exhaustivo, no arrays de
    punteros a función
- [ ] Cobertura completa de opcodes existentes en C++:
  - SSE/SSE2/SSE3/SSE4.x completo
  - AVX-128/256 (VEX C4/C5)
  - FMA3
  - BMI1/BMI2
  - AES/SHA-NI
  - MOVBE, CRC32
  - x87 (reduced-F64)
- [ ] C API bridge para decoder Rust desde C++
- [ ] Fuzzing: `cargo-fuzz` target + AFL++ harness
- [ ] Tests:
  - Differential: cada instrucción decodificada igual en C++ y Rust
  - Property: bytes aleatorios nunca crash (replica bridge proptest)
  - E2E: mismos programas producen mismo IR

**Criterio de éxito:** Decoder Rust reemplaza al C++ en el pipeline.
Suite C++ + Rust tests verdes. Fuzzing sin crashes en 24h.

---

### Fase 4 — Runtime (dispatcher + signals + SMC + syscalls)

**Duración:** 8 semanas
**Crate:** `shell/prisma-runtime`
**Dependencias:** `prisma-ir`, `prisma-cache`
**Líneas C++ a migrar:** ~1,826 (8 files)

**Deliverables:**

- [ ] `JitMemory` — MAP_JIT allocation:
  - macOS: `pthread_jit_write_protect_np` vía `libc`
  - Linux: `mmap` con `PROT_READ|PROT_WRITE|PROT_EXEC`
  - Windows: `VirtualAlloc` con `PAGE_EXECUTE_READWRITE`
- [ ] `JitBufferPool` — recycling pool de buffers pre-alocados
- [ ] `SignalHandler` — SIGSEGV/SIGILL/SIGBUS:
  - `sigaction` vía `nix` crate
  - thread-local `jmp_buf` → `std::panic::catch_unwind`
  - SMC integration: `SmcGuard::handle_fault`
- [ ] `SmcGuard` — page tracking + mprotect + fault queue:
  - Page-granularity write protection
  - Spinlock-based fault queue
  - `drain_pending` en normal context
- [ ] `Dispatcher` — main dispatch loop:
  - fetch → translate → execute
  - Halt PCs, step budget
  - Return-stack predictor
  - Direct JIT branch threading (stages 2-6)
  - Guest signal delivery
  - DispatchStats (RAS, direct threading, JIT patches)
- [ ] `SyscallHandler` — x86 syscall → host OS:
  - `match` sobre nr de syscall
  - Soporte para Linux + macOS
- [ ] `HostFeatures` — FEAT_* detection:
  - macOS: `sysctlbyname` vía `libc`
  - Linux: `getauxval(AT_HWCAP)` vía `libc`
- [ ] C API bridge completa
- [ ] Tests:
  - Signal handler recovery (replicar `test_signal_handler.cpp`)
  - SMC guard edge cases (replicar `test_smc_guard.cpp`)
  - Host features override (replicar `test_host_features.cpp`)
  - Property: dispatcher no crash con programas random
  - E2E: mismos programas producen misma salida

**Criterio de éxito:** Prisma corre un programa x86 real (e.g. hello
world PE) usando runtime Rust, desde dispatcher hasta signal
handling.

---

### Fase 5 — Backend (emitter + lowering)

**Duración:** 12 semanas (o diferido)
**Crate:** `shell/prisma-backend`
**Dependencias:** `prisma-ir`
**Líneas C++ a migrar:** ~4,698 (emitter.cpp, lowering.cpp, abi.cpp)

**Bloqueador:** vixl (Google ARM64 assembler). No existe en Rust.

**Opciones:**

1. **Portar vixl a Rust.** ~15k líneas de assembly templates C++
   → Rust macros procedurales. Riesgo alto, recompensa alta.

2. **Usar crate existente.** `iced-aarch64` (WIP), `faerie`
   (object file writer), o `goblin`. Ninguno maduro para un DBT.

3. **Assembler minimal custom.** Solo las instrucciones que Prisma
   emite (~200-300 instrucciones ARM64). Viendo el `emitter.cpp`,
   la mayoría son ALU (add/sub/mul), SIMD (advsimd), branches
   (b/bl/br/ret), memory (ldr/str/stp), y algunas crypto (aese/aesd/
   sha256). Un assembler custom de ~3,000 líneas Rust cubre todo.

4. **Mantener backend en C++ indefinidamente.** La shell Rust llama
   al C++ backend vía C API. El resto del core migrado a Rust.

**Recomendación:** Opción 3 (assembler minimal custom) + Opción 4
como fallback. Si el assembler custom toma más de 12 semanas,
congelar y mantener C++ backend.

**Deliverables:**

- [ ] `Arm64Assembler` — emite instrucciones ARM64 a un buffer:
  - ALU (add/sub/mul/udiv/sdiv/lsl/lsr/asr/ror/clz/cls/rbit)
  - SIMD (advsimd: add/sub/mul/min/max/shift/compare/blend/shuffle)
  - FP (fadd/fsub/fmul/fdiv/fmin/fmax/fcmp/fcvt)
  - Memory (ldr/str/ldp/stp/ldr q/str q — escalado, post-index, etc.)
  - Branches (b/bl/br/ret/b.eq/b.ne/b.lt/.../cbz/cbnz)
  - Crypto (aese/aesd/aesmc/aesimc/sha256h/sha256su1/...)
  - Barriers (dmb/dsb/isb)
  - Exclusives (ldxr/stxr/lse cas/ldadd/...)
- [ ] `Lowerer` — IR op → secuencia de instrucciones ARM64:
  - Replica `lowering.cpp` (2,476 líneas) en Rust
- [ ] ABI helpers: `abi.cpp` → `fn caller_save_regs()`, etc.
- [ ] C API bridge para que runtime Rust use backend Rust
- [ ] Differential tests: misma salida de lowering + emission
    que C++ backend
- [ ] Performance benchmark: generated code quality vs C++ backend

**Criterio de éxito:** Backend Rust compila un bloque IR a ARM64,
se ejecuta, y produce mismo resultado que el backend C++.

---

### Fase 6 — Translator facade + retire C++ core

**Duración:** 2 semanas
**Crates:** `shell/prisma-translator`
**Dependencias:** Todos los anteriores
**Líneas C++ a migrar:** ~552 (translator.cpp) + ~212 (capi.cpp)

**Deliverables:**

- [ ] `Translator` en Rust: orquesta decoder → passes → lowerer →
    emitter → cache → dispatch
- [ ] Reemplazar `capi.cpp` con `extern "C"` en Rust
- [ ] Eliminar `libprisma_core_c.so` del build
- [ ] Renombrar `shell/core-sys` → `shell/native-sys` (ya no hay C++ core)
- [ ] CI: eliminar step de build C++, mover todo a `cargo test`

**Criterio de éxito:** `cargo build --workspace` produce el binario
completo. Zero archivos .cpp en el pipeline.

---

## Dependency Graph (Rust crates)

```
prisma-translator
  │
  ├── prisma-decoder
  │     └── prisma-ir
  │
  ├── prisma-passes
  │     └── prisma-ir
  │
  ├── prisma-backend
  │     └── prisma-ir
  │
  ├── prisma-runtime
  │     ├── prisma-ir
  │     ├── prisma-cache
  │     │     └── prisma-ir
  │     └── prisma-backend
  │
  ├── prisma-cache
  │     └── prisma-ir
  │
  └── prisma-ir
```

## Testing Strategy

### Por fase

| Fase | Tests unitarios | Differential C++↔Rust | Property-based | Fuzzing |
|------|----------------|----------------------|----------------|---------|
| 0 IR | Cada tipo Op | validate() + pretty | Serialización round-trip | — |
| 1 Cache | LRU, persistencia | — | Save/load idempotente | — |
| 2 Passes | Cada pass | Pipeline output | Idempotencia, no-crece | — |
| 3 Decoder | Cada opcode | IR output idéntico | Bytes arbitrarios | cargo-fuzz + AFL++ |
| 4 Runtime | Signal, SMC | — | Dispatcher no crash | — |
| 5 Backend | Cada instruc. ARM64 | Lowering output | — | — |
| 6 Translator | Orchestración | E2E output | — | — |

### Differential testing framework

Para Fase 0-3, mantener un harness que:
1. Toma un programa IR (o bytes x86)
2. Lo corre por el implementación C++
3. Lo corre por el implementación Rust
4. Compara output (validate, pretty-print, serialize, pipeline output)
5. Reporta diferencias

Esto garantiza que la migración no cambia semántica.

## Risk Register

| Riesgo | Probabilidad | Impacto | Mitigación |
|--------|-------------|---------|------------|
| vixl no portable a Rust | Alta | Alto — backend migración incompleta | Assembler minimal custom + mantener C++ backend como fallback |
| Performance regression en passes Rust | Media | Medio | Benchmarks en CI; profiling con `criterion` |
| Lean spec y Rust IR divergen | Media | Medio — bugs silenciosos | Script cross-check automático en CI |
| Decoder Rust más lento que C++ (pattern matching vs lookup table) | Baja | Medio | Benchmark decoder throughput; hot paths pueden usar lookup tables |
| Signal handling en Rust no-portable entre plataformas | Baja | Medio | `nix` crate abstrae; tests por plataforma |
| FFI overhead en llamadas frecuentes | Baja | Bajo | Batch calls, minimizar fronteras; fase 6 elimina FFI |
| Mantenimiento de dos codebases durante transición | Alta | Medio | Automatizar differential tests; cada fase reemplaza completamente un componente antes de avanzar |

## Timeline

| Fase | Semanas | Inicio estimado | Hito |
|------|---------|----------------|------|
| 0 — IR types | 4 | Inmediato | Rust IR types + validator, differential tests |
| 1 — Cache | 2 | Semana 5 | Cache Rust reemplaza C++ en pipeline |
| 2 — Passes | 6 | Semana 7 | 14 passes migrados, pipeline idempotente |
| 3 — Decoder | 10 | Semana 13 | Decoder completo, fuzzing 24h sin crash |
| 4 — Runtime | 8 | Semana 23 | Runtime Rust: dispatcher, signals, syscalls |
| 5 — Backend | 12 | Semana 31 | Assembler minimal + lowering completo |
| 6 — Translator | 2 | Semana 43 | C++ core retirado, 100% Rust |

**Total estimado:** ~45 semanas (~10.5 meses). Se solapa con el
cronograma de Pillars existente (Fase 2 activa, Fase 3 en diseño).

## Integration with Pillar timeline

| Pillar | Semana (desde inicio) | Actividad principal | Solapamiento con migración |
|--------|----------------------|-------------------|---------------------------|
| F1 (core) | 1-52 | DBT engine, IR, passes | Fundaciones (sem 1-4), decoder (sem 13-22) |
| F2 (SIMD) | 26-78 | SSE/AVX/FMA, x87 completo | Decoder AVX paths (sem 15-22), lowering (sem 31-43) |
| F3 (optimization) | 52-104 | Pipeline avanzado, profile-guided | Passes Rust + LICM/GCSE (sem 7-12) |
| F4 (P2P) | 78-130 | Cache network, firmware distribution | Cache Rust + async save (sem 5-6), runtime networking |
| F5 (Android) | 104-156 | App, JNI, surface | Translator Rust + retire C++ (sem 43-45) |
| F6 (open source) | 130-182 | MIT release, papers, blog | Codebase 100% Rust, single build system |

La migración Rust **no retrasa ningún Pillar** — los equipos (o el
dev único con agentes) trabajan en paralelo: código nuevo se escribe
en Rust, el mantenedor C++ se congela gradualmente.

## Open questions

1. **Lean 4 → Rust code generation.** ¿Generamos tipos Rust desde
   la spec Lean, o mantenemos ambos sincronizados manualmente?
   Ideal: un script `lean-to-rust` que produce `#[derive(serde)]`
   types en Rust desde la spec.

2. **vixl replacement priority.** ¿Arrancamos el assembler custom
   desde Fase 0 en paralelo, o esperamos a Fase 5? Si hay riesgo
   de timeline, arrancar temprano reduce el blocker.

3. **SIMD lowering en Rust.** La lowering de AVX-256/FMA son los
   algoritmos más complejos del backend. ¿Migrarlos primero (Fase
   2.5) o dejarlos para Fase 5?

4. **Rust edition.** 2021 hoy. ¿Bump a 2024 cuando estable? Las
   features nuevas (gen asoc, RPITIT) simplificarían el Pass trait.

5. **Tokio sí o no.** `save_to_file_async` en cache necesita async.
   ¿Integrar tokio al workspace o usar `std::thread`? Tokio añade
   ~30 dependencias transitivas.

6. **Panic policy.** `panic=abort` en release para evitar
   unwind a través de FFI. ¿Capturar panics en cada entry point
   C API con `catch_unwind`?

## References

- RFC 0014: C-ABI FFI boundary core↔shell
- RFC 0007: Cache file format
- RFC 0009: IR binary format
- `docs/ARCHITECTURE.md` — FFI bridge section
- `shell/README.md` — Rust shell rationale
- `PROYECTO_PLAN_EJECUCION.md` — Timeline global
