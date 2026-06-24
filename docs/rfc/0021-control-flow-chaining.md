---
id: 0021
title: Control-flow chaining — call/ret/indirect via the guest stack
status: accepted
authors: [Claude]
created: 2026-06-23
updated: 2026-06-23
supersedes: []
superseded_by: null
---

# RFC 0021: Control-flow chaining — call/ret/indirect via the guest stack

## Summary

The run loop chains a guest program across **all** its control transfers —
relative branches, direct `call`/`ret`, indirect `call reg`/`jmp reg`, and
nested calls — by lowering each terminator to a *block exit* that records the
next guest PC in `CpuStateFrame::next_pc` and returns to the host loop, which
resumes by translating the block at that PC. `call`/`ret` use the **guest stack
itself** as the return-address stack (real memory since RFC 0020): a `call`
pushes its return address, a `ret` pops it. No host call frame, no separate
shadow stack, and no change to the run loop to add call/ret — they reuse the
existing `EXIT_BRANCH` ABI. This lifts the `NonSyscallExit` ceiling that
previously halted the loop on any `ret` or indirect transfer.

## Motivation

Before this, the run loop (RFC 0019) chained only relative branches; a `ret`
ended its block with a *host* return, and the loop reported `NonSyscallExit`
because it could not follow a dynamic target. Indirect `call`/`jmp` emitted a
host `blr`/`br` to the guest PC *as a host address* — a wild jump. Real programs
are full of function calls, returns, function pointers, and vtables, so the loop
could not run anything past its first `ret`. RFC 0020 made the guest stack real
host memory; this RFC builds the control-flow layer on top of it.

## Context

- The run loop executes one translated block, then resumes by `exit_reason`
  (`EXIT_NORMAL` fall-through, `EXIT_SYSCALL`, `EXIT_BRANCH`). `EXIT_BRANCH`
  resumes at `CpuStateFrame::next_pc` (RFC 0019).
- The fused translator lowers with the **branch-exit ABI**
  (`ExitAbi::branch_via_frame`): relative branches store the taken PC in
  `next_pc` + mark `EXIT_BRANCH` instead of branching to a sibling label
  (single host-wrapped block, no sibling exists).
- RFC 0020: every guest memory access is rebased to `host = mem_base + guest_va`,
  so the guest stack is addressable host memory.
- Invariant: a terminator runs through the full AAPCS64 block epilogue
  (`emit_branch_exit` → `emit_block_epilogue_and_ret`) before returning to the
  host — a bare `ret` would corrupt the host stack and break the JIT unmap.

## Considered alternatives

1. **Guest stack as the return-address stack (chosen).** `call` pushes
   `return_guest_pc` to `[rsp-8]` (rebased through `mem_base`), `rsp -= 8`, and
   block-exits to the target; `ret` loads the target from `[rsp]`, `rsp += 8`,
   and block-exits there. Pros: matches x86 semantics exactly (a guest that
   manipulates its own return address — setjmp, trampolines, ROP-like patterns —
   just works); zero extra state; reuses `EXIT_BRANCH`. Cons: a load+store per
   call/ret (cheap; the stack is hot in cache).
2. **Host-native call/ret (the prior behaviour).** Let the JIT `bl`/`ret` mirror
   the guest. Rejected: the host return address ≠ the guest return address, the
   guest can't see/modify its own stack return slot, and a `ret` returns to the
   host loop with no way to follow the guest target.
3. **A separate shadow return-address stack (RAS) in the run loop.** Faster
   return prediction, but it desyncs from a guest that writes its stack directly,
   and duplicates state. Deferred as a *pure optimization* layered on top of the
   correct guest-stack model (the `ras_pushes`/`ras_pops` counters anticipate
   this).

## Decision

In branch-exit mode, lower the control-transfer terminators as block exits:

- `CallRel` / `CallReg`: `rsp -= 8`; store the return address at `[rsp]` (rebased);
  `EXIT_BRANCH` to the target (a constant for `CallRel`, a value register for
  `CallReg`, captured into the exit register before the push clobbers scratch).
- `Return` / `RetAdjusted`: load the target from `[rsp]` (rebased); `rsp += 8`
  (or `pop_bytes`); `EXIT_BRANCH` to the target.
- `JumpReg`: `EXIT_BRANCH` to the target register (no stack change).

The non-branch-exit (fused intra-block) path is unchanged: `CallRel` tail-jumps
to a sibling label, `Return` emits the host epilogue/ret.

Separately, **state-touching syscalls are serviced in the run loop, not the
`dispatch` layer.** `arch_prctl` (FS/GS segment base) sets/reads
`CpuStateFrame::fs_base`/`gs_base`, which the JIT reads via FS/GS-relative
addressing; `dispatch` sees only `ctx`/`mem`, so the run loop special-cases it.

## Consequences

- **Benefits:** real programs with functions/recursion/function-pointers run end
  to end; the `NonSyscallExit` ceiling is lifted for call/ret/indirect; exact x86
  stack semantics; TLS via `arch_prctl` works.
- **Costs:** a load/store per call/ret; the run loop now has an `arch_prctl`
  special-case (a documented pattern for state-touching syscalls).
- **Reversibility:** gated on `ExitAbi::branch_via_frame`; the host-native path is
  preserved for the fused mode, so reverting is local to the lowerer arms.

## Resource discipline (standing clause)

This layer allocates **no** OS resources — it only reads/writes the guest stack
that `BackedAddressSpace` already owns (freed on drop, RFC 0020). The translated
blocks' W^X buffers remain owned by the cache (RFC 0017), unmapped on eviction /
session drop. The full epilogue on every terminator preserves the host stack so
the JIT `munmap` stays correct.

## Implementation notes

Landed across PRs #305 (direct call/ret), #306 (indirect call/jmp), #307 (nested
calls e2e), #308 (`arch_prctl`). Lowerer helpers: `lower_call_rel_exit`,
`lower_return_exit`, `lower_call_reg_exit`, `lower_jump_reg_exit`
(`shell/prisma-backend/src/lowerer.rs`). ARM64 e2e: `run_call_ret_e2e`,
`run_tls_e2e` (`shell/prisma-session/tests/`).

Two latent **decoder** correctness bugs were found by the executing ARM64 e2e's
and fixed alongside (both pre-existing, untested paths):

- SIB index field `0b100` is *no index* in x86 (unless REX.X); it had decoded as
  an index equal to the base, doubling it (`[rsp-8]` → `2*rsp-8`). Fixed in #304.
- Near indirect `call`/`jmp` force a 64-bit target operand; without REX.W the
  target was truncated to 32-bit, losing a >4 GiB guest PC. Fixed in #306.

Lesson: `ffi-link-arm64` is **not** a required check, so an ARM64-only failure can
merge silently — verify it green by hand before closing an execution milestone.

## Open questions

- A shadow RAS (alternative 3) as a return-prediction optimization, kept in sync
  with the guest stack.
- Indirect transfers that miss the translation cache repeatedly (e.g. a hot
  vtable) may want inline caching at the block exit.

## References

- RFC 0017 (guest execution session), RFC 0019 (OS-ABI execution loop),
  RFC 0020 (guest memory addressing).
- PRs #304, #305, #306, #307, #308.
