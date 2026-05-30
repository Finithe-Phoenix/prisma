# CLAUDE.md

Guia operativa persistente para futuras sesiones de AI trabajando en este
repositorio. Debe permitir que una sesion nueva continue sin depender de memoria
conversacional.

Ultima actualizacion: 2026-05-30 America/Mexico_City.

## Snapshot actual

- Proyecto: Prisma, Dynamic Binary Translator x86/x64 a ARM64 para emulacion
  Windows en Android.
- Estrategia: DBT propio, formalmente guiado. Danny rechazo conscientemente
  forkear Winlator el 2026-04-19; no reabrir esa decision sin evidencia nueva.
- Repo remoto activo: `https://github.com/Finithe-Phoenix/prisma.git`.
- Rama principal: `main`.
- Estado Git observado el 2026-05-30: `main...origin/main`, arbol limpio.
- HEAD local/remoto observado: `10c5cf2 chore(backlog): codex claims F1-IR-004 F1-IR-005`.
- Commit previo relevante: `85bc189 chore(backlog): codex claims F1-IR-003`.
- Ultimo commit de codigo core con CI verde observado: `dd1c6f0 test(ir): fix clang u16 read warning`.
- GitHub Actions activos: `core sanitizers`, `core (C++20) - stub`,
  `ir-spec (Lean 4)`, `lint-docs`, `shell (Rust) - stub`.
- Ultimas ejecuciones remotas observadas con `gh run list`:
  - `lint-docs` verde en `10c5cf2` y `85bc189` el 2026-05-18.
  - `core (C++20)` y `core sanitizers` verdes en `dd1c6f0` el 2026-05-18.
- Branch protection de `main`: no activa al 2026-05-30
  (`gh api .../branches/main/protection` devuelve 404).
- Webhooks: ninguno visible con el token actual al 2026-05-30.

## Principios no negociables

1. Correctness primero. No sacrificar pruebas, validadores, sanitizers ni la
   especificacion Lean para avanzar rapido.
2. `docs/BACKLOG.md` es la fuente de verdad de trabajo.
3. Commits pequenos, en ingles, con forma `<scope>: <what>`.
4. Cada cambio de codigo debe tener tests proporcionales al riesgo.
5. No copiar codigo de FEX, Box64, QEMU, Wine, Winlator ni proyectos similares.
   Se permite estudiar arquitectura, no transportar implementacion.
6. No agregar dependencias nuevas sin RFC o justificacion documentada.
7. No hacer cambios destructivos de Git. Nunca usar `git reset --hard` ni
   revertir trabajo ajeno sin instruccion explicita.
8. Si una guia vieja contradice el codigo o el backlog, creer primero al codigo
   y al backlog; luego actualizar la guia.

## Mapa de documentos

- `README.md`: elevator pitch del proyecto.
- `PROYECTO_PLAN_EJECUCION.md`: manifiesto tecnico y plan 48-54 meses.
- `docs/BACKLOG.md`: cola de trabajo multi-fase y estado de cada item.
- `docs/COORDINATION.md`: protocolo de claims multi-agente.
- `docs/ARCHITECTURE.md`: tour del monorepo.
- `docs/CONTRIBUTING.md`: guia futura de contribucion OSS.
- `docs/rfc/`: decisiones arquitectonicas aceptadas o propuestas.
- `docs/research_notes.md`: notas de investigacion persistentes.
- `core/README.md`: orientacion del core, pero tiene texto historico
  desactualizado que dice que el core aun no existe.
- `ir-spec/README.md`: proposito de la especificacion Lean, tambien conserva
  texto historico de Fase 0.

## Estado de claims

Claims activos observados en `docs/BACKLOG.md`:

```text
[~|codex] F1-IR-003: Add Flags SSA value type
[~|codex] F1-IR-004: Add WriteFlags{op, lhs, rhs, size}
[~|codex] F1-IR-005: Add ReadFlag{flags, which}
```

No reclamar ni editar estos items como `claude` salvo que Danny lo pida o que
Codex los abandone formalmente en el backlog.

Tambien hay varios items antiguos marcados como `[x] (pending commit)`. No
asumir que estan mal implementados; significa que el backlog todavia necesita
auditoria historica para reemplazar `pending commit` por el SHA correcto.

## Estado tecnico del core

El core C++ ya no es scaffolding: compila, ejecuta tests y contiene decoder,
IR, passes, backend ARM64, runtime, translator facade y cache.

Directorios principales:

