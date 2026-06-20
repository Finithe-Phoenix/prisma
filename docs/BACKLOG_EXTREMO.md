# Prisma — Backlog EXTREMO

> **Qué es esto.** Un backlog *ejecutable* de alta resolución para guiar el
> trabajo de los próximos meses. No reemplaza a [BACKLOG.md](BACKLOG.md) (el mapa
> canónico de ~530 ítems a 48-54 meses) ni a [WORK_QUEUE.md](WORK_QUEUE.md) (el
> estado de la sesión activa): los **enlaza** y los **decompone** en paquetes de
> trabajo (EWP) listos para fan-out de agentes, cada uno con frontera de archivos
> cerrada, criterios de aceptación verificables, lane de owner, dependencias y
> forma de paralelización.
>
> **Cómo se usa.** Elegí un EWP del *camino crítico* (§1) o de un EPIC abierto.
> Claimealo en [BACKLOG.md](BACKLOG.md) (`[~|<agente>]` + commit del claim, ver
> [COORDINATION.md](COORDINATION.md)). Fan-out según la columna *Forma*. Cuando
> cierra, marcá `[x] (<sha>)` en el ID canónico y movés la fila a ✅ aquí.
>
> Última generación: 2026-06-19 America/Mexico_City. Anclado a
> [STATUS.md](STATUS.md) (estado verificado en fuente) + [ROADMAP.md](ROADMAP.md)
> §2 (track inmediato).

---

## 0. "Estás aquí" — el único dato que importa antes de elegir

El core C++ **ya traduce y ejecuta** x86→ARM64 para un ISA amplio en hardware
real. Lo que **bloquea correr software real** no es más ISA — es el **entorno de
ejecución**: (1) un modelo de flags NZCV de verdad, (2) syscalls completos +
threading, (3) el puente a Windows/Wine. Todo lo demás (más SIMD, más opt) es
incremental y de menor palanca.

**Movimiento de mayor valor hoy:** **completar el syscall layer + threading
(EPIC A: `futex`/`clone`)** — es el verdadero bloqueo para correr binarios Linux
reales (coreutils). Co-igual en palanca: **terminar la migración al pilar de
flags NZCV (EPIC C)**.

> **⚠️ Corrección verificada (2026-06-19).** Una versión previa decía que "los
> compares materializan booleano y no hay ADC/SBB/RCL/RCR correctos". La
> auditoría de fuente lo matiza: (1) el IR **ya tiene** un pilar de flags NZCV
> real — `WriteFlags`/`ReadFlag`/`CondJumpFlags` bajan a ADDS/SUBS/ANDS +
> CSET/CSEL (`ir.hpp:332-354`, `lowering.cpp:1248-1280`); el legacy `Compare`→
> CSET-booleano (`lowering.cpp:624-639`) coexiste y es lo que falta retirar.
> (2) **ADC/SBB/RCL/RCR con carry real YA aterrizaron en `main`** (PRs #54/#55,
> subsistema de CF persistente). Así que EPIC C es *completar una migración*, no
> construir de cero — más tractable, igual de valioso (TSO medible + paridad
> Rust + base de Pilar 2).

---

## 0.5. Lane permanente: liberar memoria y recursos SIEMPRE *(no opcional)*

