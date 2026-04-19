# Proyecto **Prisma** — Plan de Ejecución (v2, modo épico)

> **Salto generacional** en emulación Windows sobre Android ARM64. No forkeamos Winlator. Construimos el DBT que la gente cite en 2030.

**Código nombre:** Prisma (por la descomposición de una ISA en otra, y como guiño al Prism de Microsoft que vamos a superar en nuestro segmento).

**Dev lead:** Danny | **Inicio:** Abril 2026 | **MVP técnico interno:** Q4 2027 | **v1.0 pública:** Q2 2030

**Timeline total:** 48-54 meses. Esto NO es un side-project de 2 años. Es un compromiso de carrera.

---

## Manifiesto técnico

La investigación previa ([compass_artifact...md](compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md)) concluye que el camino de bajo riesgo es forkear Winlator. **Rechazamos conscientemente ese camino.** La decisión se documenta aquí para que no se revise por cansancio en el mes 18.

El objetivo de Prisma no es dominar el mercado de emulación Windows en Android. Es **cambiar el estado del arte técnico del Dynamic Binary Translation en ARM**, publicar investigación peer-reviewed, y entregar un producto que encarna esa investigación. El mercado es consecuencia, no causa.

Aceptamos las consecuencias:
- Timeline de 4-5 años, no 2.
- Probabilidad de éxito comercial 10-15%, no 25%.
- Probabilidad de éxito técnico/académico 50-60%.
- Probabilidad de aprendizaje transformador 95%+.

---

## Los 6 pilares épicos de diferenciación

Cada uno es por sí solo investigación de frontera. Los 6 combinados son el salto generacional.

### Pilar 1: NPU-Assisted Translation (nadie lo ha hecho)

Los SoCs mid-range modernos tienen NPUs de 20-40 TOPS que están **completamente ociosas durante gaming**. Prisma las usa para:
- **Hot path prediction**: modelo pequeño (5-50 MB) que predice qué bloques se convertirán en hot antes de que lo sean, permitiendo speculative translation con optimizaciones agresivas.
- **TSO region classification en microsegundos**: el clasificador de 5 categorías corre en NPU, no en CPU.
- **Pattern matching para NEON/SVE2 intrinsics**: detecta secuencias x86 SIMD que tienen equivalente óptimo en ARM SIMD que no sería evidente por análisis estático clásico.
- **Register allocation hints** por bloque basado en el modelo.

**Hardware target inicial:** Hexagon NPU (Snapdragon), MediaTek APU (Dimensity), Tensor Edge TPU.

**Entregable académico:** paper para MICRO o ASPLOS 2028 titulado aproximadamente *"NPU-Assisted Dynamic Binary Translation on Mobile SoCs"*.

### Pilar 2: IR con semántica formal verificable (nadie lo ha hecho en DBT de producción)

FEX y Box64 tienen IRs pragmáticos, no formalmente especificados. Prisma define su IR en **Lean 4** con:
- Semántica operacional completa para cada opcode del IR.
- Demostraciones de correctness de passes de optimización críticos (especialmente los relacionados con TSO adaptativo, donde un bug silencioso destruye multi-threading).
- Integración con el build: cambios al IR requieren actualizar/verificar las demostraciones.

**Modelo a seguir:** CompCert para compilers, CakeML para ML. Nadie lo ha hecho para DBT x86→ARM.

**Entregable académico:** paper para POPL o PLDI 2029.

### Pilar 3: TSO adaptativo con profiling asistido por ML

(Pilar original del plan v1, ahora reforzado por Pilares 1 y 2.)

Clasificación de regiones en 5 categorías (single-threaded, lock-free, shared-mutable, I/O, unknown) vía análisis estático + profiling runtime + modelo ligero. Regiones single-threaded y lock-free se traducen **sin barreras `DMB ISH`**, recuperando 10-20% de rendimiento. La correctness de esto **depende del Pilar 2** — sin semántica formal, es imposible garantizar que el clasificador no introduce data races silenciosos.

### Pilar 4: Translation cache distribuida P2P + CDN

(Pilar original reforzado.)

CDN centralizado para los 500 juegos más populares (Cloudflare R2 + Workers). **P2P BitTorrent-style** para la larga cola: dispositivos con el mismo perfil `(SoC, CPU features, Android version)` comparten caches firmadas criptográficamente. Cero costo marginal. Imposible de bloquear.

