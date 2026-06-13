# HANDOFF: prisma-runtime (Fase 4)

> Para Claude. Cómo migrar el runtime (dispatcher, signals, SMC, syscalls) a Rust.

## Dependencias

- `prisma-ir` (para tipos de datos)
- `prisma-cache` (para lookup/store de traducciones)
- `libc` crate (para syscalls POSIX, mmap, sigaction)
- `nix` crate (para signal handling ergonómico)

## Archivos C++ a migrar

| C++ | Rust | Líneas | Dificultad |
|-----|------|--------|-----------|
| `dispatcher.cpp` (238) | `dispatcher.rs` | 238 | 🟡 Media |
| `signal_handler.cpp` (136) | `signal_handler.rs` | 136 | 🟡 Media |
| `smc_guard.cpp` (239) | `smc_guard.rs` | 239 | 🟡 Media |
| `jit_memory.cpp` (175) | `jit_memory.rs` | 175 | 🟢 Baja |
| `jit_buffer_pool.cpp` (210) | `jit_buffer_pool.rs` | 210 | 🟢 Baja |
| `host_features.cpp` (116) | `host_features.rs` | 116 | 🟢 Baja |
| `syscall_handler.cpp` (660) | `syscall_handler.rs` | 660 | 🔴 Alta |
| `guest_signal.cpp` (52) | `guest_signal.rs` | 52 | 🟢 Baja |

## Dispatcher

```rust
pub struct Dispatcher<'a> {
    translator: &'a mut Translator,
    cache: &'a mut TranslationCache,
    reader: MemReaderFn,
    reader_ctx: *mut c_void,
    halt_pcs: Vec<u64>,
    max_steps: u64,
    stats: DispatchStats,
    return_stack: Vec<u64>,  // RAS
}

impl Dispatcher {
    pub fn run(&mut self, entry_pc: u64) -> RunOutcome;
    pub fn gpr(&self, reg: Gpr) -> u64;
    pub fn set_gpr(&mut self, reg: Gpr, value: u64);
    pub fn guest_pc(&self) -> u64;
}

pub struct DispatchStats {
    pub blocks_executed: u64,
    pub steps_taken: u64,
    pub unique_pcs_seen: u64,
    pub ras_hits: u64,
    pub ras_misses: u64,
    pub direct_thread_hits: u64,
    pub direct_thread_misses: u64,
    pub jit_patch_attempts: u64,
    pub jit_patch_applies: u64,
    pub jit_patch_rejects: u64,
    pub jit_patch_unpatches: u64,
    pub jit_patch_executes: u64,
}
```

## Signal Handler

```rust
pub struct SignalHandler;

impl SignalHandler {
    pub fn install() -> Result<()>;
    pub fn scoped_protected<F>(f: F) -> Result<(), FaultKind>
    where F: FnOnce();
}

pub enum FaultKind {
    None, Segv, Ill, Bus, Unknown,
}
```

Estrategia: usar `sigaction` vía `nix` crate + `thread_local!` para
el jmp_buf. Alternativa: `std::panic::catch_unwind` + signal handler
que convierte señales en panics.

```rust
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, Signal};

pub fn install() -> Result<()> {
    let handler = SigHandler::Handler(handle_signal);
    let sa = SigAction::new(handler, SaFlags::SA_SIGINFO, SigSet::empty());
    unsafe {
        sigaction(Signal::SIGSEGV, &sa)?;
        sigaction(Signal::SIGILL, &sa)?;
        sigaction(Signal::SIGBUS, &sa)?;
    }
    Ok(())
}
```

## SmcGuard

```rust
pub struct SmcGuard {
    protected_pages: Mutex<HashMap<u64, PageEntry>>,
    pending: Mutex<Vec<u64>>,
}

pub struct PageEntry {
    keys: Vec<u64>,
    dead: bool,
}

impl SmcGuard {
    pub fn on_translate(&self, addr: u64, len: u32, cache_key: u64);
    pub fn on_invalidate(&self, cache_key: u64);
    pub fn handle_fault(&self, fault_addr: *mut c_void) -> bool;
    pub fn drain_pending(&self, callback: impl FnMut(u64));
    pub fn tracked_page_count(&self) -> usize;
    pub fn is_tracked(&self, addr: u64) -> bool;
}
```

