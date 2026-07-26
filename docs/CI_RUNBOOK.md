# Prisma CI stabilization runbook

This runbook defines how to diagnose and repair failing pull-request checks without contaminating product branches or producing misleading matrices.

## Non-negotiable rules

1. **Trust only the latest PR head SHA.** Results from superseded commits are historical evidence, not the current state.
2. **Never use a workflow to modify source code.** GitHub Actions validates commits; it must not manufacture corrective commits inside a feature PR.
3. **Do not add temporary diagnostic workflows to product branches.** Improve an existing permanent workflow or reproduce the failure locally.
4. **Keep diagnostic and product changes separate.** CI infrastructure belongs in its own branch and PR.
5. **Create a backup branch before a forced rollback.** Record both the backup ref and the restored checkpoint in the tracking issue.
6. **Apply one bounded source fix at a time.** Do not mix ABI changes, test expectation changes, runtime fixes, and CI changes in one commit.
7. **Do not rerun a failed matrix without new evidence.** Reruns are appropriate for known runner failures or flakes, not deterministic compiler/test failures.

## Validation sequence

Validate in this order so inexpensive failures stop the process early:

1. Formatting and documentation.
2. Rust and Lean checks.
3. C++ configure and x86_64 compilation.
4. Portable x86_64 tests.
5. ARM64 compilation and JIT tests.
6. Sanitizers.
7. FFI bridge on Linux, ARM64, and Windows.
8. CodeQL and benchmarks.

A later stage must not be interpreted until every earlier stage has either passed or has an explicitly documented independent failure.

## Failure classification

Every red check must be classified before code changes are made:

- **Configure failure:** dependency, generator, compiler, or CMake graph problem.
- **Build failure:** compiler error, warning promoted to error, link failure, or generated-code mismatch.
- **Test failure:** executable built successfully and an assertion, signal, timeout, or sanitizer finding failed.
- **Runner failure:** cancellation, unavailable runner, network/package outage, or infrastructure timeout.
- **External failure:** a provider outside GitHub Actions; report its URL and do not treat it as a core regression without evidence.

## Required evidence

For every corrective commit, record:

- failing workflow and job;
- exact failing phase;
- concise compiler/test diagnostic;
- root cause;
- files changed by the fix;
- validation head SHA;
- residual risk or platform not yet verified.

## Merge policy

A PR is mergeable only when the latest head SHA has all required checks green. Cancelled, skipped, action-required, or superseded runs do not count as validation.