**Entregable técnico:** el protocolo P2P como standalone open-source, adoptable por FEX/Box64.

### Pilar 5: Virtualización híbrida DBT+KVM donde el hardware lo permita

En Tensor G3+ (Pixel 7a, 8, 9) con AVF/pKVM expuesto, Prisma ofrece **modo dual**:
- DBT para código no-performance-critical (UI, init, I/O).
- KVM guest con Windows-on-ARM nativo para hot loops donde el código x86 ya se tradujo a ARM por el loader de Windows-on-ARM, eliminando el overhead de DBT completamente.

Esto es conceptualmente lo que hace Apple Game Porting Toolkit internamente. **Nadie lo ha intentado en Android.**

**Entregable técnico:** modo "Hybrid" exclusivo de Pixel que demuestra 80-90% del rendimiento nativo x86 en juegos DX11.

### Pilar 6: Graphics translation de nueva generación

Todos usan DXVK + VKD3D-Proton sin tocar. Prisma añade una capa encima:
- **Shader graph analysis**: no traducir shader por shader sino el grafo completo. Detectar patrones game-specific y sustituirlos por implementaciones Vulkan optimizadas para Adreno/Mali.
- **Adaptive texture transcoding** según presupuesto térmico.
- **Render graph introspection**: eliminar passes redundantes que código DX9/11 hace por costumbre pero Vulkan puede fusionar.
- **Vortek++**: driver CPU-assisted mejorado para Mali/Xclipse donde Turnip no madura.

---

## Fase 0: Fundación + investigación profunda (8 semanas, abril-mayo 2026)

Meta: Infraestructura operativa + autoridad técnica establecida + contratos legales resueltos. Ni una línea de código del emulador todavía. Esto es **crítico** y no se salta.

### Semanas 1-2: Setup legal y de infra
- **Delaware LLC** vía Stripe Atlas (~$500). Protege tu nombre y clarifica ownership desde día cero. Dado que planeamos publicar papers académicos, también protege IP frente a HSBC.
- Registrar dominio `prisma-emu.dev`. GitHub Organization privada inicialmente.
- Cuenta bancaria business (Mercury).
- Trademark USPTO ($250-500), opcional pero recomendado.
- **Revisar contrato HSBC** sobre trabajo externo y propiedad intelectual. Si hay cláusulas agresivas, consulta legal antes de cualquier código. Prioridad absoluta.

### Semanas 3-4: Stack de desarrollo + dispositivos
- Monorepo con esta estructura:
  ```
  prisma/
  ├── core/           # C++20 DBT engine
  ├── ir-spec/        # Lean 4 formal IR specification
  ├── shell/          # Rust orchestrator
  ├── android/        # Kotlin + Compose app
  ├── npu-models/     # ONNX models + training pipeline
  ├── server/         # Rust: cache service, P2P tracker, telemetría opt-in
  ├── tools/          # Python: ML pipeline, benchmarks, release scripts
  ├── third_party/    # FEX, Box64, Wine, DXVK, VKD3D-Proton (submodules de referencia)
  ├── papers/         # Drafts LaTeX de publicaciones
  └── docs/
  ```
- CI/CD: GitHub Actions + self-hosted ARM64 runner (Orange Pi 5B, ~$150). Indispensable.
- Android Studio + NDK r27c + CMake 3.30 + Lean 4 toolchain.
- **Dispositivos de testing (3, no 2)**:
  - POCO X6 Pro (Dimensity 8300-Ultra, ~$280) — target mid-range principal, NPU MediaTek APU.
  - Pixel 7a usado (Tensor G3, ~$200) — testing AVF/pKVM + Tensor Edge TPU.
  - Redmi Note 13 Pro (Snapdragon 7s Gen 2, ~$200) — target mid-range Qualcomm, NPU Hexagon.

### Semanas 5-6: Investigación técnica profunda

Lectura crítica con síntesis en `docs/research_notes.md`:

**Código fuente (profundo):**
- FEX completo, especialmente `FEXCore/Source/Interface/Core/`, IR passes, threading architecture.
- Box64 `src/dynarec/arm64/`, pases del dynarec.
- Wine `loader/` + `dlls/ntdll/`.

