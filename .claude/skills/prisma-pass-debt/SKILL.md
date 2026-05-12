---
name: prisma-pass-debt
description: Periodic review of the 10-pass IR optimization pipeline (core/src/passes/) for duplication, missed helper reuse, and abstractions that are/are-not warranted. Use quarterly or before Fase 2 boundary to catch debt before it ossifies. Aligned with Prisma's "don't abstract prematurely, but don't tolerate three-copies-of-the-same-thing either" principle.
---

# Prisma pass-pipeline debt review

## Why

The default pipeline in `core/src/passes/pass_manager.cpp` runs 10 passes in order:

```
const_prop → algebraic → strength_reduce → const_prop (2nd) →
redundant_load → CSE → copy_propagate → dead_store →
branch_fold → dead_code_eliminate
```

Plus out-of-default: `flag_write_elimination`, `tail_call_optimise` (F1-PS-015).

Each pass is a pure statement-list transformer. The risk: duplicated walking patterns, ad-hoc IR predicates copied between passes, and missed opportunities to share a helper. Pass count will grow toward 15-20 in Fase 2 (SIMD, atomics, TSO-adaptive). Pay the debt now.

## What to look for

### 1. Duplicated walks
Each pass typically iterates statements and rewrites in-place. Look for repeated patterns:
- The same "find use sites of Ref N" walk in multiple files.
- The same "is this op a no-op" check.
- The same "replace operand X with constant K" mutation.

If a pattern appears in ≥3 passes verbatim: extract to `passes/util.hpp` or wherever Claude's IR helpers live. Two copies is fine; three is debt.

### 2. Inconsistent predicates
`is_constant(ref)`, `is_pure(op)`, `evaluates_to_zero(op)` — these are predicates that ALL passes need. If two passes disagree on the definition of "pure," that's a correctness bug waiting.

Audit: grep for predicate-like inline lambdas across passes; check they implement the same definition.

```bash
grep -rn "is_\(constant\|pure\|nop\|dead\)" core/src/passes/
```

### 3. Pass-order assumptions baked in
`pass_manager.cpp` runs passes in a fixed monotonic order. Some passes implicitly assume the previous pass ran (e.g. CSE assumes copy_propagate hasn't unified copies the wrong way).

Audit each pass for an opening comment that states:
- What invariant it expects on input.
- What invariant it establishes on output.

Passes missing this comment are debt — they could be reordered or moved out of the default pipeline without anyone noticing the silent breakage.

### 4. Over-abstraction
Just as dangerous as duplication. Look for:
- A "pass framework base class" that one pass overrides differently than another.
- Visitor patterns with three implementations and one call site.
- Helper functions parameterized over five flags where two would suffice.

CLAUDE.md: *"Don't add features, refactor, or introduce abstractions beyond what the task requires. ... Three similar lines is better than a premature abstraction."*

### 5. Missing benchmarks
Each pass should have a `[.benchmark]`-tagged Catch2 case in `core/tests/test_benchmarks.cpp` measuring its per-statement cost on a synthetic IR. If a pass lacks one, performance regressions are invisible until Fase 2 benchmarks hit them.

### 6. Default-pipeline inclusion criterion
Two passes exist but are not in the default pipeline: `flag_write_elimination`, `tail_call_optimise`. For each: is the omission deliberate (still maturing) or stale (should be in now)?

Look at: opt-in API in `passes.hpp`, recent test coverage, blog/RFC notes.

## Process

1. Run a structural scan:
   ```bash
   ls core/src/passes/
   wc -l core/src/passes/*.cpp
   grep -n "TODO\|FIXME\|XXX\|HACK" core/src/passes/
   ```

2. Read each pass header (file top + first function). Note its declared invariant.

3. For each pair of passes that operate on the same IR feature (e.g. const_prop + algebraic both fold constants): diff their helper functions, check if duplication is real or just superficial.

4. Run pass-pipeline tests with timing dump:
   ```bash
   core/build/prisma_core_tests "[passes]" --reporter compact
   ```
   Look for per-pass `_ns` values way outside the others.

5. Output: a table of findings, sorted by severity.

## Severity bands

- **HIGH** — Two passes disagree on a predicate, or a pass-order assumption is implicit and not documented. Correctness risk.
- **MED** — ≥3 copies of the same helper. Refactor opportunity.
- **LOW** — Out-of-default pass that should be in (or vice versa). Naming / cleanup.
- **INFO** — Pass missing benchmark, missing invariant comment.

## Output format

```
Pass-pipeline debt review — <date>

Default pipeline (10 passes): <count of files, total LOC, total benchmark coverage %>
Out-of-default: <list>

Findings:
1. [HIGH] <file:line + file:line> — <description> — <fix>
2. [MED] core/src/passes/{a,b,c}.cpp — same is_constant lambda 3×. Extract.
3. [LOW] flag_write_elimination not in default pipeline — review whether F1-PS-NNN should land it.
4. [INFO] strength_reduce.cpp has no benchmark — add [.benchmark] case.

Recommendation: <claim F1-PS-NNN to refactor / open RFC for pipeline shape change / status-quo>
```

Run this quarterly. Before Fase 2 starts (sem 33+), do a full sweep so the SIMD/atomics passes land into a clean foundation.
