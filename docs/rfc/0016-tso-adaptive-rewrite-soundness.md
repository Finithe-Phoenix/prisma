---
id: 0016
title: TSO-adaptive rewrite soundness (F1-LN-016) — plan
status: draft
authors: [Claude]
created: 2026-06-20
updated: 2026-06-20
supersedes: []
superseded_by: null
---

# RFC 0016: TSO-adaptive rewrite soundness (F1-LN-016)

## Summary

Scopes the remaining Pillar-2 work: a machine-checked proof that the
TSO-adaptive rewrite (Pillar 3) — which drops `DMB ISH` / downgrades TSO
memory ops to relaxed ones where it is unobservable — **preserves the program's
observable behaviour**. This is the formal guarantee that makes the headline
optimisation safe to ship, and the part of the épico that distinguishes Prisma
from every other DBT.

This RFC turns "F1-LN-016 is a large effort" into a concrete plan, building on
the state-level TSO model already landed.

## Background: what already exists

`ir-spec/PrismaIR/TSO.lean` is a standalone, machine-checked x86-TSO weak-memory
model (Sewell et al.): per-core FIFO store buffers, store forwarding, FIFO
publication, and the single relaxation (W→R). It proves **13 lemmas, 0 sorries**,
covering:

- **F1-LN-015 ordering axioms** — `load_after_fence` (a fence restores W→R),
  `fence_publishes_two` (W→W preserved in program order).
- **F1-LN-016 soundness cores** — `load_unaffected_by_fence` (a core's own load
  is unchanged by its own fence → fence-drop is sound for single-threaded
  regions) and `load_unaffected_by_propagate` (drain *timing* is unobservable to
  the issuing core).
- **Inter-core visibility** — `load_store_other` (buffered stores are private),
  `store_visible_to_idle_core` (fenced stores publish), plus the full
  core-isolation triad (`store`/`fence`/`propagate`_other_sb).

The gap: `PrismaIR.MachineState` gives the IR a **sequentially-consistent**
memory (`mem : Nat → UInt8`, direct `write64`). The TSO model lives apart, at the
abstract `Addr → Val` word level. F1-LN-016 must connect them and lift the
single-state lemmas to whole-program executions under the rewrite.

## Design choice

Two ways to connect TSO.lean to the IR; this RFC recommends **(A)**.

**(A) Refine the IR memory semantics to TSO, then prove the rewrite is an
observation-preserving simplification.** Add a per-core store buffer to
`MachineState` (or a parallel `TSOMachineState`), give the TSO-flavoured memory
ops (`storeTSO`/`loadTSO`/`fence`, already distinct constructors in
`Syntax.lean`) their buffered semantics, and keep the relaxed ops as direct SC
accesses. The rewrite replaces a `storeTSO`/`fence` with its relaxed form; the
theorem is that, *under the classifier's invariant*, the observable trace is
unchanged.

**(B) Translate IR memory ops into TSO.lean steps** and reason entirely in the
abstract model. Lighter weight but leaves a translation-correctness gap between
the IR's own semantics and the abstract one.

(A) is preferred: it keeps the proof anchored in the IR's actual operational
semantics (the same `Step` relation the constant-prop/DCE proofs use), so the
soundness statement is about the real artefact.

## Proof obligations

1. **Classifier invariant (F25-TS-001).** Formalise the predicate the Pillar-3
   classifier asserts per region — e.g. *single-threaded* (no other core
   observes this core's buffer) or *lock-free with no W→R dependence*. The
   single-threaded case reduces directly to the already-proven
   `load_unaffected_by_fence` / `load_unaffected_by_propagate`.
2. **Per-op rewrite lemma.** Dropping one `fence` (or downgrading one `storeTSO`)
   preserves the observable state, *given* the invariant holds at that point.
3. **Whole-program lift.** Compose (2) over a `Steps`/trace, mirroring the
   `constant_propagate` soundness lift (`F1-LN-010/013`): an observable-trace
   relation, then induction over the rewrite applied pointwise.
4. **Conservative fallback.** Where the invariant is `unknown`, the rewrite is
   the identity → soundness is trivial (no obligation), matching the runtime's
   deny-by-default policy.

## Milestones

- **M1** — `TSOMachineState` + buffered semantics for the TSO memory ops; prove
  it refines SC for a single core (uses the existing soundness cores). *(L)*
- **M2** — classifier invariant + per-op rewrite lemma for the single-threaded
  case. *(L)*
- **M3** — whole-program lift over the trace relation. *(XL)*
- **M4** — lock-free / shared-mutable cases (the harder classifications),
  Pillar-3 paper draft (F25-LN-004). *(XL)*

Sorry budget stays at 0 throughout; each milestone lands fully proven or not at
all. M1–M2 are the next concrete steps; M3–M4 are the research frontier.

## Trade-offs / risks

- Adding a store buffer to `MachineState` touches every existing proof that
  pattern-matches on it; mitigated by introducing a *separate* `TSOMachineState`
  and a refinement lemma rather than mutating the SC state in place.
- The lock-free case (M4) may need a happens-before relation richer than the
  current model exposes; that is the honest research risk and a candidate for a
  negative result if it does not close (principle 4).
