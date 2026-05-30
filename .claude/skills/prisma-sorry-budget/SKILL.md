---
name: prisma-sorry-budget
description: Verify the Lean 4 IR spec still builds and the sorry count stays within ir-spec/.sorry-budget. Mirrors the CI gate in .github/workflows/ir-spec.yml. Use after any edit to ir-spec/, before claiming F1-LN-* or any Pillar 2 work as done.
---

# Prisma Lean sorry budget

The `ir-spec/` Lean 4 formalization is Pillar 2's correctness ground truth. CI enforces a `.sorry-budget` cap — the number of unproven `sorry` placeholders allowed in `PrismaIR/*.lean`. Adding a sorry without bumping the budget fails CI. This skill runs the same gate locally so you catch it before the push.

## What the gate does (.github/workflows/ir-spec.yml)

1. Installs elan / Lean 4 using the toolchain pinned in `ir-spec/lean-toolchain`.
2. Runs `lake build` in `ir-spec/`. Any type-check failure fails the job.
3. Counts `sorry` occurrences across `PrismaIR/*.lean`:
   ```bash
   grep -rn --include='*.lean' '^[[:space:]]*sorry\b' PrismaIR | wc -l
   ```
4. Compares against `ir-spec/.sorry-budget`. Strictly greater fails; strictly less emits a warning to reduce the budget.

## Local invocation

```bash
cd ir-spec
lake build
budget=$(tr -d '[:space:]' < .sorry-budget)
actual=$(grep -rn --include='*.lean' '^[[:space:]]*sorry\b' PrismaIR | wc -l | tr -d ' ')
echo "budget: $budget, actual: $actual"
[ "$actual" -le "$budget" ] && echo PASS || echo FAIL
```

## When invoked, do

1. Read `ir-spec/.sorry-budget` — note the current cap.
2. Run `lake build`. Report any type-check failure with the file:line from lean output.
3. Count sorrys. If over budget, locate each one:
   ```bash
   grep -rn --include='*.lean' '^[[:space:]]*sorry\b' ir-spec/PrismaIR
   ```
4. Compare against the previous count to know which is new (use `git diff` on the touched files).
5. Recommend one of:
   - Finish the proof (preferred).
   - Bump `.sorry-budget` upward and document in the commit why this proof is currently out of reach (e.g. waiting on mathlib import, structural blocker, scoped to a later F1-LN-* item).
   - Revert the offending change.

## Sorry-bumping policy

A new `sorry` is sometimes correct — the proof depends on a lemma not yet in mathlib, or formalizes a theorem that won't land until later in Fase 1. In that case:

- Same PR bumps `.sorry-budget` by exactly +1 per added sorry.
- Commit message names the lemma/theorem and links to the backlog item that will retire it (e.g. *"depends on F1-LN-008: import Mathlib.Data.Nat.Basic"*).
- Add a comment on the `sorry` line itself: `-- TODO(F1-LN-NNN): finish proof; depends on <lemma>`.

## When a reducer succeeds

If `actual < budget`: the CI warning ("consider lowering the budget") is a real signal. Lower `.sorry-budget` in the same commit that retired the proof. Locking in the win prevents accidental regression later.

## Reading PrismaIR

If you need to understand a sorry's context before touching it:

- `ir-spec/PrismaIR.lean` — module entry.
- `ir-spec/PrismaIR/Syntax.lean` — IR opcodes as `inductive`.
- `ir-spec/PrismaIR/Semantics.lean` — `evalBinOp`, `maskToSize`, `evalPure`.
- `ir-spec/PrismaIR/MachineState.lean` — register file + SSA env.
- `ir-spec/PrismaIR/Lemmas.lean` — current theorem set.

Currently (May 2026): 12 opcodes formalized, 3+ base lemmas, exactly 1 sorry (`maskToSize_idem` in Lemmas.lean) awaiting mathlib wiring. Budget = 1.

## Output format

```
Sorry budget check — ir-spec/

lake build: PASS / FAIL
  <type-check errors if any, file:line>

sorrys: <actual> / <budget>
  - <file:line> (NEW since <ref>) — <lemma name + reason>
  - <file:line> (existing) — <lemma name>

Recommendation: PASS / bump budget to N / finish proof / revert
```
