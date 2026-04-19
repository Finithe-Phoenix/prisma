---
id: 0001
title: Prisma IR shall be SSA-based, not template-based
status: accepted
authors: [Danny]
created: 2026-04-19
updated: 2026-04-19
supersedes: []
superseded_by: null
---

# RFC 0001: Prisma IR shall be SSA-based, not template-based

## Summary

Prisma usa un IR SSA-like con estructura de nodos ordenados y offsets comprimidos como representación intermedia central del DBT. **Rechazamos** el enfoque template-based directo de Box64. La espec formal del IR vive en Lean 4 (ver `ir-spec/`) y es autoritativa sobre la implementación C++ del core.

## Motivation

El IR es la columna vertebral del DBT. La decisión condiciona 4 de los 6 pilares épicos:

- **Pilar 2 (IR formalmente verificado)** — requiere estructura bien definida con semántica operacional. Template-based directo no tiene semántica compositional clara.
- **Pilar 3 (TSO adaptativo)** — requiere análisis cross-instruction (¿qué regiones son single-threaded / lock-free?). Template-based no permite ese análisis sin reconstruir algo parecido a IR.
- **Pilar 1 (NPU-assisted)** — el clasificador NPU consume features extraídas del IR. Sin IR, no hay features estables.
- **Pilar 6 (graphics translation whole-graph)** — inspiración en shader analysis: sin representación intermedia, no hay análisis de grafo.

Si empezamos template-based, el refactor para añadir IR es enorme y atrasa todos los pilares. Si empezamos con IR, el overhead de compile-time es mayor pero el diseño escala.

## Context

### Restricciones

- **Timeline Fase 1:** 24 semanas (semanas 9-32). 6 de esas se destinan exclusivamente a diseño del IR en Lean 4 antes de tocar C++.
- **Memoria:** queremos compactness para traducir binarios grandes. FEX usa offsets 32-bit (límite 4M nodos) y esto ha demostrado ser suficiente.
- **Iteration speed:** el IR debe permitir compile-time razonable. Un IR too heavy (ej: LLVM IR completo) no es opción.
- **Solo-dev:** sin equipo para mantener IR extravagante. Diseño minimalista.

### Prior art

Referencias técnicas en `docs/research_notes.md`:

- **FEX** usa IR SSA-like con `OrderedNode` (nodos doblemente enlazados) y `OpNodeWrapper` (offset 32-bit). Semántica implícita, no formalizada.
- **Box64** NO usa IR. Pipeline de 4 passes que decodifica y emite directamente. Rapid pero limita optimización.
- **Valgrind VEX** usa IR con `IRStmt` y `IRExpr` (algo más rico). Bien documentado, paradigma académico.
- **QEMU TCG** usa IR propio con ops tipo load/store/binop. Optimizaciones limitadas cross-block.
- **CompCert Clight / CakeML source IR** — referencias de IR formal con semántica Coq/Lean probada.

### Invariantes del sistema

- Cada valor SSA tiene un único `Ref` (well-formedness requirement).
- Basic blocks terminan en op de control flow (jump / condJump / ret).
- Memory ops existen en dos variantes (TSO y non-TSO) que son semánticamente distintas; passes pueden rewrite TSO→non-TSO solo si prueban correctness bajo invariantes.

## Considered alternatives

### Alt A — Template-based directo (estilo Box64)

**Qué es:** decodificar instrucción x86, emitir directamente secuencia ARM64 sin representación intermedia. Passes organizados como fases de transformación sobre texto ensamblador o sobre buffers de bytes.

**Ventajas:**
- Compile time rápido (sin IR allocation).
- Codebase más pequeño.
- Fácil de arrancar para solo-dev.

**Desventajas:**
- No permite análisis cross-instruction (imposible hacer TSO adaptativo real, impossible hacer NPU classification sobre features estables).
- Formalización matemática requiere inventar IR ad-hoc de todos modos para el paper.
- Sin análisis de dataflow, optimizaciones quedan restringidas a peephole local.

**Por qué descartada:** bloquea 4 de los 6 pilares épicos. Es el camino que Box64 eligió y por eso no diferencia técnicamente.

### Alt B — IR tree-based (AST-like)

**Qué es:** IR como árbol de expresiones con operandos referenciando hijos directamente, tipo C AST.

**Ventajas:**
- Más cerca del código x86 original (mental model más simple).
- Pattern matching directo.

**Desventajas:**
- Dataflow analysis es molesto (no tienes edges explícitos entre definiciones).
- Optimizaciones comunes (DCE, constant propagation) requieren transformaciones no triviales.
- Menos estándar académicamente — la literatura se inclina a SSA.

**Por qué descartada:** SSA es state-of-the-art para compilers, la literatura de verificación formal asume SSA, y escribir papers sobre Prisma es más fácil si el IR se describe en términos estándar.

### Alt C — Adoptar LLVM IR / Cranelift IR

**Qué es:** no diseñar IR propio. Usar uno existente.

**Ventajas:**
- Trabajo enorme ya hecho.
- Herramientas existentes (verificación, serialización).

