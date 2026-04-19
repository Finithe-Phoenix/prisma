# ir-spec — Formal IR Specification in Lean 4

**Pilar 2 del proyecto.** Especificación formal con semántica operacional del IR de Prisma. Demostraciones de correctness de passes de optimización críticos.

## Responsabilidad

Definir el IR de Prisma como un objeto matemático en Lean 4. Este NO es el IR que ejecuta en producción — esa implementación vive en `../core/src/ir/` y se valida contra esta spec.

La relación entre ambos es la misma que CompCert Clight (spec Coq) tiene con el compiler real de C: la spec es la autoridad.

## Estructura (futura)

```
ir-spec/
├── PrismaIR/
│   ├── Syntax.lean          # Sintaxis abstracta del IR
│   ├── Semantics.lean       # Semántica operacional (step relation)
│   ├── TypeSystem.lean      # Tipos del IR (si aplica)
│   └── Passes/
│       ├── DCE.lean         # Dead Code Elimination + proof de soundness
│       ├── ConstProp.lean   # Constant propagation + proof
│       └── TSOAdaptive.lean # TSO adaptive pass + proof (Pilar 3)
├── lakefile.toml
└── lean-toolchain
```

## Herramientas

- **Lean 4** (stable + mathlib como dependencia).
- **Lake** (build system oficial de Lean 4).
- **VSCode Lean4 extension** para desarrollo.

## Entregables académicos asociados

Paper target: **POPL 2029** o **PLDI 2030** — *"A Formally Verified IR for x86→ARM64 Dynamic Binary Translation"*.

## No existe aún

Fase 1 (semanas 9-14) arrancará con el diseño. Antes de eso, **semana 5-6 de Fase 0** incluye tutorial básico de Lean 4 + exploración de mathlib para calibrar curva de aprendizaje.
