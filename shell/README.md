# shell — Prisma Orchestrator (Rust)

**Lenguaje:** Rust 1.75+ (edition 2024 cuando estable).
**Build:** Cargo workspace.
**Target:** `.so` compilado para Android ARM64, consumido por la app Kotlin vía JNI.

## Responsabilidad

Todo lo que NO es DBT y NO es UI:

- **Container lifecycle** — crear, levantar, pausar, eliminar contenedores Wine.
- **Overlay filesystem** — FUSE-based o custom, base RO + overlay RW por contenedor.
- **Configuration management** — TOML/YAML por contenedor, validación.
- **Component downloader + verifier** — descarga Wine/DXVK/VKD3D bundles con sha256 + firma.
- **Translation cache networking** — cliente del servidor CDN + P2P (Pilar 4).
- **Bridging a `../core/` (C++ DBT)** — FFI C-ABI.
- **Bridging a Android (Kotlin)** — vía `jni` crate.

## Por qué Rust aquí (y no en el core)

Rust brilla en I/O + networking + parsing + verificación criptográfica, todo con garantías de memory safety que NO implican unsafe ubicuo. El core DBT es el opuesto: unsafe por naturaleza (`mmap(W|X)`, backpatching, SMC), así que queda mejor en C++20.

Este split es el mismo modelo que Firefox usa (Servo/Gecko) adaptado a mobile emulation.

## Dependencias principales (futuro)

- `tokio` — async runtime.
- `reqwest` + `rustls` — HTTPS + verificación de certs.
- `sha2` + `ed25519-dalek` — integrity checking + cache signature verification.
- `jni` — bridge a JVM.
- `serde` + `toml` — config.
- `libp2p` — P2P translation cache (Pilar 4).
- `zstd` — compresión de caches.

## No existe aún

Fase 3 (semanas 93-104) arranca el container system. Antes de eso, semana 3-4 de Fase 0 instalará el toolchain Rust pero sin código real.
