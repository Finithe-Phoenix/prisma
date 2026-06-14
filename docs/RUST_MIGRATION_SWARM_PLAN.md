# Plan de migración Rust — modo **extremo paralelo**

Este documento activa una ejecución por ondas con agentes internos para cubrir
la migración de `core/` a Rust de forma paralela.

## Estado base (ya levantado)

- Workspace Rust existente en `shell/` con 7 crates: `orchestrator`, `core-sys`,
  `core`, `prisma-ir`, `prisma-cache`, `prisma-passes`, `prisma-decoder`,
  `prisma-runtime`, `prisma-backend`.
- `prisma-ir` ya tiene implementación real y tests básicos.
- Los módulos migrados en Rust aún tienen muchos stubs explícitos por fase:
  - `prisma-decoder`: `decode.rs`, `tables.rs`
  - `prisma-passes`: 13 módulos de `pass` + `pipeline.rs`
  - `prisma-cache`: `cache.rs`, `compress.rs`, `sha256.rs`
  - `prisma-runtime`: `dispatcher`, `guest_signal`, `host_features`, `jit_buffer_pool`,
    `jit_memory`, `signal_handler`, `smc_guard`, `syscall_handler`
  - `prisma-backend`: `abi.rs`, `assembler.rs`, `lowerer.rs`

## Protocolo de coordinación (obligatorio)

1. Cada agente declara ownership escribiendo una marca en
   `docs/BACKLOG.md`: `[~|<agente>]` (si no existe aún el ítem, se crea una
   entrada `F25-RS-XXX` con estado en progreso).
2. No tocar módulos marcados por otro agente activo.
3. Al terminar, el agente cambia el estado del item a `[x] (<sha>)` con su SHA.
4. Cada agente trabaja sobre una frontera de archivos cerrada definida aquí.

## Agentes de trabajo

### 1) Agente **IR-CORE** (origen/puente)

- **Owner sugerido:** `codex`
- **Ámbito:** `shell/prisma-ir` + validadores/serialización puente C++/Rust
- **Objetivo:** estabilizar tipos y contratos compartidos para que los demás
  bloques no diverjan.
- **Tareas:**
  - Añadir/actualizar tipos faltantes.
  - Tests de round-trip de IR y serialización.
  - Exponer invariantes para differential testing.

### 2) Agente **DECODER**

- **Owner sugerido:** `codex`
- **Ámbito:** `shell/prisma-decoder/src/{decode.rs,tables.rs}`
- **Objetivo:** migrar grupos de instrucciones con puente C++ contra
  `core/src/decoder/x86_decoder.cpp`.
- **Entregables:**
  - decode de al menos los opcodes actualmente cubiertos por tests C++
  - diferencias cero en pruebas C++↔Rust en corpus canónico
  - pruebas unitarias Rust + fuzz harness básico

### 3) Agente **CACHE**

- **Owner sugerido:** `codex`
- **Ámbito:** `shell/prisma-cache/src/{cache.rs,compress.rs,sha256.rs}`
- **Objetivo:** reemplazo funcional de `core/src/cache` con semántica compatible.
- **Entregables:**
  - hit/miss/evict equivalentes a C++
  - FNV-1a actual y SHA-256 listo para `v2`
  - compresión gzip/zstd por configuración
  - pruebas de persistencia y reaparición

### 4) Agente **PASSES**

- **Owner sugerido:** `claude`
- **Ámbito:** `shell/prisma-passes/src/{pipeline.rs,const_prop.rs,algebraic.rs,
  strength_reduce.rs,redundant_load.rs,cse.rs,global_cse.rs,copy_prop.rs,
  dead_store.rs,branch_fold.rs,dce.rs,flag_write_elim.rs,tail_call.rs,
  x87_stack.rs,peephole.rs}`
- **Objetivo:** migrar la canalización de optimización sin regresión funcional.
- **Entregables:**
  - `Pass` + `PassPipeline` en Rust
  - equivalencia por `differential_validate` contra C++
  - tests de idempotencia y orden de pases

### 5) Agente **RUNTIME**