## Syscall handler

```rust
pub fn handle_syscall(number: u32, args: &[u64; 6]) -> Result<u64, SyscallError>;

pub enum SyscallError {
    Unsupported(u32),
    InvalidAddress,
    PermissionDenied,
    Interrupted,
}
```

Mapear las syscalls x86 más comunes:
- read/write/open/close → Rust `std::fs` / `std::io`
- mmap/munmap/mprotect → `libc::mmap` etc.
- exit/exit_group → `std::process::exit`
- brk → gestión de heap
- sigaction/rt_sigaction → signal handling
- arch_prctl (ARCH_SET_FS/GS) → TLS management

## Checklist

- [ ] `Dispatcher::run()` implementa fetch → translate → execute loop
- [ ] Halt PCs y step budget funcionan
- [ ] `SignalHandler::install()` registra SIGSEGV/SIGILL/SIGBUS
- [ ] `ScopedProtected` pattern funciona con catch_unwind
- [ ] `SmcGuard::handle_fault()` tombstones page y retorna true/false
- [ ] `drain_pending()` invoca callback por cada cache_key
- [ ] `JitMemory` soporta mmap (Linux), MAP_JIT (macOS), VirtualAlloc (Windows)
- [ ] `JitBufferPool` recicla buffers
- [ ] `HostFeatures` detecta FEAT_LSE, FEAT_SHA*, FEAT_AES, etc.
- [ ] Syscall handler cubre 30+ syscalls comunes
- [ ] Tests: signal recovery, SMC edge cases, syscall dispatch
- [ ] `cargo test --package prisma-runtime` verde (en Linux/macOS)

## SPARK audit 2026-06-12: runtime/cache integration gap

Scope reviewed:

- `shell/prisma-cache/src/cache.rs`, `compress.rs`, `sha256.rs`
- `shell/prisma-runtime/src/dispatcher.rs`, `smc_guard.rs`, `jit_*`, `signal_handler.rs`, `syscall_handler.rs`, `guest_signal.rs`
- Current handoff docs and work queue entries for runtime/cache

Findings:

- Cache persistence is no longer just scaffold. `TranslationCache` now has deterministic FNV-1a content keys, live-entry filtering, binary save/load, optional zstd payload compression, async save, and unit coverage for missing-file preservation plus superseded-entry persistence.
- Runtime still has scaffold-level execution only. `Dispatcher` now has cache probe/install contracts plus a no-execute state machine for guest-byte fetch, cache probe, translation callback, cache install, SMC registration, and pending invalidation application. `GuestFetcher`/`GuestTranslator` are the stable trait boundary, and `RustSmokeTranslator` wires Rust decoder -> backend for NOP, `mov rax, imm64`, `mov rax, rcx`, `add/sub/and/or/xor rax, rcx`, and `add/or/adc/sbb/and/sub/xor/cmp rax, imm8`.
- `SmcGuard` now tracks page-to-cache-key state and exposes drainable pending invalidation keys/pages. `dispatcher::apply_smc_invalidations` drains pending pages and applies them to `TranslationCache::invalidate_page`.
- Targeted unit tests are green on the repository Windows/MSVC gate: `cargo +1.95.0-x86_64-pc-windows-msvc test -p prisma-cache` passes 8 tests; `cargo +1.95.0-x86_64-pc-windows-msvc test -p prisma-runtime` passes 20 unit tests plus the runtime `smoke_differential` integration fixture after the dispatcher cache probe/install helpers, no-execute run state machine, stable fetch/translate traits, Rust decoder/backend smoke translator, `SmcGuard` invalidation state, and cache-application adapter landed. `cargo +1.95.0-x86_64-pc-windows-msvc test -p prisma-core --test smoke_differential` also passes and uses the live C++ FFI translator against the C++-accepted smoke subset.
- `stable-x86_64-pc-windows-msvc` may still fail when starting doctests because that local toolchain reports `rustdoc.exe` is not applicable. Use `+1.95.0-x86_64-pc-windows-msvc`, matching `scripts/validate-rust-workspace.ps1`, for the project gate.

