# Prisma xtajit64 lifecycle provider

This crate is the architecture-correct provider surface for running AMD64
Windows code through Wine 11.14 on an ARM64EC host. Wine loads
`C:\windows\system32\xtajit64.dll` from its ARM64EC `ntdll`; the older
`wow64cpu.dll` / `xtajit.dll` contract is for 32-bit x86 guests and cannot run
the pinned AMD64 Oh My Posh fixture.

F3-WN-003 implements the loadable export surface. F3-WN-004 implements the
current Wine 11.14 process/thread lifecycle handshake. Every callback
probed by `third_party/wine/dlls/ntdll/signal_arm64ec.c` is exported with the
corresponding `WINAPI` signature, along with the three transition thunks:

- `BTCpu64FlushInstructionCache`
- `BTCpu64IsProcessorFeaturePresent`
- `BTCpu64NotifyMemoryDirty`
- `BTCpu64NotifyReadFile`
- `BeginSimulation`
- `FlushInstructionCacheHeavy`
- `NotifyMapViewOfSection`
- `NotifyMemoryAlloc`
- `NotifyMemoryFree`
- `NotifyMemoryProtect`
- `NotifyUnmapViewOfSection`
- `ProcessInit`
- `ProcessTerm`
- `ResetToConsistentState`
- `ThreadInit`
- `ThreadTerm`
- `UpdateProcessorInformation`
- `ExitToX64`
- `DispatchJump`
- `RetToEntryThunk`

`ProcessInit()` and `ThreadInit()` use the current `xtajit64` signatures, not
the older `pBTCpuProcessInit` / `pBTCpuThreadInit` WOW64 names. Initialization
is idempotent, each thread owns one generation-tagged context, and the two-phase
`ProcessTerm(HANDLE, BOOL, NTSTATUS)` callback clears all contexts and mapping
ownership before process termination. A failed post-call returns the provider
to an initialized but resource-empty state so it can recover honestly.

`BeginSimulation` now executes real Prisma-translated blocks against Wine's
complete AMD64-compatible context. The bridge generates its dense 264-entry
Win64 syscall table from the pinned Wine 11.14 source, marshals R10/RDX/R8/R9
and stack arguments, and resolves the corresponding native `Nt*` export.

The ARM64EC transition thunks are non-returning ABI bridges. They capture the
hybrid context, transfer x64 calls and tail jumps into the Prisma loop, keep a
per-thread LIFO stack for nested native returns, and use Wine's `NtContinue`
boundary to restore native ARM64EC state. Thread and process teardown drop the
translation runtime and every pending native-return frame deterministically.

This is an implemented and link-audited provider, not yet a compatibility
claim. F3-WN-019 remains the acceptance gate: the DLL must be installed into
the ARM64 Wine prefix and run the pinned Oh My Posh fixture on Android with
real exit code and captured output.

Host-side lifecycle tests run with:

```powershell
cargo test --manifest-path shell\Cargo.toml -p prisma-xtajit64
```

The ARM64EC DLL build and export audit run with:

```powershell
& .\scripts\build-prisma-xtajit64.ps1
```

That command requires the Rust `arm64ec-pc-windows-msvc` standard library and
Visual Studio 2022's ARM64/ARM64EC C++ build tools. It never substitutes an
AMD64 DLL when those prerequisites are absent.
