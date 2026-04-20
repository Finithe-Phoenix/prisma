---
id: 0002
title: Integrate vixl as the ARM64 emitter via FetchContent + custom wrapper
status: accepted
authors: [Danny]
created: 2026-04-19
updated: 2026-04-19
supersedes: []
superseded_by: null
---

# RFC 0002: Integrate vixl as the ARM64 emitter via FetchContent + custom wrapper

## Summary

Prisma fetches vixl from the Linaro GitHub mirror at a pinned commit and
builds the aarch64 subset of it through a CMake wrapper we maintain ourselves
(`cmake/BuildVixl.cmake`). No SCons, no submodules. A pimpl-wrapped
`prisma::backend::Emitter` exposes the subset of the MacroAssembler that
Prisma needs, hiding vixl headers from library consumers.

## Motivation

RFC 0001 established that Prisma uses an SSA-based IR. The backend that
turns that IR into ARM64 machine code needs an emitter. Research
(`docs/research_notes.md` — vixl scan) validated vixl as the right choice:
BSD-3-Clause, covers SVE2/LSE2/LRCPC2 (critical for Pillars 1/3), mantained
by Linaro/ARM, ships its own simulator + disassembler.

Writing a custom emitter would cost 12-18 months for commodity work. vixl
integration is ~1 day of wrapper work. The time saved goes into IR + TSO
adaptive + NPU integration — the places Prisma is actually novel.

## Context

### Constraints

- vixl ships a SCons build, not CMake. We need a CMake path.
- vixl targets C++14. Prisma is C++20. The compilers must agree but each
  component can live at its preferred standard.
- Prisma builds with `-Werror` + aggressive warnings. vixl does not. Mixing
  requires care.
- We want reproducible builds: vixl version must be pinned, never "latest".
- CI runs on Ubuntu x86_64; developer machine is macOS arm64; future self-
  hosted runner is ARM64 Linux. The integration must work on all three.

### Prior art

- FEX and Box64 both wrote custom emitters. Rationale for not adopting vixl
  analysed in research notes: FEX wanted multi-backend, Box64 pre-dated
  mature vixl. Neither applies to Prisma.
- V8 (Google), Dart SDK, and several JIT-based runtimes use vixl in
  production. We are not blazing a trail.

## Considered alternatives

### Alt A — Git submodule + invoke SCons from CMake

**How:** add vixl as submodule, use `ExternalProject_Add` to call SCons.

**Pros:** uses vixl's own build system (any fixes upstream apply directly).

**Cons:** requires SCons installed on every dev + CI machine (Python setup
pain on Windows/macOS); SCons and CMake disagree on incremental builds;
output paths are awkward for CMake to consume; no easy way to apply our
sanitizer or coverage flags.

**Decision:** rejected.

### Alt B — Vendored copy of vixl + our CMakeLists inside `third_party/vixl/`

**How:** check vixl sources into our repo at a pinned version, maintain
CMakeLists alongside.

**Pros:** hermetic build (no network at compile time).

**Cons:** ~100k LOC added to our repo; licence attribution becomes noisy;
upgrading vixl requires manual copy.

**Decision:** rejected for now. Could revisit if FetchContent proves fragile
on low-bandwidth environments.

### Alt C — FetchContent + our wrapper CMakeLists ← DECISION

**How:** `FetchContent_Declare` vixl at a pinned commit; a Prisma-maintained
`cmake/BuildVixl.cmake` declares a single `add_library` listing the source
files we need, with our own compile flags.

**Pros:**
- Zero dependency on SCons.
- Works on macOS, Linux, any platform with git + CMake.
- Version pinning explicit and visible in `core/CMakeLists.txt`.
- Sanitizers and coverage flags apply cleanly to both our code and vixl.
- Small diff to upgrade vixl (change commit hash).

**Cons:**
- We maintain the file list. If upstream vixl adds/removes a translation
  unit our build may need updating.
- We chose specific sub-options (e.g. simulator off, `VIXL_CODE_BUFFER_MALLOC`)
  that are not what the upstream SCons build defaults to.

**Why we accept these cons:** the file list is ~15 entries and changes
rarely; the build options are load-bearing enough that we want them visible
in our own code rather than hidden in a third-party SConstruct.

### Alt D — Linker to prebuilt vixl binary

**How:** download a release tarball with precompiled .a.

