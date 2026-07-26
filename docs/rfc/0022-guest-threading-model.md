---
id: 0022
title: Guest threading model — clone, futex, and the multi-thread run loop
status: proposed
authors: [Claude]
created: 2026-06-24
updated: 2026-06-24
supersedes: []
superseded_by: null
---

# RFC 0022: Guest threading model — clone, futex, and the multi-thread run loop

## Summary

Proposes how Prisma runs a multi-threaded guest: `clone(CLONE_VM|...)` spawns a
**host OS thread per guest thread**, each running its own run loop over the
**shared** `BackedAddressSpace` arena (RFC 0020), with its own per-thread
`CpuStateFrame`. `futex(WAIT/WAKE)` blocks/wakes those host threads through a
host-side wait table keyed by guest address. This is the next blocker for real
Linux programs (pthreads, glibc) and Wine bring-up. No code yet — this RFC fixes
the model before the highest-blast-radius runtime change.

## Motivation

`run()` today drives a single `CpuStateFrame` to completion (RFC 0019/0021).
Anything that calls `pthread_create` (→ `clone`) or blocks on a `futex` (every
mutex/condvar) cannot run. `clone` and `futex` are the gate to coreutils' threaded
binaries and to Wine. They are also where Pillar 3 (adaptive TSO) lives, so the
threading model must be correct *and* expose the points where memory-ordering
relaxation can later be measured.

## Context

- One guest, one contiguous host arena (RFC 0020): `mem_base` maps every guest VA
  for the JIT; `CLONE_VM` threads simply share the same arena and `mem_base`.
- Each guest thread needs its own register/segment state → its own
  `CpuStateFrame` (the JIT addresses state through x27, set per call).
- `arch_prctl`/TLS (RFC 0021/#308): each thread sets its own `%fs` base, so
  per-thread frames already carry per-thread TLS.
- Translation cache + JIT W^X buffers are shared, read-only at execute time, and
  SMC-guarded — safe to share across threads (RFC 0007/0017). Concurrent *writes*
  to the cache (a translate on a miss) need synchronization.
- Resource clause: each thread's `CpuStateFrame` + guest stack + host thread must
  be released on join/exit; nothing leaks across a restart.
- TSan is a required check — the model must be data-race-free under it.

## Considered alternatives

### Run model

1. **OS thread per guest thread (chosen).** `clone` spawns a host `std::thread`
   that runs a run loop over the shared arena with a fresh `CpuStateFrame`. Pros:
   true parallelism; maps guest scheduling onto the host scheduler; `futex` waits
   map to host blocking; simplest mental model. Cons: shared mutable state (cache
   writes, the arena's region map) needs locking; thousands of guest threads cost
   host threads (acceptable for the target workloads; a pool can come later).
2. **Cooperative N:1 (one host thread, switch frames at futex/clone points).**
   Pros: no host-thread races, no locking. Cons: no real parallelism (defeats a
   multicore guest), and a guest spin-loop without a futex never yields →
   deadlock. Rejected: real pthread workloads need parallelism.
3. **N:M green threads.** A scheduler multiplexing M guest threads onto N host
   threads. Most flexible, but a large scheduler to build and to make race-free;
   premature. Deferred — alternative 1 can evolve into it.

### futex backing

1. **Host wait table keyed by guest address (chosen).** A shared map from guest
   VA → a host `Condvar`/parking primitive; `FUTEX_WAIT` re-checks the guest word
   (in the arena) under the lock then parks; `FUTEX_WAKE` signals N waiters.
   Portable, exact futex semantics, works with the arena.
2. **Map directly onto host `libc::syscall(SYS_futex, mem_base+va, ...)`.** Lets
   the host kernel do the waiting on the arena word. Tempting on Linux/ARM64 (the
   only execution host), but couples to host futex ABI, complicates spurious-wake
   and requeue handling, and won't port. Deferred as a possible fast path.

## Decision (proposed)

- `clone` with `CLONE_VM|CLONE_THREAD` spawns a host thread sharing the arena +
  cache; it gets a new `CpuStateFrame` seeded from the parent (child RSP/TLS from
  the `clone` args), and starts its run loop at the child entry. Non-`CLONE_VM`
  `clone`/`fork` (separate address space) is **out of scope** here (needs an arena
  copy; later).
- `futex` is serviced via a shared host wait table keyed by guest VA, reading the
  futex word from the arena. Start with `FUTEX_WAIT`/`FUTEX_WAKE`
  (`FUTEX_PRIVATE` and bitset/requeue/PI later).
- The shared mutable state — translation cache writes and the arena region map —
  is guarded (a `Mutex`/`RwLock` to start; finer-grained later). Execute-time
  reads of installed W^X code stay lock-free (SMC guard handles invalidation).
- Each thread frees its `CpuStateFrame` + guest stack on exit; `clone`/thread
  join releases the host thread. A leak test (or ASan/TSan) covers it.
- `set_tid_address`, `gettid`, `set_robust_list`, `rseq` (stub) round out the
  thread-startup surface glibc touches.

## Consequences

- **Benefits:** pthreads / threaded coreutils / Wine become runnable; real
  parallelism; a concrete home for the adaptive-TSO measurement (Pillar 3).
- **Costs:** the run model goes multi-threaded — locking around the cache + arena
  map, and TSan must stay green; per-thread frames/stacks to own and free.
- **Reversibility:** large but layered — `clone`/`futex` are additive syscalls;
  the single-thread `run()` stays for single-threaded programs. The locking is an
  internal detail, swappable for finer-grained schemes.

## Implementation notes (suggested order)

1. Thread-startup syscalls (`set_tid_address`, `gettid`, `set_robust_list`,
   `rseq` stub) — small, unblock glibc startup, no threading yet.
2. `futex` wait table (single-thread-testable: WAIT on an already-satisfied word
   returns immediately; WAKE with no waiters is a no-op).
3. A multi-thread run driver + `clone(CLONE_VM|CLONE_THREAD)`; lock the cache +
   arena map; a two-thread e2e (producer/consumer via a futex) under TSan on
   `ffi-link-arm64`.
4. `execve` (separate; re-enter the translator with a new image).

## Open questions

- Cache/arena locking granularity vs. contention — measure before optimizing.
- Signal delivery to a specific thread (interacts with the future signal work).
- Host-thread exhaustion for pathological guest-thread counts (pool? cap?).
- Whether to take the direct host-futex fast path on Linux/ARM64.

## References

- RFC 0017 (session), 0019 (OS-ABI loop), 0020 (memory), 0021 (control flow).
- `docs/BACKLOG_EXTREMO.md` EPIC A (futex/clone) + EPIC D (TSO).
- Task #7 in the active session task list.
