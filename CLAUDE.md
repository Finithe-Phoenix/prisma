# CLAUDE.md

Guía para futuras sesiones de AI trabajando en este repositorio.

## Contexto del proyecto

**Prisma** es un Dynamic Binary Translator x86/x64 → ARM64 para emulación Windows en Android. Timeline 48-54 meses (abril 2026 – Q2 2030). Dev lead único: Danny.

El manifiesto técnico y las decisiones estratégicas están en [PROYECTO_PLAN_EJECUCION.md](PROYECTO_PLAN_EJECUCION.md). La investigación previa del ecosistema está en [compass_artifact_wf-*.md](compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md) — esa investigación proponía forkear Winlator; **Danny rechazó conscientemente ese camino el 2026-04-19** a favor del plan épico actual. No revisar esta decisión sin razones nuevas.

## Principios de trabajo en este repo

1. **Ambición técnica sobre pragmatismo de time-to-market.** La meta es impacto académico y salto generacional, no revenue temprano. Sugerencias tipo "mejor usa X que existe" o "forkea Y" violan el manifiesto — salvo en los decision points explícitos del plan.

2. **Research output es entregable de primera clase.** Cada pilar técnico tiene un paper asociado. Blog posts cada 2-3 meses. LaTeX drafts viven en [papers/](papers/).

3. **Correctness > performance, siempre.** El IR se especifica formalmente en Lean 4. Las optimizaciones críticas (especialmente TSO adaptativo) requieren demostración formal antes de merge.

4. **Honest failure mode.** Los decision points del plan son reales. Si un pilar no funciona, publicar resultados negativos honestamente. No esconder ni glosar.

## Convenciones de código

- **C++**: C++20, concepts, std::span, std::atomic_ref. Estilo Google con tabwidth 2 spaces. Clang-format + clang-tidy obligatorio.
- **Rust**: edition 2024 cuando esté estable, 2021 mientras tanto. `cargo clippy -- -D warnings` en CI.
- **Kotlin**: Kotlin 2.0 + Jetpack Compose. Detekt + ktlint.
- **Lean 4**: mathlib como dependencia principal. Lake como build.
- **Python**: 3.12+, black + ruff + mypy strict.

## Testing

- **C++**: Catch2 para unit tests. Cada instrucción x86 decodificada tiene test: bytes → IR → interpreter → comparar con QEMU de referencia.
- **Rust**: `cargo test` + proptest para fuzzing de parsers.
- **Kotlin**: JUnit5 + instrumentation tests para APIs Android específicas.
- **Lean 4**: las proofs son el test.
- **Fuzzing continuo**: AFL++ contra el decoder desde el momento que existe.

## Commit discipline

- Commits en inglés (research is global).
- Formato: `<scope>: <what>` (ej `core/decoder: add SIB byte handling for ModR/M`).
- Nunca commits gigantes — prefer small atomic commits.
- No commits con código sin tests (excepción documentada: WIP prototypes en una rama feature).

## Qué NO hacer

- No añadir dependencias sin justificación documentada en `docs/rfc/`.
- No copiar código de FEX/Box64/QEMU al core. Inspiración sí, copia no (licencias + originalidad técnica).
- No bypassear el fuzzing/tests para "acelerar".
- No escribir docstrings multi-párrafo. Comentarios solo para el WHY no-obvio.
- No usar emojis en código/commits.

## Comandos frecuentes (futuros)

```bash
# Build del core
cmake --preset debug && cmake --build --preset debug

# Tests del core
ctest --preset debug

# Build del shell Rust
cargo build --workspace

# Tests del shell
cargo test --workspace

# Verificar proofs Lean 4
lake build

# Build de la app Android
./gradlew assembleDebug
```

Estos comandos aún no existen (fase 0). Se añadirán conforme cada subproyecto arranque.

## Memoria persistente

Este repo tiene memoria AI en `~/.claude/projects/-Users-...Prisma/memory/`. Si se te pide "recordar" algo estable del proyecto, guardarlo ahí siguiendo las reglas del sistema de memoria.
