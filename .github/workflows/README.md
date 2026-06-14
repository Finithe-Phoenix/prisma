# CI Workflows

| Workflow | Activates when | Purpose |
|---|---|---|
| `core-stub.yml` | any change in `core/` | C++20 DBT engine build + test (x86_64 + ARM64). |
| `core-sanitizers.yml` | any PR / push to `main` | ASan+UBSan + TSan builds. |
| `clang-format.yml` | any PR / push to `main` | Verifies all C/C++ code matches `.clang-format`. |
| `codeql.yml` | any PR / push to `main`, weekly | CodeQL security analysis. |
| `ir-spec.yml` | any change in `ir-spec/` | Builds Lean 4 project, verifies proofs type-check. |
| `lint-docs.yml` | any `.md` change | markdownlint + RFC frontmatter validation. |
| `shell-stub.yml` | any change in `shell/` | Scaffolding check until `shell/Cargo.toml` exists (Fase 3). |
| `ffi-bridge.yml` | any PR / push to `main` | C++/Rust cross-language gate (Linux x86_64, Linux ARM64, Windows/MSVC smoke + Rust workspace). |
| `benchmarks.yml` | any PR / push | Dhrystone smoke-run via `tools/benchmarks`. |

## Future workflows (not yet created)

- `android.yml` — Gradle build + Lint + instrumentation tests (Fase 3).
- `server.yml` — Rust cache service build + test (Fase 2.5).
- `npu-models.yml` — Python training pipeline lint + unit tests (Fase 2.5).
- `release.yml` — tag-triggered APK build + sign + GitHub release (Fase 5).

## Self-hosted ARM64 runner

El plan prevé un Orange Pi 5B (~$150) como self-hosted runner ARM64 para builds nativos del DBT y benchmarks realistas. Setup difiere a cuando tengamos tracción (final Fase 2).
