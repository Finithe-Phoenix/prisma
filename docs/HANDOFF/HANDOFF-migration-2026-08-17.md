# HANDOFF: high-memory host migration

Last updated: 2026-08-17 America/Mexico_City.

## Authoritative checkout

- Remote: `https://github.com/Finithe-Phoenix/prisma.git`
- Branch: `codex/real-execution`
- Remote migration checkpoint: `1cf1734f383f0f4e97aee5adfb999015428f408c`
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

Therefore the next diagnosis starts between `ThreadInit` returning and Wine
calling `BeginSimulation`. Do not reintroduce raw fault dumps or broad temporary
instrumentation; use the existing symbolic phase markers and preserve the exact
20-export provider contract.

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

The complete branch was also exported and verified as:

- file: `prisma-codex-real-execution-20260817-1cf1734f383f.bundle`;
- SHA-256: `E27D7F28C6B4B05B250B94624D08CC16307CB63EFE6970DA21AF618445F8B4F2`;
- bundle ref: `codex/real-execution` at `1cf1734f383f0f4e97aee5adfb999015428f408c`.

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
