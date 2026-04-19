# core — Prisma DBT Engine

**Lenguaje:** C++20 (concepts, std::span, std::atomic_ref).
**Build:** CMake 3.30+.
**Test:** Catch2 + AFL++ fuzzing.

## Responsabilidad

El núcleo del Dynamic Binary Translator: decoder x86_64, IR lowering, passes de optimización, backend ARM64 emitter, translation cache, signal handling, integración NPU.

## Estructura

```
core/
├── include/        # Headers públicos
├── src/
│   ├── decoder/    # x86_64 decoder
│   ├── ir/         # IR data structures (impl C++; spec en ../ir-spec/)
│   ├── passes/     # Optimization passes
│   ├── backend/    # ARM64 emitter (via vixl)
│   ├── cache/      # Translation cache on-disk + memoria
│   ├── runtime/    # Signal handlers, dispatcher, syscall layer
│   └── npu/        # ONNX Runtime integration (Pilar 1)
└── tests/          # Unit + integration + differential vs QEMU
```

## No existe aún

Fase 0 en progreso. Primer código real en Fase 1 (semana 9+).

## Dependencias externas (futuro)

- **vixl** (ARM, Apache 2.0) — ARM64 emitter library.
- **ONNX Runtime** (MIT) — NPU delegation.
- **Catch2** (BSL-1.0) — testing.
- **xxHash** (BSD) — translation cache keys.
