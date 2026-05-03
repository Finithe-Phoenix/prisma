---
id: 0009
title: Page-protection-based SMC detection (SmcGuard)
status: accepted
authors: [Danny]
created: 2026-05-03
updated: 2026-05-03
supersedes: []
superseded_by: null
---

# RFC 0009: Page-protection-based SMC detection (SmcGuard)

## Summary

Add a `prisma::runtime::SmcGuard` component that maps every guest
page covered by a cached translation as `PROT_READ | PROT_EXEC`
via `mprotect`, then catches the resulting `SIGSEGV` when the
guest writes that page. On fault, every translation that covers
the page is invalidated, the page is flipped back to writable so
the store can retry, and execution resumes. This complements the
existing FNV-1a content-hash check in `TranslationCache`: the
hash is the correctness backstop; page protection moves the cost
of SMC detection off the dispatcher hot path.

## Motivation

The translation cache currently re-hashes the guest bytes on
every lookup to detect self-modification (`Key.content_hash`).
That cost is paid on every dispatch, regardless of whether SMC
is happening. Real Windows binaries almost never self-modify;
games, anti-debug shells, and JITs do. We want the common case
free and the rare case correct.

Page protection is the standard DBT solution: the MMU does the
detection for us. QEMU, FEX-Emu, Box64, and DynamoRIO all use
some variant of it. The only cost is one `mprotect` call per
new translation page, paid once at translate time.

## Context

Constraints:

- POSIX-only for now (Linux + Darwin). Windows uses
  `VirtualProtect`; that wiring lives in the host port (Fase 3+).
- Apple Silicon: the *guest* code pages we protect are not
  MAP_JIT-allocated; they are guest memory that the runtime
  treats as code. We do **not** request `PROT_EXEC` on these
  pages — the host CPU never executes guest bytes directly
  (the JIT translates them into host code in `JitBuffer`),
  so `PROT_READ` alone is enough to catch SMC writes. Dropping
  `PROT_EXEC` also keeps us out of macOS Apple Silicon's
  MAP_JIT W^X enforcement. The MAP_JIT dance only applies to
  the *host* JIT buffer, which is managed by `JitBuffer` —
  out of scope here.
- Page size: x86-64 guests use 4 KiB pages. We hard-code
  `kGuestPageSize = 4096` for Phase 1. macOS hosts have a
  16 KiB host page size; `mprotect` on a 4 KiB region will
  silently extend to the host page granularity. That over-
  protects but does not cause incorrect behaviour. A
  follow-up RFC will promote the constant to a runtime value
  once we cross-compile for Apple Silicon hosts.
- Threading: `protected_pages_` is guarded by `std::mutex`. The
  fault path is rare; lock-free is deferred. The mutex is
  briefly acquired and released around `mprotect`, never held
  across the user callback.

Existing components touched:

- `core/src/runtime/signal_handler.cpp` gains a hook that, on
  `SIGSEGV`, consults a process-global `SmcGuard*` and, if the
  fault address belongs to a tracked page, returns from the
  handler instead of `longjmp`'ing.
- `TranslationCache` is **not** modified by this RFC. The
  integration commit lives separately and will pass an
  `invalidate_cb` that calls `cache.invalidate_key(...)`.

## Considered alternatives

### A. Pure content-hash (status quo)

Hash on every dispatch, like today.

- Pros: simple, no platform dependency.
- Cons: O(translation length) per dispatch hop. Profiling on
  representative traces is pending but the analytical cost is
  obvious — every basic block recomputes its hash.

Rejected because it scales poorly and pays for SMC detection
even when no SMC is happening.

### B. Page protection only, no content hash

Drop the hash entirely; trust mprotect.

- Pros: cheapest dispatch.
- Cons: a missed `mprotect` failure (e.g. the page belongs to
  a region we cannot protect) silently degrades to "no SMC
  detection at all". Also vulnerable to `mremap`, copy-on-write
  forks, and any guest mechanism that bypasses page faults
  (think VirtualAlloc + manual permission flips on a page
  outside our tracking).

Rejected on correctness grounds. Hash stays as a backstop.

### C. Single-step + reprotect inside the handler

When SMC is detected, lift protection, single-step the trapping
instruction (`PTRACE_SINGLESTEP` on Linux, mach exception ports
on macOS), then re-protect.

- Pros: page is RW for exactly one instruction; no observable
  window where another thread could cache a stale translation.
- Cons: ptrace is process-wide and conflicts with debuggers;
  mach exception ports are involved and require entitlements.
  Nontrivial cross-platform investment.

