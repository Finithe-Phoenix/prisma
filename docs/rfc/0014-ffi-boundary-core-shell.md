---
id: 0014
title: C-ABI FFI boundary between the C++ core and the Rust shell
status: accepted
authors: [Danny, Claude]
created: 2026-06-09
updated: 2026-06-09
supersedes: []
superseded_by: null
---

# RFC 0014: C-ABI FFI boundary between the C++ core and the Rust shell

## Summary

Define the contract that lets the Rust `shell/` workspace drive the C++20
DBT core: a pure C ABI exported from a new `prisma_core_c` shared library,
consumed by a hand-written Rust `-sys` crate plus a safe wrapper crate. No
new third-party bridge dependencies (`cxx`, `bindgen`, Corrosion) in the
first cycle — the boundary is small enough to maintain by hand, and every
dependency we do not take is one we do not have to audit, version, and
justify.

This RFC fixes the rules of the boundary, not the full function list. The
authoritative surface is `core/include/prisma/capi.h`; this document
constrains how that header is allowed to grow.

## Motivation

The project plan splits responsibilities deliberately (see
`shell/README.md`): Rust owns I/O, parsing of untrusted input, networking,
and crypto; C++ owns the DBT core, which is unsafe by nature
(`mmap(W|X)`, SMC backpatching, JIT execution). Both sides exist today,
but there is no bridge: the Rust orchestrator cannot hand a loaded PE
image to the dispatcher, cannot drive the translator for fuzzing, and
cannot reuse the core's SHA-256/cache machinery for the Pillar 4 trust
envelope.

The first consumer is the hybrid milestone "PE loader feeds the DBT":
`shell/orchestrator`'s `pe_loader` parses a minimal x86-64 PE in safe
Rust, maps the image, and the C++ dispatcher translates and executes it.
Subsequent consumers: property-based fuzzing of the decoder from Rust
(proptest), and the persistent-cache signing pipeline.

## Design

### Boundary shape

- One C header: `core/include/prisma/capi.h`. C99-compatible, no C++
  constructs, includes only `<stdint.h>`/`<stddef.h>`/`<stdbool.h>`.
- One CMake target: `prisma_core_c`, a **shared** library that links the
  existing static `prisma_*` libraries (and vixl/zstd transitively). A
  shared library keeps the Rust link line trivial (`-lprisma_core_c`
  plus an rpath) and carries `libstdc++` with it; it is also the shape
  Android will need (JNI loads a `.so`). The core's static targets gain
  `POSITION_INDEPENDENT_CODE` to make this linkable.
- Rust side, two crates in the existing workspace:
  - `shell/core-sys` (`prisma-core-sys`): `#![no_std]`-style raw
    `extern "C"` declarations mirroring `capi.h` one-to-one, plus a
    `build.rs` that emits the link directives. All `unsafe` of the
    boundary is confined here and in the wrapper's call sites.
  - `shell/core` (`prisma-core`): safe RAII wrapper. Handles are owned
    types with `Drop`; fallible calls return `Result<_, CoreError>`;
    borrowed guest memory is expressed as lifetimes so a buffer cannot
    be freed while a dispatcher can still fetch from it.

### Contract rules

1. **Naming and linkage.** Every exported symbol starts with `prisma_`
   and is declared `extern "C"`. The header carries
   `PRISMA_CAPI_VERSION` (monotonically increasing integer) and the
   library exports `prisma_capi_version(void)`; callers verify at
   startup and refuse to run on mismatch.
2. **Opaque handles.** C++ objects cross as opaque pointers
   (`prisma_translator`, `prisma_dispatcher`). Layout is never exposed.
   Every handle has a `_create` / `_destroy` pair; whoever creates,
   destroys. Destroy functions accept NULL as a no-op.
3. **Error model.** Fallible functions return `prisma_status` (an enum;
   `PRISMA_OK == 0`). Out-parameters are written only on `PRISMA_OK`.
   No C++ exception may cross the boundary: every export wraps its body
   in a catch-all that maps to `PRISMA_STATUS_INTERNAL`. Symmetrically,
   no Rust panic may cross: callback trampolines use `catch_unwind` and
   convert a panic into the callback's failure value.
