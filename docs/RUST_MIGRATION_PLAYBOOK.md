# Prisma Rust Migration — Agent Playbook

> Para CODEX y Gemini. Cómo migrar el core C++ → Rust de forma
> incremental, segura, y coordinada entre agentes.

## Overview

Migración en 6 fases (RFC 0015). Cada fase produce un crate funcional
que puede operar de forma independiente. La C API existente (`capi.h`)
permite que C++ y Rust coexistan en el mismo proceso.

**Regla de oro:** nunca romper los tests C++ existentes. Cada fase
debe pasar `core/build/prisma_core_tests` antes de declararse completa.

## Stack proposal

```
shell/
  Cargo.toml                     # Workspace root
  prisma-ir/                     # Fase 0 — IR types (listo)
  prisma-cache/                  # Fase 1 — translation cache
  prisma-passes/                 # Fase 2 — optimization pipeline
  prisma-decoder/                # Fase 3 — x86_64 → IR decoder
  prisma-runtime/                # Fase 4 — dispatcher + signals
  prisma-backend/                # Fase 5 — ARM64 assembler + lowering
  core-sys/                      # Raw FFI bindings (existente)
  core/                          # Safe Rust wrapper (existente)
  orchestrator/                  # Container lifecycle (existente)
```

## Agent territory split

| Agente | Crates | Responsabilidad |
|--------|--------|-----------------|
| **Claude** | `prisma-ir`, `prisma-passes`, `prisma-runtime` | IR types, passes pipeline, runtime (dispatcher, signals, SMC, syscalls) |
| **Codex** | `prisma-cache`, `prisma-decoder`, `prisma-backend` | Translation cache, x86 decoder, ARM64 emitter + lowering |

**Compartido:** `prisma-ir` (ambos lo consumen), differential testing
framework, Lean IR cross-check.

## Phase-by-phase recipe

### Cómo empezar una fase

1. **Leer RFC 0015** — entender qué componente se migra y sus dependencias
2. **Leer el archivo fuente C++** — entender la implementación actual
3. **Leer los tests C++** del componente — entender qué cobertura existe
4. **Crear el crate Rust** con `cargo init` + registrar en `Cargo.toml`
5. **Escribir los tipos** (si aplica, extender `prisma-ir`)
6. **Escribir la implementación** — función por función, test por test
7. **Differential testing** — probar contra la implementación C++
8. **C API bridge** — exponer la función principal del crate vía `extern "C"`
9. **Commit** con mensaje `feat(rust-<crate>): <descripción>`

### Cómo extender prisma-ir (recipe para nuevos Op)

Cuando se añade un nuevo IR op en el decoder C++, hay que:

1. Añadir variante al `Op` enum en `core/include/prisma/ir.hpp`
2. **Añadir variante al `Op` enum en `shell/prisma-ir/src/lib.rs`**
3. Añadir struct de datos si tiene campos
4. Añadir al validator C++ (`validate.cpp`)
5. **Añadir al validator Rust** (`prisma-ir/src/validate.rs` cuando exista)
6. Añadir serialización C++ (`serialize.cpp`)
7. **Añadir serialización Rust** (serde derive automático)
8. Añadir profiler C++ (`profiler.cpp`)
9. **Añadir profiler Rust** (`OpCounter`)
10. Añadir lowering C++ (`lowering.cpp` y `emitter.cpp`)
11. Añadir tests C++ (decoder + lowering)
12. **Añadir tests Rust** (round-trip + validate)
13. Commit en C++ primero, luego en Rust

### Cómo hacer differential testing

```rust
// Plantilla: test que compara output C++ vs Rust
#[test]
fn differential_validate() {
    // 1. Obtener programa IR desde C++ (vía FFI o dump)
    let cpp_program: Vec<Stmt> = get_program_from_cpp_translator();

    // 2. Procesar con Rust
    let rust_validation = ir::validate(&cpp_program);

    // 3. Obtener resultado C++ (vía FFI)
    let cpp_result = get_cpp_validation(&cpp_program);

    // 4. Comparar
    assert_eq!(rust_validation.ok, cpp_result.ok);
}
```

## Convenciones Rust

### Estilo

- `cargo clippy -- -D warnings` obligatorio antes de commit
- `cargo fmt` obligatorio
- Rust edition 2021 (bump a 2024 cuando estable)
- `#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]` en todo crate

### Tipos

- `#[repr(u8)]` en enums que cruzan FFI
- `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` en todos
  los tipos IR
- `pub type Ref = u32;` — SSA references son u32
- `#[non_exhaustive]` en enums que pueden crecer (Op, BinOpKind, etc.)

### Performance

- `#[inline]` en hot paths pequeñas (bit_width, mask, etc.)
- `Box<[Stmt]>` para programas grandes (evita `Vec` realloc)
- Evitar `clone()` en hot paths; preferir `Cow<'_, T>` o referencias

### Testing

- `#[cfg(test)] mod tests { ... }` en cada archivo
- `proptest` para property-based tests (ya en workspace deps)
- Tests de round-trip serialización con `bincode` + `serde_json`
- Tests de validator con programas válidos E inválidos

## Cómo coordinar con Gemini

Cuando Gemini revisa código Rust de CODEX o Claude:

1. **Copia de seguridad:** verificar que `cargo test` + `cargo clippy` pasan
2. **Revisión de tipos:** Op enum match exhaustivo? serde derives?
3. **Revisión de unsafe:** cada bloque `unsafe` tiene `// SAFETY:` comment
4. **Revisión FFI:** tipos C-compatibles? `#[repr(C)]`? `no_mangle`?
5. **Differential:** el test contra C++ existe y pasa?

Formato de review: `docs/REVIEWS/2026-<mes>-rust-<crate>.md`

## Checklist de commit

- [ ] `cargo test --workspace` verde (al menos los crates puros)
- [ ] `cargo clippy --workspace -- -D warnings` limpio
- [ ] `cargo fmt --check` pasa
- [ ] Tests differentiales existen y pasan
- [ ] C API bridge documentada en el módulo
- [ ] README del crate actualizado
- [ ] WORK_QUEUE.md actualizado con SHA

## Errores comunes

| Error | Causa | Fix |
|-------|-------|-----|
| `cargo build` fails sin Rust instalado | CI runner sin toolchain | `rustup target add aarch64-unknown-linux-gnu` |
| FFI link error | `PRISMA_CORE_LIB_DIR` no apunta al build | `export PRISMA_CORE_LIB_DIR=$PWD/core/build` |
| Serialization mismatch | C++ y Rust serializan en orden diferente | Ver RFC 0009; usar `bincode` little-endian |
| `proptest` test lento por default | 256 cases por defecto | Usar `#![proptest_config = ProptestConfig::with_cases(64)]` |
| Validate diff C++ vs Rust | Algún tipo IR no sincronizado | Correr script `lean-cross-check` |

## Referencias

- RFC 0015 — Rust Core migration roadmap
- RFC 0014 — C-ABI FFI boundary core↔shell
- RFC 0009 — IR binary format
- `core/include/prisma/ir.hpp` — IR types C++
- `ir-spec/PrismaIR/Syntax.lean` — Lean spec (authoritative)
- `docs/COORDINATION.md` — Agent coordination protocol
- `docs/AGENT_PLAYBOOK.md` — Original agent playbook
