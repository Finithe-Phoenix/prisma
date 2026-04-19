# CI Workflows

| Workflow | Activates when | Purpose |
|---|---|---|
| `ir-spec.yml` | any change in `ir-spec/` | Builds Lean 4 project, verifies proofs type-check. **Active now.** |
| `lint-docs.yml` | any `.md` change | markdownlint + RFC frontmatter validation. **Active now.** |
| `core-stub.yml` | any change in `core/` | Scaffolding check until `core/CMakeLists.txt` exists (Fase 1). Then real build. |
| `shell-stub.yml` | any change in `shell/` | Scaffolding check until `shell/Cargo.toml` exists (Fase 3). Then real build. |

## Future workflows (not yet created)

- `android.yml` — Gradle build + Lint + instrumentation tests (Fase 3).
- `server.yml` — Rust cache service build + test (Fase 2.5).
- `npu-models.yml` — Python training pipeline lint + unit tests (Fase 2.5).
- `benchmarks.yml` — nightly runs of Dhrystone/CoreMark/SPEC subset on self-hosted ARM64 runner (Fase 2+).
- `release.yml` — tag-triggered APK build + sign + GitHub release (Fase 5).

## Self-hosted ARM64 runner

El plan prevé un Orange Pi 5B (~$150) como self-hosted runner ARM64 para builds nativos del DBT y benchmarks realistas. Setup difiere a cuando tengamos tracción (final Fase 2).
