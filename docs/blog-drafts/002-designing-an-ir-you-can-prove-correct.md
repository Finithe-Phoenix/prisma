---
title: "Designing an IR you can prove correct"
status: draft
author: Danny
date: 2026-04-20
tags: [ir, lean4, formal-verification, dbt, ssa]
---

# Designing an IR you can prove correct

The first post in this series walked a single `add rax, rbx` through
Prisma's pipeline. The passes rewrote the IR ten different ways and
the emitter produced six ARM64 words. A question I did not answer:
how do we know the passes didn't lie?

"The tests passed" is not enough for the kind of optimisations a
DBT needs to ship. The TSO-adaptive pass (Pillar 3) will downgrade
acquire/release loads to plain loads whenever it can prove a region
is single-threaded. A single miscompile in that pass is a weeks-to-
debug data race in a multithreaded guest binary. Unit tests find
this only if they happen to exercise the exact race — and they
don't, because the race doesn't reproduce deterministically.

So: we prove it. In Lean 4. Formally. That decision shaped the IR.

## What "prove correct" means for an IR

Tests are existential: for these inputs, the output matches the
oracle. Proofs are universal: for *every* input satisfying the
preconditions, the output is correct.

For a pass `P : IR → IR`, "correct" means observable-equivalence:

```
∀ stmts s, ∀ initial machine-state m,
    observed(interpret(s, m)) = observed(interpret(P(s), m))
```

where `observed` projects out just the guest-visible state (GPRs,
memory, flags — the things the guest can read). If that equality
holds for every `s` and `m`, then no program can tell whether we
ran the original or the optimised version. That's what gets to ship.

Writing that statement in Lean requires every piece of the LHS and
RHS to be a mathematical object the prover can reason about. "A
vector of C++ variants" is not such an object. So the IR had to be
designed from the proof side up.

## Decision: SSA

Prisma's IR uses static single assignment. Every `ref` is defined
exactly once, and its defining statement precedes every use. In
Lean this becomes:

```lean
inductive Stmt where
  | const  (r : Ref) (value : UInt64) (size : Size)
  | binop  (r : Ref) (op : BinOpKind) (a b : Ref) (size : Size)
  | store  (reg : Gpr) (v : Ref) (size : Size)
  -- ...

def evalPure (env : Ref → Option UInt64) : Op → Option UInt64
  | Op.const v _       => some v
  | Op.binop op a b sz =>
      env a >>= fun x =>
      env b >>= fun y =>
      some (maskToSize (evalBinOp op x y) sz)
  -- ...
```

The `Ref → Option UInt64` environment is a total function from refs
to "the value bound to this ref if any." It's the cleanest way to
model SSA's "each ref defined at most once" invariant in a language
where inductive types + total functions are first-class.

Two payoffs:

1. **Dataflow is arithmetic.** The passes are substitutions over
   this environment. Constant propagation reads to *update* the
   env; DCE reads to *filter* statements based on it.
2. **Proofs are structural induction.** "For every Stmt" becomes
   case analysis over the `inductive Stmt` — Lean's type checker
   runs the exhaustiveness check for you.

We rejected the traditional "virtual register" IR (where a mov Rax,
Rbx writes *into* a reused name) because it forces Lean proofs to
drag a version number around on every access. The SSA-with-refs
form pays a small memory tax (every result is a fresh Ref) and buys
us clean proofs. RFC 0001 has the full argument.

## The semantics function is the contract

Each IR op has a line in `evalPure` (for the pure fragment) or a
step relation `⟶` (for the side-effecting ops: stores, jumps,
fences). Those are the *only* specs. A pass is correct iff its
output evaluates to the same thing as its input.

We already landed three base lemmas (`PrismaIR/Lemmas.lean`):

- **Determinism** — `evalPure` is a function. Same input, same
  output, always. Obvious but worth pinning in the prover so later
  proofs can cite it.
- **Constant reduction** — `evalBinOp op ca cb = maskToSize (... the
  arithmetic ...)`. Justifies `constant_propagate`.
- **BinOp reduction** — extends that to the general case.

And one remaining `sorry` in `maskToSize_idem` that I'll close in
the same week this post publishes. The sorry-budget CI gate
(F1-TC-009) ensures it doesn't silently regrow.

The interesting proofs are still ahead — `constant_propagate`
soundness (F1-LN-010), `dead_code_eliminate` soundness (F1-LN-011),
the compositional-preservation proof that DCE ∘ CP still computes
the right thing (F1-LN-012). Each of those takes 2-6 weeks
depending on how much mathlib lifting we can get.