```text
core/include/prisma/      headers publicos
core/src/ir/              IR, CFG, analisis, serializacion, validacion
core/src/decoder/         decoder x86_64 a Prisma IR
core/src/passes/          optimizaciones IR
core/src/backend/         emitter vixl + lowering IR a ARM64
core/src/cache/           translation cache, SHA-256, zstd
core/src/runtime/         JIT memory, signals, dispatcher, host features
core/src/translator/      facade de alto nivel
core/tests/               Catch2 tests
fuzz/                     harness AFL++
```

Hay 23 archivos de test C++ bajo `core/tests/` y 73 archivos bajo
`core/include`, `core/src` y `core/tests` al 2026-05-30.

### IR

`prisma_ir` contiene:

- `Ref` SSA comprimido.
- Operaciones base: constantes, load/store de registros, ALU, compare,
  memoria, jumps, calls, returns, select, extend/truncate, fences y markers.
- `BasicBlock` y `Function`.
- `build_cfg(std::span<const Stmt>)`.
- `flatten(Function)`.
- Analisis CFG: `build_cfg_graph`, `postorder`, `reverse_postorder`,
  `compute_dominators`.
- Deteccion de loops naturales por back-edge: `detect_natural_loops`.
- Serializacion/deserializacion binaria.
- Pretty-print cache.
- Validador de refs, tamanos y operandos.

Estado especial de flags:

- Existe `CmpFlags` como operacion side-effecting que setea flags implicitos.
- Existe `CondJumpFlags{cc, if_true, if_false}` como terminador sobre flags
  implicitos.
- No existe todavia el modelo SSA explicito de flags. Esta reclamado por Codex
  en `F1-IR-003/004/005`.

### Decoder

El decoder x86_64 cubre gran parte de Fase 1:

- MOV, ALU, shifts, rotates, bit ops.
- CALL/JMP/Jcc, CMOVcc, SETcc.
- Atomics base y LOCK/HLE.
- String ops base (`STOS`, `MOVS`, `CMPS`, `SCAS`) sin REP completo.
- FS/GS, RIP-relative, SIB, REX, operand/address overrides.
- CPUID, RDTSC/RDTSCP, SYSCALL, INT3, HLT reject.
- Table-driven dispatch e instruction-length validator.

Deuda de decoder de alto valor:

- `F1-DC-066`: REP / REPE / REPNE prefixes.
- `F1-DC-081`: differential test contra Zydis.
- `F1-DC-087`: migracion Zydis-free como fuente canonica.

### Backend y lowering

`prisma_emitter` usa vixl y soporta emision ARM64 para:

- ALU, memoria, branches, labels.
- Atomics, rotates, bit ops, fences.
- Literal pools.
- Spills y reloads.
- Integracion de I-cache.

`Lowerer` soporta lowering de listas planas y `Function` con bloques. Para
control flow con labels, preferir lowering de `Function`. `CondJumpFlags` baja a
branch condicional ARM64 con fallback. `CondJump` por valor SSA baja en contexto
de `Function`; en lista plana sigue rechazado cuando no hay contexto de labels.

Deuda backend de alto valor:

- `F1-BK-012`: NEON SIMD 128-bit.
- `F1-BK-013`: floating point.
- Revisar register allocator cuando entren valores SIMD/FP.

### Passes

El pipeline por defecto incluye:

```text
constant_propagate -> algebraic_simplify -> strength_reduce ->
constant_propagate_2 -> redundant_load_eliminate ->
common_subexpression_eliminate -> copy_propagate ->
dead_store_eliminate -> branch_fold -> flag_write_eliminate ->
dead_code_eliminate
```

Punto de atencion: con flags implicitos, DCE y flag-write elimination deben
conservar `CmpFlags` que alimenten branches o reads de flags. Cuando entren
`Flags`, `WriteFlags` y `ReadFlag`, actualizar:

- operand collection;
- liveness;
- serialization/deserialization;
- pretty printer;
- validator;
- CSE/DCE/branch folding;
- decoder lowering de ALU/CMP/TEST/bit ops;
- backend lowering;
- Lean spec.

### Runtime y cache

`prisma_runtime` incluye:

- `JitBuffer` y pool thread-safe.
- signal handlers para SIGSEGV/SIGILL/SIGBUS.
- dispatcher/trampoline.
- `CpuStateFrame`.
- host feature detection (`FEAT_LSE2`, `LRCPC2`, `FlagM`, etc.).

`prisma_cache` incluye:

- cache en memoria;
- invalidacion;
- formato persistente;
- LRU y byte budget;
- writer thread;
- SHA-256;
- zstd compression.

Deuda runtime/cache de alto valor:

- `F1-RT-010`: page-protection based SMC detection.
- `F1-RT-011`: guest signal delivery.
- `F1-RT-012`: FPU state save/restore.
- `F1-CA-008`: cache compaction pass.

