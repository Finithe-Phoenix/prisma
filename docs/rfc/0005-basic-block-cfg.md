---
id: 0005
title: Basic block + CFG representation for Prisma IR
status: draft
authors: [Danny]
created: 2026-04-20
updated: 2026-04-20
supersedes: []
superseded_by: null
---

# RFC 0005: Basic block + CFG representation

## Summary

Today Prisma's IR is a flat `std::vector<ir::Stmt>` per translated
region. That has carried us through Fase 1 for single-block
translations but blocks several planned items (CFG simplification,
cross-block register allocation, loop-based optimisations, dominator
analyses). This RFC proposes the `BasicBlock` + `Function`
representation, a CFG builder that consumes the current flat form,
and a migration plan that lets the existing single-block passes keep
running unchanged while new CFG-aware passes come online.

**Status: draft.** Captures the design before implementation. No
code landed yet. When the first CFG pass lands, status flips to
`accepted` and we backfill `updated` with the implementation commit.

## Motivation

Several F1 items are blocked on a real CFG:

- **F1-IR-021/022/023** — define `BasicBlock`, `Function`, and a CFG
  builder.
- **F1-IR-024/025** — dominator tree + loop detection. Neither makes
  sense over a flat stmt list.
- **F1-BK-003/004/006** — Jump / CondJumpFlags lowering + block-level
  emission with label fix-up. Possible in principle over the flat
  form but would conflate translation units with control flow and
  make the allocator's CFG-aware upgrade (open question in RFC 0006)
  harder.
- **F1-PS-013** — CFG simplification (remove empty blocks). Trivially
  impossible without a CFG.
- **F1-PS-015** — tail-call optimisation. CFG-level pattern.

We also hit a runtime pain point: `CondJumpRel` and `JumpRel` use
guest-address sentinels in x0 as the "next PC" convention. This works
for the Fase 1 dispatcher but means every branch pays a round trip
through the dispatcher even when the branch target is another block
we just translated. A CFG lets a pass stitch direct ARM64 branches
between blocks that share the same translation region.

## Context

- **Current IR (RFC 0001).** `ir::Stmt` is `(optional<Ref>, Op)` with
  SSA-by-construction: each Ref is defined exactly once and its def
  precedes all uses. A translation is a `std::vector<Stmt>`.
- **Terminators already exist at stmt level.** `Jump`, `JumpRel`,
  `JumpReg`, `CondJump`, `CondJumpRel`, `Return` — these end a block
  even in the flat list. The CFG builder has a clear edge-detection
  rule to work from.
- **The Lowerer's liveness pass is block-local today.** Linear-scan
  is designed around "one block, last_use is within the vector". CFG
  adds cross-block lifetime (a Ref defined in block A and used in
  block B). Cross-block is the planned upgrade (RFC 0006 open
  question 1) but we want the representation to land first so the
  upgrade can be a follow-up.
- **Existing passes.** All nine current passes
  (`constant_propagate`, `dead_code_eliminate`, `algebraic_simplify`,
  `common_subexpression_eliminate`, `copy_propagate`,
  `strength_reduce`, `redundant_load_eliminate`,
  `dead_store_eliminate`, `branch_fold`) are intra-block and pure.
  They must keep working.

## Considered alternatives

### 1. Keep the flat list; attach CFG metadata externally
A side table `flat_index → block_id` plus a `CFG` struct that maps
blocks to their successors.

- **+** Existing passes don't move.
- **+** Cheap to build.
- **−** Dual representation splits invariants across two structures;
  easy to desync on rewrites.
- **−** Adding a stmt in the middle of a block requires updating both.

### 2. Re-representation via `std::vector<BasicBlock>` (accepted)
A `BasicBlock` owns a `std::vector<Stmt>` and successor IDs. A
`Function` owns the blocks plus an entry ID. The CFG is implicit in
the blocks' terminators.

