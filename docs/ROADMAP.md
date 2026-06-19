# Prisma — Roadmap completo

> Plan ejecutable hacia la v1.0. Deriva de
> [PROYECTO_PLAN_EJECUCION.md](../PROYECTO_PLAN_EJECUCION.md) (las 8 fases
> estratégicas, abril 2026 – Q2 2030) y lo expande en tareas concretas y
> accionables. Para el estado verificado actual ver [STATUS.md](STATUS.md).

Última actualización: 2026-06-19 America/Mexico_City.

---

## 0. Dónde estamos parados (resumen)

- **Técnicamente: mitad de Fase 2.** El core C++ es un DBT x86→ARM64 funcional
  que **JIT-ejecuta en ARM64 real** (Apple Silicon / runner ARM64 de CI) para un
  subconjunto amplio del ISA. 1,202 tests Catch2.
- **Migración a Rust en marcha** (RFC 0015, `shell/`): existe el primer motor de
  traducción Rust integrado (`prisma-translator`: decode → optimizar → lower →
  cache, + fusión de bloques con renumeración SSA), todavía sin ejecución y con
  un subconjunto de instrucciones más chico que el core C++.
- **Vacío todavía:** entorno de SO invitado (PE loader, Win32/NT, Wine), app
  Android real, servidor P2P, stack gráfico.
- **El hito "correr una `.exe` de Windows en la Mac" es Fase 3** (Wine), aún no
  empezada.

Detalle con evidencia de archivo:línea en [STATUS.md](STATUS.md).

---

## 1. Principio de trabajo: agentes en paralelo

El trabajo se organiza en **workstreams independientes** que se ejecutan con
agentes en paralelo (ver [COORDINATION.md](COORDINATION.md) y
[AGENT_PLAYBOOK.md](AGENT_PLAYBOOK.md)). Reglas:

- Cada agente toma una **frontera de archivos cerrada** y la marca en
  `docs/BACKLOG.md` (`[~|<agente>]`).
- Fan-out típico para una familia de instrucciones nueva: 1 agente decoder, 1
  backend/lowering, 1 tests/differential, sincronizados por el IR.
- Fan-out para revisión/auditoría: N agentes por dimensión (correctness,
  seguridad, perf), verificación adversarial, síntesis.
- Antes de mergear a `main`: validar en rama/PR de integración y esperar CI
  verde (los contexts requeridos: `core-build`, `asan-ubsan`, `tsan`,
  `ir-spec-build`, `ffi-link`/`ffi-link-arm64`/`ffi-link-windows`, `markdownlint`,
  `check-rfc-frontmatter`, `shell-check`, `benchmarks-smoke`).

---

## 2. Track inmediato (ahora → próximas semanas)

Dos workstreams paralelos. **A** estabiliza el core C++ y empuja Fase 2; **B**
completa la reescritura estratégica a Rust.

### Track A — Cerrar ISA + Linux user-mode (core C++)

Objetivo: ejecutar binarios x86-64 Linux reales (coreutils) con correctness.

- [ ] **Syscall translation layer (Linux x86-64 → ARM64).** ~80 syscalls
      críticos con flags y edge cases. `clone()`, `futex()`, `mmap()` son cada
      uno semanas. Primer multithreading real. *(agente: runtime)*
- [ ] **x87 FPU**: emulación software mínima (FLD/FST/FADD/FMUL/FILD/FISTP + el
      stack de 8 registros). *(agente: decoder + backend)*
- [ ] **Completar huecos SIMD** (según STATUS): SSE4.2 PCMPxSTRx, AVX2 enteros
      (VPBROADCAST, VINSERTI128, shifts variables), BMI1 (ANDN/BEXTR/BLSI/
      BLSMSK/BLSR), F16C, PCLMULQDQ, thin spots SSSE3/SSE4.1. *(N agentes por
      familia)*
- [ ] **Modelo de flags NZCV en el IR** (Pilar 2 / materialización diferida) —
      reemplazar el `Select` booleano por flags reales con flag-write-elim. *(IR
      + passes)*
- [ ] **TSO experiment**: flag switch conservative/relaxed en LOCK/XCHG/CMPXCHG,
      empezar a medir. *(runtime + passes)*
