---
id: 0017
title: Guest execution session — load → translate → execute wiring
status: draft
authors: [Claude]
created: 2026-06-20
updated: 2026-06-20
supersedes: []
superseded_by: null
---

# RFC 0017: Guest execution session (load → translate → execute)

## Summary

The Rust shell now has every piece needed to run a guest block end to end, but
nothing wires them together. The PE loader (`orchestrator::load_pe`) produces a
mapped, relocated, import-bound image; the translator (`prisma-translator`)
turns guest bytes at an address into an ARM64 block; the runtime
(`prisma-runtime::execute_block`) runs that block on an ARM64 host. This RFC
scopes the **execution session**: the component that drives that loop —
load → translate the entry block → execute → handle the exit → translate the
next block — and the crate-topology change it requires.

## Motivation

The three layers were built independently and, by design, do not depend on each
other:

- `orchestrator` → no dep on `prisma-translator` or `prisma-runtime`.
- `prisma-translator` → `prisma-ir`/`decoder`/`passes`/`backend`/`cache`; **no
  `prisma-runtime`** (it lowers to ARM64 bytes, it does not execute them).
- `prisma-runtime` → no dep on the other two.

So there is no place today where a loaded `MappedImage`'s `entry_pc` becomes a
running guest. Closing that gap is the step from "execution-ready image" to
"executing guest" — the next real milestone after the loader pipeline.

## Design

A new crate **`prisma-session`** depending on `orchestrator`,
`prisma-translator`, and `prisma-runtime` (the only place those three meet).
Per the repo rule, this dependency fan-in is justified here and nowhere else:
the session is the top of the shell, the composition root.

### The execution-loop seam already exists

`prisma-runtime::dispatcher::run_with_adapters(entry_pc, fetch, translate,
cache, guard)` is the seam (its own doc calls it "the migration seam that later
grows into `Dispatcher::run()`"). It drives the
fetch → cache-probe → translate → install loop and owns the guest-PC stepping —
which is why `CpuStateFrame` carries no `RIP`: the PC lives in the dispatcher,
not the register frame. So the session does **not** write a new loop; it supplies
two adapters and calls the existing one:

- a **`Fetch`** adapter (`guest_pc -> Option<Vec<u8>>`) reading from the
  `MappedImage` `load_pe` produced;
- a **`Translate`** adapter (`guest_pc, bytes -> Option<Vec<u8>>`) delegating to
  `prisma-translator::Translator::translate_block` (decode → optimize → lower →
  cache).

This keeps the cross-crate surface tiny: the session is mostly two trait impls
plus ownership/teardown, and the heavy lifting stays in the layers that already
exist and are tested.

### Execution loop

```
let image   = orchestrator::load_pe(bytes, &modules)?;   // mapped + import-bound
let mut cpu = CpuStateFrame::at(image.entry_pc, stack);  // initial register state
loop {
    let guest_pc = cpu.pc();
    let block    = translator.translate_block(image.bytes_at(guest_pc))?; // ARM64
    match runtime.execute_block(&block.code, &mut cpu)? {
        Exit::Continue(next) => cpu.set_pc(next),     // fall-through / direct branch
        Exit::IndirectJump   => { /* resolve via cpu.pc() */ }
        Exit::Fault(fault)   => deliver_seh(&mut cpu, fault, &image)?, // RFC-0017bis
        Exit::Halt           => break,
    }
}
```

The translator's block cache (RFC 0007) means each `(guest_pc, content_hash)` is
translated once; the session only re-enters the cache on a miss or SMC
invalidation.

### Exit ABI

`execute_block` already returns through the block epilogue (Op::Return) into the
dispatcher. The session reads the resulting `CpuStateFrame` to decide the next
action. Block terminators that need session help — indirect jumps, calls into
unresolved imports, syscalls, faults — surface as distinct `Exit` variants
rather than the runtime guessing.

### Fault delivery

A host fault during `execute_block` (SIGSEGV/SIGILL/…) is classified by
`prisma-runtime::guest_exception` → an NTSTATUS code, packaged by
`exception_record` into the 152-byte payload, and the session pushes it onto the
guest stack and redirects `cpu.pc()` to the guest's SEH dispatcher. The
classification/record machinery already exists (#87/#93/#96); the session is
what places it and unwinds. Full SEH dispatch (handler search) is a follow-up.

## Resource discipline (mandatory clause)

The session owns the executed program's OS resources and frees them
deterministically on drop / shutdown, never leaking across a restart:

- The translated blocks' **W^X JIT buffers** (`JitSlabPool` / `ExecBuffer`) are
  owned by the translator's cache and unmapped on eviction / `clear_cache`; the
  session drops the translator, draining the cache.
- The **guest mapping** (`MappedImage` bytes + any `AddressSpace` regions) is
  owned by the session and freed on drop.
- The session's `Drop` runs flush → close → unmap explicitly; it does not rely
  on process exit (a restart may not be a clean exit).

## Milestones

- **M1** — `prisma-session` crate + the load → translate-one-block → execute-one
  -block happy path on an ARM64 host (gated like `executor`'s `is_arm64`).
- **M2** — the multi-block loop with the cache, direct branches, and `Halt`.
- **M3** — indirect jumps / calls (including into resolved imports via the IAT).
- **M4** — fault delivery (push `EXCEPTION_RECORD`, redirect to SEH); handler
  search is RFC-0018.

## Open questions

- Where does the initial stack / TEB/PEB setup live — session or a dedicated
  loader step feeding the session? (Leaning: a `GuestThread` init in the
  session.)
- Ordinal import resolution (skipped by `load_pe` today) before M3.