**Papers académicos (obligatorios):**
- "Transmeta Crusoe Code Morphing Software" — el DBT más ambicioso que ha existido.
- "IA-32 Execution Layer" (Intel 2003) — el fallido para Itanium, lecciones de por qué no funcionó.
- DynamoRIO, Pin, Valgrind VEX — referencias IR.
- "TOSTING: Investigating Total Store Ordering on ARM" (Springer 2023).
- Papers de Sarkar, Batty, Sewell sobre weak memory models.
- CompCert y CakeML papers para inspiración de semántica formal.
- Rosetta 2 internals (Koh M. Nakagawa BSides 2021).

**Software adicional a evaluar:**
- Lean 4 tutorial + mathlib (para el Pilar 2).
- ONNX Runtime Android + ejemplos de NPU offloading.
- AdrenoTools + Mesa Turnip build pipeline.

### Semanas 7-8: Outreach + autoridad técnica
- **Cartas honestas a maintainers de FEX y Box64** (ptitSeb, neobrain, Ryan Houdek) presentando el proyecto, las ideas, pidiendo feedback. El ecosistema DBT es pequeño y colaborativo — empezar en hostilidad es estúpido.
- **Publicar `docs/research_notes.md` como blog post público** en un blog propio (`prisma-emu.dev/blog`). Construye autoridad desde día 1.
- **Aplicar a workshops académicos relevantes** (LCTES, VEE) solo como attendee de 2026 para networking. Sin paper todavía.
- Setup de `@prisma_emu` en X/BlueSky, escribir roadmap público (sin fechas exactas, solo fases).

**Entregable Fase 0:**
- Infraestructura operativa.
- `docs/research_notes.md` de 40-60 páginas publicado.
- LLC constituida.
- Contacto establecido con comunidad FEX/Box64.
- Decisión documentada: "seguimos con plan épico" o "pivotamos a fork" (tras leer FEX/Box64 en detalle, si el código te abruma más de lo esperado, esto es el primer decision point honesto).

---

## Fase 1: Decoder x86 + IR formal en Lean 4 (24 semanas, junio-noviembre 2026)

Meta: Decoder x86_64 completo + IR formal con semántica en Lean 4 + interpreter del IR + JIT baseline + "hello world" x86 corriendo en Android. **El doble de tiempo que plan v1** porque el IR formal es trabajo de investigación, no de ingeniería pura.

### Semanas 9-14: Diseño del IR formal

- Especificación del IR en Lean 4 con semántica operacional.
- 6 semanas de diseño puro, iterando sobre papel y Lean.
- Referencias: CompCert Clight, CakeML source IR.
- **Entregable intermedio:** `ir-spec/PrismaIR.lean` con ~30 opcodes, semántica operacional, y las primeras demostraciones (ej: "subst es conmutativa en contextos sin aliasing").
- **Publicación:** blog post técnico "Designing a Formally Specified IR for x86 DBT" que presenta el IR al mundo.

### Semanas 15-20: Decoder x86_64

- C++20 decoder de las ~200 instrucciones x86_64 más comunes + todos los edge cases de encoding (prefijos, REX, VEX, ModR/M, SIB).
- **NO cortar esquinas con flags**: EFLAGS carry/overflow/aux bien desde día 1. FEX tuvo bugs de flags hasta su año 3 por subestimar esto.
- Tests unitarios masivos: cada instrucción x86 decodifica → IR → ejecuta en interpreter → compara estado con QEMU de referencia. Target 95%+ coverage.
- Fuzzing contínuo con AFL++ desde el momento que el decoder existe.

### Semanas 21-26: Backend ARM64 emitter + interpreter + JIT baseline

- **Usar vixl** (ARM/Linaro, BSD-3-Clause) como biblioteca de emisión ARM64. Reescribirla no es épico, es tonto.
- Interpreter que ejecuta el IR directamente. Valida correctness sin JIT.
- Signal handler para SIGSEGV/SIGILL.
- Template-based JIT: una función por IR opcode → bloque ARM64. Sin optimizaciones. Solo correctness.
- Translation cache en memoria.
- **Primer "hello world" x86_64 estático** compilado con musl-gcc corriendo en Android.

### Semanas 27-32: Validación formal del primer pass