- [ ] **Suite coreutils x86-64**: target 85%+ passing. *(tests)*
- [ ] **Fuzzing continuo** (AFL++ ya existe en `fuzz/`): acumular compute. *(infra)*
- [ ] **Benchmark académico** (Dhrystone/CoreMark/nbench/SPEC subset) vs
      QEMU/Box64/FEX/nativo. Target 30-45% del nativo. *(tests + docs/paper)*

**Decision point honesto (fin Fase 2):** si el benchmark < 25% del nativo, hay
algo estructural en el IR — parar y rediseñar.

### Track B — Completar la migración Rust (`shell/`)

Objetivo: paridad funcional con el core C++, todo en Rust (RFC 0015).

- [ ] **Decoder Rust: cerrar la brecha** con el C++ (hoy cubre un subset).
      Validar cada familia contra el differential C++↔Rust (ya verde en CI).
      *(agente: codex-decoder)*
- [ ] **Backend Rust: completar lowering** (Pdep/Pext bit-loop, Rcl/Rcr, vector
      ops). *(agente: codex-backend)*
- [ ] **Runtime Rust: ejecución JIT real** — hoy `prisma-runtime` tiene
      `jit_memory` (W^X) y dispatcher de contrato; falta wirear el translator
      al loop de ejecución en ARM64 y correr e2e. *(agente: claude-runtime)*
- [ ] **Syscall handler Rust real** (hoy es boundary tipado `Ok(0)`). *(runtime)*
- [ ] **Utilidad de renumeración SSA** ya existe (`Op::map_refs`); úsala para la
      optimización cross-instrucción en el translator (ya hecho parcialmente).
- [ ] **Sustitución incremental**: reemplazar tramos del pipeline C++ por
      llamadas a Rust vía el C-ABI (RFC 0014) sin regresión. *(integración)*
- [ ] **Paridad de cobertura de fuzzing** (decoder/cache/passes/backend/
      translator ya tienen harness proptest).

---

## 3. Fase 2.5 — Investigación de frontera (los 6 pilares épicos)

Prototipos funcionales, no producto. Esto diferencia Prisma de "otro Winlator".

- [ ] **Pilar 1 — NPU-assisted translation**: pipeline Python (dataset de
      binarios x86 + trazas → features → clasificador de regiones), ONNX Runtime
      Android con delegate NPU (NNAPI/NeuroPilot), integración con el DBT para
      hints antes de traducir bloques calientes. Medir tiempo de traducción,
      calidad, energía. → paper MICRO 2028.
- [ ] **Pilar 2 — Validación formal (Lean 4)**: probar que las transformaciones
      preservan semántica bajo los invariantes del clasificador TSO. Cerrar el
      sorry budget; completar specs de los IR ops nuevos.
- [ ] **Pilar 3 — TSO adaptativo**: clasificador de 5 categorías
      (single-threaded, lock-free, shared-mutable, I/O, unknown); regiones
      single-threaded/lock-free sin `DMB ISH`; suite de 30 programas multi-hilo,
      cero regresiones. → blog post con prueba de correctitud.
- [ ] **Pilar 4 — Translation cache distribuida**: servidor Rust/Axum +
      Cloudflare R2 + P2P estilo BitTorrent con firma Ed25519 (Fase 2.5 del
      trust envelope; `prisma-cache` ya tiene SHA-256). → llena `server/`.
- [ ] **Pilar 5 — Virtualización híbrida** (Pixel 7a/8/9): integración con AVF,
      Windows-on-ARM guest en crosvm, bridge para que el DBT detecte regiones
      ejecutables nativamente por el guest.

**Decision point crítico:** si un pilar no funciona, reportarlo honestamente
(paper negativo / blog post).

---

## 4. Fase 3 — Wine + Windows real (**el hito que importa**)

Meta: Notepad/Calc/Paint XP y luego un programa "serio" corriendo en Android.

- [ ] **Loader PE propio** (referencia: Wine `loader/`). Parse de imports,
      resolución de DLLs. *(hoy solo se mapea el PE en memoria — ver
      `shell/orchestrator/pe_loader.rs`)*.