## The validator is the near-term line of defence

Proofs are slow. Writing one takes weeks. But the IR has to stay
well-formed *right now*, today, while the proofs catch up. So
we shipped a validator (F1-IR-016, `core/src/ir/validate.cpp`)
that runs at every translation. It checks:

- Every operand Ref has a def earlier in the list. (SSA
  def-before-use.)
- Every result Ref is unique. (SSA single-assignment.)
- Side-effecting ops (StoreReg, Jump, …) carry no result Ref.
- Pure ops (Constant, LoadReg, BinOp, …) carry exactly one.

Those rules are a subset of what the Lean spec enforces at the
type level. Running them at translation time catches bugs the
proofs will eventually catch statically — but with a much shorter
turnaround. We lose no strength by running both; the validator's
cost is O(n) and the proof is O(forever).

There's one more thing the validator gives us that the proofs
don't: **fuzz-testing.** AFL++ against a random stream of IR
bytes surfaces edge cases the proofs haven't reached yet. The
validator is the predicate AFL++ uses to classify a result as
"OK" vs "buggy". F1-TC-004 (the fuzz harness) and F1-IR-016
(the validator) are paired tools.

## Why Lean 4 over Coq

Both would work. I picked Lean for three reasons:

1. **Mathlib is alive.** The automation and tactic library around
   Lean's mathlib is ahead of Coq's equivalent for the kind of
   arithmetic reasoning DBT proofs need (bitwise ops, modular
   arithmetic, integer sign extension).
2. **Build times.** Lake is faster than Coq's `_CoqProject`
   workflow on a fresh checkout by a factor of 3-5×, and I have to
   run CI on every PR.
3. **The long-term goal.** I want to use these proofs as material
   for a paper targeting POPL 2029 or PLDI 2030. Lean is where the
   front of PL research has been living the past five years; the
   reviewers will be fluent.

Coq would have worked. This isn't a religious fight. It's a tooling
optimisation.

## What "provably sound" buys you

When the TSO-adaptive pass ships (Fase 2.5, ~2028), it will have a
Lean proof sitting next to it that says: under the assumption that
the analysis returns `SingleThreaded`, the relaxation is
observation-equivalent to the TSO-every-access baseline.

If that proof holds, we get three things:

- **Performance.** The pass delivers a 2-3× speedup over the
  conservative default on single-threaded hot loops. That's a
  generation of mobile CPU of free headroom.
- **Academic credibility.** "We proved the optimisation sound" is
  a top-tier paper claim. Nobody else in the DBT space has done it.
- **Sleep.** The TSO bug that takes three weeks to track down
  because it only reproduces on a 32-core server under memory
  pressure — that bug can't exist. The proof says so.

If the proof *doesn't* hold, we know before ship. We publish the
negative result (per the manifesto) and fall back to a weaker
analysis.

## What's in the Lean repo today

```
ir-spec/
├── PrismaIR/
│   ├── Syntax.lean        -- Stmt, Op, Ref, Size, BinOpKind
│   ├── Semantics.lean     -- evalPure, evalBinOp, maskToSize
│   ├── MachineState.lean  -- RegFile, MachineState, observed
│   └── Lemmas.lean        -- 3 base lemmas + 1 sorry
├── lakefile.toml
├── lean-toolchain         -- pinned at v4.30.0-rc2
└── .sorry-budget          -- CI fails if it grows
```

~400 lines of Lean. Builds in ~40 seconds on a fresh checkout.
Grows as each pass gets its soundness proof.

This is what an IR designed to be proved looks like: small, total
functions; inductive types; environments-as-functions; no
pointer-chasing. It costs a bit more memory at runtime and a lot
more effort at the passes stage. It buys verifiability.

## What comes next

- Blog 4 (queued): *Translating x86 memory ordering to ARM64
  without losing your mind* — the TSO-adaptive pass in detail.
- RFC 0004 (not yet drafted): the flags model. The elephant we
  haven't specified yet.

Papers incoming: *A Formally Verified IR for x86→ARM64 Dynamic
Binary Translation* at POPL 2029 / PLDI 2030 is the target. First
drafts appear at the end of Fase 2 (~week 55).

---

*Status: draft. Publishes to `prisma-emu.dev/blog/02-designing-an-ir`
once the site is up.*
