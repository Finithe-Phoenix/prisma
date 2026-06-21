---
id: 0018
title: Region assembly — multi-block lowering so intra-region branches resolve
status: draft
authors: [Claude]
created: 2026-06-21
updated: 2026-06-21
supersedes: []
superseded_by: null
---

# RFC 0018: Region assembly (multi-block lowering)

## Summary

RFC 0017 M2 discovers the statically-reachable control-flow graph from the entry
point (`Session::reachable_blocks`, decode-only). The next milestone is to lower
that whole CFG **as one region** so a relative branch from one block lands on the
ARM64 label of its sibling block — the step that turns a discovered CFG into a
single executable translation. This RFC specifies that region-assembly seam and
the two backend-ABI changes it needs.

The branch lowering itself **already exists** and does not need to be rebuilt
(this corrects a stale note in an earlier session): `prisma-backend`'s
`Lowerer::lower_function` builds a `block.id -> Label` map for every block in the
function (`lowerer.rs:127-132`), then resolves each transfer against it —
`Op::JumpRel` -> `block_label(target_guest_pc)` + `b_label` (`:434`),
`Op::CondJumpRel` -> `b_cond_label` + `b_label` (`lower_cond_jump_rel`,
`:1186-1196`), `Op::CallRel` (`:438`), `Op::Return` via the epilogue (`:453`).
Landed in #56/#57 ("intra-region control flow on ARM64").

## Why the per-block facade cannot do this today

`prisma-translator`'s `translate_decoded` wraps **one** instruction's statements
in a single-block `Function { entry: 0, blocks: [{ id: 0, .. }] }` and calls
`lower_function` (`lib.rs:349-360`). A branch terminator in that lone block has
no sibling block in the label map, so `block_label` returns
`LowerError::MissingTargetBlock`. That is the correct reason `translate_block`
cannot lower a branch-terminated block in isolation — not a missing lowering.
The decode-only `decode_block_successors` / `reachable_blocks` walk stays the
right tool for *discovery*; this RFC is about *lowering* the discovered set.

## Design

A new facade entry point assembles the reachable blocks into one multi-block
`Function` and lowers it once:

```text
Translator::translate_region(entry_guest_pc, blocks: &[(guest_pc, bytes)], max_insns)
  -> Result<RegionTranslation, TranslateError>
```

For each `(guest_pc, bytes)` it decodes the straight-line block (reusing the
`translate_block` decode loop), emits one `BasicBlock` whose `id` is the block's
guest-PC surrogate, sets `Function::entry` to the entry block's surrogate, runs
the optimization pipeline over the whole function, and calls `lower_function`
once. Codex's branch lowering then resolves every intra-region `JumpRel` /
`CondJumpRel` to a real `B` / `B.cond`. Targets that leave the discovered set
(indirect transfers, returns, unresolved externals) stay block exits, handled by
the dispatcher's dynamic path (RFC 0017 M3).

## Backend-ABI blockers (codex territory)

Two changes in `prisma-backend` / the IR must land before `translate_region` is
correct. They are the reason this is a **joint** milestone, not a facade-only one:

1. **64-bit block ids.** `block_label` does `u32::try_from(guest_pc)`
   (`lowerer.rs:1260-1265`), so any guest PC above `u32::MAX` — i.e. every
   realistic Windows image base, e.g. `0x1_4000_0000` — fails to resolve. The
   block-id space must become a `u64`-keyed surrogate (or a dense index assigned
   per region with a `guest_pc -> id` side table), and `JumpRel`/`CondJumpRel`/
   `CallRel` target fields must key into the same space.

2. **Cross-block SSA assembly.** The decoder numbers `Ref`s per instruction;
   `translate_fused` already renumbers refs across instructions within a single
   block via `Op::map_refs`. Region assembly extends that to renumber across
   blocks so a value defined in one block and read in a successor resolves, while
   keeping per-block boundaries for the label map.

Both touch the decoder/IR/backend ABI, which is codex's split. The facade side
(`translate_region`, the `RegionTranslation` type, the discovery-to-assembly
glue) is Claude's and can land behind these once they exist.

## Resource discipline

`RegionTranslation` owns its emitted code bytes; once installed in the
translation cache (RFC 0007) the cache owns the W^X buffer and unmaps it on
eviction / `clear_cache` / session drop, per the mandatory resource clause. No
new long-lived OS resource is introduced — region assembly produces one cache
entry instead of N per-block entries.

## Milestones

- **R1 (codex):** 64-bit block-id surrogate in `block_label` + branch target
  fields; existing single-block tests stay green.
- **R2 (codex):** cross-block SSA ref assembly across a multi-block `Function`.
- **R3 (Claude):** `Translator::translate_region` + `RegionTranslation` + a
  `Session::translate_reachable_region` that feeds `reachable_blocks` into it;
  exact-encoding test that a two-block `JMP` region lowers to one `B` between
  the bound labels.
- **R4 (codex):** dispatcher installs the region translation and drives dynamic
  PC dispatch for the block exits that leave the region (RFC 0017 M3).
