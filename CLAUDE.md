# CLAUDE.md

Guía operativa persistente para futuras sesiones de AI trabajando en este
repositorio. Este archivo debe permitir que una sesión nueva continúe sin
depender de memoria conversacional.

Última actualización: 2026-05-18 UTC.

## Snapshot actual

- Proyecto: Prisma, un Dynamic Binary Translator x86/x64 a ARM64 para
  emulación Windows en Android.
- Estrategia: DBT propio y formalmente guiado. Danny rechazó conscientemente
  forkear Winlator el 2026-04-19; no reabrir esa decisión sin evidencia nueva.
- Repo remoto activo: `https://github.com/Finithe-Phoenix/prisma.git`.
- Rama principal: `main`.
- Último commit remoto verde para core: `dd1c6f0 test(ir): fix clang u16 read warning`.
- Estado Git esperado al iniciar: `main...origin/main` limpio.
- Cuenta GitHub CLI autenticada en este entorno: `Finithe-Phoenix`.
- Email de Git configurado localmente para este repo: `daedgora@hotmail.com`.
- GitHub Actions verdes en `dd1c6f0`: `core (C++20) - stub` y `core sanitizers`.
- No hay branch protection activa en `main` al 2026-05-18. Los checks existen,
  pero GitHub no obliga a que pasen antes de integrar.
- No hay webhooks visibles/configurados con el token actual. Para CI no son
  necesarios porque GitHub Actions ya valida pushes y PRs.

## Principios no negociables

1. Correctness primero. No sacrificar pruebas, validadores, sanitizers o
   especificación Lean para avanzar rápido.
2. El backlog en `docs/BACKLOG.md` es la fuente de verdad de trabajo.
3. Commits pequeños, en inglés, con forma `<scope>: <what>`.
4. Cada cambio de código debe tener tests proporcionales al riesgo.
5. No copiar código de FEX, Box64, QEMU, Wine, Winlator ni proyectos similares.
   Se permite estudiar arquitectura, no transportar implementación.
6. No añadir dependencias nuevas sin RFC o justificación documentada.
7. Sin cambios destructivos de Git. Nunca usar `git reset --hard` ni revertir
   trabajo ajeno sin una instrucción explícita.

## Mapa de documentos

- `PROYECTO_PLAN_EJECUCION.md`: manifiesto técnico y plan 48-54 meses.
- `docs/BACKLOG.md`: cola de trabajo multi-fase y estados de cada item.
- `docs/COORDINATION.md`: protocolo de claims multi-agente.
- `docs/ARCHITECTURE.md`: tour de la arquitectura del monorepo.
- `docs/rfc/`: decisiones arquitectónicas aceptadas o propuestas.
- `docs/research_notes.md`: notas de investigación persistentes.
- `core/README.md`: estado del core C++.
- `ir-spec/README.md`: build y propósito de la especificación Lean.

## Estado técnico del core

El core C++ ya no es scaffolding: compila, ejecuta tests y tiene una base amplia
de decoder, IR, passes, backend ARM64, runtime y cache.

### IR

- `prisma_ir` contiene operaciones, pretty-printer, serialización,
  deserialización y validador.
- Existen `BasicBlock` y `Function`.
- Existe builder CFG desde lista plana de `Stmt`:
  `build_cfg(std::span<const Stmt>)`.
- Existe `flatten(Function)`.
- Existe análisis CFG:
  `build_cfg_graph`, `postorder`, `reverse_postorder`,
  `compute_dominators`.
- Existe detección de loops naturales por back-edge:
  `detect_natural_loops`.
- `CondJumpFlags{cc, if_true, if_false}` existe como terminador sobre flags
  implícitos. Todavía no existe SSA explícito de flags.

Items IR completados en la última ola:

- `F1-IR-023` en `10d6ac3`: CFG builder.
- `F1-IR-024` en `4a0e946`: dominator tree y traversals.
- `F1-IR-025` en `8608326`: natural loops.
- `F1-IR-007` en `071bae3`: `CondJumpFlags`.

Siguiente deuda IR importante:

