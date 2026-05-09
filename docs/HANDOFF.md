# HANDOFF.md — Continuación del trabajo en otro equipo

> Para el agente AI que recoge la rama `claude/hopeful-taussig-051239`
> en el equipo de Danny en otra máquina. Léelo COMPLETO antes de
> tocar código.

## 0. Auto-setup en máquina nueva

Para arrancar desde cero en otro equipo:

```bash
git clone https://github.com/Finithe-Phoenix/prisma.git
cd prisma
git checkout claude/hopeful-taussig-051239    # rama de la sesión saliente

# Onboarding de un solo paso (idempotente):
./tools/setup-agent-env.sh --with-skills
```

Lo que hace `setup-agent-env.sh`:
1. Verifica toolchain (cmake / ninja / clang / git / gh).
2. Opcional (con `--with-skills` o confirmación interactiva): instala
   los 14 skills de [obra/superpowers](https://github.com/obra/superpowers)
   en `~/.claude/skills/`. Skills: test-driven-development,
   using-git-worktrees, finishing-a-development-branch,
   systematic-debugging, subagent-driven-development,
   dispatching-parallel-agents, brainstorming, writing-plans,
   executing-plans, requesting-code-review, receiving-code-review,
   verification-before-completion, writing-skills, using-superpowers.
3. Configura `core/build` (Debug, Ninja) si no está.
4. Build + test (debe terminar verde con 797+ casos).
5. Imprime apuntadores a CLAUDE.md / HANDOFF.md / SESSION_TRACE.md.

Flags útiles: `--no-skills` (skip skill install), `--no-build` (skip
cmake/build), `--yes` (no preguntar confirmación TTY).

## 1. Lo primero — orientación rápida

- **Repo**: Prisma — DBT x86_64 → ARM64 para emulación Windows en Android.
  Manifesto en `PROYECTO_PLAN_EJECUCION.md`.
- **Rama actual**: `claude/hopeful-taussig-051239`. 41 commits sobre `main`
  desde `8884efd` (`docs(claude.md): refresh — F2-IR-048 AVX-128 VEX dispatch
  landed`).
- **HEAD**: `e59b944`.
- **Working tree**: limpio.
- **Tests**: 797 casos / 4836 assertions verde, ejecutando JIT real en
  Apple Silicon. Run `core/build/prisma_core_tests --reporter compact "~signal_handler*"`.
- **Reglas del repo**: `CLAUDE.md` en raíz. Léelo, especialmente las
  secciones de convenciones de código, commits, y "Qué NO hacer".
- **Coordinación multi-agente**: `docs/COORDINATION.md`.
  Hay otros agentes potenciales (`codex`); reclama items de `docs/BACKLOG.md`
  con `[~|claude]` antes de tocar.

## 2. Estado al cierre de la sesión previa

Revisa **`docs/SESSION_TRACE.md`** para la trazabilidad completa de los
41 commits agrupados por bloque temático. Resumen de alto nivel:

- **AVX-128 + AVX-256 SSE..SSE4.2 cobertura**: prácticamente completa.
  Casi todos los handlers SSE/SSE2/SSE3/SSSE3/SSE4.1/SSE4.2 que usan
  per-128-bit-lane semantics están extendidos a `VEX.L=1` ymm via la
  representación pair-of-Vec128 (`xmm[i]` + `ymm_hi[i]` en
  `CpuStateFrame`, IR ops `LoadVecRegHi`/`StoreVecRegHi`).
- **AVX-256 lane-crossing**: VBROADCAST{SS,SD,F128}, VINSERTF128,
  VEXTRACTF128, VPERM2F128, VPMOVZX/SX BW/BD/BQ/WD/WQ/DQ — todos
  implementados sin nuevos IR ops.
- **FMA3 completo**: 96 opcodes (4 familias × 3 orderings × {PS,PD}×{xmm,ymm}
  + scalar SS/SD + MADDSUB/MSUBADD packed). Dos IR ops nuevos:
  `VecFpFma`, `VecFpScalarFma`.
- **MUL/DIV proper**: x86 MUL/IMUL ahora escriben rdx:rax completo
  (UMulHi/SMulHi); DIV/IDIV escriben quotient + remainder. Nuevos
  `BinOpKind`: UMulHi, SMulHi, UDiv, SDiv, UMod, SMod.
- **F2-PS-002 flag elimination**: DCE marca WriteFlags/ReadFlag como
  puros + cierra operand-collect gaps.

## 3. Qué NO hacer

Errores comunes que el agente saliente cometió y aprendió:

- **No remover el gate `vex.L → UnsupportedEncoding` globalmente sin
  añadir un allowlist por opcode.** Hay un único punto de admisión
  L=1 en `core/src/decoder/x86_decoder.cpp` (busca `// F2-IR-005 — AVX-256
  allowlist`) que debe crecer cuando se añade un opcode nuevo.
- **No olvidar añadir `vex.present && vex.L` checks en los handlers
  que aún no soportan L=1.** Si un handler no se ha extendido,
  asegúrate de que el allowlist NO admite su opcode.
- **No olvidar que `compute_liveness` en `core/src/backend/lowering.cpp`
  necesita una rama por cada IR op nuevo.** Sin eso, la expiry FP
  recicla refs vivas → DanglingRef. (Bug latente con `FpBinOp` arreglado en
  `0597402`.)
- **No olvidar que `VecConstant` lowering ahora carga 128 bits
  completos** (commit `f340201`). El comportamiento previo (truncar
  la mitad alta) era un bug latente que solo se manifestó con FMADDSUB.
- **No copiar código de FEX/Box64/QEMU al core.** Inspiración sí,
  copia no (licencias + originalidad técnica, manifiesto del proyecto).
- **No usar emojis en código/commits.** Comentarios solo para el
  WHY no-obvio.

## 4. Convenciones técnicas críticas

### Cuando añadas un IR op nuevo

Es un cambio multi-archivo. Toca, en orden:

1. `core/include/prisma/ir.hpp` — struct + variant + `operator==` declaration.
2. `core/src/ir/operation.cpp` — `operator==` impl.
3. `core/include/prisma/profiler.hpp` — `OpCounter::Kind` enum.
4. `core/src/ir/profiler.cpp` — `kind_for` mapping.
5. `core/src/ir/pretty_print.cpp` — print branch.
6. `core/src/ir/validate.cpp` — `op_is_pure` + `for_each_operand_ref`.
7. `core/src/ir/serialize.cpp` — `OpKind` enum + `kind_for` + `write_payload` + `read_payload_*` + dispatch case.
8. `core/src/passes/dce.cpp` — `is_pure_for_dce` (si aplica) +
   `collect_operand_refs`.
9. `core/src/backend/lowering.cpp` — `compute_liveness` rama +
   `lower_stmt` rama.
10. `core/src/backend/emitter.cpp` — primitives ARM64 si necesarias
   (declaradas en `core/include/prisma/emitter.hpp`).
11. `core/tests/test_profiler.cpp` — añadir el nuevo Kind al test
   "OpCounter visits one of every kind".

Si saltas alguno, el build falla con `-Wswitch` (todos los `switch`
sobre IR variants tienen exhaustive matching) o el sorry de Lean
crece. Usa **F2-IR-006 scalar FMA** (commit `afccedd`) como receta
canónica de "todo el tour".

### Cuando extiendas un handler existente a `vex.L=1`

Patrón consolidado:

1. Hoist el cómputo de address antes del primer load (necesario
   para reusarlo +16 en el high half).
2. Emite la secuencia low-half (existente).
3. Si `vex.present && vex.L`, emite la secuencia hi-half usando
   `LoadVecRegHi` para registros y `LoadVec` con `addr+16` para memoria.
4. Cierra con `StoreVecRegHi`.
5. Añade el opcode al allowlist global en
   `core/src/decoder/x86_decoder.cpp` cerca del comentario
   `// F2-IR-005 — AVX-256 allowlist`.

### Build & test loop

```bash
# Build (lleva 1-2 min en caché):
cmake --build core/build

# Test rápido (excluye flaky signal handler):
core/build/prisma_core_tests --reporter compact "~signal_handler*"

# Test específico:
core/build/prisma_core_tests "e2e: VFMADD231PS xmm2*"

# Sanitizers (más lento; F1-TC-008):
cmake -S core -B /tmp/prisma-asan -DPRISMA_ENABLE_ASAN=ON \
    -DPRISMA_ENABLE_UBSAN=ON -G Ninja
cmake --build /tmp/prisma-asan --target prisma_core_tests
/tmp/prisma-asan/prisma_core_tests --reporter compact "~signal_handler*"

# Lean spec (un sorry pendiente, F1-TC-009):
cd ir-spec && lake build
```

Tests E2E corren JIT real en Apple Silicon — confirma con `uname -m`
que estás en `arm64` antes de esperar que pasen.

## 5. Hot spots — dónde meter mano siguiente

Priorizado por valor / esfuerzo:

### A. F2-BK-008 + F2-BK-009 — REP STOSB / MOVSB JIT loop

**Por qué importa**: real binaries usan REP STOSB/MOVSB para `memset`/
`memcpy`. Hoy decodifican como `InlineAsm` placeholder (handler
en `core/src/decoder/x86_decoder.cpp:2899`, función
`rep_string_inline_asm`). Performance horrible — un syscall por
iteración.

**Plan recomendado**:

1. Nuevo IR op `RepStos { OpSize size; bool reverse; }` (size = I8/I16/I32/I64;
   reverse = guest DF flag, asume 0 por ahora).
2. Decoder: cambiar el branch `if (has_f2 || has_f3)` en
   STOSB/STOSW/STOSD/STOSQ (línea ~2912) para emitir `RepStos` en
   lugar de fallback a `InlineAsm`.
3. Lowering: usar el emitter actual (que ya tiene labels + branches
   de F1-BK) para emitir un loop nativo:
   ```
   loop:
     cbz x_rcx, end
     strb/strh/str w_al/_ax/_eax/x_rax, [x_rdi]
     add x_rdi, x_rdi, #size
     sub x_rcx, x_rcx, 1
     b loop
   end:
   ```
4. Después MOVSB/MOVSW/MOVSD/MOVSQ con `RepMovs` similar (loop con
   load + store).

Estimado: 4-6 commits.

### B. F2-PS-004 Global CSE via dominators

**Por qué importa**: el `dominators.hpp` ya está listo (F1-DC). El
`PassManager` actual solo soporta `Stmts → Stmts`. Lo bloqueado es
extender a `Function → Function`.

**Plan recomendado**:

1. Añadir `using FunctionPassFn = std::function<ir::Function(const ir::Function&)>;`
   en `core/include/prisma/passes.hpp`.
2. Extender `PassManager::add_function(name, fn)` y `run_function(input)`.
3. Implementar `global_cse(const ir::Function&) → ir::Function` que
   usa los dominators para encontrar redundancias cruzando bloques.
4. Wire en `default_pipeline` después del CSE intra-bloque actual.

**Caveat**: la mayoría de las `ir::Function` que produce el translator
hoy son single-block (un x86 instr → un Decoded → un BB). El valor
real de Global CSE viene cuando integremos múltiples instrucciones
en el mismo `Function`. Mira `core/src/translator/...` para ver qué
nivel de unidad funcional se está armando.

Estimado: 3-4 commits.

### C. F2-PS-003 LICM

Mismo prerrequisito (function-level pass). Necesita además detección
de loops (ya tenemos backedges via dominators). Es claramente
post-B.

### D. VPTEST ymm + VPBLENDVB ymm

VPTEST ymm está bloqueado: `WriteFlagsPtest{lhs,rhs}` no compone
para ymm porque setea Z y C como `(lhs & rhs) == 0` y `(~lhs & rhs) == 0`
respectivamente — necesitamos OR de dos halves para cada uno. Plan:
nuevo IR op `WriteFlagsPtestYmm{lo_lhs, lo_rhs, hi_lhs, hi_rhs}`.

VPBLENDVB ymm: encoding VEX usa `imm8[7:4]` como mask register (4to
operando). El handler xmm actual usa SSE4.1 implicit XMM0; necesita
nueva rama en el decoder para leer el imm8 byte y usar `vex.vvvv`
como src1.

Estimados: 2-3 commits cada uno.

### E. F2-IR-007/008 + F2-BK-005 — x87

Dominio nuevo grande. ARM64 no tiene 80-bit FP nativo. Plan mínimo
viable: tratar x87 stack como doubles (precisión reducida; documentar
divergencia). Mira `core/include/prisma/cpu_state.hpp:43-48` —
`X87Slot` ya existe pero no se usa.

Estimado: 6-8 commits para FLD/FST/FADD/FMUL/FDIV/FXCH mínimo.

### F. F2-BK-010 — Call/ret return-stack

Toca dispatcher + runtime. Hoy CALL/RET hacen un dispatcher round-trip
por instrucción. La idea: predicción de retorno via stack interno
(análogo al RAS de un CPU real). Mira `compass_artifact_*.md` para
el state-of-the-art en DBTs.

## 6. Cosas en limbo / decisiones pendientes

- **Pair-allocator para ymm en lowerer**: `0597402` lo dejó deferred
  ("hasta que haya measured demand"). Probablemente innecesario si
  bumpamos `kFpScratchPoolSize` más, pero re-evaluar cuando aterrice
  más AVX-256 código real.
- **FP spill plumbing**: igualmente deferred. Plumbing presente pero
  desactivado.
- **Lean specs para los IR ops nuevos** (`LoadVecRegHi`, `StoreVecRegHi`,
  `VecFpFma`, `VecFpScalarFma`, las 6 nuevas `BinOpKind`s): no han
  sido añadidas a `ir-spec/PrismaIR/Syntax.lean` ni a
  `Semantics.lean`. El sorry budget está en 1; añadir cualquiera
  con un sorry sube a 2 — flag explícito en commit message si lo
  haces.
- **Trust de alignment**: todos los memory loads en AVX-256 confían
  en el guest (no fault-check). Consistente con xmm. Re-evaluar si
  apuntamos a hardware Apple silicon strict-alignment-mode.

## 7. Para empezar — checklist práctica

1. `cd <repo-root>`.
2. `git fetch origin claude/hopeful-taussig-051239 && git checkout claude/hopeful-taussig-051239`.
3. `cmake --build core/build` — confirma build limpio.
4. `core/build/prisma_core_tests --reporter compact "~signal_handler*"` —
   confirma 797 cases / 4836 assertions verde.
5. Lee `CLAUDE.md`, `docs/COORDINATION.md`, `docs/BACKLOG.md`.
6. Lee `docs/SESSION_TRACE.md` para entender qué se hizo y por qué.
7. Pick a hot spot de la sección 5. Reclama el item del backlog
   con `[~|claude]` ANTES de tocar código.
8. Bug fixes, regresiones, o cualquier descubrimiento sobre código
   existente: documenta en el commit message.

## 8. Contacto / continuidad

- **Dueño del repo**: Danny (`finitofinite@gmail.com`).
- **Convenciones de commit**: inglés, `<scope>: <what>`, sin emojis,
  con `Co-Authored-By: <agente>` al final.
- **No hagas push a main sin PR**. Push a `claude/hopeful-taussig-051239`
  o crea una nueva rama topical.

Buena suerte. El working state está sólido — 41 commits sin
regresiones, una buena base para la siguiente capa de scope.