### Lean

`ir-spec/` compila con Lake/Lean 4 y CI controla el budget de `sorry`.
La implementacion C++ va por delante de la spec en CFG y flags.

Deuda Lean prioritaria:

- `F1-LN-002`: Flags type en `Syntax.lean`.
- `F1-LN-003`: Block / Function en Syntax.
- `F1-LN-007`: step relation para `CondJumpFlags`.

## CI y validacion remota

Workflows activos:

- `.github/workflows/core-stub.yml`
  - Trigger: `core/**` o el workflow.
  - Si existe `core/CMakeLists.txt`, instala CMake/Ninja/clang-17, build real
    y `ctest`.
- `.github/workflows/core-sanitizers.yml`
  - Trigger: push a `main` y PRs que toquen `core/**`.
  - Jobs: ASan+UBSan y TSan.
- `.github/workflows/ir-spec.yml`
  - Trigger: `ir-spec/**`.
  - Ejecuta `lake build` y chequeo de `.sorry-budget`.
- `.github/workflows/lint-docs.yml`
  - Trigger: Markdown.
  - Ejecuta `markdownlint-cli2` y valida frontmatter RFC.
- `.github/workflows/shell-stub.yml`
  - Trigger: `shell/**`.
  - Si aparece `shell/Cargo.toml`, corre fmt, clippy y tests.

Comandos utiles de GitHub:

```bash
gh workflow list --repo Finithe-Phoenix/prisma
gh run list --repo Finithe-Phoenix/prisma --limit 20
gh run view <run-id> --repo Finithe-Phoenix/prisma --log-failed
gh run watch <run-id> --repo Finithe-Phoenix/prisma --exit-status
gh api repos/Finithe-Phoenix/prisma/branches/main/protection
gh api repos/Finithe-Phoenix/prisma/hooks --jq '.[] | {id, name, active, events}'
```

Si `branches/main/protection` devuelve 404, `main` sigue sin proteccion.

## Comandos locales obligatorios

Antes de tocar codigo C++:

```bash
git pull --ff-only origin main
git status -sb
rg -n "\[~\|" docs/BACKLOG.md
cmake --build core/build --target prisma_core_tests -j2
core/build/prisma_core_tests --reporter compact
```

Build limpio equivalente al workflow core:

```bash
cmake -S core -B /tmp/prisma-core-clang -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug \
  -DCMAKE_CXX_COMPILER=clang++-17
cmake --build /tmp/prisma-core-clang --target prisma_core_tests -j2
ctest --test-dir /tmp/prisma-core-clang --output-on-failure
```

Sanitizers locales:

```bash
cmake -S core -B /tmp/prisma-asan -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug \
  -DCMAKE_CXX_COMPILER=clang++-17 \
  -DPRISMA_ENABLE_ASAN=ON \
  -DPRISMA_ENABLE_UBSAN=ON
cmake --build /tmp/prisma-asan --target prisma_core_tests -j2
ASAN_OPTIONS=detect_leaks=1:halt_on_error=1:abort_on_error=1 \
UBSAN_OPTIONS=print_stacktrace=1:halt_on_error=1 \
  /tmp/prisma-asan/prisma_core_tests --reporter compact

cmake -S core -B /tmp/prisma-tsan -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug \
  -DCMAKE_CXX_COMPILER=clang++-17 \
  -DPRISMA_ENABLE_TSAN=ON
cmake --build /tmp/prisma-tsan --target prisma_core_tests -j2
TSAN_OPTIONS=halt_on_error=1:second_deadlock_stack=1 \
  /tmp/prisma-tsan/prisma_core_tests --reporter compact
```

Lean:

```bash
cd ir-spec
lake build
```

Markdown:

```bash
npx --yes markdownlint-cli2 "**/*.md" "!third_party/**" "!**/node_modules/**"
```

## Protocolo autonomo de trabajo

Usar este ciclo para continuar sin pedir instrucciones salvo bloqueo externo:

1. Sincronizar:

   ```bash
   git pull --ff-only origin main
   git status -sb
   gh run list --repo Finithe-Phoenix/prisma --limit 10
   ```

2. Revisar claims:

   ```bash
   rg -n "\[~\|" docs/BACKLOG.md
   ```

3. Si hay CI rojo en el ultimo commit propio, arreglar CI primero.
4. Si hay un claim activo propio, terminarlo o abandonarlo formalmente.
5. Elegir un item `[ ]` de alto valor que no choque con claims activos.
6. Reclamar en `docs/BACKLOG.md` cambiando `[ ]` por `[~|claude]` o
   `[~|codex]`.
