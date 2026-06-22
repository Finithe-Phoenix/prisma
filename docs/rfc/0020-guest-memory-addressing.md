---
id: 0020
title: Guest memory addressing — host rebasing for JIT loads and stores
status: draft
authors: [Claude]
created: 2026-06-22
updated: 2026-06-22
supersedes: []
superseded_by: null
---

# RFC 0020: Guest memory addressing — host rebasing for JIT loads and stores

## Summary

Until now the Rust JIT lowered a guest memory access (`LoadMem`/`StoreMem` and
their TSO variants) to a bare `LDR/STR Rt, [Raddr]`, dereferencing the guest
virtual address *as a host address*. That only works when the guest address
space is identity-mapped onto the host — which the Vec-backed
`BackedAddressSpace` (RFC 0019) is not: each guest region owns a `Vec<u8>` at an
arbitrary host heap address, so a guest VA like `0x400000` points nowhere valid
on the host.

This RFC introduces **host rebasing**: every guest memory access is lowered to

```
host_addr = mem_base + guest_va
```

where `mem_base` is a per-run base held in `CpuStateFrame::mem_base` (byte offset
832, mirrored by the lowerer's `MEM_BASE_OFFSET`). A `mem_base` of `0` reproduces
the legacy `host == guest` identity behaviour, so the change is backward
compatible: GPR-only blocks and all existing fixtures are unaffected, and the new
field defaults to `0`.

## Motivation

This is the "Stage 2B" capability on the critical path to running real code: a
guest program that touches memory must reach bytes the host actually owns. The
run loop allocates guest memory as host buffers; the JIT needs a cheap, correct
way to translate a guest pointer into the host pointer for that buffer.

## Design

### Lowering

`emit_load_mem` / `emit_store_mem` (in `prisma-backend`) now prefix each access
with a rebase sequence:

```
LDR  x24, [x27, #MEM_BASE_OFFSET]   ; x24 = mem_base   (x27 = CpuStateFrame*)
ADD  x24, Raddr, x24                ; x24 = guest_va + mem_base = host_addr
LDR/STR Rt, [x24]                   ; the access, off the rebased host address
```

`x24` (`MEM_ADDR_SCRATCH`) is outside the value-register pool (x9..x16), so it
never aliases the `addr`/`value`/`dst` operands, and inside the prologue's
callee-saved set, so the block body may freely clobber it. `mem_base` is reloaded
per access rather than pinned in a register for the whole block — this keeps the
change localized to the two memory-emit helpers (no prologue/ABI change) at the
cost of one extra `LDR` per access. Pinning `mem_base` in a callee-saved register
at block entry is a future optimization (it would touch `abi::emit_block_prologue`
and is deferred until the hot path warrants it; correctness first).

### Frame layout

`CpuStateFrame` gains `mem_base: u64` at byte offset 832, immediately after
`next_pc` (824), inside what was previously tail padding. The lowerer and the
executor share the offset by construction (the existing pattern for `fs_base`,
`cf`, `exit_reason`, `next_pc`), and a layout test pins it.

### Resource discipline

`mem_base` is a plain integer — it owns nothing. The host arena it points at is
owned by the run loop's `BackedAddressSpace` (RFC 0019), whose regions free their
`Vec<u8>` on `unmap`/`clear`/drop. This RFC adds no new OS resource and therefore
no new `Drop` obligation; it only changes how an *existing* owned buffer is
addressed from JIT code.

## Alternatives considered

1. **Identity mapping with `MAP_FIXED` at the guest VAs.** Reserve a host mapping
   *at* the guest virtual addresses so `guest_va == host_va` and no rebasing is
   needed. Rejected as the default: `MAP_FIXED` can collide with the host
   process's own mappings (the translator, the Rust runtime, ASLR'd libraries),
   and it does not generalize to running two guests in one host process. It
   remains attractive for a single-guest, address-space-isolated deployment and
   can be layered on later (it is simply `mem_base == 0` with a fixed mapping).
   **This is the open question for Danny** — see below.

2. **Per-region base.** Look up the containing region per access and add its
   specific base. More general (handles a fragmented guest address space) but
   needs a guest-side region table consulted from JIT code or a guard/fault path;
   heavier than the single contiguous-arena model the run loop uses today.

3. **Pin `mem_base` in x28 at block entry.** Strictly faster (one load per block
   instead of per access) but requires an ABI/prologue change. Deferred as a perf
   optimization once the arena wiring (next step) lands.

## Open question (Danny)

The single-`mem_base` model assumes the guest address space is one contiguous
host arena. That holds for the user-mode bring-up (one process, a heap that grows
via `brk`/`mmap` within one arena). For multi-region or `MAP_FIXED` identity
mapping we would revisit (alternative 1/2). No action needed now — the default
`mem_base == 0` is identity, and the contiguous-arena wiring is the next PR.

## Status / follow-up

This PR lands the lowering + frame field + tests (an ARM64 e2e proving a non-zero
`mem_base` rebases a load and a store into a host arena). The follow-up wires the
run loop to allocate the contiguous arena from `BackedAddressSpace` and set
`state.mem_base`, with an end-to-end guest program that reads and writes real
guest memory through the DBT.
