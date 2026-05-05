# `tools/coverage/` — F1-TC-006 local coverage HTML generation

Generates an HTML coverage report for `prisma_core_tests` using
clang's source-based coverage instrumentation.

## Quick start

```sh
tools/coverage/gen.sh
open tools/coverage/html/index.html  # macOS
```

The script:

1. Configures and builds `core/build-cov/` under
   `-DPRISMA_ENABLE_COVERAGE=ON -DCMAKE_CXX_COMPILER=clang++`.
2. Runs the test suite under `LLVM_PROFILE_FILE=...prisma-%p.profraw`,
   excluding the macOS-specific signal_handler flake that's a
   known platform issue.
3. Merges raw profiles via `llvm-profdata merge -sparse`.
4. Emits HTML via `llvm-cov show -format=html`, ignoring
   `_deps/` (third-party) and `/tests/` (the tests themselves —
   only production code coverage is meaningful).
5. Prints the headline number from `llvm-cov report`.

## Flags

```
--skip-build         Reuse the existing core/build-cov/ directory
                     (faster iteration when only rerunning tests).
--filter PATTERN     Restrict tests run, e.g. --filter "[zydis]".
--help / -h          Show this help.
```

## Requirements

- `clang++` on PATH. (Apple silicon: `xcode-select --install` or
  `brew install llvm`. Linux: distro `clang` package.)
- `llvm-profdata` and `llvm-cov` on PATH (same toolchain).

## CI workflow (planned)

The `prisma-linux-arm64` runner (F0-DX-014) will wrap this script
and upload `tools/coverage/html/` to `prisma-emu.dev/coverage`. The
upload step is gated on F0-LG-003 (domain registration) and active
CI infrastructure; until those land, this script is the local
"how green is the test surface" tool.