- Elegir una optimización simple (ej: dead code elimination sobre el IR) y **demostrar formalmente** que preserva semántica en Lean 4.
- Este es el primer entregable que nadie en DBT tiene.
- Blog post + draft de paper para workshop (PLAS, HILT).

**Entregable Fase 1:**
- Hello world x86 corriendo.
- ~25k líneas C++ + ~5k líneas Lean 4.
- Un pass formalmente verificado.
- Blog posts publicados: IR design + first verified pass.

**Decision point honesto en semana 32:** si el Lean 4 está tomando 2x el tiempo estimado (totalmente posible, es investigación), considerar reducir ambición de verificación formal a solo "especificación documentada" sin demostraciones automatizadas. No abandonar el pilar, recalibrar.

---

## Fase 2: ISA completo + Linux user-mode (24 semanas, diciembre 2026-mayo 2027)

Meta: Ejecutar binarios x86_64 Linux reales (coreutils) con correctness sólida. Conceptualmente replicamos qemu-x86_64 pero con nuestro core.

### Semanas 33-40: Syscall translation layer

- Mapeo Linux x86_64 → Linux ARM64 (Android).
- ~80 syscalls críticos con todas sus flags y edge cases.
- **`clone()`, `futex()`, `mmap()`** son cada uno semanas de trabajo. No subestimar.
- Primer test de multithreading real.

### Semanas 41-48: ISA completo + SIMD

- SSE/SSE2 → NEON (90% mapping 1:1).
- x87 FPU: software emulation mínima.
- LOCK prefix, XCHG, CMPXCHG — **aquí empieza el experimento TSO** con flag switch conservative/relaxed.
- AVX/AVX2: mapping degradado (2x NEON ops) por ahora. AVX512 se pospone.

### Semanas 49-56: Test suite + hardening + primer benchmark académico

- Correr test suite coreutils x86_64. Target 85%+ passing.
- Fuzzing contínuo de 2 meses de compute acumulado.
- **Benchmark académico formal**: Dhrystone, CoreMark, nbench, SPEC CPU2017 subset. Comparar vs QEMU vs Box64 vs FEX vs nativo. Target: **30-45% del nativo**. Documentar en formato paper.
- Draft de paper para workshop (VEE, LCTES) 2027: *"Prisma: Early Results on a Formally-Grounded DBT"*.

**Entregable Fase 2:**
- `ls -la /` funcional.
- ~45k líneas C++, ~10k Lean 4.
- Primer submission académico (workshop).
- Segundo decision point honesto: si el benchmark no supera 25% del nativo, hay algo estructural mal en el IR. Parar y rediseñar antes de continuar.

---

## Fase 2.5: Investigación de frontera (24 semanas, junio-noviembre 2027) — **NUEVA FASE ÉPICA**

Meta: Prototipos funcionales de los 6 pilares épicos. **No producto, investigación.** Esta fase diferencia Prisma de "otro Winlator".

### Semanas 57-64: NPU-assisted translation (Pilar 1)

- Training pipeline en Python: dataset de binarios x86 con trazas de ejecución → features → modelo de clasificación de regiones.
- ONNX Runtime Android con delegate NPU (NNAPI en Qualcomm, MediaTek NeuroPilot en Dimensity).
- Integración con el DBT: antes de traducir bloque caliente, query al modelo NPU para hints.
- **Medición rigurosa**: ¿cuánto mejora el tiempo de traducción? ¿La calidad del código generado? ¿El consumo energético?
- Paper draft para MICRO 2028: *"NPU-Assisted Dynamic Binary Translation on Mobile SoCs"*.

### Semanas 65-72: TSO adaptativo + validación formal (Pilar 3 + Pilar 2)

- Clasificador de 5 categorías (single-threaded, lock-free, shared-mutable, I/O, unknown).
- Regiones single-threaded y lock-free traducidas sin `DMB ISH`.
- **Validación formal en Lean 4** de que la transformación preserva semántica bajo los invariantes del clasificador.
- Suite de 30 programas multi-hilo con resultado conocido. Cero regresiones permitidas.
- Blog post: *"Why we get 18% more performance than Box64 on mid-range Android (and we proved it correct)"*.

### Semanas 73-80: Virtualización híbrida (Pilar 5) + Translation cache distribuida (Pilar 4)