- `F1-IR-003`: `Flags` SSA value type.
- `F1-IR-004`: `WriteFlags{op, lhs, rhs, size}`.
- `F1-IR-005`: `ReadFlag{flags, which}`.

### Decoder

El decoder x86_64 cubre una parte grande de Fase 1: MOV, ALU, shifts, rotates,
bit ops, CALL/JMP/Jcc, CMOVcc, SETcc, atomics base, string ops base, FS/GS,
RIP-relative, SIB, REX, CPUID, RDTSC/RDTSCP y SYSCALL.

Trabajo de decoder aún abierto de alto valor:

- `F1-DC-066`: REP / REPE / REPNE prefixes.
- `F1-DC-081`: differential test contra Zydis.
- `F1-DC-087`: migración Zydis-free como fuente canónica.

### Backend y lowering

- `prisma_emitter` usa vixl y soporta emisión ARM64 para ALU, memoria,
  branches, labels, atomics, rotaciones, bit ops, fences, literal pools e
  integración de I-cache.
- `Lowerer` ya soporta lowering de funciones con bloques y fixups de labels.
- `CondJumpFlags` baja a branch condicional ARM64 con fallback.
- `CondJump` por valor SSA baja en contexto de `Function`; en lista plana sigue
  rechazado cuando no hay contexto de labels.

Items backend recientes:

- `F1-BK-004` en `497e078`: lowering de `CondJumpFlags`.
- `F1-BK-006` en `1d357da`: cobertura de label fixups en CFG lowering.

Siguiente deuda backend importante:

- `F1-BK-012`: NEON SIMD 128-bit.
- `F1-BK-013`: floating point.
- Revisar que el register allocator siga siendo correcto cuando entren valores
  SIMD/FP.

### Passes

El pipeline por defecto incluye:

```text
const_prop -> algebraic -> strength_reduce -> const_prop_2 ->
redundant_load -> CSE -> copy_propagate -> dead_store ->
branch_fold -> dead_code_eliminate
```

Punto de atención: con flags implícitos, DCE debe conservar operaciones que
alimenten branches por flags. Cuando se implemente `Flags` SSA explícito, hay
que actualizar operand collection, liveness y pruebas de eliminación.

### Runtime y cache

- `prisma_runtime` incluye JIT memory, signal handlers, dispatcher y detección
  de features ARM64.
- `prisma_cache` incluye cache en memoria, invalidación, formato persistente,
  zstd y SHA-256.

Siguiente deuda runtime/cache importante:

- `F1-RT-010`: page-protection based SMC detection.
- `F1-RT-011`: guest signal delivery.
- `F1-RT-012`: FPU state save/restore.

### Lean

- `ir-spec/` compila con Lake/Lean 4.
- El workflow `ir-spec (Lean 4)` está activo y controla el budget de `sorry`.
- La implementación C++ avanzó más rápido que la spec en CFG/flags.

Siguiente deuda Lean importante:

- `F1-LN-002`: Flags type en `Syntax.lean`.
- `F1-LN-003`: Block / Function en Syntax.
- `F1-LN-007`: step relation para `CondJumpFlags`.

## CI y validación remota

Workflows activos:

- `.github/workflows/core-stub.yml`
  - Trigger: cambios en `core/**` o el workflow.
  - Hace build real con `clang++-17`, CMake y Ninja.
  - Ejecuta `ctest --test-dir core/build --output-on-failure`.
- `.github/workflows/core-sanitizers.yml`
  - Trigger: push a `main` y PRs que toquen `core/**`.
  - Jobs: ASan+UBSan y TSan.
  - Ambos compilan y ejecutan tests.
- `.github/workflows/ir-spec.yml`
  - Trigger: cambios en `ir-spec/**`.
  - Ejecuta `lake build` y chequeo de `.sorry-budget`.
- `.github/workflows/lint-docs.yml`
  - Trigger: cambios Markdown.
  - Ejecuta markdownlint y frontmatter de RFCs.
- `.github/workflows/shell-stub.yml`
  - Trigger: cambios en `shell/**`.
  - Hoy valida scaffolding; cuando exista `shell/Cargo.toml`, correrá fmt,
    clippy y tests.

