# shell — Prisma Rust Workspace

**Lenguaje:** Rust 1.82+ (edition 2024 cuando estable).
**Build:** Cargo workspace multi-crate.
**Target:** `.so` compilado para Android ARM64, consumido por la app Kotlin vía JNI.

## Workspace crates

| Crate | Estatus | Descripción |
|-------|---------|-------------|
| `orchestrator` | ✅ Existente | Container lifecycle, PE loader, config, integrity |
| `core-sys` | ✅ Existente | Raw FFI bindings a `libprisma_core_c` (C ABI) |
| `core` | ✅ Existente | Safe Rust wrapper sobre C API |
| **`prisma-ir`** | ✅ Fase 0 | IR types (Op enum, Stmt, Function, serde) |
| `prisma-cache` | 🚧 Fase 1 | Translation cache LRU + zstd + SHA-256 |
| `prisma-passes` | 🚧 Fase 2 | 14 optimization passes pipeline |
| `prisma-decoder` | 🚧 Fase 3 | x86_64 → IR decoder |
| `prisma-runtime` | 🚧 Fase 4 | Dispatcher, signals, SMC, syscalls |
| `prisma-backend` | 🚧 Fase 5 | ARM64 assembler + IR lowering |

**Roadmap completo:** RFC 0015 (`docs/rfc/0015-rust-migration-roadmap.md`)

## Responsabilidad

Core DBT engine (unsafe — mmap, backpatching, SMC) → plan de migrar
a Rust incrementalmente (RFC 0015). Shell (networking, containers,
config, crypto) ya está en Rust.

## Build

```bash
# Solo crates puros (sin FFI)
cargo build --manifest-path shell/Cargo.toml

# Con FFI bridge (requiere PRISMA_CORE_LIB_DIR)
cmake --build core/build --target prisma_core_c
PRISMA_CORE_LIB_DIR=$PWD/core/build \
  cargo test --manifest-path shell/Cargo.toml --workspace

# Un crate específico
cargo test --manifest-path shell/Cargo.toml --package prisma-ir
```

## Testing

- `cargo test` — unit tests + property-based tests
- `proptest` — fuzzing de decoder y passes
- Differential tests (C++ vs Rust) — ver `docs/DIFFERENTIAL_TESTING.md`
- FFI integration — ver `shell/core/tests/integration.rs`

## Dependencias principales

- `serde` + `bincode` — serialización del IR y cache
- `sha2` + `zstd` — integridad + compresión
- `proptest` — property-based testing
- `libc` — FFI a syscalls POSIX (Fase 4)
- `tokio` (futuro) — async para cache I/O y networking
- `reqwest` + `rustls` (futuro) — HTTPS downloads
- `libp2p` (futuro) — P2P translation cache

## Convenciones

Ver `docs/RUST_MIGRATION_PLAYBOOK.md` para el protocolo completo
de trabajo entre CODEX y Claude.
