# HANDOFF: high-memory host migration

Last updated: 2026-08-17 America/Mexico_City.

## Authoritative checkout

- Remote: `https://github.com/Finithe-Phoenix/prisma.git`
- Branch: `codex/real-execution`
- Remote migration checkpoint: `49a301057c3810b8a6179534cc6df39a875c9084`
- Active backlog claim: `F3-WN-019`
- Wine submodule: `1012f3d99507b80d4869eabf0853567660a7ecbb`

Clone into the native Linux filesystem of Ubuntu 22.04 under WSL2:

```bash
git clone --branch codex/real-execution --recurse-submodules \
  https://github.com/Finithe-Phoenix/prisma.git ~/prisma
cd ~/prisma
git status --short --branch
```

Do not build from the Windows checkout or under `/mnt/c`. The runtime's real
JIT, signal, syscall, and mmap paths are POSIX-only.

## Current execution checkpoint

The remaining Phase 1 target is real `oh-my-posh.exe version` execution through
Wine ARM64EC and Prisma with captured stdout/stderr and exit code zero.

The last focused CI evidence established:

- Wine bootstrap completes and the Prisma `xtajit64` provider loads.
- Provider process and thread initialization callbacks enter and return.
- `BeginSimulation` is not reached.
- Execution then fails with a null execute access and fixture exit code 5.

The gap is now isolated. Disassembly of the locally produced Wine runtime shows
that ARM64EC `RtlUserThreadStart` dispatches the native ARM64
`BaseThreadInitThunk`, whose body then invokes the x64 entrypoint with a direct
`blr x1`. That inner call bypasses `__os_arm64x_dispatch_icall`, so Prisma never
receives `BeginSimulation`. The versioned Wine patch now dispatches the x64
entrypoint itself and then calls `RtlExitUserThread`; the Wine Dockerfile rejects
the previous compiled thunk by inspecting `RtlUserThreadStart` disassembly.

The patch applies cleanly to pinned Wine 11.14 and its three lightweight
regression tests pass. The local Wine manifest is now schema v3 and binds both
the Dockerfile and patch SHA-256, so the old v2 runtime cannot be reused as a
cache hit. A fresh Wine build plus the exact three lifecycle cycles remain
required before closing `F3-WN-019`; they were not started below the 6 GiB
physical-memory gate. Do not reintroduce raw fault dumps or broad temporary
instrumentation, and preserve the exact 20-export provider contract.

While the new high-memory host is pending, Danny requested local serial work on
the current machine: no subagents, no remote fan-out and no simultaneous heavy
jobs. The 6 GiB physical-memory gate remains mandatory.

## Migration checkpoint commits

The Windows worktree was separated into atomic migration commits:

- `5fd3cc4` policies, agent skills, CI coverage, and compatibility roadmap;
- `0cb6140` Compose dashboard, terminal demos, tests, and Android launcher;
- `b86290d` Android JNI execution integration;
- `254dfa9` PE loader and WoW64 provider prototypes;
- `38cfa98` optimization and GUI fixture prototypes;
- `0688b55` Wine and ARM64 worker build tooling;
- `7ca79fd` verified Windows application target manifest.
- `1cf1734` this migration handoff and the authoritative resume instructions.
- `49a3010` synchronized agent guidance and the serial local-work checkpoint.
- `24c9bfa` corrected the initial x64 thread-entry dispatch and added regression
  gates.
- `d1fe480` documented the isolated ARM64EC root cause and pending validation.

The complete branch was also exported and verified as:

- file: `prisma-codex-real-execution-20260817-d1fe480.bundle`;
- SHA-256: `6FF255A659EA3498EEFEF9B41C5ACA6B90F147F05F3AD76F7742B1FE66C9B17E`;
- bundle ref: `codex/real-execution` at
  `d1fe480238107c5a8d22c430e916015bc2f17761`.

`cargo metadata --locked` passed. Changed Rust files passed `rustfmt`; the
workspace-wide format check still reports pre-existing whitespace in
`shell/prisma-vortek`. PowerShell AST parsing and Python bytecode compilation
passed for the new worker tooling. A full Gradle/Cargo/Wine build was not run on
the old host because available physical RAM was below the 6 GiB safety gate.

## Deliberately excluded local state

The following are reproducible or machine-private and were not committed:

- CMake, Cargo, Gradle, Kotlin, Android, Wine, and Docker build outputs;
- downloaded Windows installers and the Oh My Posh executable;
- `android/prisma-debug.keystore`;
- temporary test inventories and console captures;
- local Claude/Codex queues, locks, and review scratch files.

Reacquire Windows fixtures from the pinned metadata and hashes under
`tools/windows-apps/`. The Wine source modifications are represented by the
tracked patch under `third_party/wine-prisma/`; the submodule itself is clean.

## First checks on the new host

Before any heavy build, record host and WSL RAM, commit limit, pagefile/swap,
and active Prisma processes. Then run only targeted validation for the touched
component. Keep `docs/BACKLOG.md` as the claim ledger and preserve deterministic
cleanup of every process, pipe, mapping, handle, container, and temporary file.