4. **Callbacks.** Host-provided callbacks are C function pointers with a
   `void* ctx` first argument. The guest memory reader follows the
   dispatcher's existing semantics: returning length 0 means "no memory
   here" and surfaces as a fetch fault — which is exactly what a Rust
   panic converts to.
5. **Structs by value.** POD result structs (`prisma_block_info`,
   `prisma_run_result`, stats) are fixed-layout, contain only integer
   types and fixed-size char arrays, and may only grow by appending in
   tandem with a `PRISMA_CAPI_VERSION` bump. Human-readable context
   travels in a fixed `char message[128]` (truncated, always
   NUL-terminated) — no cross-boundary string ownership.
6. **Threading.** Handles are not thread-safe; a handle and everything
   reachable from it must be used from one thread at a time. This
   matches the C++ objects' current guarantees.
7. **Memory.** The library never frees caller memory and never returns
   ownership of internal memory. Pointers returned inside result structs
   (e.g. JIT code entry) are borrows tied to the owning handle's
   lifetime, documented per field.

### Build and CI integration

Builds stay separate — CMake is not taught about Cargo and vice versa:

```
cmake --build core/build --target prisma_core_c
PRISMA_CORE_LIB_DIR=$PWD/core/build \
  cargo test --manifest-path shell/Cargo.toml -p prisma-core-sys -p prisma-core
```

`core-sys`'s `build.rs` reads `PRISMA_CORE_LIB_DIR`, emits
`rustc-link-search` + `rustc-link-lib=dylib=prisma_core_c`, and an rpath
link-arg for test binaries. If the variable is unset the bridge crates
fail fast at build time with an actionable message (the rest of the
workspace builds normally; `cargo build` without the core stays green
for `prisma_orchestrator`).

CI gains an `ffi-link` job (`.github/workflows/ffi-bridge.yml`) that
builds `prisma_core_c` on `ubuntu-latest`, then runs clippy + tests for
the bridge crates against it. JIT execution assertions are gated on
`cfg!(target_arch = "aarch64")` exactly like the C++ e2e corpus gates on
`__aarch64__`, so the job validates translation and the boundary
mechanics everywhere and full execution on ARM64 runners when available.

## Alternatives considered

- **`cxx` crate.** Strong type safety, but it couples the two build
  systems (cargo must compile C++), adds a proc-macro + codegen
  dependency to audit, and its idioms (shared structs, `UniquePtr`)
  leak C++ shapes into Rust. Our boundary is a handful of handle types;
  hand-written C is smaller than the tooling.
- **`bindgen`.** Generates the `-sys` layer from the header, but drags
  libclang into every build environment. Worth revisiting if `capi.h`
  grows past a few hundred lines; at the current size, drift is caught
  by the integration tests (a signature mismatch fails the `ffi-link`
  job immediately).
- **Corrosion (CMake↔Cargo).** Convenient single-command builds, but
  couples the build graphs and adds a CMake-module dependency. The
  two-command workflow above is explicit and CI-friendly; revisit when
  the Android build (Gradle + NDK) forces unified orchestration anyway.
- **Static `prisma_core_c`.** Avoids PIC, but pushes the whole
  transitive link line (seven static libs + vixl + zstd + libstdc++)
  into every Rust consumer's `build.rs`, which is exactly the kind of
  fragile duplication this RFC exists to avoid.

## Consequences

- The core's static libraries compile as PIC. Negligible cost on
  AArch64 (PC-relative addressing is the default idiom) and acceptable
  on x86-64 test hosts.
- The C API is a second public surface that must evolve in lockstep
  with the C++ facade. The integration tests in `shell/core` are the
  tripwire; `PRISMA_CAPI_VERSION` is the escape hatch.
- `unsafe` in the Rust workspace stays auditable: it is legal only in
  `core-sys` and in `shell/core`'s FFI call sites. Clippy keeps
  `unsafe_op_in_unsafe_fn = deny` across the workspace.
- Fase 3's JNI bridge gets its `.so` for free: `prisma_core_c` is
  already the artifact the Android loader will ship.
