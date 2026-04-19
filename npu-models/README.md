# npu-models — Pilar 1: NPU-Assisted Translation

**Lenguaje training:** Python 3.12+ (PyTorch).
**Formato deployment:** ONNX.
**Runtime deployment:** ONNX Runtime Android con NNAPI / NeuroPilot / Hexagon delegate.

## Responsabilidad

Modelos pequeños (target 5-50 MB) que aceleran decisiones del DBT corriendo en NPU:

1. **Hot path prediction** — clasifica bloques traducidos por probabilidad de ser ejecutados muchas veces, permitiendo speculative optimization.
2. **TSO region classification** — 5 categorías: single-threaded, lock-free, shared-mutable, I/O, unknown.
3. **SIMD pattern matching** — detecta secuencias x86 SIMD con equivalente óptimo no-obvio en NEON/SVE2.
4. **Register allocation hints** — pre-computa sugerencias por bloque.

## Estructura

```
npu-models/
├── training/       # PyTorch training pipeline + datasets
├── inference/      # ONNX models listos para deployment + test harness
└── (futuro) eval/  # Scripts de evaluación de accuracy per-SoC
```

## Datasets (futuro)

Capturas de ejecución de binarios x86 reales con etiquetas:
- Juegos representativos (ética: solo juegos con ownership del dev, no distribución).
- coreutils compilados como x86_64.
- Benchmark suites (SPEC, Dhrystone, CoreMark).

**Regla legal:** NO publicar trazas que contengan código protegido. Datasets derivados se publican como features numéricas anonimizadas.

## Entregable académico

**Paper MICRO 2028 o ASPLOS 2029:** *"NPU-Assisted Dynamic Binary Translation on Mobile SoCs"*.

## No existe aún

Fase 2.5 (semanas 57-64) arranca el trabajo concreto. Antes de eso, solo lectura (Tier 6 de `docs/research_notes.md`).