**Desventajas críticas:**
- **LLVM IR** es demasiado grande. Traduce C/C++/Rust, no se diseñó para DBT x86. Compile times son hostiles a DBT (iteración lenta).
- **Cranelift CLIF** fue diseñado para WebAssembly: código bien tipado, sin self-modifying code, sin memoria compartida con semántica TSO. No es apto para x86.
- Semántica formal de estos IRs NO está verificada al nivel que necesitamos (CompCert sí está, pero es para C, no DBT).

**Por qué descartada:** el manifiesto épico ya evaluó esto. Investigación previa en `compass_artifact_wf-*.md` concluye "ningún DBT de producción usa estos IRs por razones estructurales".

### Alt D — IR SSA-like con offsets 32-bit (estilo FEX) ← DECISIÓN

**Qué es:** SSA con nodos en lista ordenada y referencias como offsets 32-bit a `IROp_Header`. Representación implícita de dominadores vía posición en la lista (los scheduled blocks ya están en topological order).

**Ventajas:**
- Compactness (memoria suficiente para binarios realistas).
- Dataflow analysis natural (edges explícitos).
- Compatible con formalización (SSA se traduce a álgebras de CFG con variable única).
- Semántica operacional clara (step relation sobre `MachineState`).

**Desventajas:**
- Límite 4M nodos por traducción — aceptable (FEX lo ha usado 6 años sin hit).
- Offset computation requiere cuidado de invariantes.
- Más complejidad de diseño upfront (6 semanas dedicadas en Fase 1).

**Por qué elegida:** único camino que satisface formalización + dataflow + compactness + precedente de producción.

## Decision

1. Prisma IR es SSA-like. Cada valor tiene un `Ref` único (modelado como `Nat` en Lean, `uint32_t` en C++).
2. Basic blocks son listas ordenadas de statements (`Stmt { result : Option Ref, op : Op }`). Último statement es control-flow (jump/condJump/ret).
3. La **fuente de verdad** de la sintaxis y semántica del IR es `ir-spec/PrismaIR/Syntax.lean` y `ir-spec/PrismaIR/Semantics.lean`. La implementación C++ en `core/src/ir/` debe refinar esta spec (refine vs implement refine).
4. El IR tiene dos formas de memory ops: TSO (`loadMemTSO`, `storeMemTSO`) y non-TSO (`loadMem`, `storeMem`). El decoder x86 emite SIEMPRE TSO por defecto. El TSO-adaptive pass (Pilar 3) reescribe TSO→non-TSO solo cuando tiene prueba (estática o runtime-verified) de que la región es single-threaded o lock-free.

## Consequences

### Beneficios

- Los 6 pilares épicos son implementables sobre esta base.
- El paper de Pilar 2 (POPL/PLDI) tiene fundación técnica sólida.
- FEX ya demostró viabilidad de esta arquitectura — reducimos riesgo estructural.
- Refactor futuro a offsets 16-bit por bloque (si compactness lo exige) es local.

### Costes

- 6 semanas de Fase 1 dedicadas exclusivamente a diseño IR + primer semántica formal, antes de cualquier código C++.
- Dos implementaciones (Lean 4 spec + C++ runtime) que deben mantenerse consistentes.
- Disciplina de no añadir opcodes al IR sin actualizar la spec primero.

### Reversibilidad

**Baja.** Cambiar de SSA a template-based sería rehacer el core. Cambiar de 32-bit offsets a otra representación es local (capa de storage). Cambiar la forma del `Op` inductivo es moderado (afecta passes pero no semántica fundamental).

## Implementation notes

- `ir-spec/` ya tiene el scaffolding mínimo (Fase 0 sem 1).
- Los 6 opcodes iniciales (constant, binop, loadReg/storeReg, loadMem/storeMem/loadMemTSO/storeMemTSO, compare, jump/condJump/ret) son el MVP para hello world.
- El set completo para coreutils-level se estimará en RFC 0003 (pendiente) — probablemente ~80 opcodes.
- C++ runtime hereda nombres y semántica del IR Lean. Si hay discrepancia, el Lean gana.

## Open questions

- **Split `Op` en `ValOp` / `EffectOp`?** El IR actual mezcla ops puras (que producen valor) con efectos (stores, jumps). Un split puro es más limpio pero complica pattern matching. Decisión diferida a RFC 0002 (pendiente).
- **¿Typed IR o untyped?** Actualmente `OpSize` es metadato no-tipado; los refs son todos `Nat`. Un IR tipado (cada ref tiene tipo `i8/i16/i32/i64/vec128/...`) facilita validación pero complica la grammar. Decisión diferida.
- **¿Block arguments (phi elimination via parámetros) vs phi nodes explícitos?** CompCert usa block args, LLVM usa phis. Diferido a RFC 0004.

## References

- `docs/research_notes.md` — scan comparativo FEX/Box64.
- `ir-spec/PrismaIR/Syntax.lean` — implementación actual.
- [FEX IR design notes (inferidas)](https://github.com/FEX-Emu/FEX/tree/main/FEXCore/Source/Interface/IR)
- [CompCert Clight semantics](https://compcert.org/doc/html/Clight.html) — referencia de IR con semántica formal.
- [Valgrind VEX IR reference](https://valgrind.org/docs/manual/manual-core.html).
