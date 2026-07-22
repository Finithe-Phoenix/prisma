# Prisma Active Roadmap

> Execution guide for the current engineering program. The canonical multi-year inventory remains [`BACKLOG.md`](BACKLOG.md); this document is the prioritized, dependency-aware operating view.

## Command center

- Tracking epic: [#326](https://github.com/Finithe-Phoenix/prisma/issues/326)
- Current integration batch: [PR #312](https://github.com/Finithe-Phoenix/prisma/pull/312)
- Threading architecture: [RFC 0022](rfc/0022-guest-threading-model.md)
- Detailed session queue: [`WORK_QUEUE.md`](WORK_QUEUE.md)

## Priority model

| Priority | Meaning | Merge policy |
|---|---|---|
| P0 | Blocks the next executable milestone | Work before unrelated feature expansion |
| P1 | Correctness or architecture follow-up | Start after overlapping P0 work lands |
| P2 | Quality, performance, infrastructure, or research | Run in parallel when ownership does not conflict |
| Governance | Owner-controlled external prerequisite | Track explicitly; never pretend engineering can close it alone |

## Phase A — stabilize the baseline

### [#315 — Stabilize and merge W1/W2 coding-wave batch](https://github.com/Finithe-Phoenix/prisma/issues/315)

**Outcome:** PR #312 lands with benchmarks, C++20, sanitizers, FFI, CodeQL, Rust, Lean, formatting, and docs checks green.

This is the prerequisite for work that depends on persistent RFLAGS, real atomic CAS, WideDiv, PCMPxSTRx, PCLMULQDQ, BMI1, or F16C.

## Phase B — unlock pthreads and glibc

Dependency chain:

```text
#315 ──┬──> #316 thread-startup syscalls ──┐
       └──> #317 futex WAIT/WAKE ──────────┼──> #318 multi-thread Session + clone ──> #319 execve
                                         ┘
```

### [#316 — glibc thread-startup syscall surface](https://github.com/Finithe-Phoenix/prisma/issues/316)

Implement `gettid`, `set_tid_address`, `set_robust_list`, and an explicit `rseq` stub without changing single-thread execution semantics.

### [#317 — FUTEX_WAIT/FUTEX_WAKE host wait table](https://github.com/Finithe-Phoenix/prisma/issues/317)

Add the portable wait-table foundation required by pthread mutexes and condition variables, with value re-checking, bounded wakeups, cleanup, and TSan coverage.

### [#318 — multi-thread Session and clone](https://github.com/Finithe-Phoenix/prisma/issues/318)

Run one host thread per guest thread over a shared arena and cache, with one `CpuStateFrame` per guest thread and a real ARM64 producer/consumer e2e.

### [#319 — execve image replacement](https://github.com/Finithe-Phoenix/prisma/issues/319)

Replace the current guest process image safely, rebuild the initial process state, and re-enter the translator without leaking mappings, threads, stacks, or waiters.

## Phase C — close correctness gaps

### [#320 — variable-count RCL/RCR](https://github.com/Finithe-Phoenix/prisma/issues/320)

Complete the remaining CL-count rotate-through-carry forms after PR #312 to minimize decoder/backend conflicts.

### [#321 — Rust/C++/Lean parity and differential corpus](https://github.com/Finithe-Phoenix/prisma/issues/321)

Automate IR/serializer parity, widen cross-language fixtures, and make missing visitors or semantic mirrors fail loudly in CI.

## Phase D — performance and delivery confidence

### [#322 — benchmark harness and baselines](https://github.com/Finithe-Phoenix/prisma/issues/322)

Harden pytest/Ruff/mypy gates, test report generation, make corpora reproducible, upload artifacts, and define a non-flaky regression policy.

### [#323 — ARM64 CI and sanitizer infrastructure](https://github.com/Finithe-Phoenix/prisma/issues/323)

Distinguish code failures from runner failures, retain useful artifacts, document recovery, and evaluate a self-hosted ARM64 runner when justified.

## Phase E — product and program enablers

### [#324 — Wine, graphics, Android, and NPU research](https://github.com/Finithe-Phoenix/prisma/issues/324)

Turn the research backlog into architecture notes, decision records, and explicit go/no-go criteria without destabilizing the core runtime.

### [#325 — legal, organization, domain, and community prerequisites](https://github.com/Finithe-Phoenix/prisma/issues/325)

Track owner-controlled IP, visibility, organization, domain, telemetry, and outreach decisions separately from engineering completion.

## Working agreement

1. Use one bounded branch and PR per independently reviewable slice.
2. Define acceptance criteria and tests before implementation.
3. Do not merge unexplained red CI.
4. Record the landing SHA in the relevant issue.
5. Keep `BACKLOG.md` complete and this document focused on active execution.
6. Prefer small vertical slices that end in an executable or observable result.
7. Avoid parallel edits to the same decoder/backend files unless ownership is explicit.

## Immediate attack order

1. Finish #315 and merge PR #312.
2. Implement #316 as the lowest-blast-radius threading prerequisite.
3. Build #317 in parallel only where file ownership is separate.
4. Land #318 after the foundations are green under TSan.
5. Take #320 after PR #312 to avoid conflict with the persistent flags batch.
6. Continuously improve #321–#323 as supporting quality streams.