Completed bounded task:

`shell/prisma-runtime/src/dispatcher.rs` now wraps `prisma_cache::TranslationCache::lookup` behind `probe_cache(cache, guest_addr, guest_bytes) -> DispatchCacheProbe`. Runtime unit tests cover:

1. cache hit returns code bytes and records the hit path;
2. unknown address maps to a runtime-visible miss reason;
3. stale guest bytes map to a runtime-visible stale reason;
4. save/load through `TranslationCache` still produces a hit when used through the runtime probe.

This stayed test-only/adapter-level: no backend/decoder call, no real translation, and no cache format change.

Completed bounded task:

`shell/prisma-runtime/src/smc_guard.rs` now tracks translated ranges by guest page, invalidates the page containing a fault/write address, exposes `drain_pending()`, and covers disabled tracking, adjacent-page selectivity, cross-page ranges, and repeated empty drains.

Completed bounded task:

`shell/prisma-runtime/src/dispatcher.rs` now exposes `apply_smc_invalidations(cache, guard)`, which drains invalidated guest pages from `SmcGuard` and calls `TranslationCache::invalidate_page` for each page. Runtime tests cover two cached addresses on separate pages, one SMC fault, one page invalidated, the other cache hit preserved, and repeated application as a no-op.

Completed bounded task:

`shell/prisma-runtime/src/dispatcher.rs` now exposes `install_translation(cache, guard, guest_addr, guest_bytes, code_bytes)`, which upserts the translation cache and registers the translated range with `SmcGuard`. Runtime tests tie cache insertion, SMC registration, `probe_cache`, fault handling, `apply_smc_invalidations`, and post-invalidation cache miss behavior together.

Completed bounded task:

`shell/prisma-runtime/src/dispatcher.rs` now exposes `Dispatcher::run_with_callbacks`, which applies pending SMC invalidations, fetches guest bytes, probes cache, calls an injected translator on miss, installs translated bytes, records SMC tracking, and returns a typed `DispatchRunOutcome`. Runtime tests cover install-on-miss, cache-hit without translator invocation, SMC invalidation before probe, fetch failure, translate failure, and step-limit behavior.

Completed bounded task:

`shell/prisma-runtime/src/dispatcher.rs` now defines `GuestFetcher` and `GuestTranslator`. `Dispatcher::run_with_adapters` uses those traits, while `run_with_callbacks` remains a closure-friendly wrapper. `RustSmokeTranslator` decodes one x86 instruction through `prisma-decoder`, wraps the resulting statements in a one-block `prisma-ir::Function`, lowers through `prisma-backend::Lowerer`, and returns little-endian backend bytes. Tests cover trait-based dispatch, NOP, `mov rax, imm64`, `mov rax, rcx`, `add/sub/and/or/xor rax, rcx`, and `add/or/adc/sbb/and/sub/xor/cmp rax, imm8` through the Rust decoder -> backend contract without JIT execution.

Completed bounded task:

`shell/prisma-runtime/tests/smoke_differential.rs` now records byte fixtures for NOP, `mov rax, imm64`, `mov rax, rcx`, `add/sub/and/or/xor rax, rcx`, full `83 /0..7` `rax` imm8 coverage, `add rbx, imm8`, REX.B `cmp r11, imm8`, base-memory `add/cmp/or/and/xor [rbx], imm8`, SIB+disp8 `add [rax + rcx*4 + disp8], imm8`, disp32 `add [rbx + disp32], imm8`, REX.X/B SIB `add [r8 + r9*4 + disp8], imm8`, and RIP-relative `add/or/and/xor [rip + disp32], imm8`, and checks the Rust smoke translator's backend bytes for each fixture. `RustSmokeTranslator` now passes the fixture `guest_pc` into `prisma_decoder::decode_one_at` so RIP-relative address materialization matches the dispatch PC. `shell/core/tests/smoke_differential.rs` consumes the same smoke fixture subset through `prisma_core::Translator::translate`, proving the live C++ core accepts/consumes those one-instruction inputs and caches repeated input. This is a live translation-metadata comparator, not yet a C++ backend byte comparator, because `capi.h` does not expose emitted backend bytes. The C++ decoder gap for `83 /0..7` is closed for the register-direct `rax` smoke set plus RBX/R11 REX.B, `[rbx]` ADD/CMP/logical memory, SIB+disp8 ADD/CMP, disp32, REX.X/B SIB, RIP-relative ADD/OR/AND/XOR smoke probes, RIP-relative CMP decoder coverage, and negative disp8/imm8 sign-extension; ADC/SBB are placeholders matching existing C++ behavior.

