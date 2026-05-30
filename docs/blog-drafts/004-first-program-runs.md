---
title: "Prisma's first program just ran end-to-end"
status: draft
date: 2026-05-05
tags: [milestone, dbt, arm64]
---

# Prisma's first program just ran end-to-end

Three lines of x86_64 just executed on Apple silicon through the
Prisma DBT. No Wine. No DXVK. No Box64. Just our 36-opcode IR, our
12-pass optimiser, our vixl-backed ARM64 emitter, and Apple's
MAP_JIT giving us the four pages we needed:

```
$ echo "48 B8 0A 00 00 00 00 00 00 00 \
        48 B9 20 00 00 00 00 00 00 00 \
        48 01 C8 \
        C3" | prisma_run --hex -

{
  "exit": "Halted",
  "rax":  "0x2a",
  "rcx":  "0x20",
  ...
}
```

That's `mov rax, 10; mov rcx, 32; add rax, rcx; ret` — and `0x2A`
is 42, exactly what we expected. The path that produced it:

1. **Decode** — the 24 guest bytes hit `core/src/decoder/x86_decoder.cpp`,
   which classifies the REX prefixes, the `B8+rd` MOV-imm64 form,
   the `48 01 /r` ADD r/m64,r64 form, and the `C3` RET. Out comes
   a flat `vector<ir::Stmt>`.
2. **Optimise** — the default 12-pass pipeline runs:
   `constant_propagate → algebraic_simplify → strength_reduce →
   peephole → constant_propagate (again) → redundant_load_eliminate →
   common_subexpression_eliminate → copy_propagate →
   dead_store_eliminate → branch_fold → flag_write_elimination →
   dead_code_eliminate`. Constant propagation collapses the
   `add 10, 32` chain on the spot.
3. **Lower** — `Lowerer::lower(stmts)` walks the SSA and asks
   `Emitter` for ARM64 instructions. `mov_imm64` for the constants,
   `add` between the pinned host registers (rax → x10, rcx → x11),
   then the prologue/epilogue from `backend::abi::emit_block_prologue`
   handles the AAPCS64 callee-saved discipline.
4. **JIT** — the ARM64 bytes go into a slab from
   `JitBufferPool::acquire`. Apple silicon's MAP_JIT requires us to
   toggle `pthread_jit_write_protect_np(0)` for the byte copy and
   back to `1` before execution; the pool wraps that automatically.
5. **Execute** — the dispatcher calls the slab via a function
   pointer cast; the block sets x0 to the halt sentinel (0) and
   returns; the dispatcher reads x0, sees the halt, and exits the
   run loop. Our prologue/epilogue stored host x10/x11 back into
   the `CpuStateFrame::gpr[]`, so the JSON output reads RAX = 0x2A
   and RCX = 0x20.

## Why this matters

A DBT is the kind of system where every layer holds up for the
"hello world" case or none of them do. We've been building each
layer in isolation against unit tests for months — 588 C++ test
cases, 17 Rust tests, a Lean spec with zero `sorry`s — but
running an actual x86 program through the whole stack is the
proof we don't have a hidden coupling bug.

This is also the F1-AC-005 trigger we've been waiting on. The
first benchmark numbers (Dhrystone / CoreMark / coreutils) are
the next milestone, gated on real ARM64 Android hardware
arriving (Orange Pi 5B for CI, POCO X6 Pro / Pixel 7a / Redmi
Note 13 Pro for Fase 3).

## What's next

Now that the pipeline holds end-to-end:

- **A larger program**. `dhrystone` is ~30 KB of x86_64; running
  it through `prisma_run` will push the decoder coverage hard.
  The Zydis differential corpus already locks 35 instruction
  families against Zydis's reference decoder, so the regressions
  will surface as Prisma DecodeErrors rather than silent miscompiles.
- **Real-hardware numbers**. The benchmark harness in
  `tools/benchmarks/bench.py` already probes for QEMU / Box64 /
  FEX / Prisma in parallel. As soon as the Linux ARM64 CI runner
  is up (F0-DX-014) we'll have wall-time comparisons.
- **The first proven pass-soundness lemma**. We landed
  `cp_fold_op_sound` in F1-LN-010 last week — the per-op proof
  that constant propagation preserves `evalPure`. F1-LN-012
  composes that with DCE soundness (F1-LN-011, also proven) over
  whole programs, conditional on the `exec : Function → Trace`
  interpretation we're writing now.

The slow part of building a DBT is the years before the first
program runs. After that, every milestone is incremental.
Prisma just crossed the slow part.

— Danny
