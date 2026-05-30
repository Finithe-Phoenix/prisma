---
id: 0013
title: Model x87 with reduced 64-bit precision first
status: accepted
authors: [Danny, Codex]
created: 2026-05-17
updated: 2026-05-17
supersedes: []
superseded_by: null
---

# RFC 0013: Model x87 with reduced 64-bit precision first

## Summary

Prisma will initially lower a small x87 subset by storing logical x87 stack
values as 64-bit IEEE double bits inside the existing 16-byte `X87Slot`.
This unlocks common legacy `FLD`, `FST`, `FSTP`, `FXCH`, and basic arithmetic
paths without committing the runtime to full 80-bit extended-precision
semantics yet.

## Motivation

Phase 2 needs enough x87 to pass through old compiler output and library
fragments that still use the x87 stack for scalar floating point. Blocking on
a complete 80-bit model would tie decoder, IR, lowering, exception flags,
tag-word handling, and `FXSAVE`/`FRSTOR` compatibility together before the DBT
has enough execution coverage to justify that complexity.

The reduced model is intentionally explicit. It favors forward progress for
non-adversarial scalar workloads while marking the precision boundary so later
correctness work can replace it cleanly.

## Context

The runtime already reserves eight 16-byte `X87Slot` entries in
`CpuStateFrame`. The slot size leaves room for full 80-bit payloads and future
tag/control metadata, but the current ARM64 backend has mature scalar F64
helpers and no native 80-bit floating-point path.

The first useful decoder slice is:
- `FLD m32fp/m64fp` and `FLD ST(i)`.
- `FST/FSTP m32fp/m64fp`.
- `FXCH ST(i)`.
- `FADD`, `FMUL`, `FSUB`, `FSUBR`, `FDIV`, and `FDIVR` for memory and
  `ST(0), ST(i)` register forms.

This subset excludes x87 exceptions, condition codes, tag words, environment
save/restore, transcendental ops, BCD/integer memory forms, and MMX aliasing.

## Considered Alternatives

## Full 80-bit software model now

Represent every x87 value as an 80-bit extended float and lower operations to
runtime helper calls.

Benefits:
- Highest architectural fidelity.
- Cleaner route for precise exceptions, tag word behavior, and state save.

Costs:
- Requires a helper ABI, software FP implementation or compiler extension, and
  extensive test oracles before the decoder slice can land.
- Slows down common scalar cases that can run through host F64 today.

Decision: deferred until workloads require bit-exact x87 behavior.

## Reject x87 until full fidelity is ready

Keep all D8..DF opcodes unsupported or route them through `InlineAsm`.

Benefits:
- No temporary semantic gap.
- Keeps the IR smaller.

Costs:
- Leaves common legacy binaries blocked.
- Prevents backend and dispatcher validation on real x87 stack traffic.

Decision: rejected because it delays integration feedback.

## Reduced 64-bit stack model

Store logical x87 stack entries as F64 bit patterns, using explicit IR stack
operations and the existing scalar FP backend.

Benefits:
- Small IR surface: `X87Load`, `X87Store`, `X87Push`, `X87Pop`.
- Uses the reserved `CpuStateFrame::x87[]` layout without shifting existing
  state offsets.
- Lets decoder/lowerer tests cover stack movement and basic arithmetic now.

Costs:
- Not bit-exact for programs depending on 80-bit intermediates, precision
  control, exceptions, NaN payload details, or tag-word semantics.
- Requires documentation and future tests to prevent this from becoming an
  accidental permanent limitation.

Decision: accepted for the Phase 2 bridge.

## Decision

Implement a reduced-precision x87 bridge:

- `CpuStateFrame::x87[idx].lo` stores the low 64-bit double representation.
- `CpuStateFrame::x87_status_control` keeps the x87 TOS pointer in byte 4.
- `X87Push` decrements TOS modulo 8 and writes ST(0).
- `X87Pop` reads ST(0), increments TOS modulo 8, and returns the value bits.
- `X87Load` and `X87Store` address logical `ST(i)` as `(TOS + i) mod 8`.
- Decoder x87 memory forms use `LoadMemTSO`/`StoreMemTSO` plus scalar FP
  conversions for m32fp/m64fp.

The IR names and comments must keep calling this reduced precision. The full
80-bit implementation can reuse the 16-byte slot layout and replace the
payload semantics without moving the state frame.

## Consequences

Benefits:
- Unblocks a practical x87 decoder/lowerer slice.
- Keeps generated code direct for simple scalar floating-point cases.
- Preserves a migration path to full 80-bit payloads.

Costs:
- Some x87-heavy numerical software will observe different results.
- x87 exception state, tag word, and environment instructions remain
  unsupported.

Reversibility:
- Replace the `X87Slot` payload interpretation and lowering helpers with
  80-bit helper calls.
- Keep the same frame offsets where possible; if metadata expands, add fields
  after the current x87 block and update the static layout asserts.

## Implementation Notes

The first implementation should touch:

- `core/include/prisma/ir.hpp` for explicit x87 stack ops.
- `core/include/prisma/cpu_state.hpp` for stable x87 offsets.
- `core/src/decoder/x86_decoder.cpp` for the small D8/D9/DC/DD subset.
- `core/src/backend/emitter.cpp` and `core/src/backend/lowering.cpp` for ARM64
  TOS/slot access.
- IR serializer, validator, profiler, pretty-printer, DCE, and targeted tests.

## Open Questions

- Which workload justifies switching to full 80-bit helpers?
- Should x87 exceptions update guest status flags before full precision lands?
- How will MMX aliasing share or invalidate the x87 stack slots?
- Which state-save instructions become the compatibility gate:
  `FXSAVE`, `FNSAVE`, `FLDENV`, or `FNSTENV`?

## References

- Intel 64 and IA-32 Architectures Software Developer's Manual, x87 FPU
  instruction set and floating-point environment chapters.
- `core/include/prisma/cpu_state.hpp`
- `core/include/prisma/ir.hpp`