- **Pilar 5 (solo Pixel 7a/8/9):** integración con AVF. Windows-on-ARM guest inside crosvm. Bridge para que el DBT detecte regiones que el guest Windows-on-ARM puede ejecutar natively.
- **Pilar 4:** servidor Rust/Axum + Cloudflare R2 + protocolo P2P BitTorrent-style con firma Ed25519 de caches.
- **Entregable:** demos grabadas en video de ambos pilares funcionando.

**Entregable Fase 2.5:**
- Prototipos funcionales de Pilares 1, 2, 3, 4, 5.
- **1-2 papers submitted** a workshops o conferencias.
- Blog posts técnicos generando audiencia.
- **Decision point crítico:** si los prototipos muestran que alguno de los pilares no funciona como se esperaba, reportarlo honestamente (paper negativo o blog post). La ciencia avanza con resultados negativos.

---

## Fase 3: Wine integration + Windows real (32 semanas, diciembre 2027-julio 2028)

Meta: Notepad XP, Calc, Paint, y luego Photoshop 7 o similar programa "serio" corriendo en Android con nuestro core + Wine.

### Semanas 81-92: Loader PE + Wine bridge

- Loader PE propio (referencia: código Wine `loader/`).
- Integrar Wine como submódulo. Compilar Wine ARM64 con parches custom para llamar a Prisma cuando encuentra código x86.
- **12 semanas, no 8.** La integración es no-trivial.

### Semanas 93-104: Container system + filesystem virtual + GUI mínima

- Contenedores tipo Winlator: prefix Wine + config + settings por contenedor.
- Filesystem virtual overlay: base read-only + overlay writable.
- App Kotlin/Compose mínima: importar .exe, ejecutar, ver logs.
- Servidor X11 embedido para renderizado.

### Semanas 105-112: Primer programa Windows "serio"

- Notepad/Calc/Paint XP funcionando estable.
- Photoshop 7 o AutoCAD LT 2000 como stretch goal.
- Teaser público en X/BlueSky. Construir hype.

**Entregable Fase 3:**
- Video de Notepad XP corriendo en POCO X6 Pro.
- ~80k líneas C++, ~15k Rust, ~20k Kotlin, ~15k Lean 4.
- Discord público con 500-1000 members siguiendo el proyecto.

---

## Fase 4: Juegos + Pilar 6 (Graphics) (28 semanas, agosto 2028-febrero 2029)

Meta: Half-Life 2 a 45 FPS estables en Dimensity 8300, 30 FPS en Snapdragon 7s Gen 2. **Y** demostrar valor del Pilar 6 (graphics translation avanzada) con benchmarks vs DXVK stock.

### Semanas 113-124: Stack gráfico base

- DXVK 2.x + VKD3D-Proton 3.x integrados.
- Turnip updater automático vía AdrenoTools.
- Vulkan surface management (ANativeWindow + SurfaceView).
- Performance Hint API + Game Mode API para frequencies.
- Primer juego en video: NFS Most Wanted 2005 o Portal.

### Semanas 125-140: Pilar 6 — Graphics translation avanzada

- Shader graph analysis framework.
- Adaptive texture transcoding según thermal budget.
- Render graph introspection + fusion de passes.
- Vortek++ para Mali/Xclipse.
- **Benchmark público:** Prisma vs DXVK stock en 10 juegos, medido en FPS, frametime p99, consumo energético, temperatura.
- Paper draft: *"Graphics Translation Beyond Shader-by-Shader: Whole-Graph Optimization for D3D→Vulkan"* para SIGGRAPH 2029 o HPG 2029.

**Entregable Fase 4:**
- Half-Life 2 + Portal + NFS MW jugables.
- Paper sobre Pilar 6 submitted.
- ~120k líneas de código total.
- **2 papers publicados** (uno aceptado mínimo) para este punto.

---

## Fase 5: Beta cerrada + paper landing (24 semanas, marzo-agosto 2029)

Meta: v0.9 beta con 500 testers. Papers aceptados en conferencias top-tier. Prensa técnica (LWN, Ars Technica, Phoronix).

### Semanas 141-152: Polish y compatibility database