Rejected for now. We accept the wider window described under
"Decision" and document it as a known trade-off.

## Decision

Adopt approach (A + page protection): keep the FNV-1a hash as
the correctness backstop, add page-protection SMC detection as
the optimisation.

Concretely:

- New `class SmcGuard` (header `prisma/smc_guard.hpp`, impl
  `core/src/runtime/smc_guard.cpp`) owning a
  `std::unordered_map<page_addr, std::vector<cache_key>>` under
  a `std::mutex`.
- API:
  - `on_translate(guest_pc, guest_byte_len, cache_key)` —
    register a translation; mark every page it touches as RO.
  - `on_invalidate(cache_key)` — drop the key from every page;
    when a page becomes empty, restore RW.
  - `handle_fault(fault_addr, invalidate_cb)` — called from the
    signal handler. If `fault_addr` is in a tracked page,
    invoke `invalidate_cb` for every key on that page, restore
    RW, return `true`. Otherwise return `false`.
- Process-global singleton accessor `set_global_smc_guard` /
  `global_smc_guard` so the signal handler can find the instance
  without per-thread plumbing.
- `signal_handler.cpp` consults the global guard on `SIGSEGV`
  before falling through to the existing `longjmp` recovery path.

## Consequences

### Benefits

- Hot-path dispatch no longer hashes per-translation bytes once
  the cache integration lands. The hash check becomes a
  per-cache-miss safety net rather than a per-hit cost.
- Symmetric with how QEMU, FEX, Box64 do it: simpler to compare
  performance against and easier to onboard contributors.
- Decoupled API: the cache integration is a separate, smaller
  change. Tests can exercise the surface without bringing the
  full JIT runtime online.

### Costs

- One `mprotect` per new page (and per page becoming empty).
  Negligible at the rate translations land; a stress benchmark
  will quantify this when the cache integration is in place.
- The "RW window" trade-off (see "Open questions"): the page
  stays writable from the moment the SIGSEGV handler returns
  until the dispatcher's next round trip has a chance to call
  `protect_page(_, false)`. This is not a correctness problem
  for single-threaded guests; a multi-threaded guest could
  technically observe a stale translation through a parallel
  thread that dispatches into the same address while the page
  is RW. The hash backstop catches it; we accept the latency
  hit.
- `mprotect` on a page that overlaps non-mappable memory will
  fail. The RFC's policy: log to stderr and continue without
  protection. The hash continues to defend correctness.

### Reversibility

High. The component is opt-in: until `TranslationCache` calls
`on_translate` / `on_invalidate`, nothing is protected and the
signal handler path is a no-op (the global pointer is null).
Removing it later is a matter of dropping the call sites and
deleting the file.

## Implementation notes

- Files added:
  - `core/include/prisma/smc_guard.hpp`
  - `core/src/runtime/smc_guard.cpp`
  - `core/tests/test_smc_guard.cpp`
- Files modified:
  - `core/src/runtime/signal_handler.cpp` — adds the SmcGuard hook.
  - `core/CMakeLists.txt` — registers the new sources.
- The header places `kGuestPageSize` at namespace scope so the
  page-base helper can be `constexpr`. Make it a `static
  constexpr` member if a future change wants per-instance page
  sizes (multi-guest support is years away).

## Open questions

1. **Exact reprotect window.** The current implementation flips
   the page to RW from inside `handle_fault` and never re-arms
   it. A follow-up RFC needs to define the dispatcher hand-off
   that re-protects on the next round trip and bounds the
   window. Honest version: today the page stays RW until the
   *next* `on_translate` on that page, which is unbounded.
   Mitigated by the content-hash backstop.
2. **Per-thread vs. process-global guard.** The current API is
   process-global. Per-thread caches (Pillar 4 distributed
   cache) may want per-thread guards. Defer until that lands.
3. **macOS host-page granularity.** On Apple Silicon, host page
   size is 16 KiB. `mprotect` will round our 4 KiB request up.
   Effects on guests with closely-packed code+data pages will
   need profiling.
4. **Windows port.** `VirtualProtect` instead of `mprotect`,
   structured exception handler instead of `sigaction`. Wiring
   lands with the Windows host work in Fase 3.

## References

- QEMU `tb_invalidate_phys_page_range` — equivalent mechanism
  on QEMU's translation block cache.
- FEX-Emu `SignalDelegator::HandleSIGSEGV` — page-protection
  flow for guest writes to translated regions.
- DynamoRIO Bruening, "Efficient, Transparent, and Comprehensive
  Runtime Code Manipulation" (2004), §3.3 "Code cache
  consistency".
