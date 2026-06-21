---
id: 0019
title: OS-ABI execution loop and guest memory-management syscalls
status: draft
authors: [Claude]
created: 2026-06-21
updated: 2026-06-21
supersedes: []
superseded_by: null
---

# RFC 0019: OS-ABI execution loop and guest memory-management syscalls

## Summary

The Rust shell now has a broad, routed, fuzz-hardened x86-64 Linux syscall
layer (`prisma-session::syscall_dispatch` + the per-family handler modules) and
an execution path that JIT-runs a translated block on ARM64 (RFC 0017). What is
missing is the loop that ties them together — *run the guest until it exits,
servicing each syscall it makes* — and the one capability that loop needs but
`dispatch` cannot currently provide: **growing and mapping the guest address
space** (`brk`, `mmap`, `munmap`, `mprotect`). `malloc` is built on `brk`/`mmap`,
so no non-trivial real program can run until they work.

This RFC proposes (1) evolving the `dispatch` signature so memory-management
syscalls can reach the guest address space, and (2) the OS-ABI run loop that
drives execution to termination, consuming the `SyscallContext::exit_status`
already wired by `exit`/`exit_group`. It explicitly scopes out — into follow-on
RFCs — thread-local storage (`arch_prctl`) and threading (`futex`/`clone`),
which are independent architectural axes.

## Background — why now

The syscall surface added recently (I/O, time, system-info, resource-limits,
scheduler/priority, process-control, tty, filesystem-stat, plus `exit`/
`exit_group`) is complete enough that the *next* blocker is no longer "more
syscalls". The verified state:

- `dispatch(ctx: &mut SyscallContext, mem: &mut GuestRegion, number, args)`
  takes a single, already-resolved `GuestRegion`. That is sufficient for a
  syscall that reads/writes *within* an existing mapping, but a memory-mapping
  syscall must *change the set of mappings* — which `dispatch` has no handle to.
- `brk`/`mmap`/`munmap`/`mprotect` are therefore unrouted, and stubbing them
  would be a correctness lie: returning a fake break pointer without backing
  memory corrupts `malloc`. Per the project's honest-failure principle they are
  left `-ENOSYS` until they can be done correctly.
- `SyscallContext::exit_status: Option<i32>` is set by `exit`/`exit_group` but
  nothing reads it: there is no run loop to stop.

## Proposal

### 1. `dispatch` reaches the guest address space

Replace the per-call `mem: &mut GuestRegion` with a handle to the guest address
space (the `orchestrator::address_space` owner), from which the existing
per-region view is derived on demand. Concretely:

```text
dispatch(ctx: &mut SyscallContext,
         mem: &mut GuestAddressSpace,   // was: &mut GuestRegion
         number: u64,
         args: [u64; 6]) -> i64
```

- Every existing handler that needs a region keeps working: it asks the address
  space for the region covering its pointer argument (the same range-check it
  does today, just one indirection earlier). The pointer-checked-`EFAULT`
  contract is unchanged.
- Memory-management handlers gain what they need: `GuestAddressSpace` exposes
  `brk(new_break)`, `mmap(addr, len, prot, flags)`, `munmap(addr, len)`,
  `mprotect(addr, len, prot)`.

This is a mechanical, well-bounded refactor (one parameter type, threaded
through the handler call sites). It is sequenced as a single dispatcher PR to
avoid the stacked-edit hazard documented in the session log.

### 2. Memory-management semantics

- **`brk(0)`** returns the current program break; **`brk(addr)`** grows or
  shrinks the heap mapping to `addr`, returning the resulting break (the Linux
  contract: on failure it returns the *unchanged* break, never `-ENOMEM`).
- **`mmap`** services anonymous, fixed and hint mappings against the address
  space's free-region allocator; file-backed `mmap` is deferred (the loader
  already maps the image, the common `malloc` path is anonymous).
- **`munmap`/`mprotect`** unmap / re-protect an existing range.

Resource discipline (the standing clause, RFC 0017 §teardown): every mapping is
owned by the address space and unmapped on `munmap` or when the session drops —
nothing survives a restart, and `munmap` releases deterministically rather than
waiting on process exit.

### 3. The OS-ABI run loop

A `Session::run()` that drives execution to termination:

```text
loop {
    let outcome = execute_current_block();   // RFC 0017 path
    match outcome {
        Syscall { number, args } => {
            let rax = dispatch(&mut ctx, &mut addr_space, number, args);
            set_guest_rax(rax);
            if let Some(code) = ctx.exit_status { return code; }
        }
        BlockBoundary(next_pc) => continue,   // chain to the next block
        Fault(f) => return signal_or_abort(f),
    }
}
```

The loop is host-gated exactly as the executor is (ARM64 executes; other hosts
take the translate-only path), so it stays safe to unit-test anywhere by
driving `dispatch` directly with synthetic outcomes — which the existing
`dispatch_robustness` property fuzz already does.

## Out of scope (follow-on RFCs)

- **Thread-local storage** — `arch_prctl(ARCH_SET_FS, addr)` must store an
  FS-base the JIT-translated code reads for `fs:`-relative TLS access. That is an
  execution-engine change (a guest segment-base slot), not a syscall change;
  stubbing `arch_prctl` would silently break TLS. Its own RFC.
- **Threading** — `futex`/`clone` require a real wait-queue + thread model.
  `FUTEX_WAIT` on a matching value has no honest single-threaded answer, so the
  primitive is left unrouted until the threading model lands. Its own RFC (the
  named EPIC A).

## Trade-offs / alternatives considered

- *Keep `GuestRegion`, add a side-channel for mappings.* Rejected: it splits the
  memory model across two handles and invites aliasing bugs; the address space
  is the single owner, so `dispatch` should hold it.
- *Stub `brk`/`mmap` to a fixed pre-reserved heap.* Rejected: a fixed heap that
  silently caps at its size produces the exact "looks like it worked" failure
  the no-silent-caps rule forbids; a real allocator is not much more code and is
  correct.

## Acceptance criteria

- `dispatch` takes the address-space handle; all existing handler tests +
  `dispatch_robustness` fuzz pass unchanged.
- `brk`/`mmap`/`munmap`/`mprotect` routed with unit tests (grow/shrink break,
  anonymous map/unmap, re-protect, `EFAULT`/`EINVAL`/unchanged-break paths) and
  a leak check that an unmapped region is released.
- `Session::run()` returns the guest's exit code for a program that ends in
  `exit_group`, validated on ARM64 CI by a small program that `brk`s, writes,
  and exits.