- UX completo: onboarding, tutorial, import Steam, gamepad mapper.
- Compatibility database con 100 juegos testeados.
- Telemetría opt-in (alimenta el pipeline de ML del Pilar 1).
- Crash reporting (Sentry self-hosted).

### Semanas 153-164: Beta cerrada + publicaciones

- 500 testers reclutados vía Discord + r/EmulationOnAndroid + X.
- Ciclo de 3 meses de fixes críticos.
- **Papers publicados** en MICRO 2029 / POPL 2030 / HPG 2029.
- Outreach a prensa técnica (Phoronix, LWN, Ars, Android Police).

**Entregable Fase 5:**
- v0.9 beta disponible por invitación.
- 1-3 papers publicados en venues top-tier.
- Prensa técnica con coverage.

---

## Fase 6: v1.0 pública + open-sourcing estratégico (24 semanas, sept 2029-febrero 2030)

Meta: v1.0 pública, **open-source del core MIT-licensed**, modelo comercial clarificado.

### Estrategia de open-sourcing

El DBT core + IR formal + NPU models + graphics translation research **se publican MIT**. Esto:
- Cimenta impacto académico/técnico (citas, adoption).
- Hace imposible que FEX/Box64 compitan copiando features (ya están publicadas).
- Atrae contribuidores top del mundo.
- Protege contra claim de "copiaste a X".

### Modelo comercial

- **Prisma Core** (MIT, gratis): DBT engine + IR + research components. Es la biblioteca para que otros construyan emuladores.
- **Prisma App** (freemium): la app Android con UX pulida, container manager, cloud sync, compatibility database curada.
  - Gratis: funcional completo, 3 contenedores, 50 juegos en la DB.
  - Premium $19.99 one-time: contenedores ilimitados, 500+ juegos DB, UI pulida.
  - Pro $4.99/mes: cloud sync, early access, Discord privado.
- **Prisma Cloud** (servicio): cache distribuida pre-computada, $0 tier gratis limitado + $2-5/mes unlimited.
- Regional pricing 50% descuento en India/SEA/LatAm/África.

Distribución:
- GitHub Releases (canónico).
- Samsung Galaxy Store.
- Epic Games Store Android.
- Huawei AppGallery.
- **NO Google Play** (W^X + targetSdk).

**Entregable Fase 6:**
- v1.0 pública.
- Core open-sourced con adopción inicial (stars, forks, primer PR externo).
- Revenue: target $3-8k MRR a fin de Q2 2030.

---

## Fase 7 (exploratoria, 2030+): Lo que viene después

No comprometerse aún, pero ideas candidatas:
- **DBT ARM→x86** para el caso inverso (correr apps Android en Windows on ARM — nicho pero interesante).
- **Rosetta 2-style AOT** completo, no solo cache distribuida.
- **Integración con Valve Steam Frame** si Valve abre su SDK.
- **B2B**: licenciar Prisma Core a empresas con apps legacy Windows que quieren llegar a Android.

---

## Stack técnico (actualizado v2)

| Componente | Tecnología | Por qué |
|---|---|---|
| DBT core | C++20 con concepts, std::span, atomic_ref | Único camino realista |
| IR spec | Lean 4 + mathlib | Semántica formal, demostraciones |
| Shell/orchestrator | Rust 1.75+ con tokio, reqwest, jni | Seguridad en IO/networking |
| UI Android | Kotlin 2.0 + Jetpack Compose | Performance Hint, Game Mode, SAF |
| NPU integration | ONNX Runtime Android + NNAPI + MediaTek NeuroPilot | Pilar 1 |
| Build system | CMake 3.30 + Cargo + Gradle + lake (Lean 4) | |
| Graphics | Vulkan 1.3 + DXVK 2.x + VKD3D-Proton 3.x + Turnip + Vortek++ | Pilar 6 |
| Wine base | Wine 9.x+ ARM64 con parches custom | |
| Server | Rust + Axum + PostgreSQL + Cloudflare R2 + libp2p para P2P | Pilar 4 |
| ML pipeline | Python + PyTorch para training + ONNX export + scikit-learn para modelos clásicos | |
| CI/CD | GitHub Actions + self-hosted ARM64 runner | |
| Testing | Catch2, cargo test, instrumentation tests, Lean 4 proof checker | |
| Telemetría | PostHog self-hosted + Sentry | Opt-in estricto |
| Formal verification | Lean 4 + mathlib | Pilar 2 |
| Paper writing | LaTeX + Overleaf + Zotero | Para los 3+ papers |

