# Prisma active roadmap

The next deliverable is a guest futex call that synchronizes two threads through the existing wait table. This operating view uses the integration state of `main` at `f0089a3` on 2026-11-27. The tracking epic is [#326](https://github.com/Finithe-Phoenix/prisma/issues/326). Each inventory row has an implementation or owner decision in the [canonical backlog](BACKLOG.md#inventory-review-at-f0089a3).

## Integrated baseline

| Issue or change | Landing commit | Evidence to reuse |
|---|---|---|
| #315 · [PR #312](https://github.com/Finithe-Phoenix/prisma/pull/312) | `aa52753` | W1/W2 ISA and persistent RFLAGS |
| #316 · [PR #327](https://github.com/Finithe-Phoenix/prisma/pull/327) | `b9653d5` | Per-thread startup syscall state |
| [PR #333](https://github.com/Finithe-Phoenix/prisma/pull/333) | `2396063` | Fused translation respects the available guest bytes |
| #317 · [PR #328](https://github.com/Finithe-Phoenix/prisma/pull/328) | `f0089a3` | Portable wait table, value recheck, bounded wakes and shutdown |

## Runtime milestones

| Order | Deliverable | Acceptance |
|---|---|---|
| 1 · [#334](https://github.com/Finithe-Phoenix/prisma/issues/334) | Wire guest syscall 202 to the shared wait table through a thread-local context and explicit guest-word reader | Direct WAIT/WAKE and private variants pass two-host-thread tests, including bounded wake counts, mismatch, invalid address and shutdown. Unsupported commands, flags and timeouts return the specified errors. |
| 2 · [#318](https://github.com/Finithe-Phoenix/prisma/issues/318) | Implement guest thread creation and Session ownership under [RFC 0022](rfc/0022-guest-threading-model.md) | A real ARM64 guest producer/consumer synchronizes through clone/futex. Verify independent registers, TLS and stack, shared arena/cache, cleanup and TSan. |
| 3 · [#319](https://github.com/Finithe-Phoenix/prisma/issues/319) | Replace the running image on execve and return to translation | A second guest executable starts with its own entry state. Failed loads preserve the first image. Repeated replacements release mappings, threads, stacks and waiters. |
| 4 · Linux compatibility | Run the x86-64 coreutils corpus on ARM64, classify failures by subsystem and fix reproducible regressions | Record corpus version, pass count and denominator. Target at least 85% passing. |

## Work alongside the runtime

- **Instruction semantics and parity.** [#320](https://github.com/Finithe-Phoenix/prisma/issues/320) covers RCL/RCR by CL. Both decoders already handle CL forms. Compare the implemented paths against reference results and flags at count boundaries, then fix observed discrepancies. [#321](https://github.com/Finithe-Phoenix/prisma/issues/321) tracks C++/Rust fixtures, serializer agreement and Lean coverage. Keep the Lean sorry budget at zero.
- **Performance and CI.** [#322](https://github.com/Finithe-Phoenix/prisma/issues/322) owns reproducible corpora, the packaged report command, per-backend results and baselines on comparable hardware. [#323](https://github.com/Finithe-Phoenix/prisma/issues/323) owns actionable logs, sanitizer and ARM64 reliability. Use the [CI runbook](CI_RUNBOOK.md) to distinguish code and runner failures.
- **Product and research.** [#324](https://github.com/Finithe-Phoenix/prisma/issues/324) tracks PE/Wine, Android, graphics, AVF, NPU and signed distributed caching. Reuse PE mapping, relocations and import binding. Finish Wine, container start/stop, overlay, JNI, display and input until Notepad runs on an Android device. Measure graphics and research prototypes against documented baselines before beta and release.
- **Owner decisions.** [#325](https://github.com/Finithe-Phoenix/prisma/issues/325) tracks IP and licensing, repository ownership/visibility, domain, devices, privacy and external outreach. Record decisions, dates and dependencies. Use the [multi-year roadmap](ROADMAP.md) for beta, publication and distribution milestones.

## Integration and maintenance

Review [PR #335](https://github.com/Finithe-Phoenix/prisma/pull/335) by subsystem. Against this revision the changed files are the C API header, dispatcher and syscall-handler headers and implementations, and the backlog. Check the wider claims about MMX, Lean, terminal handling and benchmarks against the integrated tree. Attach each useful change to its issue with a focused test and record its landing SHA after integration.

Review dependency updates independently under CI. [#311](https://github.com/Finithe-Phoenix/prisma/pull/311) updates cache, [#314](https://github.com/Finithe-Phoenix/prisma/pull/314) sets up Python, [#336](https://github.com/Finithe-Phoenix/prisma/pull/336) uploads artifacts, [#337](https://github.com/Finithe-Phoenix/prisma/pull/337) checks Markdown and [#338](https://github.com/Finithe-Phoenix/prisma/pull/338) runs CodeQL. Keep one reviewable change per PR, capture relevant CI and architecture results, and link its issue and merge SHA in the backlog.