## SPARK 2026-06-13 (claude): real JIT memory + host features + syscall layer

Bounded runtime slices landed on `claude/rust-passes-pipeline`:

- `host_features.rs` — modelo ARM64 real (11 FEAT_*: LSE/LSE2/LRCPC/LRCPC2/
  FlagM/FlagM2/DotProd/CRC32/SHA1/SHA256/AES) + detección HWCAP Linux/aarch64
  (`getauxval`) + singleton cacheado + override de test. Reemplaza el shim
  sse2/avx2 (que era modelo x86 equivocado).
- `syscall_handler.rs` — `SyscallError` + `SyscallClass` + `classify()`/
  `is_known()` sobre el ABI x86-64 syscall_64.tbl + `dispatch()` con frontera
  tipada (deny-by-default preservado).
- `jit_memory.rs` — **`ExecBuffer` real W^X**: VirtualAlloc (Windows, FFI
  kernel32 cruda, sin nueva dependencia) / mmap (libc, unix), write, flip a
  R+X (VirtualProtect/mprotect), invalidate icache. W^X enforced. Probado
  ejecutando código JIT real en el host: `jit_and_execute_returns_42` y
  `exec_pool_installs_and_runs_two_blocks` escriben x86-64 (`mov eax,N; ret`),
  flip executable, transmute a `extern "C" fn()->u32`, lo LLAMAN. `ExecPool`
  espeja `JitBufferPool::add` del C++.
- `jit_buffer_pool.rs` — `encode_aarch64_b` (encoder del B imm26 para direct-
  threading, ±128 MiB, 4-byte align), parte testeable del `patch_aarch64_branch`
  C++.

Docker/QEMU: el C++ core compila y pasa los 1118 test cases en Linux/clang-18
(`scripts/docker-test-core.sh`); arm64 e2e (ejecución real x86->ARM64 JIT) via
`docker run --platform linux/arm64` con `docker/Dockerfile.arm64-build`
(`-DCMAKE_CXX_SCAN_FOR_MODULES=OFF` porque el cmake del wheel pip activa el
escaneo de módulos C++20 y falta clang-scan-deps).

VALIDADO en aarch64 (bajo QEMU, imagen `rust:1-slim-bookworm`): el path de
ejecución JIT ARM64 funciona — mmap RW + write `mov w0,#42; ret`
(40 05 80 52 / C0 03 5F D6) + mprotect RX + secuencia `dc cvau`/`ic ivau`/
barreras + transmute a fn + LLAMADA => devuelve 42. Confirma que el fix del
BLOCKER de I-cache es correcto en hardware ARM64 real (el `jit_and_execute_
returns_42_aarch64` de jit_memory.rs hace lo mismo via ExecBuffer cuando la
suite corre en arm64).

Pendiente runtime: signal_handler (POSIX sigaction / Windows VEH — divergente),
wire ExecPool al dispatcher para ejecutar bloques traducidos reales (necesita
host ARM64 porque el backend emite ARM64), validar ExecBuffer path POSIX en
contenedor Linux con cargo.

Next bounded implementation task:

Design a versioned C ABI extension if byte-for-byte C++ backend emission is needed, or broaden decoder/runtime smoke coverage beyond Group 1 memory forms. The backend now has minimal `LoadReg`/`StoreReg` lowering against `CpuStateFrame::gpr[]` plus register-register `Add/Sub/And/Or/Xor`, immediate `Add/Sub`, `LoadMem`/`StoreMem`, and `CmpFlags` lowering, so new work should avoid redoing the existing Group 1 `rax`, RBX, R11 REX.B, `[rbx]`, SIB+disp8, disp32, REX.X/B SIB, RIP-relative, and negative disp8/imm8 probes.
