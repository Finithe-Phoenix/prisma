<div align="center">
  <h1>⚡ PRISMA</h1>
  <p><strong>x86/x64 → ARM64 Dynamic Binary Translator</strong><br>
  <em>Rewriting the rules of Windows-on-Android emulation</em></p>

  [![core build](https://github.com/anomalyco/prisma/actions/workflows/core-stub.yml/badge.svg)](https://github.com/anomalyco/prisma/actions/workflows/core-stub.yml)
  [![sanitizers](https://github.com/anomalyco/prisma/actions/workflows/core-sanitizers.yml/badge.svg)](https://github.com/anomalyco/prisma/actions/workflows/core-sanitizers.yml)
  [![clang-format](https://github.com/anomalyco/prisma/actions/workflows/clang-format.yml/badge.svg)](https://github.com/anomalyco/prisma/actions/workflows/clang-format.yml)
  [![codeql](https://github.com/anomalyco/prisma/actions/workflows/codeql.yml/badge.svg)](https://github.com/anomalyco/prisma/actions/workflows/codeql.yml)
  [![ir-spec](https://github.com/anomalyco/prisma/actions/workflows/ir-spec.yml/badge.svg)](https://github.com/anomalyco/prisma/actions/workflows/ir-spec.yml)

  <strong>Phase:</strong> 2 · ~1166 tests · Running on real ARM64 hardware · May 2026
</div>

---

## 🌋 The Manifesto

Every Windows-on-Android emulator shipping today — Winlator, GameHub, GameNative — is a fork of FEX or Box64. They get the job done for legacy games. But they all share the same glass ceiling: 10-15% overhead vs native, with no path to close it.

**Prisma is not another fork. Prisma is a generational leap.**

This repo builds a DBT from scratch — not because it's easier, but because the 6 pillars we need don't fit inside FEX's architecture. NPU-assisted translation. Formally verified IR in Lean 4. ML-driven adaptive TSO. Cryptographically signed P2P distributed caches. Hybrid DBT+KVM virtualization on Tensor SoCs. Shader graph analysis for Mali GPU translation.

48 months. One dev lead. Zero funding. Zero shortcuts.

> *"The goal is not to compete with Winlator. The goal is to make Winlator look like a student prototype."*

[📜 Full Manifesto →](PROYECTO_PLAN_EJECUCION.md) · [🔭 Ecosystem Research →](compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md)

---

## 🏗️ The 6 Pillars

| # | Pillar | Status | Target |
|---|---|---|---|
| 1 | **NPU-Assisted Translation** — Hot path prediction + TSO classification on idle NPUs | 📝 Research | Phase 2.5 |
| 2 | **Formal IR in Lean 4** — CompCert-style correctness proofs for critical optimizations | ✅ **Active** | Phase 2 |
| 3 | **ML-Driven Adaptive TSO** — Eliminate `DMB ISH` where the model proves aliasing-free | 📝 Research | Phase 2.5 |
| 4 | **Distributed P2P+CDN Cache** — Ed25519-signed caches shared across devices | 🔧 In development | Phase 2.5 |
| 5 | **Hybrid DBT+KVM Virtualization** — Dual-mode on Tensor SoCs with AVF/pKVM | 📐 Design | Phase 4 |
| 6 | **Advanced Graphics Translation** — Shader graph analysis + Vortek++ for Mali | 📐 Design | Phase 5 |

---

## 🚀 What Already Works

```
                     x86_64 guest code
                           │
                           ▼
             ┌─────────────────────────┐
             │    Decoder (352 tests)  │  ← SSE..SSE4.2, BMI1, AVX-128/256, FMA3
             │    x86_64 bytes → IR    │
             └─────────┬───────────────┘
                       │ IR (SSA)
                       ▼
             ┌─────────────────────────┐
             │    Pass pipeline         │  ← 10 passes · const_prop, CSE, DCE, LICM…
             │    (150 tests)           │
             └─────────┬───────────────┘
                       │ optimized IR
                       ▼
             ┌─────────────────────────┐
             │    Lowerer + Emitter    │  ← ALU, mul/div, LSE atomics, literal pool
             │    IR → ARM64 (34 tests) │
             └─────────┬───────────────┘
                       │ ARM64 machine code
                       ▼
             ┌─────────────────────────┐
             │    Translation Cache     │  ← LRU + byte-budget + persistent v2
             │    (61 tests)            │
             └─────────┬───────────────┘
                       │
                       ▼
             ┌─────────────────────────┐
             │    Runtime               │  ← JIT memory, signals, dispatcher
             │    + Syscall Handler     │  ← 36+ syscalls (read/write/open/brk/mmap/
             │    (48 tests)            │     epoll/poll/arch_prctl/getdents64…)
             └─────────────────────────┘
                       │
                       ▼
              🖥️  REAL ARM64 HARDWARE
```

| Subsystem | Tests | Key achievement |
|---|---|---|
| **IR SSA** + validator | 120 | 3 lemmas formalized in Lean 4 |
| **x86_64 decoder** | 352 | SSE..SSE4.2 + AVX-128 + AVX-256 batch 1 + FMA3 |
| **Optimization pipeline** | 150 | 10 passes with timing and dump |
| **ARM64 emitter** | 34 | ALU 3-reg, LSE atomics, literal pool |
| **Translation cache** | 61 | Save/load v2, zstd compression, async save |
| **Runtime + syscalls** | 48 | 36 x86_64 → POSIX syscalls + strace logger |
| **Formal spec (Lean 4)** | — | CI build + sorry budget enforcement |
| **TOTAL** | **~1166** | **Zero failures on real ARM64** |

---

## 🧱 Repository

```
prisma/
├── core/                  🟢 C++20 — 50k+ lines, the DBT engine
│   ├── src/ir/            IR SSA + pretty-print + validate
│   ├── src/decoder/       x86_64 → IR (SIB, RIP-rel, REX, VEX C4/C5)
│   ├── src/passes/        const_prop → algebraic → CSE → DCE (10 passes)
│   ├── src/emitter/       Lowering + vixl-backed ARM64 codegen
│   ├── src/cache/         TranslationCache LRU + persistent + async save
│   ├── src/runtime/       JIT memory, signals, dispatcher, syscalls
│   └── tests/             40 files, 1166 TEST_CASEs
├── ir-spec/               🟢 Lean 4 — Formal IR specification
├── shell/                 ⚪ Rust — Orchestrator (scaffolding, Phase 3)
├── android/               ⚪ Kotlin — App (scaffolding, Phase 3)
├── npu-models/            ⚪ Python — ONNX models (research, Phase 2.5)
├── server/                ⚪ Rust — P2P cache service (design, Phase 2.5)
├── tools/                 🟢 Python — Benchmarks framework
├── third_party/           📚 References (FEX, Box64, Wine, DXVK)
├── papers/                📝 LaTeX drafts
└── docs/                  📖 RFCs, backlog, research notes
```

**Legend:** 🟢 Active · 🔧 In development · ⚪ Scaffolding · 📝 Research · 📐 Design

---

## ⚡ Quick Start

```bash
# Requirements: cmake 3.25+, ninja, clang-17 (or AppleClang on macOS)

# Build (Linux ARM64 / macOS Apple Silicon)
cmake -S core -B core/build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build core/build

# Run all tests
ctest --test-dir core/build --output-on-failure

# With sanitizers (Linux)
cmake -S core -B /tmp/prisma-asan -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug -DCMAKE_CXX_COMPILER=clang++-17 \
  -DPRISMA_ENABLE_ASAN=ON -DPRISMA_ENABLE_UBSAN=ON
cmake --build /tmp/prisma-asan
ctest --test-dir /tmp/prisma-asan --output-on-failure

# Coverage (Linux, clang-only)
cmake -S core -B /tmp/prisma-cov -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug -DCMAKE_CXX_COMPILER=clang++-17 \
  -DPRISMA_ENABLE_COVERAGE=ON
cmake --build /tmp/prisma-cov
LLVM_PROFILE_FILE="prisma-%p.profraw" /tmp/prisma-cov/prisma_core_tests
llvm-profdata merge -o prisma.profdata *.profraw
llvm-cov show /tmp/prisma-cov/prisma_core_tests -instr-profile=prisma.profdata
```

---

## 🔬 CI: 10 Required Checks Per PR

Every Pull Request runs this full battery before merging:

| Workflow | What it catches | Typical time |
|---|---|---|
| `core-build` (x86_64) | Compilation errors + test regressions | ~3 min |
| `core-build` (ARM64) | JIT execution on real hardware | ~4 min |
| `asan-ubsan` | Memory leaks, buffer overflows, UB | ~5 min |
| `tsan` | Data races | ~5 min |
| `clang-format` | C++ style violations | ~30 s |
| `codeql` | Security vulnerabilities | ~3 min |
| `ir-spec-build` | Lean 4 formal proof regressions | ~2 min |
| `markdownlint` | Broken links, doc formatting | ~30 s |
| `ffi-link` | C++/Rust cross-language gate (x86_64 + ARM64) | ~4 min |
| `benchmarks-smoke` | Benchmarks framework regressions | ~1 min |

---

## 🤖 Multi-Agent Coordination

Prisma is developed by two autonomous AI agents. The protocol lives in [docs/COORDINATION.md](docs/COORDINATION.md):

```
┌─────────────────────────────────────────────────────┐
│                   CODEZ (Codex)                      │
│  decoder · IR variants · dispatcher                  │
│  Claim → Implement → Commit → Mark done              │
└─────────────────────────────────────────────────────┘
                        ↕
              docs/BACKLOG.md (single source of truth)
                        ↕
┌─────────────────────────────────────────────────────┐
│                   CLAUDE                             │
│  emitter · passes · lowerer · cache · runtime · CI   │
│  Claim → Implement → Commit → Mark done              │
└─────────────────────────────────────────────────────┘
```

Every backlog item is marked `[~|codex]` or `[~|claude]` before work begins.  
Commits in English, format `<scope>: <what>`. New code **always** comes with tests.

---

## 📐 Tech Stack

| Layer | Language | Standard | Tooling |
|---|---|---|---|
| DBT Engine | **C++** | C++20 (concepts, span, atomic_ref) | CMake + Ninja + Clang-17 |
| IR Specification | **Lean 4** | — | Lake + mathlib |
| Orchestrator | **Rust** | Edition 2024 | Cargo + Clippy |
| Android App | **Kotlin** | 2.0 + Jetpack Compose | Gradle + Detekt |
| ML Pipeline | **Python** | 3.12+ | Black + Ruff + Mypy |
| Formal proofs | **Lean 4** | — | Lake build |

**Minimum requirements:**
- C++: clang-17+ (Linux), AppleClang 15+ (macOS), MSVC 2022 (Windows, partial)
- CMake 3.25+, Ninja build
- Lean 4: elan + lake (ir-spec/ only)
- Rust: nightly (shell/ only)

> ⚠️ `prisma_runtime` (JIT, signals, syscalls) is POSIX-only. On Windows (MSVC) the runtime compiles as a stub. E2E tests require real ARM64 hardware.

---

## 📅 Timeline

```
Phase 0 ── Foundation ───────────────── Q2 2026  🟢 Complete
Phase 1 ── Core IR + decoder ───────── Q2 2026  🟢 Complete
Phase 2 ── SSE/AVX/FMA ─────────────── Q3 2026  🟡 In progress
Phase 2.5 ─ NPU research + P2P cache ─ Q4 2026  📝 Research
Phase 3 ── Android app + loader ────── Q1 2027  ⚪ Scaffolding
Phase 4 ── Hybrid virtualization ───── Q3 2027  📐 Design
Phase 5 ── Graphics translation ────── Q1 2028  📐 Design
Phase 6 ── Open source + papers ────── Q2 2030  🎯 Target
```

Public v1.0: **Q2 2030**. 48-54 months. This is not a side project — it's a career commitment.

---

## 📚 Research Output

| Type | Target | Status |
|---|---|---|
| Top-tier venue papers (MICRO, ASPLOS, POPL, PLDI) | ≥ 1 | 📝 Drafting |
| Total publications | ≥ 3 | 📝 Drafting |
| Technical blog posts | Every 2-3 months | 📝 Ongoing |
| LaTeX drafts | `papers/` | 📝 Active |

---

## ⚖️ License (future)

| Component | License | From |
|---|---|---|
| Core DBT + IR spec + NPU models + graphics research | **MIT** | Phase 6 (~2030) |
| Android app + cloud services | **Commercial freemium** | Launch |

The repository is private during Phases 0-5.

---

<div align="center">
  <br>
  <strong>Prisma</strong> — 48 months · 1 dev lead · 0 shortcuts · 6 pillars · 1 mission.
  <br><br>
  <em>"We are not building an emulator. We are building the future of emulation."</em>
  <br><br>
  <a href="PROYECTO_PLAN_EJECUCION.md">📜 Manifesto</a> ·
  <a href="docs/BACKLOG.md">📋 Backlog</a> ·
  <a href="docs/rfc/">📖 RFCs</a>
  <br><br>
</div>