> **Directiva de Danny (2026-06-19): vamos liberando memoria siempre, a la par de
> todo lo demás.** No es un EWP que se "termina" — es un **lane continuo que corre
> en paralelo a cada otro paquete de trabajo**. Toda fila de este backlog que
> toque memoria ejecutable, mmaps, fds o archivos de cache **arrastra esta
> disciplina como criterio de aceptación**. Es la cláusula obligatoria de
> [CLAUDE.md](../CLAUDE.md#cláusula-obligatoria-disciplina-de-memoria-y-recursos).

**Por qué importa estructuralmente:** el server (runtime host / backend P2P en
`server/`) **se reinicia**. Un recurso filtrado no debe sobrevivir un reinicio,
filtrarse entre reinicios, ni corromper estado. Por eso liberar es *parte del
diseño*, no una limpieza posterior.

**Reglas operativas (aplican a cada commit, en paralelo a todo):**

- Cada allocación tiene un dueño cuyo `Drop` (Rust) / destructor RAII (C++) la
  libera. **Preferir RAII sobre free manual. Nada de leaks.**
- Rutas de shutdown/restart liberan **explícitamente** (flush → close → unmap);
  no confiar en que la salida del proceso reclame (un reinicio puede no ser
  salida limpia).
- Nada de `mem::forget` (Rust) ni ownership-leak de un recurso del SO sin
  justificación en `docs/rfc/`.
- **Código nuevo que asigna mem ejecutable / mmap / fd debe añadir el `Drop` + un
  test (o chequeo ASan/leak) de que el recurso se libera** — en el *mismo* PR, no
  después.

**Superficies sensibles a reinicio que vigila este lane** (cada una con su EWP de
verificación en EWP-H4):

| Recurso | Dueño / liberación esperada |
|---------|------------------------------|
| Mem ejecutable JIT (`ExecBuffer`/`JitSlabPool`/MAP_JIT/mmap W^X) | desmapeo en `Drop` / al evictar |
| Mapeos del guest (PE loader, espacio de direcciones invitado) | unmap en teardown del contenedor |
| File handles + archivos de translation cache (RFC 0007) | flush + close limpio en shutdown |
| Entradas en memoria de la cache | liberadas en eviction LRU/byte-budget y en `clear_cache`/invalidación SMC |
| Waiters de futex, frames/stacks por hilo (EPIC A/D) | liberados en join del hilo guest |
| Slabs/buffers del executor Rust (EWP-B3) | `Drop` del slab pool |

**Forma de trabajo:** cuando un agente toma cualquier EWP, **antes de marcarlo
Done** corre el sub-check de RAII (EWP-H4) sobre su frontera de archivos. El lane
no se cierra nunca: es el chequeo que acompaña a cada merge. Verificación
automatizable vía ASan/leak-check en CI (`asan-ubsan`) y tests de `Drop`
explícitos.

---

## Bitácora de ejecución (viva)

Progreso del loop de ejecución (rama por historia → validar → integrar → limpiar).
El mapa canónico sigue siendo [BACKLOG.md](BACKLOG.md); esto registra qué EWPs
avanzaron para que la campaña multi-mes no pierda el hilo.

**Sesión 2026-06-19 (autónoma):**

- **EWP-F1 (PE loader → Wine)** — avanzado sustancialmente. El loader pasó de
  "solo mapea memoria" a: parsear la **import table** (DLLs + símbolos por
  nombre/ordinal, `parse_imports`), aplicar **base relocations** (HIGHLOW/DIR64,
  `apply_relocations`), un **cap anti-OOM** sobre `size_of_image` no confiable
  (`ImageTooLarge`), y un **fuzz proptest** del parser. PRs #62/#63/#64/#65.
  Falta: resolución/binding de imports (necesita proveedor de DLLs / Wine),
  TLS callbacks, forwards.
- **EWP-F4 (container lifecycle)** — `create`/`destroy` reales sobre el prefix
  (PR #66); `destroy` limpia el árbol completo (cláusula de recursos). Falta:
  `start`/`stop` (backend Wine), registry/listado, overlay FS (EWP-F5).
- **EWP-H6 (paridad de fuzzing)** — cerrada la 5.ª superficie proptest que
  faltaba: `prisma-cache` (round-trip, archivo corrupto, bytes arbitrarios,
  budget de eviction) — PR #60. Más el fuzz del PE loader (PR #65).
- **Reconciliación docs↔código** (PR #61): flags NZCV pillar ya existe,
  ADC/SBB/RCL/RCR ya landeados, 97 Op variants, 13 pases, sorry-budget 0,
  benchmarks es harness real.
- **Lane de memoria** — aplicado: DoS de deserialización de cache, cap de
  `size_of_image`, `destroy()` sin fugas, parsing/relocations acotados.

**Lección de proceso:** strict-mode + builds C++ ~10min serializan los merges; no
re-actualizar ramas en cascada (reinicia CI). Evitar PR stacks profundos; preferir
PRs independientes off-main en archivos distintos (mergean en paralelo).

---

## 1. Camino crítico hacia "correr una `.exe` de Windows"

La línea que de verdad importa. Cada nodo es un EPIC; las flechas son
dependencias duras.

```
  [C] Flags NZCV reales ──┐
                          ├──> [D] Threading (clone/futex) ──┐
  [A] Syscall layer ──────┘                                  │
                                                             ├──> [F] Wine bring-up ──> Notepad XP
  [B] Rust parity + cutover ─────────────────────────────────┘        (Fase 3)
                                                             │
  [E2.5] Pilares de investigación (NPU/TSO/cache P2P/AVF) ───┘ (en paralelo, no bloquean)
```

**Secuencia recomendada** (no estricta — son lanes paralelos):
`C` y `A` arrancan ya y en paralelo → habilitan `D` → `B` corre todo el tiempo en
su propio lane → cuando `A+C+D` están sólidos, `F` (Wine) es el hito de Fase 3.
Los pilares de Fase 2.5 (`E`) son investigación paralela que no está en el camino
crítico de "correr un .exe" pero **sí** es lo que hace a Prisma épico vs. "otro
Winlator".

---

## 2. Convenciones

- **ID:** `EWP-<epic><n>` (paquete de trabajo extremo). Cada uno cita los IDs
  canónicos de [BACKLOG.md](BACKLOG.md) que cierra.
- **Estado:** `[ ]` TODO · `[~|owner]` en progreso · `[x] (sha)` hecho ·
  `[!] motivo` bloqueado · `[?]` diferido.
- **Tamaño:** `XS` (<1h) · `S` (1-4h) · `M` (1-2d) · `L` (3-10d) · `XL` (semanas).
- **Lane:** quién dirige (split de territorio de CLAUDE.md): `claude` =
  emitter/passes/lowerer/cache/runtime/infra · `codex` = decoder/IR
  variants/dispatcher/backend. `*` = cualquiera.
- **Forma:** cómo se paraleliza — `solo`, `fan-out:N` (N agentes por frontera),
  `pipeline` (decoder→backend→tests sincronizados por IR), `audit` (N
  dimensiones + verificación adversarial + síntesis).
- **Aceptación:** condición verificable de "Done". **Todo EWP que asigna memoria
  ejecutable / mmap / fd / archivo de cache hereda la cláusula obligatoria de
  CLAUDE.md:** `Drop`/RAII determinista + test (o ASan/leak-check) de liberación.

---

## 3. EPIC C — Completar la migración al pilar de flags NZCV

> **Estado verificado:** el pilar de flags NZCV **ya existe** —
> `WriteFlags`/`ReadFlag`/`CondJumpFlags` (`ir.hpp:332-354`) bajan a ADDS/SUBS/
> ANDS + CSET/CSEL leyendo NZCV real (`lowering.cpp:1248-1280`). En paralelo
> sobrevive el legacy `Compare`→CSET-booleano (`lowering.cpp:624-639`). Además,
> **ADC/SBB/RCL/RCR con carry real ya aterrizaron en `main`** (PRs #54/#55,
> subsistema de CF persistente). Por eso EPIC C es **terminar la migración y
> retirar el camino booleano**, más síntesis PF/AF y verificación — NO construir
> de cero. Habilita TSO medible (EPIC D) y es la base de Pilar 2 (Lean).

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-C1 | RFC 0016 — política de migración de flags: cuándo el decoder emite `WriteFlags` vs `Compare`, retiro del path booleano, política PF/AF (ARM64 no las tiene nativas) | `docs/rfc/0016-*.md` | S | codex | — | solo | RFC mergeado, frontmatter válido; documenta el pilar existente y el plan de retiro de `Compare` legacy |
| EWP-C2 | Decoder emite `WriteFlags`/`CondJumpFlags` para compares/Jcc (en vez de `Compare`→bool); el lowering NZCV ya existe | `core/src/decoder`, `passes/` | M | codex | C1 | pipeline | e2e ARM64: CMP+Jcc por las 16 condiciones usa NZCV real; ningún `Compare`-booleano nuevo emitido en el corpus |
| EWP-C3 | Retirar `Compare`→CSET legacy una vez sin emisores; limpiar `lowering.cpp:624-639` | `core/src/backend/lowering.cpp`, `core/src/passes/flag_write_elim.cpp` | M | claude | C2 | solo | Suite verde sin el path booleano; `flag_write_elim` sigue borrando writes muertos |
| EWP-C4 | PF/AF síntesis (paridad por popcount-byte-bajo; aux-carry por nibble) cuando hay lector real; elidir vía `flag_write_elim` si no | `core/src/backend/lowering.cpp`, `core/src/passes/flag_write_elim.cpp` | M | claude | C3 | solo | Tests: LAHF/PUSHFQ leen PF/AF correctos; elididos cuando nadie lee |
| EWP-C5 | RCL/RCR **by CL** (variable) — el by-1 ya landó (#55, CF persistente); falta el conteo variable + verificación | `shell/prisma-{decoder,backend}`, `core/` | M | codex | — | pipeline | e2e ARM64: RCL/RCR by CL coincide con referencia; differential C++↔Rust verde |
| EWP-C6 | Lean: semántica NZCV de `WriteFlags`/`ReadFlag` (hoy constructor-mirror); el budget está en **0** (sorry-free), mantenerlo monótono | `ir-spec/PrismaIR/*.lean`, `ir-spec/.sorry-budget` | L | claude | C1 | solo | `lake build` verde; budget vuelve a 0 tras cada prueba; skill `prisma-sorry-budget` pasa |
| EWP-C7 | Audit adversarial del modelo de flags: OF en SUB, CF en shifts, PF/AF en 8/16/32/64, paridad ADC/SBB ya landeado | tests across | M | * | C2-C5 | audit:3 | 3 dimensiones (signed corner, shift CF, sub-word) + verificación adversarial; findings → tests |

---

## 4. EPIC A — Linux user-mode: syscall layer + coreutils *(cierra Fase 2 Track A)*

> Objetivo de Fase 2: pasar ≥85% de coreutils vía el translator user-mode en un
> ARM64 Linux de referencia. Hoy hay ~30 syscalls (STATUS §1: "syscalls Linux
> x86-64"). Lo que falta es threading, sockets, señales al guest, y la suite.

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-A1 | `futex()` completo (WAIT/WAKE/REQUEUE/PI subset) — crítico para pthread | `core/src/runtime/syscalls*`, `shell/prisma-runtime` | L | claude | — | solo | Cierra F2-SY-007; test: dos hilos guest sincronizan vía futex sin busy-spin; RAII de waiters |
| EWP-A2 | `clone()` + creación de hilo guest real (dispatcher multi-hilo, CpuStateFrame por hilo en TLS) | `core/src/runtime/dispatcher.cpp`, `jit_memory`, `runtime/thread*` | XL | claude | A1 | fan-out:2 | Cierra F2-SY-006; primer multithreading; TSan verde; cada hilo libera su frame/stack en join (cláusula RAII) |
| EWP-A3 | Señales al guest: `sigaction` bridge + `rt_sigprocmask`/`rt_sigsuspend` + entrega #PF/#UD/#DE al handler del guest | `core/src/runtime/signal*`, `runtime/dispatcher` | L | claude | — | pipeline | Cierra F2-SY-017/018 + F1-RT-011; test: SIGSEGV del guest enruta a su handler y reanuda |
| EWP-A4 | Sockets: socket/bind/listen/accept/connect + read/write families | `core/src/runtime/syscalls*` | M | * | — | solo | Cierra F2-SY-015/016; test loopback echo guest↔host |
| EWP-A5 | `execve` cross-ISA (re-entrada del translator con imagen nueva) | `core/src/runtime`, `translator` | L | claude | — | solo | Cierra F2-SY-011; test: guest hace execve de otro x86 ELF y sigue traduciendo |
| EWP-A6 | Syscall fuzz harness (AFL++ sobre nº + args) + strace ya existe (`PRISMA_STRACE`) | `fuzz/`, `core/src/runtime` | M | claude | — | solo | Cierra F2-SY-037; corre en CI nightly time-boxed; ningún crash en N horas |
| EWP-A7 | **Suite coreutils x86-64**: harness que corre N binarios reales bajo el translator y compara salida con nativo | `tools/diff-qemu/`, `tools/coreutils/` (nuevo) | L | * | A1-A4 | fan-out:N | Cierra F2 objetivo; reporta % passing; target ≥85%; tabla en docs |
| EWP-A8 | Audit de seguridad del syscall boundary (passthrough de structs, validación de punteros guest, TOCTOU) | runtime, audit | M | claude | A1-A5 | audit:3 | skill `prisma-security-review`; 3 dimensiones; findings → fixes + tests |

---

## 5. EPIC B — Paridad Rust + cutover incremental *(Track B, lane permanente)*

> El Rust ya ejecuta en ARM64 (`executor`, PR #43) y tiene el translator
> integrado. La meta es **paridad funcional con el C++** y luego sustituir tramos
> del pipeline C++ por Rust vía el C-ABI (RFC 0014) sin regresión. El gate real
> es `ffi-link` (ubuntu): clippy+test del workspace contra el DLL C++.

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-B1 | Decoder Rust: cerrar familias faltantes vs C++ (SSE/AVX/FMA/SHA/AES/x87) por differential | `shell/prisma-decoder` | XL | codex | — | pipeline | Cada familia: fixture differential C++↔Rust verde en `ffi-link`; superset/igualdad documentada |
| EWP-B2 | Backend Rust: completar lowering (vector ops, Rcl/Rcr tras C5, x87 bridge) | `shell/prisma-backend` | L | codex | EWP-C5 | pipeline | Lowerer cubre todo Op que el decoder emite; 0 `UnsupportedOp` en el corpus; fuzz robustez verde |
| EWP-B3 | Runtime Rust: dispatcher loop real con chaining + RAS (hoy es contrato) sobre el `executor` | `shell/prisma-runtime` | L | claude | — | solo | e2e ARM64 multi-bloque con chaining; W^X liberado en `Drop` del slab pool (cláusula RAII) |
| EWP-B4 | Syscall handler Rust real (hoy boundary tipado `Ok(0)`) — espejo del C++ EPIC A | `shell/prisma-runtime/syscall*` | L | claude | A1-A4 | solo | Paridad con el set C++; differential de efectos donde aplique |
| EWP-B5 | Cutover incremental: reemplazar el pase/etapa X del C++ por llamada Rust vía C-ABI, medir no-regresión | `core/`, `shell/`, RFC | L | claude | B1-B3 | solo | Un tramo (ej. passes) corre desde Rust en el pipeline C++; suite C++ sigue verde |
| EWP-B6 | Flags NZCV en Rust (espejo de EPIC C) | `shell/prisma-{ir,backend,passes}` | L | claude | C2 | pipeline | Differential de flags C++↔Rust verde |
| EWP-B7 | Lean ↔ Rust IR parity check automatizado (E1 del WORK_QUEUE, F25-RS-001) | `shell/prisma-ir`, `ir-spec/` | M | claude | — | solo | CI cross-check enum/layout Rust↔Lean↔C++; falla si divergen |

---

## 6. EPIC D — Threading, memoria débil y TSO *(habilita guest MT real)*

> Depende de A1/A2 (futex/clone). Es donde vive el Pilar 3 (TSO adaptativo) y el
> Pilar 2 (su prueba formal). Hoy todo emite variantes TSO conservadoras
> (`DMB ISH`); la oportunidad es bajar las que sean probadamente seguras.

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-D1 | CAS atómico real (cierra la nota de WORK_QUEUE 14j: "LoadMemTSO/StoreMemTSO split no es un CAS real bajo guest MT") | `core/src/backend/lowering.cpp`, runtime | M | codex | A2 | pipeline | CMPXCHG baja a LDAXR/STLXR o CASAL real; test MT no pierde actualizaciones |
| EWP-D2 | TSO experiment: flag switch conservative/relaxed en LOCK/XCHG/CMPXCHG + instrumentación para medir % de barreras elidibles | runtime, passes | M | claude | D1 | solo | Cierra F25-TS-002/007; reporta % ops downgradeables en workloads de referencia |
| EWP-D3 | Clasificador TSO de 5 categorías (ST/lock-free/shared-mutable/IO/unknown), análisis estático + hint en IR | `core/src/passes/tso_classify.cpp` (nuevo) | L | claude | D2 | fan-out:2 | Cierra F25-TS-001/004/005; rewrite TSO→plain donde seguro; fallback conservador en unknown |
| EWP-D4 | Suite de 30 programas multi-hilo, cero regresiones con TSO relajado | `tools/benchmarks/mt/` | L | * | D3 | fan-out:N | Cierra F25-TS-008 (target 15-20% speedup ST); TSan verde en los 30 |
| EWP-D5 | Lean: weak memory model skeleton + axiomas TSO como lemmas (Pilar 2) | `ir-spec/PrismaIR/Memory.lean` (nuevo) | XL | claude | C6 | solo | Cierra F1-LN-014/015/016 + F25-LN-001/002; prueba que el rewrite preserva semántica bajo invariantes |
| EWP-D6 | Aserción runtime (debug) que la clasificación TSO se sostiene + conexión a los invariantes Lean | runtime, ir-spec | M | claude | D3,D5 | solo | Cierra F25-TS-006/F25-LN-003; aserción dispara en violación bajo stress |

---

## 7. EPIC E — Huecos de ISA restantes *(incremental, fan-out por familia)*

> Lane de relleno siempre disponible. Forma canónica: **pipeline decoder →
> backend/lowering → tests** sincronizado por el IR (CLAUDE.md §Trabajo con
> agentes). STATUS §2 lista lo ausente deliberadamente.

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-E1 | SSE4.2 string: PCMPESTRI/PCMPISTRI/PCMPESTRM/PCMPISTRM (re-anuncia SSE4.2 en CPUID) | decoder, backend, `translator.cpp:506` | L | codex | — | pipeline | Cierra el hueco STATUS §2; e2e vs SDM; bit SSE4.2 honesto en CPUID |
| EWP-E2 | BMI1: ANDN/BEXTR/BLSI/BLSMSK/BLSR (`translator.cpp:520`) | decoder, backend | M | codex | — | pipeline | Cierra F2 BMI1; e2e ARM64; bit BMI1 en leaf 7 |
| EWP-E3 | F16C (VCVTPH2PS/VCVTPS2PH) | decoder, backend | M | codex | — | pipeline | Conversión half↔single vía FCVT; e2e |
| EWP-E4 | PCLMULQDQ (carry-less multiply) | decoder, backend | M | codex | — | pipeline | ARM64 PMULL; e2e vs vectores conocidos |
| EWP-E5 | AVX2 enteros faltantes: VPBROADCAST*, VINSERTI128/VEXTRACTI128, shifts variables (VPSLLVD/Q, VPSRLVD/Q, VPSRAVD) | decoder, backend | L | codex | — | pipeline | Cierra el batch AVX2; lane-crossing reutiliza VecTbl2 |
| EWP-E6 | Thin spots SSSE3/SSE4.1 (auditoría dirigida de lo que falta) | decoder, audit | S | codex | — | audit:1 | Lista cerrada + cierre; CPUID refleja la capacidad real |
| EWP-E7 | x87 80-bit helper path (precisión completa; hoy reduced-F64, RFC 0013) | backend, runtime | L | codex | — | solo | Path opt-in de 80-bit para casos que lo requieran; tests de precisión |
| EWP-E8 | Decoder fuzz continuo (AFL++ ya existe) — acumular compute, triage de hallazgos | `fuzz/`, CI nightly | S | claude | — | solo | Nightly time-boxed; crashes → tests de regresión |

---

## 8. EPIC F — Windows / Wine bring-up *(Fase 3 — el hito que importa)*

> "Correr Notepad XP en Android." No empezado. Depende de A+C+D sólidos. Hoy
> `shell/orchestrator/pe_loader.rs` solo mapea memoria; `container.rs::start()`
> devuelve `NotImplemented`. **12 semanas el bridge de Wine.**

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-F1 | PE loader real: parse de imports, resolución de DLLs, relocations, TLS callbacks | `shell/orchestrator/pe_loader.rs` | XL | * | — | fan-out:2 | Cierra F3-WN-009..012; carga un .exe XP simple con su import table resuelta |
| EWP-F2 | Win32/NT syscall surface que Wine necesita (hoy solo Linux/POSIX) | `core/src/runtime`, `shell` | XL | claude | A-EPIC | fan-out:N | Subset NT suficiente para Notepad; documentado |
| EWP-F3 | Wine bridge: submódulo Wine ARM64 + `wow64cpu.dll` stub (BTCpu interface) | `third_party/wine`, `shell` | XL | * | F1,F2 | pipeline | Cierra F3-WN-001..008; `pBTCpuSimulate` llama a Prisma en código x86 |
| EWP-F4 | Container system tipo Winlator: prefix Wine + config TOML por contenedor | `shell/orchestrator/container.rs` | L | claude | F3 | solo | `start()` deja de ser `NotImplemented`; crea/lista/borra contenedores; recursos liberados en teardown (cláusula RAII) |
| EWP-F5 | Filesystem overlay (base RO + overlay RW) | `shell/orchestrator/fs*` | L | claude | F4 | solo | Cierra F3-SH-005; escrituras del guest van al overlay |
| EWP-F6 | App Kotlin/Compose mínima: importar .exe, ejecutar, ver logs (JNI→Rust) | `android/` | XL | * | F4 | fan-out:2 | Cierra F3-ND-001..008; importa y lanza un .exe en device real |
| EWP-F7 | Servidor X11 embebido + Vulkan surface (ANativeWindow) | `shell`, `android` | XL | * | F6 | fan-out:2 | Cierra F3-XS-*; Notepad XP renderiza en pantalla Android |
| EWP-F8 | **Hito: Notepad XP estable end-to-end en hardware Android** + video demo | integración | XL | * | F1-F7 | solo | El entregable de Fase 3; captura de video; Discord 500-1000 |

---

## 9. EPIC G — Pilares de investigación Fase 2.5 *(lo que hace a Prisma épico)*

> Prototipos funcionales, no producto. Investigación paralela al camino crítico.
> Cada pilar tiene paper asociado (principio 2 de CLAUDE.md). Llena `server/`,
> `tools/`, modelos ML.

| ID | Título | Archivos | Tam | Lane | Deps | Forma | Aceptación |
|----|--------|----------|-----|------|------|-------|------------|
| EWP-G1 | Pilar 1 — pipeline de datos NPU: captura (bytecode, traza) + features (opcode hist, branch density, footprint) | `tools/npu/`, Python | XL | * | — | fan-out:2 | Cierra F25-NP-005/006; dataset reproducible |
| EWP-G2 | Pilar 1 — clasificador hot-path (PyTorch ~10MB) + ONNX Runtime en C++ con fallback CPU | `core/`, `tools/npu/` | XL | * | G1 | fan-out:2 | Cierra F25-NP-001/007/012; hint pre-traducción medible; latencia NPU benchmarkeada |
| EWP-G3 | Pilar 4 — servidor Rust/Axum de cache distribuida (POST/GET por hash, firma Ed25519, R2) | `server/` (vacío hoy) | XL | claude | — | fan-out:2 | Cierra F25-CA-001..005; cliente verifica firma antes de mmap+exec (cláusula RAII + seguridad) |
| EWP-G4 | Pilar 4 — P2P libp2p (DHT, peer discovery, trust anchors, elegibilidad por SoC) | `server/`, `shell` | XL | claude | G3 | fan-out:2 | Cierra F25-CA-007..010; intercambio P2P entre dos peers del mismo SoC |
| EWP-G5 | Pilar 5 — AVF hybrid: detección AVF (Pixel 7a+), Windows-on-ARM guest en crosvm, bridge native↔DBT | `shell`, `android` | XL | * | — | fan-out:2 | Cierra F25-AV-*; regiones ARM64 nativas corren en guest, x86 en DBT |
| EWP-G6 | Pilar 6 — shader graph analyser (Python offline) + 20 patrones hot-loop de Portal/HL2 | `tools/graphics/` | L | * | — | solo | Cierra F25-GX-001/002; 20 patrones identificados con snippets Vulkan |
| EWP-G7 | Benchmark académico: Dhrystone/CoreMark/nbench/SPEC subset vs QEMU/Box64/FEX/nativo | `tools/benchmarks/` | L | * | A7 | fan-out:N | Cierra F2-BM-*; target 30-45% nativo; tabla pública; **decision point honesto: <25% = rediseño** |
| EWP-G8 | Papers 1-3 (drafts LaTeX en `papers/`) conforme aterrizan resultados | `papers/` | XL | * | G2,D5,G7 | solo | Cierra F2-AC/F25-NP-017/F25-LN-004; submissions a LCTES/MICRO/POPL |

---

## 10. EPIC H — Lanes continuos *(siempre disponibles, cualquier agente)*

| ID | Título | Tam | Aceptación |
|----|--------|-----|------------|
| EWP-H1 | **Infra/CI**: mantener verde los 12 contexts requeridos; nuevos runners (Orange Pi 5B self-hosted ARM64, F0-DX-012..014) | S-M | CI verde; runner registrado |
| EWP-H2 | **Hardening**: auditoría periódica del boundary host↔guest (JIT W^X, signal handlers, deserialización de cache, claves P2P) — skill `prisma-security-review` | M | Audit doc en `docs/REVIEWS/`; findings → fixes |
| EWP-H3 | **Deuda de passes**: revisión trimestral del pipeline (skill `prisma-pass-debt`) — duplicación, reuse perdido, abstracciones (in)justificadas | S | Reporte + limpieza si aplica |
| EWP-H4 | **Cláusula RAII sweep**: auditar todo allocador de mem ejecutable/mmap/fd/archivo de cache tiene `Drop`/destructor + test de liberación ([memoria CLAUDE.md](../CLAUDE.md#cláusula-obligatoria-disciplina-de-memoria-y-recursos)) | M | Cada recurso sensible a reinicio libera determinísticamente; sin `mem::forget` sin RFC; ASan/leak verde |
| EWP-H5 | **Lean sorry budget**: bajar el budget conforme aterrizan pruebas; specs de cada IR op nuevo (skill `prisma-sorry-budget`) | S-L | `lake build` verde; budget monótonamente decreciente |
| EWP-H6 | **Differential C++↔Rust**: ampliar el corpus del gate `ffi-link` con cada familia nueva | S-M | Fixtures nuevas; gate verde |
| EWP-H7 | **Docs/research**: blog post cada 2-3 meses; RFC por cada decisión/dependencia (skill `prisma-dependency-audit`); `docs/GLOSSARY.md` | S | Publicado; RFC frontmatter válido |
| EWP-H8 | **Dependency hygiene**: cada entrada nueva en CMakeLists/Cargo.toml/build.gradle pasa el audit de licencias (MIT core + app comercial, no-copia-de-FEX/Box64/QEMU) | XS-S | RFC justificando; licencia compatible |

---

## 11. Definition of Done global *(gates que todo EWP debe cruzar antes de `main`)*

1. **CI verde** en los 12 contexts requeridos (CLAUDE.md): `core-build`,
   `core-build-arm64`, `asan-ubsan`, `tsan`, `ir-spec-build`, `ffi-link`,
   `ffi-link-arm64`, `ffi-link-windows`, `markdownlint`,
   `check-rfc-frontmatter`, `shell-check`, `benchmarks-smoke`.
2. **No code sin tests** (CLAUDE.md commit discipline). Cada instrucción/op tiene
   test bytes→IR→ejecución.
3. **Cláusula RAII** (obligatoria): todo recurso del SO (mem ejecutable JIT,
   mmaps guest, fds, archivos de cache) tiene dueño con `Drop`/destructor +
   test de liberación. Rutas de shutdown liberan explícitamente (flush→close→
   unmap). Nada de `mem::forget`/ownership-leak sin RFC.
4. **Two-eyes** (CONTRIBUTING.md): commits solo en territorio IR/decoder/
   lowering/emitter necesitan co-sign (tally en `docs/REVIEW_F2_SESSION.md`).
   En la práctica: review por codex + gemini en cada diff sustantivo.
5. **Sorry budget** no sube salvo corner-case firmado; CI falla si excede.
6. **RFC** para cada decisión de diseño o dependencia nueva.
7. **Claim protocol** (COORDINATION.md): `[~|owner]` + commit antes de tocar;
   `[x] (sha)` al cerrar.

---

## 12. Decision points honestos *(principio 4 de CLAUDE.md — no glosar)*

- **Fin Fase 2 (EWP-G7):** si el benchmark < 25% del nativo → algo estructural en
  el IR. **Parar y rediseñar**, publicar el resultado negativo.
- **Cada pilar de Fase 2.5:** si no funciona, paper negativo / blog post honesto.
  No esconder.
- **NPU (Pilar 1):** si el clasificador no supera baselines heurísticos, decir
  que la hipótesis ML no se sostuvo.
- **TSO adaptativo (Pilar 3):** si el % de barreras elidibles con seguridad es
  marginal, reportarlo como límite, no inflar.

---

## 13. Parking lot / stretch *(no priorizado, no se pierde)*

- MMX (F2-IR-009 — raro en binarios modernos, stub o skip).
- AVX-512 / EVEX (fuera de scope declarado).
- RDRAND / XSAVE completo / PCONFIG.
- Multi-hop direct-thread chaining + CallRel auto-patching (cola de WORK_QUEUE 15,
  detrás de SmcGuard/page invalidation).
- Pair-allocator NEON + spill (F2-BK-006, diferido hasta demanda medida).
- Juegos AAA (Skyrim SE / Fallout 3 — stretch de Fase 4).

---

> **El mapa es [BACKLOG.md](BACKLOG.md). El estado es [STATUS.md](STATUS.md). El
> ritmo es [WORK_QUEUE.md](WORK_QUEUE.md). Este documento es la *guía de
> ataque*.** La unidad de progreso sigue siendo un commit que cierra una línea.
> Vamos.