- [ ] **Wine bridge**: integrar Wine como submódulo; compilar Wine ARM64 con
      parches para llamar a Prisma cuando encuentra código x86. **12 semanas.**
- [ ] **Win32/NT syscalls**: hoy solo Linux/POSIX. Implementar la superficie
      Windows que Wine necesita.
- [ ] **Container system** tipo Winlator: prefix Wine + config por contenedor
      (`shell/orchestrator/container.rs` es el esqueleto — `start()` devuelve
      `NotImplemented`).
- [ ] **Filesystem virtual overlay**: base read-only + overlay writable.
- [ ] **App Kotlin/Compose mínima**: importar `.exe`, ejecutar, ver logs.
      *(`android/` es solo andamiaje hoy)*.
- [ ] **Servidor X11 embebido** para renderizado.
- [ ] **Primer programa serio**: Notepad XP estable; Photoshop 7 / AutoCAD LT
      2000 como stretch.

**Entregable:** video de Notepad XP en hardware Android real; Discord 500-1000.

---

## 5. Fase 4 — Juegos + Pilar 6 (Graphics)

- [ ] **Stack gráfico base**: DXVK 2.x + VKD3D-Proton 3.x, Turnip auto-updater
      (AdrenoTools), Vulkan surface (ANativeWindow/SurfaceView), Performance Hint
      + Game Mode API.
- [ ] **Pilar 6 — Graphics translation avanzada**: shader graph analysis,
      adaptive texture transcoding por thermal budget, render-graph fusion,
      Vortek++ para Mali/Xclipse.
- [ ] **Benchmark público**: Prisma vs DXVK stock en 10 juegos (FPS, frametime
      p99, energía, temperatura). → paper SIGGRAPH/HPG 2029.
- [ ] **Targets**: HL2 45 FPS (Dimensity 8300) / 30 FPS (Snapdragon 7s Gen 2);
      Portal + NFS MW jugables.

---

## 6. Fase 5 — Beta cerrada + papers

- [ ] UX completo (onboarding, import Steam, gamepad mapper).
- [ ] Compatibility database (100 juegos).
- [ ] Telemetría opt-in (alimenta el ML del Pilar 1), crash reporting.
- [ ] 500 testers; ciclo de 3 meses de fixes.
- [ ] **Papers publicados** (MICRO 2029 / POPL 2030 / HPG 2029); prensa técnica.

---

## 7. Fase 6 — v1.0 + open-sourcing estratégico

- [ ] v1.0 pública.
- [ ] **Open-source del core MIT** (DBT core + IR formal + NPU models + graphics
      research). App Android comercial.
- [ ] Modelo comercial; target $3-8k MRR a fin de Q2 2030.

---

## 8. Pilas de tareas transversales (siempre disponibles)

Tareas que pueden tomarse en cualquier momento, en paralelo:

- **Infra/CI**: mantener verde los 10+ workflows; nuevos runners; cobertura.
- **Testing**: subir el conteo de tests; differential C++↔Rust; fuzzing; e2e
  ARM64.
- **Lean 4**: bajar el sorry budget; specs de IR ops nuevos.
- **Docs/research**: blog posts cada 2-3 meses; drafts LaTeX en `papers/`;
  RFCs en `docs/rfc/` para cada decisión/dependencia.
- **Hardening**: ASan/UBSan/TSan; auditorías de seguridad del boundary
  host↔guest (JIT W^X, signal handlers, deserialización de cache, claves P2P).
- **Deuda técnica**: revisión periódica del pipeline de pases (ver skill
  `prisma-pass-debt`).

---

## 9. Cómo se elige la siguiente tarea

1. Mirar el **Track inmediato** (§2) — ahí está el trabajo de mayor valor hoy.
2. Marcar el ítem en `docs/BACKLOG.md` con `[~|<agente>]` + commit del claim.
3. Si es una familia de instrucciones nueva, **fan-out de agentes** (decoder /
   backend / tests) sincronizados por el IR.
4. Validar en PR de integración, esperar CI verde, mergear, marcar `[x] (<sha>)`.
