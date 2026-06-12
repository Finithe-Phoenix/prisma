# Differential Testing: C++ ↔ Rust

> Cómo verificar que la migración a Rust no cambia la semántica.
> Cada fase debe pasar estos tests antes de declararse completa.

## Principio

Para cada componente migrado, probamos que el mismo input produce
el mismo output en ambas implementaciones. El input puede ser IR,
bytes x86, o un programa completo.

## Arquitectura

```
Input (IR program / x86 bytes / config)
       │
       ├──→ C++ implementation ──→ C++ output
       │
       └──→ Rust implementation ──→ Rust output
                                    │
                              Compare(C++ output, Rust output)
                                    │
                              assert_eq!(c++, rust)
```

## Por fase

### Fase 0 — IR types (prisma-ir)

- **Qué comparar:** `validate()`, `pretty_print()`, serialization
- **Input:** Programa IR aleatorio (desde `random_program()` en C++ o proptest en Rust)
- **Mecanismo:** Ambos reciben el mismo `Vec<Stmt>`, se compara output de validator y pretty-printer
- **Test existente en C++:** `test_property.cpp`
- **Test en Rust:** `tests/differential_ir.rs`

### Fase 1 — Cache (prisma-cache)

- **Qué comparar:** `lookup()`, `store()`, save/load round-trip
- **Input:** Cache keys + code bytes arbitrarios
- **Mecanismo:** Mismas operaciones en ambos, comparar estado (hit_count, LRU order, entries)
- **Bridge:** `extern "C"` en Rust expone `prisma_cache_*` que el C++ existente puede consumir

### Fase 2 — Passes (prisma-passes)

- **Qué comparar:** Pipeline output (IR después de todos los passes)
- **Input:** Programa IR aleatorio (desde proptest)
- **Mecanismo:** C++ `passes::default_pipeline().run(program)` vs Rust `pipeline::default().run(program)`
- **Bridge:** `extern "C" fn prisma_run_passes(stmts: *const Stmt, len: u32) -> *mut Stmt`

### Fase 3 — Decoder (prisma-decoder)

- **Qué comparar:** IR output del decoder
- **Input:** Bytes x86 (desde harness de fuzzing)
- **Mecanismo:** C++ `decode_one(bytes)` vs Rust `decode::decode_one(bytes, 0)`
- **Especial:** Algunos opcodes producen Ref IDs distintos. Comparar estructura del IR
  (tipos de ops, operandos), no los Ref values exactos

### Fase 4 — Runtime (prisma-runtime)

- **Qué comparar:** HostFeatures, SMC guard state, signal recovery
- **Input:** Configuraciones sintéticas + secuencias de fault
- **Mecanismo:** C++ y Rust reciben mismas llamadas API, comparar estado observable

### Fase 5 — Backend (prisma-backend)

- **Qué comparar:** Output del assembler (bytes ARM64) y lowering (IR → ARM64)
- **Input:** Programa IR
- **Mecanismo:** `emitter.finalize()` vs `assembler.finalize()`
  Comparar bytes producidos; pueden diferir en registro asignado,
  comparar semántica (ejecutar ambos y verificar estado final)

## Cómo escribir un test differential

```rust
use prisma_ir::{Stmt, Op, Constant, OpSize};

/// Ejemplo: verificar que Rust y C++ validan igual un programa
#[test]
fn differential_validate() {
    // Arrange: programa IR conocido
    let program = vec![
        Stmt::new(Some(0), Op::Constant(Constant { value: 42, size: OpSize::I64 })),
    ];

    // Act (Rust): validar con Rust
    let rust_ok = true; // TODO: llamar prisma_ir::validate(&program)

    // Act (C++): validar con C++ (vía FFI)
    // TODO: llamar extern "C" fn cpp_validate(stmts, len) -> bool
    // let cpp_ok = unsafe { cpp_validate(program.as_ptr(), program.len() as u32) };

    // Assert
    assert!(rust_ok); // TODO: assert_eq!(rust_ok, cpp_ok);
}
```

## FFI bridge pattern

```rust
/// C API que C++ puede llamar para validar IR con Rust
#[no_mangle]
pub extern "C" fn prisma_rust_validate(
    stmts: *const Stmt,
    len: u32,
) -> bool {
    let slice = unsafe { std::slice::from_raw_parts(stmts, len as usize) };
    prisma_ir::validate(slice).is_ok()
}
```

## Pipeline CI

Los tests differentiales corren en el workflow `ffi-link` (ya existe).
Requieren `PRISMA_CORE_LIB_DIR` apuntando al build C++.

```yaml
# Fragmento de CI
- name: Differential tests
  run: |
    cmake --build core/build --target prisma_core_c
    PRISMA_CORE_LIB_DIR=$PWD/core/build \
    cargo test --manifest-path shell/Cargo.toml -- differential
```
