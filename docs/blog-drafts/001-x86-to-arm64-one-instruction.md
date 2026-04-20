---
title: "From x86 to ARM64, one instruction at a time"
status: draft
author: Danny
date: 2026-04-20
tags: [dbt, arm64, x86_64, compilers]
---

# From x86 to ARM64, one instruction at a time

Prisma is a dynamic binary translator — x86/x64 → ARM64 — we're
building so that Windows binaries can run on Android phones at
something approaching native speed. "We'll just JIT it" sounds like a
one-sentence project. It is not. It is every architectural mismatch
between two twenty-year-old ISAs, multiplied by a memory model gap,
divided by how much time a mobile CPU gives you before the thermal
throttle kicks in.

This post is the first in a series. Today I want to show the shape of
what happens to a single x86 instruction — one of the easier ones —
as it walks through Prisma's pipeline and comes out the other side as
a few ARM64 words. No magic; just the small decisions that compound.

## The pipeline in one diagram

```
    guest bytes              IR (SSA)              host bytes
   [ 48 01 D8 ]   ──▶   %0 = LoadReg Rax    ──▶   ldr  x0, [x27, #0x50]
                        %1 = LoadReg Rbx            ldr  x1, [x27, #0x58]
                        %2 = Add %0, %1, I64        add  x2, x0, x1
                        StoreReg Rax, %2            str  x2, [x27, #0x50]
       decoder           passes (optional)            emitter (vixl)
```

From left to right: the **decoder** classifies the guest bytes into
IR; the **passes** rewrite the IR for locality, constant folding, and
similar wins; the **emitter** lowers the IR to ARM64. Each stage is
pure — input goes in, new output comes out. That's deliberate: every
stage is testable, and every stage has a formal counterpart in Lean 4
so we can prove its behaviour.

## The IR is SSA

`add rax, rbx` — three bytes on x86, 24 bits on ARM64 — becomes four
IR statements. That's the tax of SSA (static single assignment):

- Every ref is defined exactly once.
- Every use precedes its def in the list order.
- No destructive rewrites.

In exchange for that tax we get the thing every compiler textbook
promises: trivial dataflow. The passes become one-pass scans. The
register allocator becomes linear-scan. The Lean semantics become
structural induction.

Prisma's IR is the product of a design decision (RFC 0001) we debated
for a week: should we template the lowering (FEX's approach) or lift
to a proper IR (QEMU's approach)? The SSA IR won on two criteria —
formal verifiability is impossible without it, and dataflow is
miserable with it — but templating is faster. So we paid the
decoding-time cost up-front, and we're making it back at translation
time via the passes.

## Passes are cheap when the IR is pure

Today's default pipeline runs 10 passes:

1. `constant_propagate` — folds `(const, const)` binary ops.
2. `algebraic_simplify` — handles `x * 0 → 0`, `x & -1 → x`, and
   friends.
3. `strength_reduce` — turns `x * (1 << k)` into `x << k`.
4. `constant_propagate_2` — picks up what the first two just created.
5. `redundant_load_eliminate` — dedupes repeated `LoadMem` against
   the same address ref.
6. `common_subexpression_eliminate` — kills duplicate arithmetic.
7. `copy_propagate` — chases the move idioms CSE emitted.
8. `dead_store_eliminate` — drops overwritten `StoreMem`s.
9. `branch_fold` — collapses a condjump whose cmp is static.
10. `dead_code_eliminate` — sweeps.

Each one is ~100 lines of C++. Each one has a test file that
constructs an IR, runs the pass, and asserts on the result. None of
them fail subtly — because of SSA, there's no aliasing to get wrong.
In a non-SSA compiler "did the pass corrupt a def-use chain?" would
be a permanent source of bugs; here, def-use is the statement list
order.

On the benchmark rig (a M1 Pro, release build): the full pipeline on
a 100-BinOp block takes ~30 µs. The translation cache lookup is
another ~1 µs. Together they're 3% of an emitter run; the emitter
itself (vixl assembling into a 16KB buffer) dominates. That's the
number I care about long-term — the mobile CPUs we're targeting are
~40% of an M1 Pro's single-thread speed, so the math still works.

## The register allocator is the reason we picked SSA

The ten scratch registers of our host pool (x0..x9) are the scarcest
resource in the whole system. Every IR ref competes for one. A naive
"bump-pointer" allocator runs out after 10 refs even if most of them
are dead already — we hit that wall on week three.

Linear-scan (Poletto & Sarkar, 1999) costs one pre-pass over the
stmt list to compute `last_use[ref]` and one forward pass that expires
dying refs, returning their regs to the pool. It's O(n). It's correct
under SSA without phi-node machinery. And when it runs out, we
**spill** to a stack slot (Belady-optimal victim) and reload
on demand. Details in RFC 0006.

That register allocator is why `add rax, rbx` compiles to three ARM64
moves and one `add`, instead of the seven-instruction sequence you'd
get from "lift naively, spill everything to memory, trust the CPU to
forward the stores". The output of the Prisma pipeline on our little
ADD is indistinguishable from hand-written ARM64 — because our
allocator gets the common case right.

## The memory model is the hard part

Everything above is the *easy* side of the gap. The hard side is TSO.

x86 provides **total store order**: every CPU sees every store in the
same order, with at most one write buffer's worth of local
reordering. ARM64 provides **relaxed consistency by default**: you
get what the compiler and the hardware feel like giving you, unless
you explicitly say `ldar`/`stlr`. If you translate naively,
pathological multithreaded bugs surface weeks later in production.

The conservative fix — emit every guest load as `ldar` and every
guest store as `stlr` — is also about 3× slower than necessary on
current hardware. So the real prize is an **adaptive** pass that
proves, per-block, that a region is single-threaded and can
downgrade the acquire/release loads to plain `ldr`/`str`. That pass
needs a memory-model correctness proof — we can't just ship it and
hope. The formal model lives in `ir-spec/` and we're writing the
lemmas over 2026-2027.

If that sounds speculative, it is. Most DBTs don't bother; they ship
TSO-every-access and eat the performance hit. Prisma's whole bet is
that we can prove the relaxation sound, ship it, and pocket the
speed.

## What comes next

- Blog 2 (drafting): *Designing an IR you can prove correct*. Why
  Lean 4 over Coq, what a semantics function looks like when your
  target is bytecode, and how the validator (F1-IR-016) becomes your
  first line of defence before the proof is done.
- Blog 3 (queued): *JIT memory on Apple silicon — what actually
  works*. The platform-specific rabbit hole: `MAP_JIT`,
  `pthread_jit_write_protect_np`, code-signing, and why the
  signal-handler recovery (F0-TC-011) exists.
- Blog 4 (queued): *Translating x86 memory ordering to ARM64 without
  losing your mind*. The TSO-adaptive pass, the memory-model proof,
  and the specific patterns that let us downgrade.

This blog is the companion to the academic papers — those are the
peer-reviewed version of the claims made here, with numbers. The
first paper targets LCTES 2027 / VEE 2027.

Nothing to read yet. Come back in ten weeks.

---

*Status: draft. Will publish to `prisma-emu.dev/blog/01-x86-to-arm64`
as the site goes up (F0-DX-017). Source lives in
`docs/blog-drafts/001-x86-to-arm64-one-instruction.md`.*