7. Commit solo del claim:

   ```bash
   git add docs/BACKLOG.md
   git commit -m "chore(backlog): <agent> claims F1-XX-NNN"
   git push origin main
   ```

8. Implementar con cambios minimos y pruebas enfocadas.
9. Ejecutar pruebas locales relevantes y luego la suite completa si se toco
   `core/**`.
10. Commit del codigo:

    ```bash
    git add <files>
    git commit -m "<scope>: <what>"
    ```

11. Marcar el backlog completado con el short SHA del commit de codigo:

    ```text
    [x] (<sha>) F1-XX-NNN
    ```

12. Commit del cierre:

    ```bash
    git add docs/BACKLOG.md
    git commit -m "chore(backlog): mark F1-XX-NNN complete"
    ```

13. Push y vigilancia remota:

    ```bash
    git push origin main
    gh run list --repo Finithe-Phoenix/prisma --limit 10
    gh run watch <run-id> --repo Finithe-Phoenix/prisma --exit-status
    ```

14. Si GitHub Actions falla, leer log, corregir, testear local, commitear y
    empujar hasta dejar checks relevantes verdes.
15. Si el trabajo cambia estado estructural del proyecto, actualizar este
    archivo en el mismo ciclo o en un commit de documentacion separado.

## Protocolo para elegir siguiente tarea

Prioridad recomendada al 2026-05-30:

1. Respetar los claims activos de Codex en `F1-IR-003/004/005`.
2. Si esos claims terminan, revisar fallout de flags SSA en passes, decoder,
   backend, serializer, validator y Lean.
3. Si se trabaja como Claude mientras Codex tiene flags, preferir areas que no
   choquen:
   - `F1-DC-066` REP prefixes si se acepta tocar decoder.
   - `F1-BK-012/013` SIMD/FP si no depende del nuevo modelo de flags.
   - `F1-RT-010` SMC page protection.
   - `F1-LN-002/003/007` solo si se coordina con el modelo exacto de flags.
4. Si se toca backend, agregar tests en `core/tests/test_lowering.cpp` o
   `core/tests/test_emitter.cpp`.
5. Si se toca decoder, agregar golden tests en `core/tests/test_decoder.cpp` y,
   cuando aplique, tests e2e.
6. Si se toca runtime/cache, correr tests especificos y sanitizers.
7. Si se toca IR, actualizar serialization, pretty-print, validator y tests de
   round-trip en el mismo cambio.

## Riesgos y pendientes de infraestructura

- Activar branch protection para `main`.
- Definir required status checks minimos:
  `core (C++20) - stub`, `core sanitizers`, `ir-spec (Lean 4)` cuando aplique,
  y `lint-docs` para cambios Markdown.
- Auditar los `[x] (pending commit)` en `docs/BACKLOG.md`.
- Actualizar documentos historicos que aun dicen que el core no existe.
- Agregar runner ARM64 self-hosted cuando Danny tenga hardware dedicado.
- Decidir si se necesitan webhooks externos. Hoy no hacen falta para CI.
- Evaluar actualizaciones de Actions antes de migraciones forzadas de runtime
  en GitHub Actions.

## Cuando pedir ayuda a Danny

No detenerse para decisiones tecnicas normales. Solo pedir intervencion cuando:

- Se necesiten compras, cuentas, llaves, contratos o permisos externos.
- GitHub requiera una accion UI que el token actual no permita.
- Haya que tomar una decision estrategica que contradiga el manifiesto.
- Dos rutas tecnicas tengan tradeoffs fuertes y el backlog/RFCs no indiquen
  preferencia.
- Un claim activo de otro agente bloquee el trabajo y no haya alternativa segura.

## Que no volver a preguntar

- No preguntar si se puede correr la suite: correrla.
- No preguntar si se debe vigilar GitHub Actions: vigilarlo despues de cada push
  que dispare workflows.
- No preguntar si se puede instalar tooling local razonable cuando sea necesario
  para pruebas; instalarlo y documentar el cambio.
- No preguntar si se debe empujar al remoto cuando el trabajo esta committeado,
  probado y sincronizado con `origin/main`.

## Definicion de listo

Una tarea esta lista cuando:

- El item esta reclamado y luego cerrado en `docs/BACKLOG.md`.
- El codigo esta en commits atomicos.
- Tests locales relevantes pasan.
- Si toca `core/**`, `core (C++20)` y `core sanitizers` pasan en GitHub.
- Si toca Markdown, `lint-docs` pasa en GitHub.
- Si toca `ir-spec/**`, `ir-spec (Lean 4)` pasa en GitHub.
- El arbol local queda limpio y `main...origin/main` queda sincronizado.