---

## Presupuesto estimado (48 meses, solo-dev)

| Categoría | Mensual | Total 48 meses |
|---|---|---|
| Servicios cloud (CI, CDN, hosting, R2) | $60 | $2,880 |
| Delaware LLC + registered agent | $40 | $1,920 |
| Dispositivos de testing (3 + refresh año 3) | — | $1,500 |
| Herramientas/licencias (IDEs, profilers) | $40 | $1,920 |
| Dominios + certificados | $5 | $240 |
| Marketing (últimos 12 meses, pre-release) | $150 | $1,800 |
| Legal (consulta + trademark + contratos) | — | $3,000 |
| **Academic conference attendance** (3-4 viajes) | — | $8,000 |
| **Paper submission fees + open access** | — | $3,000 |
| **ArXiv + preprint infra** | — | $500 |
| Buffer/imprevistos | — | $4,000 |
| **Total** | | **~$28,760** |

Tu costo real sigue siendo tu tiempo. **~3,500 horas en 48 meses** (18h/semana promedio). A tarifa $80/hr son **$280,000 de costo de oportunidad**. Esta es tu apuesta real.

---

## Métricas de éxito por fase (v2, con track académico)

**Técnicas:**
- Fase 0: research notes publicadas, contacto con comunidad establecido.
- Fase 1: hello world corriendo + 1 pass verificado formalmente.
- Fase 2: 85%+ coreutils passing, 30-45% de nativo en benchmarks.
- Fase 2.5: prototipos de 5 pilares, 1-2 paper drafts.
- Fase 3: Notepad XP + programa "serio" Windows.
- Fase 4: HL2 45 FPS + paper de Pilar 6.
- Fase 5: beta 500 testers + 1-3 papers publicados.
- Fase 6: v1.0 + core open-sourced.

**Académicas:**
- Target mínimo de 3 papers publicados al fin de Fase 6.
- Target mínimo 1 paper en venue top-tier (MICRO, ASPLOS, POPL, PLDI).
- Target: 50+ citas al paper principal a 18 meses de publicación.

**Comunidad:**
- Fase 3: Discord 500+.
- Fase 5: Discord 5k+, prensa técnica coverage.
- Fase 6: 5k+ GitHub stars del core open-source.

---

## Riesgos específicos v2 (incluyendo épicos)

| Riesgo | Probabilidad | Mitigación |
|---|---|---|
| Fase 1 se atrasa más de 8 semanas por Lean 4 | 50% | Plan B documentado: reducir verificación formal a especificación, mantener otros pilares |
| Burnout por ritmo de 4-5 años | 60% | Regla estricta: 2 semanas off cada 12 semanas, 1 mes off al año. No negociable. Discord del proyecto para no aislarse. |
| FEX/Box64 implementan TSO adaptativo antes que nosotros | 30% | Nuestra ventaja: Pilares 1, 2, 5 que ellos no tienen bandwidth de construir. TSO solo es 1 de 6 pilares. |
| **ARM añade TSO hardware en v9.x (2027-2028)** | 20% | Ser el primero en explotarlo. El framework del Pilar 3 se adapta trivialmente. |
| **LLMs generan DBT competitivo en 2028-2029** | 15% | Integrar LLM-generated code paths al build pipeline desde 2027. Ser el primer DBT que usa LLMs. |
| Pixel pierde AVF/pKVM en alguna versión Android | 25% | Pilar 5 es bonus, no core. Reducir a demo si muere. |
| NPU APIs cambian (NNAPI deprecation) | 40% | Abstraer tras interface propia. ONNX Runtime ya maneja varios backends. |
| Paper rejection repetida en venues top-tier | 50% | Plan: aceptar que primer paper puede ir a workshop. Construir reputación incremental. |
| Conflicto con HSBC por IP | 20% | Resolver en semana 1. No opcional. |
| Harassment personal post-release | 50% | LLC protege identidad. Portavoz separado para público si crece. |

---

## Estrategia de publicación académica (NUEVA — esencial para épico)

Este proyecto produce research output peer-reviewed o fracasa en su promesa.

