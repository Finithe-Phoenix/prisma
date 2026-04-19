# Prisma

> Dynamic Binary Translator de nueva generación x86/x64 → ARM64 para emulación Windows en Android.

**Estado:** Fase 0 (fundación) — abril 2026.

## Por qué existe

Existen emuladores Windows para Android hoy (Winlator, GameHub, GameNative), todos construidos sobre FEX o Box64. Prisma **no es otro fork**. Prisma es un DBT nuevo escrito desde cero con 6 pilares técnicos que ningún emulador móvil tiene:

1. **NPU-Assisted Translation** — usar NPUs ociosas para hot path prediction y TSO classification.
2. **IR con semántica formal en Lean 4** — CompCert-style para garantizar correctness de optimizaciones críticas.
3. **TSO adaptativo con ML** — eliminar barreras `DMB ISH` en regiones probadamente seguras.
4. **Translation cache distribuida P2P + CDN** — caches firmadas criptográficamente compartidas entre dispositivos.
5. **Virtualización híbrida DBT+KVM** — modo dual en Tensor SoCs con AVF/pKVM.
6. **Graphics translation avanzada** — shader graph analysis, Vortek++ para Mali.

El plan completo y el manifiesto están en [PROYECTO_PLAN_EJECUCION.md](PROYECTO_PLAN_EJECUCION.md).

## Estructura

```
prisma/
├── core/           # C++20 DBT engine (el nuevo core)
├── ir-spec/        # Lean 4 formal IR specification
├── shell/          # Rust orchestrator
├── android/        # Kotlin + Compose app
├── npu-models/     # ONNX models + training pipeline
├── server/         # Rust: cache service, P2P tracker
├── tools/          # Python: ML pipeline, benchmarks, release
├── third_party/    # FEX, Box64, Wine, DXVK (referencias, no forks)
├── papers/         # Drafts LaTeX de publicaciones
└── docs/           # Research notes, RFCs, blog drafts
```

## Compromiso de timeline

48-54 meses. v1.0 pública target Q2 2030. Esto no es un side project — es un compromiso de carrera.

## Research output

Target mínimo 3 papers publicados al fin de Fase 6. Target mínimo 1 en venue top-tier (MICRO, ASPLOS, POPL, PLDI). Blog posts técnicos cada 2-3 meses desde hoy.

## Licencia (futuro)

El **core DBT + IR spec + NPU models + graphics research** se publicarán como **MIT** al final de Fase 6 (~2030). La **app Android + cloud services** permanecerán como producto comercial freemium. Durante Fase 0-5 el repositorio es privado.