- **Owner sugerido:** `claude`
- **Ámbito:** `shell/prisma-runtime/src/{dispatcher.rs,guest_signal.rs,host_features.rs,
  jit_buffer_pool.rs,jit_memory.rs,signal_handler.rs,smc_guard.rs,syscall_handler.rs}`
- **Objetivo:** migrar pipeline de ejecución y seguridad a Rust manteniendo
  contratos C++/Rust.
- **Entregables:**
  - ejecución de bloques con límites de seguridad
  - manejadores de señales/crash-to-guest coherentes
  - syscall bridge estable en C API
  - tests de estados de salida y de estrés

### 6) Agente **BACKEND**

- **Owner sugerido:** `codex`
- **Ámbito:** `shell/prisma-backend/src/{assembler.rs,lowerer.rs,abi.rs}`
- **Objetivo:** bajar IR a ARM64 en Rust (sin depender todavía de vixl).
- **Entregables:**
  - API de lowering mínima y estable para `Dispatcher`
  - ABI de llamadas y convención de registros
  - pruebas de ensamblador con snapshots o binarios de control

## Hoja de ruta de ejecución extrema

### Onda 1 (semana 1)
- IR-CORE + PASSES
- Resultado esperado: `prisma-ir` y los pasajes base compilan con tests unitarios.

### Onda 2 (semana 1-2)
- DECODER + CACHE
- Resultado esperado: `shell/prisma-decoder` y `shell/prisma-cache` compilan con
  tests de humo y differential contra C++ equivalente.

### Onda 3 (semana 2-3)
- RUNTIME + BACKEND en paralelo
- Resultado esperado: contratos C API de runtime y lowering compilan con un estado
  mínimo de bloque estable.

### Onda 4 (semana 3-4)
- Integración por boundary C API y sustitución por componentes Rust en el pipeline
  híbrido.

## Contratos obligatorios de integración entre agentes

- Cada cambio de API debe incluir:
  - firma exacta de función
  - tipos C-compatible (`#[repr(C)]` cuando aplique)
  - test diferencial mínimo
  - nota de migración en `docs/WORK_QUEUE.md`

## Definición de éxito de esta ola

- `cargo test --workspace` (Rust puro) verde.
- Differential C++↔Rust en fases mínimas: `decoder`, `passes`, `cache`.
- Sustitución incremental de al menos un tramo del pipeline `core` por llamadas a Rust
  sin regresión en `core/build/prisma_core_tests`.

## Extreme launch board

### Wave 0 (today)
- Agent: `codex-ir-core`
  - Scope: `docs/BACKLOG.md` claim, `shell/prisma-ir` validation API hardening.
  - First outputs: updated invariants + sync checklist for C++ Rust parity.

- Agent: `codex-decoder`
  - Scope: `shell/prisma-decoder/{decode.rs,tables.rs}`.
  - First outputs: minimal instruction family pass (1-2 families implemented + unit tests).

- Agent: `codex-cache`
  - Scope: `shell/prisma-cache/{cache.rs,compress.rs,sha256.rs}`.
  - First outputs: translation cache constructor + lookup/insert tests.

- Agent: `claude-passes`
  - Scope: `shell/prisma-passes/{pipeline.rs,*}`.
  - First outputs: `PassPipeline` + `default_pipeline` wiring + one pass parity test.

- Agent: `claude-runtime`
  - Scope: `shell/prisma-runtime/{dispatcher.rs,...}`.
  - First outputs: dispatcher skeleton state machine + signal/SMC interface stubs aligned to C API.

- Agent: `codex-backend`
  - Scope: `shell/prisma-backend/{assembler.rs,lowerer.rs,abi.rs}`.
  - First outputs: assembler buffer + one lowering smoke test.

- Agent: `codex-integracion`
  - Scope: `shell`, `core/src` differential scaffolding.
  - First outputs: CI-visible smoke harness and baseline cross-boundary checks.

### Coordination rule for this extreme mode
- No two agents edit the same Rust module.
- Every agent writes progress into `docs/WORK_QUEUE.md` row owner + notes.
- Core claims are in `docs/BACKLOG.md` with `[~|agent]`.
- Daily handoff: `docs/HANDOFF/HANDOFF-*.md` (one file per active Rust territory) plus
  `docs/HANDOFF/HANDOFF-coordination.md` for owner-level decisions.