**Pros:** no compile time.

**Cons:** no releases exist for vixl; ABI drift risk across toolchain
versions; can't apply our compile flags.

**Decision:** rejected, unviable.

## Decision

1. `core/CMakeLists.txt` uses `FetchContent_Declare` to pull vixl from
   `https://github.com/Linaro/vixl.git` at commit
   `59921852720418d0c27d443167af0df387f78f4e` (HEAD of `main` as of April
   2026). Commit hash pinned explicitly; never use a branch name.
2. `cmake/BuildVixl.cmake` provides `prisma_add_vixl(TARGET_NAME SOURCE_DIR ...)`
   that declares a STATIC library with the aarch64 subset only, simulator
   excluded, `VIXL_CODE_BUFFER_MALLOC` chosen (Prisma manages JIT-safe
   memory at a higher layer).
3. vixl include directories are exposed as `SYSTEM` so downstream consumers
   aren't forced to relax their own `-Werror` policy.
4. vixl translation units compile with `-w` (warnings silenced). We audit
   the dependency by version-pinning, not by lint.
5. Prisma-facing API is `prisma::backend::Emitter` in
   `core/include/prisma/emitter.hpp`, implemented via pimpl so vixl headers
   never leak to clients of the library.

## Consequences

### Benefits

- End-to-end CMake build: `cmake --build build` produces the emitter and
  its tests without any external tooling.
- Developers don't need SCons, Python, or any vixl-specific knowledge.
- Upgrading vixl is a one-line change + rerun the test suite.
- Sanitizer/coverage instrumentation applies uniformly.

### Costs

- We own a ~15-entry file list that tracks vixl's `src/aarch64/*.cc`. If
  upstream renames files we must update. Check-point: rerun the full build
  when bumping the commit hash.
- Simulator excluded by default. Re-enabling means adding
  `simulator-aarch64.cc`, `logic-aarch64.cc`, `debugger-aarch64.cc` to the
  file list and defining `VIXL_INCLUDE_SIMULATOR_AARCH64`. Future RFC may
  add an opt-in.
- Warnings from vixl are silenced, not fixed. We rely on vixl's own CI
  to catch real issues in their code.

### Reversibility

High. This is contained in two files:
- `cmake/BuildVixl.cmake` (wrapper).
- `core/CMakeLists.txt` (FetchContent + usage).

Switching to a different emitter library (e.g. asmjit) would require
rewriting `core/src/backend/emitter.cpp` but the Prisma-facing header
(`prisma/emitter.hpp`) is deliberately abstract.

## Implementation notes

Already implemented in this same commit:

- `cmake/BuildVixl.cmake` — the wrapper.
- `core/CMakeLists.txt` — FetchContent + `prisma_add_vixl` call.
- `core/include/prisma/emitter.hpp` — public API with pimpl.
- `core/src/backend/emitter.cpp` — vixl-backed implementation.
- `core/tests/test_emitter.cpp` — first integration tests (movz + ret +
  mov_imm64 + disassembly).

Build verified on macOS arm64 with Apple Clang 17.0.0 and CMake 4.3.1.
17/17 tests pass.

## Open questions

- **Simulator opt-in.** Whether to expose `PRISMA_VIXL_WITH_SIMULATOR`
  CMake option or keep it out entirely. Defer to when we need simulator for
  cross-host validation (Fase 2).
- **Commit hash freshness.** How often we upgrade vixl. Proposal: once
  per Fase or when a needed feature (SVE2 instruction, new opcode) is added
  upstream. Too frequent and we pay for upgrade churn; too rare and we miss
  bug fixes.
- **Disassembler mnemonics.** vixl's disassembler uses canonical aliases
  (`mov` for `movz` with 16-bit immediate). Our test assertions need to
  reflect this. Documented in test_emitter.cpp comments.
- **Literal pool strategy.** Not exposed in `Emitter` yet. When we need it,
  RFC 0003 (decoder) or a separate RFC will decide per-block vs global.

## References

- `docs/research_notes.md` — vixl scan section, decision #1 validated.
- `docs/rfc/0001-ir-ssa-over-template-based.md` — the IR the emitter targets.
- vixl repository: <https://github.com/Linaro/vixl>, pinned commit 59921852.
- LICENCE: BSD-3-Clause (verified in `~/Documents/sandbox/prisma-research/vixl/LICENCE`).