Notas de CI al 2026-05-18:

- `core (C++20) - stub` pasó en `dd1c6f0`.
- `core sanitizers` pasó en `dd1c6f0` con ASan/UBSan y TSan.
- El fallo anterior de CI fue un warning de Clang 17 en
  `core/tests/test_ir_serialization.cpp`; quedó corregido en `dd1c6f0`.
- `lint-docs` tenía deuda histórica de estilo Markdown. La configuración se
  ajustó para mantener solo checks útiles y no bloquear por formato heredado.

Validar workflows desde terminal:

```bash
gh workflow list --repo Finithe-Phoenix/prisma
gh run list --repo Finithe-Phoenix/prisma --limit 20
gh run view <run-id> --repo Finithe-Phoenix/prisma --log-failed
gh run watch <run-id> --repo Finithe-Phoenix/prisma --exit-status
```

Verificar protección y webhooks:

```bash
gh api repos/Finithe-Phoenix/prisma/branches/main/protection
gh api repos/Finithe-Phoenix/prisma/hooks --jq '.[] | {id, name, active, events}'
```

Si `branches/main/protection` devuelve 404, `main` sigue sin protección.

## Comandos locales obligatorios

Antes de tocar código C++:

```bash
git pull --ff-only origin main
rg -n "\[~\|" docs/BACKLOG.md
cmake --build core/build --target prisma_core_tests -j2
core/build/prisma_core_tests --reporter compact
```

Build limpio equivalente al workflow core con Clang 17:

```bash
cmake -S core -B /tmp/prisma-core-clang -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug \
  -DCMAKE_CXX_COMPILER=clang++-17 \
  -DCMAKE_CXX_COMPILER_CLANG_SCAN_DEPS=/usr/bin/clang-scan-deps-17
cmake --build /tmp/prisma-core-clang --target prisma_core_tests -j2
/tmp/prisma-core-clang/prisma_core_tests --reporter compact
```

