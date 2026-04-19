# tools — Scripts de desarrollo

**Lenguaje:** Python 3.12+ (black + ruff + mypy strict).

## Contenido

```
tools/
├── benchmarks/     # Harness de benchmarks académicos (Dhrystone, CoreMark, SPEC subset, per-game)
├── ml-pipeline/    # Training pipeline completo de Pilar 1 (NPU models)
└── release/        # Scripts de build/sign/package/upload APKs
```

## Convenciones

- Dependencias via `uv` (o `poetry` si `uv` no es estable en 2026).
- Cada subproyecto es un Python package independiente con su `pyproject.toml`.
- Type hints obligatorios. `mypy --strict` en CI.

## No existe aún

- `benchmarks/` arranca en Fase 2 (semana 49+).
- `ml-pipeline/` arranca en Fase 2.5 (semana 57+).
- `release/` arranca en Fase 5 (semana 141+).