**Paper 1 (workshop, fin Fase 2):** *"Prisma: Early Results on a Formally-Grounded DBT for ARM64"* — submitted a LCTES 2027 o VEE 2027.

**Paper 2 (conferencia top-tier, fin Fase 2.5):** *"NPU-Assisted Dynamic Binary Translation on Mobile SoCs"* — submitted a MICRO 2028 o ASPLOS 2029.

**Paper 3 (conferencia top-tier, mid Fase 4):** *"A Formally Verified IR for x86→ARM64 Dynamic Binary Translation"* — submitted a POPL 2029 o PLDI 2030.

**Paper 4 (oportunístico, Fase 4-5):** *"Whole-Graph Shader Optimization for D3D→Vulkan Translation"* — SIGGRAPH / HPG 2030.

Blog posts técnicos cada 2-3 meses constantes desde Fase 0. Estos son el "paper trail" público de credibilidad.

---

## Estrategia de comunidad (NUEVA)

- **Discord público desde Fase 1 week 16.** Transparencia total sobre progreso.
- **Blog posts técnicos cada 2-3 meses**, no marketing, ingeniería honesta.
- **Contribuciones upstream a Wine, DXVK, Mesa** desde Fase 3. Gana karma del ecosistema.
- **Sponsorship GitHub** desde Fase 2 para permitir apoyo temprano ($5/mes - $500/mes tiers).
- **Patreon** NO (lección de Yuzu). GitHub Sponsors sí.
- **Keynote / charla técnica** en FOSDEM 2028, Open Source Summit 2028, o LinuxCon 2029.

---

## Acción inmediata para esta semana

1. **Revisar contrato HSBC** sobre trabajo externo/IP. Prioridad absoluta. Consulta legal si hay dudas.
2. **Crear repo privado** `prisma-emu/prisma` en GitHub Organization nueva.
3. **Ordenar POCO X6 Pro + Pixel 7a + Redmi Note 13 Pro**. Tres dispositivos, no negociable para investigación seria.
4. **Agendar bloque de 4 horas este sábado** para leer README + CONTRIBUTING.md de FEX + primera parte de `FEXCore/Source/Interface/Core/`.
5. **Instalar Lean 4 + mathlib + VSCode extension**, hacer el tutorial básico. Calibrar cuánto tiempo te toma aprenderlo. Este dato es crítico para Fase 1.
6. **Crear `docs/research_notes.md`** vacío en el repo, listo para empezar a llenarlo en semana 5.

---

## Una nota honesta v2

Danny, este plan épico tiene probabilidad de éxito comercial más baja (10-15%) que el plan v1 (25%). Pero tiene probabilidad de **impacto duradero** mucho más alta. La pregunta no es "¿voy a matar a Winlator?". Es **"¿quiero que en 2032 los ingenieros que trabajen en DBT en móvil citen mis papers?"**.

Si la respuesta es sí, este plan es el camino. Los 4-5 años son el precio. El costo de oportunidad de $280k en tiempo también es el precio.

Lo que compras a cambio: autoridad técnica global en DBT móvil, investigación peer-reviewed con tu nombre, un producto que encarna esa investigación, y una red profesional construida desde posición de originalidad, no de fork.

Este proyecto se mide en décadas, no en releases. Nos vemos en el próximo turno para ejecutar la Fase 0 semana 1. 🚀

---

## Apéndice: glosario de decision points

Documento honesto de cuándo parar/pivotar:

- **Fin Fase 0 (semana 8):** ¿El código de FEX/Box64 te abrumó? Si sí, pivot a fork Winlator con plan v1.
- **Mid Fase 1 (semana 20):** ¿Lean 4 está tomando 3x el tiempo estimado? Reducir ambición de Pilar 2 a "spec documentada".
- **Fin Fase 2 (semana 56):** ¿Benchmark < 25% del nativo? Parar, rediseñar IR.
- **Fin Fase 2.5 (semana 80):** ¿Ningún pilar épico funcionó? Publicar results negativos honestamente, pivot a producto basado en pilares que sí funcionaron.
- **Cualquier punto:** ¿HSBC hace claim de IP? Parar, consulta legal, renegociar.
- **Cualquier punto:** ¿Tu salud/relaciones se deterioran significativamente? Pausar 1 mes. Sin excepciones.