- **+** Invariants live in one place (the terminator's target).
- **+** Passes convert trivially: a block-level pass is the old
  stmt-list pass run per block.
- **+** Pretty-printing a block is the obvious thing.
- **−** The flat form is still what tests and older passes want. We
  need a `flatten(Function) → vector<Stmt>` and
  `build_cfg(vector<Stmt>) → Function` for backward compatibility.
- **Verdict:** accepted. The round-trip is cheap and the conversion
  shields existing passes during migration.

### 3. LLVM-style IR graph
Each `Instruction` holds pointers to its operand instructions (use-
def) and block it's in. Full graph mutation API.

- **+** Maximum flexibility for aggressive optimisations.
- **−** Pointer-heavy IR is hostile to Lean verification (we'd need
  a separate Lean representation that serialises back-and-forth).
- **−** Major rewrite of everything built so far.
- **Verdict:** rejected. Prisma's SSA-plus-flat-vector gets us most
  of the optimisation mileage at a fraction of the complexity;
  matching LLVM's shape isn't a goal.

## Decision

We adopt alternative 2. Concrete plan below.

### Data shape

Existing types in `prisma/ir.hpp` already declare `BasicBlock` and
`Function`:

```cpp
struct BasicBlock {
    std::uint32_t       id{0};
    std::vector<Stmt>   stmts;
};

struct Function {
    std::vector<BasicBlock> blocks;
    std::uint32_t           entry{0};
};
```

We extend `BasicBlock` with explicit successor IDs (not derived from
terminators) so builders can populate them once and passes can trust
them:

```cpp
struct BasicBlock {
    std::uint32_t            id{0};
    std::vector<Stmt>        stmts;
    std::vector<std::uint32_t> successors;  // block ids
};
```

Rules:

- Every non-terminator block ends in exactly one terminator op
  (`Jump*`, `CondJump*`, `Return`). Validator (F1-IR-016) extended
  to enforce this.
- `successors` has size 0 (terminator is `Return`) or 1
  (unconditional `Jump`) or 2 (`CondJump`: `[fallthrough, taken]`).
  `JumpReg` has 0 successors because the target is computed; the
  dispatcher owns re-entering.
- `id` is unique within its `Function` and matches its index in
  `Function::blocks`. The CFG builder always emits IDs == index; no
  sparse IDs in MVP.

### CFG builder

`build_cfg(const std::vector<Stmt>&) → Function`:

1. Scan stmts once, marking block boundaries at:
   - Any terminator (end-of-block).
   - Any stmt that is the target of an earlier forward jump (start-
     of-block). Backward jumps are detected on the second pass since
     we don't know target addresses on the first pass for
     `CondJumpRel` etc. — we use `target_guest_pc` and a
     guest-pc-to-stmt-index map seeded during decoding.
2. Emit a `BasicBlock` per contiguous span between boundaries.
3. Resolve successor edges from the terminator:
   - `JumpRel{pc}` → single successor = block starting at `pc`.
   - `CondJumpRel{cc, pc_t, pc_ft}` → `[block(pc_ft), block(pc_t)]`.
   - `Jump{block_id}` / `CondJump{cond, block_id, …}` — direct IDs,
     copy through.
   - `Return` / `JumpReg` — no successors.
4. Entry = block 0 (the first stmt becomes the entry block).

The current decoder emits `JumpRel` / `CondJumpRel` with guest PCs,
not block IDs. The builder is the translation point: it converts
guest PCs to block IDs by consulting a `guest_pc → block_id` map it
builds while splitting. Post-CFG passes always see block IDs.

### Flatten

`flatten(const Function&) → std::vector<Stmt>`:

Emits blocks in ID order, concatenating their stmts. The result is
the same shape the current passes and Lowerer already consume. Used
only during migration; once all passes are block-aware we'll retire
this.

### Migration plan

Staged so nothing goes red at once.

- **Stage A — representation.** Add the `successors` field, build the
  `build_cfg` / `flatten` pair, update the validator to check block
  integrity. No caller changes. (F1-IR-021, F1-IR-022, F1-IR-023.)
- **Stage B — block-iterating passes.** Introduce a
  `for_each_block(Function&, Pass)` helper that runs a stmt-list pass
  over each block in turn. Existing passes become `Pass`es in this
  sense with zero code change. `PassManager` grows a sibling
  `run(Function) → Function` that prefers the block-level path.
- **Stage C — CFG-aware passes.** Branch folding can already collapse
  `CondJumpRel(const, const) → JumpRel`; under CFG it can also
  delete the then-unreachable successor. CFG simplification (F1-PS-
  013) removes empty blocks and merges single-successor chains.
  Loop detection (F1-IR-025) is a CFG pass from day one.
- **Stage D — cross-block register allocation.** Linear-scan with
  holes-at-branches (see RFC 0006 open question 1). Requires all
  downstream passes + the Lowerer to speak the Function form.

Each stage is shippable independently. Stage A is a pure additive
change; Stage B uses the same byte-for-byte passes; Stages C and D
unlock real wins.

## Consequences

### Benefits

- Unblocks seven backlog items (F1-IR-021/022/023/024/025,
  F1-PS-013/015).
- Makes it straightforward to emit direct ARM64 branches between
  same-region blocks, cutting dispatcher round-trips on hot loops.
- Dominator + loop analysis set the stage for the NPU-assisted pass
  (Pillar 1) which wants per-loop features.
- Validator gets real block-integrity checks: exactly-one
  terminator per block, successor IDs in range, reachable entry.

### Costs

- ~300 LOC of new code (builder + flatten + validator delta) before
  any CFG-aware pass lands.
- Pretty-printer grows a block-aware form.
- Existing tests' snapshots touch `vector<Stmt>`. They keep working
  thanks to `flatten()`, but any test that wants to exercise the CFG
  shape needs to be rewritten to produce a `Function`.
- Lean spec (F0-LN-002, F0-LN-003) gains `BasicBlock` and
  `Function` as first-class syntactic forms. That's already on the
  roadmap and doesn't depend on this RFC.

### Reversibility

High early on, low once Stage D ships. Before Stage D we can keep
flatten/build_cfg symmetric and drop the Function layer without
breaking Lowerer callers. After Stage D, the allocator assumes
block structure and reverting is a week of work.

## Implementation notes

Files the implementation will touch:

- `core/include/prisma/ir.hpp` — extend `BasicBlock`, add
  `build_cfg` / `flatten` declarations.
- `core/src/ir/build_cfg.cpp` — new; ~150 LOC.
- `core/src/ir/validate.cpp` — add block-integrity checks.
- `core/include/prisma/passes.hpp` — add
  `PassManager::run(const Function&)` overload.
- `core/tests/test_ir_cfg.cpp` — new; round-trip
  flat → build_cfg → flatten, block-splitting on each terminator,
  successor-edge correctness.

Stage A should land as one PR. Stages B/C/D are separate claims in
the backlog.

## Open questions

- **Irreducible CFGs.** x86 has indirect branches via `JumpReg`
  which can create irreducible loops. MVP treats `JumpReg` blocks
  as exit-to-dispatcher; the dispatcher re-enters the cache on the
  target address. Later we may pattern-match specific JumpReg idioms
  (jump tables) and build the CFG for them.
- **Exception unwinding.** Guest `#DE`, `#UD`, `#PF` etc. can
  trigger anywhere. MVP keeps that at the Lowerer/runtime boundary
  (signal handler). Eventually we may model it as implicit edges,
  similar to LLVM's invoke/landingpad.
- **SSA across block boundaries.** The current SSA invariant is
  per-block. Cross-block SSA requires phi nodes (or block-parameter
  form). Deferred to Stage D — the cross-block allocator will
  settle this.

## References

- Poletto, M. & Sarkar, V. (1999). *Linear scan register allocation.*
  The single-block case. Foundation for the current allocator.
- Cooper, K. D., Harvey, T. J. & Kennedy, K. (2001). *A simple, fast
  dominance algorithm.* Useful once we need dominators (F1-IR-024).
- Click, C. & Paleczny, M. (1995). *A simple graph-based
  intermediate representation.* Comparison point for alternative 3;
  why we're not going graph-of-instructions.
- RFC 0001 — IR SSA form, the invariant this RFC preserves.
- RFC 0006 — register allocator; its "CFG-aware allocation" open
  question becomes concrete once this RFC's Stage D lands.