Nota: en este entorno local fue necesario instalar `clang-17` y
`clang-tools-17` para disponer de `clang-scan-deps-17`.

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
npx --yes markdownlint-cli2 $(git ls-files '*.md')
```

## Protocolo autónomo de trabajo

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

3. Elegir un item `[ ]` de alto valor. Prioridad recomendada:
   `F1-IR-003/004/005`, luego `F1-LN-002/003/007`, luego `F1-BK-012/013`,
   luego `F1-RT-010`.

4. Reclamar en `docs/BACKLOG.md`:

   ```text
   [ ] F1-XX-NNN
   ```

   pasa a:

   ```text
   [~|codex] F1-XX-NNN
   ```

5. Commit solo del claim:

   ```bash
   git add docs/BACKLOG.md
   git commit -m "chore(backlog): codex claims F1-XX-NNN"
   git push origin main
   ```

6. Implementar con cambios mínimos y pruebas enfocadas.

7. Ejecutar pruebas locales relevantes y luego la suite completa del core si
   se tocó C++.

8. Commit del código:

   ```bash
   git add <files>
   git commit -m "<scope>: <what>"
   ```

9. Marcar el backlog completado con el short SHA del commit de código:

   ```text
   [x] (<sha>) F1-XX-NNN
   ```

10. Commit del cierre:

    ```bash
    git add docs/BACKLOG.md
    git commit -m "chore(backlog): mark F1-XX-NNN complete"
    ```

11. Push y vigilancia remota:

    ```bash
    git push origin main
    gh run list --repo Finithe-Phoenix/prisma --limit 10
    gh run watch <run-id> --repo Finithe-Phoenix/prisma --exit-status
    ```

12. Si GitHub Actions falla, leer log, corregir, testear local, commitear y
    empujar hasta dejar los checks relevantes verdes.

13. Si el trabajo cambia estado estructural del proyecto, actualizar este
    archivo en el mismo ciclo o en un commit de documentación separado.

## Protocolo para elegir siguiente tarea

Elegir la tarea por impacto y dependencia, no por comodidad:

1. Si hay CI rojo en el último commit propio, arreglar CI primero.
2. Si hay un claim activo propio, terminarlo o abandonarlo formalmente.
3. Si `CondJumpFlags` ya existe pero flags SSA sigue pendiente, preferir
   `F1-IR-003/004/005`. Esa deuda bloquea optimizaciones y spec Lean.
4. Si IR cambió semántica, actualizar Lean después de estabilizar C++.
5. Si se toca backend, añadir tests en `core/tests/test_lowering.cpp` o
   `core/tests/test_emitter.cpp`.
6. Si se toca decoder, añadir golden tests en `core/tests/test_decoder.cpp`
   y, cuando aplique, tests e2e.
7. Si se toca runtime/cache, correr tests específicos y sanitizers.

## Estado de claims

Al 2026-05-18 no hay claims activos reales en `docs/BACKLOG.md`. La única
ocurrencia de `[~|` es la leyenda del archivo.

## Última ola de trabajo relevante

Commits recientes de Codex en `main`:

```text
dd1c6f0 test(ir): fix clang u16 read warning
4030299 chore(backlog): mark F1-BK-006 complete
1d357da test(backend): cover cfg label fixups
b90e813 chore(backlog): codex claims F1-BK-006
1cc32de chore(backlog): mark F1-IR-007 complete
071bae3 feat(ir): add conditional flag jumps
2176a0b chore(backlog): mark F1-BK-004 complete
497e078 feat(backend): lower conditional block jumps
be18692 chore(backlog): mark F1-IR-025 complete
8608326 feat(ir): detect natural loops
d8dd889 chore(backlog): mark F1-IR-024 complete
4a0e946 feat(ir): add dominator analysis
381634b chore(backlog): mark F1-IR-023 complete
10d6ac3 feat(ir): add cfg builder
```

Pruebas locales confirmadas en esta ola:

- `core/build/prisma_core_tests --reporter compact`
  - `3182 assertions in 483 test cases`.
- Build local con `clang++-17` tras instalar `clang-17` y `clang-tools-17`.
- `core (C++20) - stub` remoto verde en GitHub.
- `core sanitizers` remoto verde en GitHub.

## Riesgos y pendientes de infraestructura

- Activar branch protection para `main`.
- Definir required status checks mínimos:
  `core (C++20) - stub`, `core sanitizers`, `ir-spec (Lean 4)` cuando aplique,
  y `lint-docs` para cambios Markdown.
- Decidir si se necesitan webhooks externos. Hoy no hacen falta para CI.
- Agregar runner ARM64 self-hosted cuando Danny tenga el hardware dedicado.
- Evaluar actualizar Actions antes de la migración forzada de Node 20 a Node 24
  en GitHub Actions.

## Cuándo pedir ayuda a Danny

No detenerse para decisiones técnicas normales. Solo pedir intervención cuando:

- Se necesiten compras, cuentas, llaves, contratos o permisos externos.
- GitHub requiera una acción UI que el token actual no permita.
- Haya que tomar una decisión estratégica que contradiga el manifiesto.
- Dos rutas técnicas tengan tradeoffs fuertes y el backlog/RFCs no indiquen
  preferencia.

## Qué no volver a preguntar

- No preguntar si se puede instalar tooling local razonable: instalarlo y
  documentar el cambio.
- No preguntar si se puede correr la suite: correrla.
- No preguntar si se debe empujar al remoto: sincronizar con `origin/main`
  siempre que el trabajo esté committeado y probado.
- No preguntar si se debe vigilar GitHub Actions: vigilarlo después de cada
  push que dispare workflows.

## Definición de "listo" para una tarea

Una tarea está lista cuando:

- El item está reclamado y luego cerrado en `docs/BACKLOG.md`.
- El código está en commits atómicos.
- Tests locales relevantes pasan.
- Si toca `core/**`, `core (C++20)` y `core sanitizers` pasan en GitHub.
- Si toca Markdown, `lint-docs` pasa en GitHub.
- Si toca `ir-spec/**`, `ir-spec (Lean 4)` pasa en GitHub.
- El árbol local queda limpio y `main...origin/main` queda sincronizado.
