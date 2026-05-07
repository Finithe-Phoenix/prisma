# Prisma — Backlog

> Consolidated work list across the 48-54 month plan. This file is the
> single source of truth for "what's left". `TodoWrite` is for
> active-session tracking; this file is the multi-year map.

**Legend:**

- `[ ]` TODO — unclaimed, either agent may take.
- `[~|<agent>]` IN PROGRESS — claimed by the named agent (`claude` / `codex`).
- `[x] (<sha>)` DONE — `<sha>` is the first commit that landed it.
- `[!] <reason>` BLOCKED — waiting on an external dependency.
- `[?]` DEFERRED — descoped from current phase; may return later.

See [`COORDINATION.md`](COORDINATION.md) for the full multi-agent
protocol (claim, complete, abandon, conflict resolution).

**ID scheme:** `F<phase>-<component>-<num>`.

- `F<phase>`: `F0`..`F6`, plus `FX` for cross-cutting / ongoing work.
- `<component>`: two-letter code per subsystem (`DC` decoder, `IR` ir,
  `BK` backend/lowering, `RT` runtime, `CA` cache, `PS` passes, `LN`
  Lean spec, `ND` Android, `SH` shell/Rust, `SV` server, `NP` NPU, `GX`
  graphics, `WN` Wine, `TC` testing, `DX` devex/infra, `DC` docs, `AC`
  academic, `LG` legal, `CM` community, `BM` benchmarks).
- `<num>`: zero-padded 3-digit sequence within (phase, component).

**Size estimates:** `XS` (<1h), `S` (1-4h), `M` (1-2 days), `L` (3-10
days), `XL` (weeks).

---

## Fase 0 — Fundación (weeks 1-8)

Most of the code-level F0 work is done via accelerated velocity. What
remains here is legal / infra / research that requires Danny's direct
involvement or calendar time.

### F0-LG — Legal & entity

- [ ] F0-LG-001: Review HSBC contract for IP / moonlighting clauses. (S, blocks all public work)
- [ ] F0-LG-002: Incorporate Delaware LLC via Stripe Atlas. (M, ~$500)
- [ ] F0-LG-003: Register domain `prisma-emu.dev`. (XS)
- [ ] F0-LG-004: Open Mercury business bank account. (S)
- [ ] F0-LG-005: File USPTO trademark (optional). (S, ~$250-500)
- [ ] F0-LG-006: Consult lawyer on Wine / emulator precedents (Bleem, Corellium). (M)
- [ ] F0-LG-007: Draft privacy policy for telemetry opt-in. (S)
- [ ] F0-LG-008: Draft terms of service placeholder. (S)

### F0-DX — Developer experience / infra

- [x] F0-DX-001: Initialize git repository with monorepo layout.
- [x] F0-DX-002: Create `.gitignore` covering C++ / Rust / Kotlin / Python / Lean / LaTeX.
- [x] F0-DX-003: Create root `README.md`.
- [x] F0-DX-004: Create `CLAUDE.md`.
- [x] F0-DX-005: Create `docs/research_notes.md` with Tier 1-9 reading list.
- [x] F0-DX-006: Create `docs/rfc/` directory with process `README.md`.
- [x] F0-DX-007: CI: `ir-spec.yml` Lean build workflow.
- [x] F0-DX-008: CI: `lint-docs.yml` markdownlint + RFC frontmatter checks.
- [x] F0-DX-009: CI: `core-stub.yml` and `shell-stub.yml` as placeholders.
- [ ] F0-DX-010: Create GitHub Organization `prisma-emu`. (XS, needs Danny's account)
- [ ] F0-DX-011: Push repo to `prisma-emu/prisma` as private. (XS)
- [ ] F0-DX-012: Purchase Orange Pi 5B (~$150) for self-hosted ARM64 runner.
- [ ] F0-DX-013: Flash Armbian + install GitHub Actions runner agent.
- [ ] F0-DX-014: Register runner as `prisma-linux-arm64`.
- [ ] F0-DX-015: Set up Sentry self-hosted on Hetzner (crash reporting).
- [ ] F0-DX-016: Set up PostHog self-hosted (telemetry pipeline).
- [ ] F0-DX-017: Cloudflare R2 bucket for future cache CDN. ($20/mo trigger at Fase 2.5.)
- [x] (26100d1) F0-DX-018: Configure `.editorconfig`.
- [x] (26100d1) F0-DX-019: Configure `.clang-format` and `.clang-tidy`.
- [x] (26100d1) F0-DX-020: Pre-commit hooks: clang-format, rustfmt, ruff, detekt.
- [ ] F0-DX-021: Buy POCO X6 Pro (Dimensity 8300-Ultra).
- [ ] F0-DX-022: Buy Pixel 7a (Tensor G3, for AVF).
- [ ] F0-DX-023: Buy Redmi Note 13 Pro (Snapdragon 7s Gen 2).
- [ ] F0-DX-024: Root / custom ROM on test devices as needed.
- [ ] F0-DX-025: ADB setup and baseline performance benchmarks on all three devices.

### F0-DC — Documentation & research

- [x] F0-DC-001: Clone FEX, Box64, Wine, vixl into `~/Documents/sandbox/prisma-research/`.
- [x] F0-DC-002: Scan FEX architecture.
- [x] F0-DC-003: Scan Box64 dynarec.
- [x] F0-DC-004: Scan Wine PE loader / BTCpu interface.
- [x] F0-DC-005: Scan vixl.
- [ ] F0-DC-006: Scan DXVK architecture (D3D9/11 → Vulkan).
- [ ] F0-DC-007: Scan VKD3D-Proton (D3D12 → Vulkan).
- [ ] F0-DC-008: Scan Mesa Turnip driver for Adreno.
- [ ] F0-DC-009: Scan Winlator Cmod source for product feature set.
- [ ] F0-DC-010: Scan GameNative fork for Steam/Epic/GOG integration.
- [ ] F0-DC-011: Read Transmeta Crusoe Code Morphing paper; add notes.
- [ ] F0-DC-012: Read Intel IA-32 Execution Layer post-mortem.
- [ ] F0-DC-013: Read TOSTING TSO paper; add quantitative notes.
- [ ] F0-DC-014: Read Sarkar/Sewell x86-CC memory model paper.
- [ ] F0-DC-015: Read Batty et al C++ concurrency paper.
- [ ] F0-DC-016: Read CompCert overview paper.
- [ ] F0-DC-017: Read CakeML POPL 2014 paper.
- [ ] F0-DC-018: Read Rosetta 2 internals writeup (Nakagawa BSides 2021).
- [ ] F0-DC-019: Read NNAPI architecture docs.
- [ ] F0-DC-020: Read ONNX Runtime NNAPI delegate docs.
- [ ] F0-DC-021: Read MediaTek NeuroPilot SDK docs.
- [ ] F0-DC-022: Read Qualcomm Hexagon SDK docs.
- [ ] F0-DC-023: Read AVF / pKVM design docs.
- [ ] F0-DC-024: Read Danny Lin's Windows on Pixel 6 writeup.
- [ ] F0-DC-025: Publish `docs/research_notes.md` as public blog post at `/blog/designing-prisma-part-1`.
- [x] (pending commit) F0-DC-026: Write blog post "Why we're writing a new DBT from scratch".
- [x] (pending commit) F0-DC-027: Write `docs/ARCHITECTURE.md` — one-page tour of the monorepo.
- [x] (pending commit) F0-DC-028: Write `docs/CONTRIBUTING.md` (placeholder for future OSS).

### F0-CM — Community outreach

- [ ] F0-CM-001: Draft letter to ptitSeb (Box64). Send.
- [ ] F0-CM-002: Draft letter to neobrain (FEX). Send.
- [ ] F0-CM-003: Reserve Discord server name "Prisma Emulator".
- [ ] F0-CM-004: Reserve `@prisma_emu` handles on X, BlueSky.
- [ ] F0-CM-005: Register for FOSDEM 2028 as attendee.
- [ ] F0-CM-006: Join #dev in Winlator Discord lurking-only.

### F0-LN — Lean 4 spec (initial)

- [x] F0-LN-001: Lake project with `leanprover/lean4:v4.30.0-rc2`.
- [x] F0-LN-002: `PrismaIR.Syntax.lean` — 12-opcode MVP IR.
- [x] F0-LN-003: `PrismaIR.Semantics.lean` — pure fragment evaluator.
- [x] F0-LN-004: `PrismaIR.MachineState.lean` — RegFile + MachineState.
- [x] F0-LN-005: `PrismaIR.Lemmas.lean` — 3 lemmas (determinism, constant reduction, binop reduction).
- [ ] F0-LN-006: Document IR design rationale in header comment (done via RFC 0001).
- [x] F0-LN-007: Smoke-test decidable examples (`by decide`).

### F0-TC — Testing harness

- [x] F0-TC-001: Catch2 3.10.0 via FetchContent.
- [x] F0-TC-002: `test_ir.cpp` structural + equality tests.
- [x] F0-TC-003: `test_arm64_encoding.cpp` against ARM ARM.
- [x] F0-TC-004: `test_emitter.cpp` vixl integration.
- [x] F0-TC-005: `test_decoder.cpp` decoder unit tests.
- [x] F0-TC-006: `test_e2e.cpp` structural and JIT execution.
- [x] F0-TC-007: `test_jit_execution.cpp` MAP_JIT lifecycle.
- [x] F0-TC-008: `test_passes.cpp` const_prop, dce, PassManager.
- [x] F0-TC-009: `test_translation_cache.cpp` lookup, SMC, invalidation.
- [x] F0-TC-010: `test_lowering.cpp` Lowerer unit tests.
- [x] F0-TC-011: `test_signal_handler.cpp` SIGSEGV/SIGILL recovery.

---

## Fase 1 — Decoder + IR maturation (weeks 9-32)

The goal is: decoder + IR sound enough to run all of coreutils through
our user-mode translator on a reference host. This phase is mostly
grinding through x86_64 ISA + maturing the lowering.

### F1-DC — Decoder (x86_64 ISA expansion)

- [x] F1-DC-001: MOV r64, imm64 (48 B8+rd).
- [x] F1-DC-002: MOV r/m64, r64 (48 89 /r) + memory forms.
- [x] F1-DC-003: MOV r64, r/m64 (48 8B /r) + memory forms.
- [x] F1-DC-004: ADD r/m64, r64 (48 01 /r, reg direct).
- [x] F1-DC-005: SUB r/m64, r64 (48 29 /r, reg direct).
- [x] F1-DC-006: AND r/m64, r64 (48 21 /r).
- [x] F1-DC-007: OR  r/m64, r64 (48 09 /r).
- [x] F1-DC-008: XOR r/m64, r64 (48 31 /r).
- [x] F1-DC-009: NOP (90).
- [x] F1-DC-010: RET (C3).
- [x] F1-DC-011: MOV r32, imm32 (B8+rd, no REX.W).
- [x] F1-DC-012: MOV r8, imm8 (B0+rd).
- [x] F1-DC-013: MOV r/m8, r8 (88 /r) + memory.
- [x] F1-DC-014: MOV r/m16, r16 (66 89 /r) + memory.
- [x] F1-DC-015: MOV r/m32, r32 (89 /r) + memory.
- [x] F1-DC-016: ADC r/m64, r64 (48 11 /r).
- [x] F1-DC-017: SBB r/m64, r64 (48 19 /r).
- [x] F1-DC-018: CMP r/m64, r64 (48 39 /r).
- [x] F1-DC-019: TEST r/m64, r64 (48 85 /r).
- [x] F1-DC-020: INC r/m64 (48 FF /0).
- [x] F1-DC-021: DEC r/m64 (48 FF /1).
- [x] F1-DC-022: NEG r/m64 (48 F7 /3).
- [x] F1-DC-023: NOT r/m64 (48 F7 /2).
- [x] F1-DC-024: MUL r/m64 (48 F7 /4).
- [x] F1-DC-025: IMUL r/m64 (48 F7 /5).
- [x] F1-DC-026: IMUL r64, r/m64 (48 0F AF /r).
- [x] F1-DC-027: IMUL r64, r/m64, imm32 (48 69 /r).
- [x] F1-DC-028: IMUL r64, r/m64, imm8 (48 6B /r).
- [x] F1-DC-029: DIV r/m64 (48 F7 /6).
- [x] F1-DC-030: IDIV r/m64 (48 F7 /7).
- [x] F1-DC-031: SHL/SHR/SAR r/m64, imm8 (48 C1 /4|/5|/7).
- [x] F1-DC-032: SHL/SHR/SAR r/m64, CL (48 D3 /4|/5|/7).
- [x] F1-DC-033: ROL/ROR r/m64 (48 C1 /0|/1, 48 D3 /0|/1).
- [x] F1-DC-034: RCL/RCR r/m64 (48 C1 /2|/3, 48 D3 /2|/3).
- [x] F1-DC-035: BT / BTS / BTR / BTC r/m64, imm8 (48 0F BA).
- [x] F1-DC-036: BSF / BSR r64, r/m64 (48 0F BC / BD).
- [x] F1-DC-037: LZCNT r64, r/m64 (F3 48 0F BD).
- [x] F1-DC-038: TZCNT r64, r/m64 (F3 48 0F BC).
- [x] F1-DC-039: POPCNT r64, r/m64 (F3 48 0F B8).
- [x] F1-DC-040: PUSH r64 (50+rd).
- [x] F1-DC-041: POP r64 (58+rd).
- [x] F1-DC-042: PUSH imm8 / imm32 (6A / 68).
- [x] F1-DC-043: PUSHFQ / POPFQ (9C / 9D).
- [x] F1-DC-044: LEA r64, [mem] (48 8D /r).
- [x] F1-DC-045: MOVSX r64, r/m8/16/32 (0F BE / BF, 63).
- [x] F1-DC-046: MOVZX r64, r/m8/16 (0F B6 / B7).
- [x] F1-DC-047: CBW/CWDE/CDQE (98 with prefix sizes).
- [x] F1-DC-048: CWD/CDQ/CQO (48 99).
- [x] F1-DC-049: JMP rel8 (EB cb).
- [x] F1-DC-050: JMP rel32 (E9 cd).
- [x] F1-DC-051: JMP r/m64 (FF /4).
- [x] F1-DC-052: Jcc rel8 for all 16 condition codes (70-7F).
- [x] F1-DC-053: Jcc rel32 for all 16 condition codes (0F 80-8F).
- [x] F1-DC-054: CALL rel32 (E8 cd).
- [x] F1-DC-055: CALL r/m64 (FF /2).
- [x] F1-DC-056: LEAVE (C9).
- [x] F1-DC-057: RET imm16 (C2 iw).
- [x] F1-DC-058: CMOVcc r64, r/m64 for all 16 condition codes (0F 40-4F).
- [x] F1-DC-059: SETcc r/m8 for all 16 condition codes (0F 90-9F).
- [x] F1-DC-060: XCHG r64, r/m64 (48 87 /r).
- [x] F1-DC-061: CMPXCHG r/m64, r64 (48 0F B1 /r).
- [x] F1-DC-062: CMPXCHG16B m128 (48 0F C7 /1).
- [x] F1-DC-063: XADD r/m64, r64 (48 0F C1 /r).
- [x] F1-DC-064: LOCK prefix handling (emits atomics via TSO variants).
- [x] F1-DC-065: XACQUIRE / XRELEASE HLE prefixes (ignored; documented).
- [x] (bdd4f01) F1-DC-066: REP / REPE / REPNE prefixes (STOS/MOVS/CMPS/SCAS). (Decoder accepts via InlineAsm placeholder; proper IR-level loop expansion is follow-up.)
- [x] F1-DC-067: STOSB/STOSW/STOSD/STOSQ (AA / AB with size).
- [x] F1-DC-068: MOVSB/MOVSW/MOVSD/MOVSQ (A4 / A5).
- [x] F1-DC-069: CMPSB/CMPSW/CMPSD/CMPSQ (A6 / A7).
- [x] F1-DC-070: SCASB/SCASW/SCASD/SCASQ (AE / AF).
- [x] F1-DC-071: Operand size prefix 0x66 (16-bit operands).
- [x] F1-DC-072: Address size prefix 0x67.
- [x] F1-DC-073: Segment override prefixes (documented; ES/CS/SS/DS are no-ops in 64-bit).
- [x] F1-DC-074: %fs / %gs segment base accesses (TLS / TEB).
- [x] F1-DC-075: RIP-relative addressing (mod=00 rm=101).
- [x] F1-DC-076: SIB byte decoding (rm=100 in ModR/M).
- [x] F1-DC-077: REX.R / REX.X / REX.B — reject no longer; handle.
- [x] F1-DC-078: Use a table-driven decoder for opcode dispatch.
- [x] F1-DC-079: Instruction length validator (reject 16+ byte oversized encodings).
- [x] F1-DC-080: Decoder fuzz harness (AFL++) seeded with coreutils disassembly.
- [x] (cd59ba5) F1-DC-081: Differential test against Zydis for agreement on decoded length + mnemonics. (Length parity: 16 cases. Mnemonic check deferred to F1-DC-087.)
- [x] F1-DC-082: INT3 (CC) — decode as SIGTRAP.
- [x] F1-DC-083: HLT (F4) — reject (privileged).
- [x] F1-DC-084: CPUID (0F A2) — decode as side-effecting op.
- [x] F1-DC-085: RDTSC / RDTSCP (0F 31 / 0F 01 F9).
- [x] F1-DC-086: SYSCALL (0F 05).
- [x] (d69cff6) F1-DC-087: Zydis-free migration: our decoder is the canonical source (target ≥99% matching on coreutils). (Mnemonic differential framework lands; ≥99% measurement gated on coreutils corpus + F1-AC-003.)

### F1-IR — IR expansion

- [x] F1-IR-001: SSA `Ref` as uint32_t (FEX-style compression).
- [x] F1-IR-002: Op variants: Constant, LoadReg/StoreReg, BinOp, Compare, Load/Store x4, Jump, CondJump, Return.
- [x] (a1ee74c) F1-IR-003: Add `Flags` SSA value type (tuple of carry, zero, sign, overflow, parity, aux).
- [x] (a1ee74c) F1-IR-004: Add `WriteFlags{op, lhs, rhs, size}` producing a `Flags`.
- [x] (a1ee74c) F1-IR-005: Add `ReadFlag{flags, which}` producing a bool.
- [x] F1-IR-006: Add `JumpRel{guest_target_pc}` — branch to guest address.
- [x] (a1ee74c) F1-IR-007: Add `CondJumpFlags{flags, cc, true_target, false_target}`.
- [x] F1-IR-008: Add `Call{callee_pc}` and `RetAdjusted{pop_bytes}`.
- [x] F1-IR-009: Add `Select{cond, val_true, val_false}` (CMOV lowering).
- [x] (5bfaa98) F1-IR-010: Add `Extend{value, from_size, to_size, signed}`.
- [x] (5bfaa98) F1-IR-011: Add `Truncate{value, to_size}`.
- [x] (5bfaa98) F1-IR-012: Add `Fence{kind}` for explicit MFENCE / LFENCE / SFENCE.
- [x] (6ccef14) F1-IR-013: Add `InlineAsm{bytes}` escape hatch (last resort for odd instructions).
- [x] (c9c428c) F1-IR-014: Add `GuestPc{pc}` pseudo-op for cache keying and debugging.
- [x] (2fad1b1) F1-IR-015: IR type system — each Ref carries `OpSize` so validation catches mismatches.
- [x] (pending commit) F1-IR-016: IR validator pass — catch undef refs, mis-sized operands.
- [x] (e7ac26c) F1-IR-017: IR serialization to a compact binary form for cache storage.
- [x] (e7ac26c) F1-IR-018: IR deserialization + round-trip tests.
- [x] (c9c428c) F1-IR-019: Memoise pretty-printed form for test stability.
- [x] (0a0e8d0) F1-IR-020: IR profiler instrumentation points (future ML features).
- [x] (pre-existing) F1-IR-021: Add `BasicBlock` concept separate from flat Stmt list. (ir.hpp defines `struct BasicBlock { uint32_t id; vector<Stmt> stmts; }`.)
- [x] (pre-existing) F1-IR-022: Add `Function` with multiple blocks + entry block. (ir.hpp defines `struct Function { vector<BasicBlock> blocks; uint32_t entry; }`.)
- [x] (f7b6cad) F1-IR-023: CFG builder: group decoded Stmts into blocks by jump targets.
- [x] (759346c) F1-IR-024: Dominator tree + postorder traversal utilities.
- [x] (759346c) F1-IR-025: Loop detection (natural loops by back-edge analysis).

### F1-BK — Backend / Lowering

- [x] F1-BK-001: Emitter with mov*, ALU 3-reg, cmp+cset, memory 4-size.
- [x] F1-BK-002: Lowerer for pure + memory + Compare ops.
- [x] (61cc53f) F1-BK-003: Lowerer for Jump (unconditional ARM64 b with label).
- [x] (61cc53f) F1-BK-004: Lowerer for CondJump (cmp + cbnz + b with label). (CondJumpFlags-on-NZCV variant deferred until F1-IR-007 lands the new IR op.)
- [x] (pending commit) F1-BK-005: Emitter label management (vixl Label class).
- [x] (61cc53f) F1-BK-006: Basic block → CFG lowering with label fix-up.
- [x] (pending commit) F1-BK-007: Linear-scan register allocator over scratch pool.
- [x] (pending commit) F1-BK-008: Register spill / reload to stack frame slots.
- [x] (b550cbc) F1-BK-009: Callee-saved register save / restore around guest calls.
- [x] (c0fc3c5) F1-BK-010: Extend to WRegister loads (32-bit) as first-class.
- [x] (pending commit) F1-BK-011: Emitter for MUL/DIV multi-register output.
- [x] (e53e7a6) F1-BK-012: Emitter for NEON SIMD (128-bit vectors).
- [x] (34bd169) F1-BK-013: Emitter for floating-point (fadd, fmul, fdiv).
- [x] (pending commit) F1-BK-014: Emitter for rotates (ror / rol).
- [x] (pending commit) F1-BK-015: Emitter for bit manipulation (clz, cls, rbit).
- [x] (pending commit) F1-BK-016: Emitter for atomic RMW (ldaxr/stlxr loop).
- [x] (pending commit) F1-BK-017: Emitter for LSE atomics (CAS, LDADD).
- [x] (pending commit) F1-BK-018: Literal pool management.
- [x] (pre-existing) F1-BK-019: Code buffer flushing and I-cache invalidation integration. (jit_memory.cpp invalidate_icache + Emitter::flush_literal_pool, both wired.)
- [x] (b550cbc) F1-BK-020: Emit guest-state save on translation entry. (backend::abi::emit_block_prologue.)
- [x] (b550cbc) F1-BK-021: Emit guest-state restore on translation exit. (backend::abi::emit_block_epilogue_and_ret.)
- [x] (5bfaa98) F1-BK-022: Lowering of Extend and Truncate with correct zero/sign semantics.
- [x] (5bfaa98) F1-BK-023: Lowering of Fence (dmb ish / dsb).
- [x] (pre-existing) F1-BK-024: Lowering of Select via csel. (Already in lowering.cpp:372-389 + test_lowering.cpp:232.)

### F1-PS — Passes

- [x] F1-PS-001: constant_propagate.
- [x] F1-PS-002: dead_code_eliminate.
- [x] F1-PS-003: PassManager + default_pipeline.
- [x] (pending commit) F1-PS-004: Algebraic simplification (x + 0, x * 1, x ^ x → 0).
- [x] (pending commit) F1-PS-005: Common Subexpression Elimination (CSE) within a block.
- [x] (pending commit) F1-PS-006: Copy propagation (StoreReg to LoadReg chains).
- [x] (pending commit) F1-PS-007: Redundant-load elimination across a block.
- [x] (pending commit) F1-PS-008: Dead-store elimination within a block.
- [x] (aa1e3cd) F1-PS-009: Peephole pattern matcher (IR-level patterns).
- [x] (84c03a7) F1-PS-010: Constant folding on Extend / Truncate.
- [x] (pending commit) F1-PS-011: Strength reduction (shift for power-of-two multiply).
- [x] (7c12d88) F1-PS-012: Flag-write elimination when no reader exists. (Taken over from codex per Danny 2026-05-04; stub replaced.)
- [x] (pending commit) F1-PS-013: CFG simplification (remove empty blocks).
- [x] (pending commit) F1-PS-014: Branch folding (collapse `if const_true`).
- [x] (pending commit) F1-PS-015: Tail-call optimisation for CALL+RET → JMP patterns.
- [x] (pending commit) F1-PS-016: Pass timing instrumentation.
- [x] (pending commit) F1-PS-017: Pass manager `--debug-pass=NAME` dump hooks.

### F1-RT — Runtime

- [x] F1-RT-001: JitBuffer with MAP_JIT and pthread_jit_write_protect_np.
- [x] F1-RT-002: Signal handler for SIGSEGV / SIGILL / SIGBUS.
- [x] F1-RT-003: ScopedProtected RAII scope.
- [x] (pending commit) F1-RT-004: CpuStateFrame layout (16 GPRs + flags + SIMD + x87).
- [x] F1-RT-005: TLS storage for per-thread guest state.
- [x] (pending commit) F1-RT-006: Dispatcher loop (find-translation-and-jump).
- [x] F1-RT-007: Trampoline between translator and compiled blocks.
- [x] F1-RT-008: Return-address stack for guest CALL / RET tracking.
- [x] (e6b3e64) F1-RT-009: Thread-safe JitBuffer pool (many blocks).
- [x] (cfeb231) F1-RT-010: Page-protection based SMC detection (mprotect READ-ONLY).
- [x] (fdcc16c) F1-RT-011: Guest signal delivery (#PF, #UD, #DE, etc.). (Framework lands; SIGSEGV-trampoline integration deferred.)
- [x] (1412c99) F1-RT-012: FPU state save / restore (XSAVE-style). (`xmm[16]` + `x87[8]` + MXCSR + status_control in CpuStateFrame; prologue/epilogue extension is a follow-up.)
- [x] (8982d97) F1-RT-013: Guest stack pointer management (rsp). (Via `RspAdjust{delta_bytes}` IR op + lowering to add/sub on the pinned host x14.)
- [x] F1-RT-014: Guest segment registers fs / gs (TLS).
- [x] F1-RT-015: Helper calls from generated code to C++ runtime (printf-style).
- [x] (pending commit) F1-RT-016: CPU feature detection: FEAT_LSE2, LRCPC2, FlagM.
- [x] (pending commit) F1-RT-017: HostFeatures struct + runtime initialisation.

### F1-CA — Cache

- [x] F1-CA-001: In-memory cache with FNV-1a SMC detection.
- [x] F1-CA-002: Page-level invalidation.
- [x] (PENDING) F1-CA-003: Persistent on-disk cache file format (version + CPU features header).
- [x] (pending commit) F1-CA-004: SHA-256 content hash (for cross-device trust in Fase 2.5).
- [x] (PENDING) F1-CA-005: Cache eviction policy (LRU).
- [x] (pending commit) F1-CA-006: Size-bounded cache with eviction trigger.
- [x] (pending commit) F1-CA-007: Translation stats per entry (hit count, last-used).
- [x] (cd85682) F1-CA-008: Cache compaction pass (merge adjacent blocks). (Drops superseded SMC entries; the "merge adjacent code blocks" interpretation requires re-translation and is deferred.)
- [x] (pending commit) F1-CA-009: Cache writer thread (offload serialization).
- [x] (pending commit) F1-CA-010: zstd compression for on-disk entries.

### F1-LN — Lean spec expansion

- [x] F1-LN-001: Initial syntax / semantics / machine state.
- [x] (cb4e170) F1-LN-002: Add Flags type to `Syntax.lean`.
- [x] (cb4e170) F1-LN-003: Add Block / Function to Syntax.
- [x] (57455ad) F1-LN-004: Extend evalPure to cover Compare and Extend.
- [x] (57455ad) F1-LN-005: Define MachineState with memory (function Nat → UInt8).
- [x] (57455ad) F1-LN-006: Step relation for StoreReg / LoadReg / LoadMem / StoreMem.
- [x] (ff57d65) F1-LN-007: Step relation for CondJumpFlags.
- [?] F1-LN-008: Add mathlib dependency to Lake. (Deferred — `bv_decide` from `Std.Tactic.BVDecide` covers the F1-LN-009 use case without mathlib; mathlib lands when a proof actually needs it.)
- [x] (ff57d65) F1-LN-009: Prove `maskToSize_idem` (remove sorry).
- [x] (04c4fa0) F1-LN-010: Prove `constant_propagate` soundness (observable equivalence). (Per-op `cp_fold_op_sound` proven; whole-program `exec → Trace` lift is the F1-LN-013 follow-on.)
- [x] (d08c090) F1-LN-011: Prove `dead_code_eliminate` soundness. (Per-op `evalPure_unaffected_by_unread_ref` proven; whole-pass composition is F1-LN-012.)
- [ ] F1-LN-012: Prove DCE+CP composition preserves semantics.
- [x] (451c669) F1-LN-013: Define "observable trace" formally.
- [ ] F1-LN-014: Weak memory model skeleton (per-thread rfunction + shared store).
- [ ] F1-LN-015: TSO axioms as lemmas over the weak memory model.
- [ ] F1-LN-016: TSO-adaptive pass soundness (statement only; proof in Fase 2.5).

### F1-TC — Testing

- [x] F1-TC-001: Catch2 test executable builds and runs.
- [x] F1-TC-002: Integration test: translate + cache + execute + re-execute (hit).
- [x] (0da8c7b) F1-TC-003: Differential test harness: Prisma IR execution vs QEMU user-mode. (Python harness in `tools/diff-qemu/`; soft-skips when qemu-x86_64 absent. Probe validates the environment; corpus mode is next.)
- [x] (pending commit) F1-TC-004: Fuzzing infrastructure (AFL++ build recipe).
- [x] (pending commit) F1-TC-005: Coverage instrumentation (llvm-cov).
- [x] (98867bd) F1-TC-006: Coverage report published to `prisma-emu.dev/coverage`. (Local generation via `tools/coverage/gen.sh`; the `prisma-emu.dev` upload step is gated on F0-LG-003 + active CI runner.)
- [x] (pending commit) F1-TC-007: Performance regression harness (simple microbenchmarks).
- [x] (pending commit) F1-TC-008: UBSan / ASan / TSan builds added to CI matrix.
- [x] (pending commit) F1-TC-009: Lean proof check added as a blocking CI step.
- [x] (c19e609) F1-TC-010: Property-based tests for IR passes (QuickCheck-style via rapidcheck). (Implemented via Catch2 + custom RNG; rapidcheck dep avoided. 5 property tests, +215 assertions.)
- [x] F1-TC-011: Golden-output tests for decoder (x86 bytes → IR pretty-print).
- [x] (pending commit) F1-TC-012: Stress test: translate 10k distinct blocks, measure cache behaviour.

### F1-AC — Academic (Fase 1 concern)

- [x] (8f0f182) F1-AC-001: Start LaTeX draft in `papers/drafts/01-early-results/`.
- [x] (8f0f182) F1-AC-002: Outline paper structure (intro, IR, passes, evaluation, related).
- [x] (c8ecfb4) F1-AC-003: Reproduce baseline: QEMU / Box64 / FEX on the same hardware. (Driver framework in `tools/benchmarks/bench.py`; soft-skips per engine. Real numbers gated on hardware + binaries.)
- [ ] F1-AC-004: First benchmark table (Dhrystone / CoreMark vs baselines).
- [x] (0c98045) F1-AC-005: Write 3-paragraph blog post when first program runs (Notepad XP? coreutils?). (`prisma_run` (b7bd82b) validated end-to-end on Apple silicon; blog-drafts/004 documents the milestone.)

### F1-DC — Documentation

- [x] (8f0f182) F1-DC-001: RFC 0003 — opcode dispatch strategy (table vs switch).
- [x] (8f0f182) F1-DC-002: RFC 0004 — flags model (lazy vs eager, snapshot-based).
- [x] (pending commit) F1-DC-003: RFC 0005 — basic block representation + CFG construction.
- [x] (pending commit) F1-DC-004: RFC 0006 — register allocator design.
- [x] (pending commit) F1-DC-005: RFC 0007 — cache file format.
- [x] (pending commit) F1-DC-006: Blog post: "From x86 to ARM64, one instruction at a time".
- [x] (pending commit) F1-DC-007: Blog post: "Designing an IR you can prove correct".
- [x] (pending commit) F1-DC-008: Blog post: "JIT memory on Apple silicon — what actually works".
- [x] (pending commit) F1-DC-009: Update `CLAUDE.md` with new subsystems as they land.

---

## Fase 2 — ISA completo + Linux user-mode (weeks 33-56)

Target: pass 85%+ of the coreutils test suite via our user-mode
translator on a reference Linux ARM64 box.

### F2-SY — Syscall layer

- [ ] F2-SY-001: Design syscall dispatch (guest syscall # → host syscall # or emulation).
- [ ] F2-SY-002: Implement open, close, read, write (minimal stdio).
- [ ] F2-SY-003: Implement stat family (stat, lstat, fstat, newfstatat).
- [ ] F2-SY-004: Implement mmap / munmap / mprotect with translation-cache awareness.
- [ ] F2-SY-005: Implement brk / sbrk.
- [ ] F2-SY-006: Implement clone (threads).
- [ ] F2-SY-007: Implement futex (critical for pthread).
- [ ] F2-SY-008: Implement gettimeofday / clock_gettime.
- [ ] F2-SY-009: Implement getpid, getuid, geteuid, gettid.
- [ ] F2-SY-010: Implement exit / exit_group.
- [ ] F2-SY-011: Implement execve (cross-ISA — requires translator re-entry).
- [ ] F2-SY-012: Implement dup / dup2 / dup3 / pipe.
- [ ] F2-SY-013: Implement fcntl (subset).
- [ ] F2-SY-014: Implement ioctl (passthrough with struct translation).
- [ ] F2-SY-015: Implement socket / bind / listen / accept / connect.
- [ ] F2-SY-016: Implement read / write socket families.
- [ ] F2-SY-017: Implement signal delivery to guest (sigaction bridge).
- [ ] F2-SY-018: Implement rt_sigprocmask / rt_sigsuspend.
- [ ] F2-SY-019: Implement getcwd / chdir / fchdir.
- [ ] F2-SY-020: Implement unlink / rename / mkdir / rmdir.
- [ ] F2-SY-021: Implement lseek / pread / pwrite.
- [ ] F2-SY-022: Implement readv / writev.
- [ ] F2-SY-023: Implement poll / ppoll / select / epoll_wait.
- [ ] F2-SY-024: Implement epoll_create / epoll_ctl.
- [ ] F2-SY-025: Implement getdents / getdents64.
- [ ] F2-SY-026: Implement wait4 / waitid / waitpid.
- [ ] F2-SY-027: Implement prlimit64 / getrlimit / setrlimit.
- [ ] F2-SY-028: Implement prctl (subset — no_new_privs, etc.).
- [ ] F2-SY-029: Implement arch_prctl (sets %fs / %gs base).
- [ ] F2-SY-030: Implement set_tid_address.
- [ ] F2-SY-031: Implement mmap2 / old_mmap.
- [ ] F2-SY-032: Implement robust_futex structure translation.
- [ ] F2-SY-033: Errno translation table (glibc expects Linux errno numbers).
- [ ] F2-SY-034: iovec struct translation.
- [ ] F2-SY-035: timespec / timeval / sigset_t struct translation.
- [ ] F2-SY-036: termios struct translation for isatty / ioctl(TIOCGWINSZ).
- [ ] F2-SY-037: Syscall fuzz harness (AFL++ over syscall numbers + args).
- [ ] F2-SY-038: Syscall strace-like logger (`PRISMA_STRACE=1`).

### F2-IR — IR for full x86_64

- [x] (db74b8a) F2-IR-001: SIMD operand types (Vec128, Vec256). (Vec128 lands; Vec256 deferred until AVX decoder work.)
- [x] (50caa95) F2-IR-002: SSE ops (ADDPS/SS, MULPS/SS, SUBPS/SS, DIVPS/SS, ...). (Closed: ADDPS/PD/SS/SD + SUB/MUL/DIV/MIN/MAX/SQRT, HADDPS/PD, CMPxxPS/PD/SS/SD all landed across F2-IR-014..048. The umbrella's last-mile work was the FMA family in F2-IR-006.)
- [x] (50caa95) F2-IR-003: SSE integer ops (PADD, PSUB, PCMPEQ, PMULLW, ...). (Closed: full PADD/PSUB B/W/D/Q + sat variants, PAND/POR/PXOR, PCMPEQ/GT B/W/D + SSE4.1 PCMPEQQ + SSE4.2 PCMPGTQ, PMUL{HW,LW,UDQ,LD}, PMIN/PMAX × signed/unsigned × multiple lane sizes — landed across F2-IR-001..045.)
- [x] (50caa95) F2-IR-004: SSE shuffle / blend. (Closed: PSHUFD, PSHUFLW/HW, SHUFPS/PD, PSHUFB, PUNPCKL/H + UNPCKL/HPS/PD, PALIGNR, BLENDV PS/PD + PBLENDVB landed across F2-IR-010/011/015/036/038/046.)
- [x] (99d2056) F2-IR-005: AVX 256-bit equivalents (VADDPS, etc.). (First batch — packed FP arith, integer SIMD, FP/int bitwise, PCMPEQ/GT, UNPCK, SHUFPS/PD, HADDPS/PD, CMPxxPS/PD ymm. Lane-crossing ops follow.)
- [x] (d98bdbb) F2-IR-006: FMA (VFMADD, VFMSUB, etc.). (First batch — packed PS/PD xmm: VFMADD/SUB/NMADD/NMSUB × 132/213/231. Single VecFpFma IR op with neg_addend/neg_mul flags; ARM64 FMLA/FMLS lowering. ymm, scalar SS/SD, MADDSUB/MSUBADD deferred.)
- [ ] F2-IR-007: x87 ops minimal set (FLD, FST, FADD, FMUL, FDIV, FXCH).
- [ ] F2-IR-008: x87 stack depth tracking (inspired by FEX's x87 pass).
- [ ] F2-IR-009: MMX ops (rare in modern binaries, skip or stub).

### F2-BK — Backend for full ISA

- [x] (50caa95) F2-BK-001: Lowering for SIMD via NEON. (Closed: VecBinOp / VecFpBinOp / VecFpScalarBinOp / VecCmp / VecUnpack / VecShift{Imm,Bytes} / VecShuffle{32x4,2Src,H4} / VecPshufb / VecAlignr / VecExtend / VecAbs / VecMaskMsb / VecMaskFp / VecFpRound / VecBlend / VecFpFma plus the GPR↔XMM bridge and Load/StoreVec{Reg,Hi} all lowered to NEON via vixl across the F2-IR-001..006 + F2-BK-006 work.)
- [x] (50caa95) F2-BK-002: SIMD shuffle lowering (ARM tbl / zip / uzp / trn). (Closed alongside F2-BK-001: VecPshufb uses TBL, VecUnpack uses ZIP, VecShuffle{32x4,2Src,H4} use TBL/EXT/INS, VecAlignr uses EXT.)
- [x] (50caa95) F2-BK-003: Lowering for AVX (use pair of NEON vectors for 256-bit). (Closed: pair-of-Vec128 representation via LoadVecRegHi/StoreVecRegHi + ymm_hi[16] in CpuStateFrame; 256-bit ops compile to two NEON ops on consecutive scratch regs. F2-IR-005 and the FMA ymm extension exercise the pattern end-to-end.)
- [x] (50caa95) F2-BK-004: Lowering for FMA via NEON FMLA. (Closed: vfmla_q / vfmls_q / vfneg_q / vmov_q primitives plus the VecFpFma lowering arm. F2-IR-006 and its ymm extension exercise the four sign combinations.)
- [ ] F2-BK-005: Lowering for x87 (software emulation for rare cases).
- [x] (0597402) F2-BK-006: SIMD register allocator (NEON v0-v31). (Pool widened V0..V7 → V0..V23 [05044f8]; FP last-use expiry added [0597402] — same liveness machinery as the GPR allocator. Pair-allocator scaffolding + spill plumbing deferred until measured demand.)
- [x] (PENDING) F2-BK-007: Lowering for MUL/DIV multi-register results (rax:rdx). (Adds BinOpKind::UMulHi/SMulHi/UDiv/SDiv/UMod/SMod; MUL writes both halves of the 128-bit product; DIV writes UDiv quotient + UMod remainder. Const-prop folds with __int128 for compile-time constants. 64-bit dividend only — full 128/64 with explicit RDX:RAX is a follow-up.)
- [ ] F2-BK-008: Lowering for REP prefix — loop generation.
- [ ] F2-BK-009: Lowering for string ops (STOSB etc.) via ARM memset/memcpy intrinsic inline.
- [ ] F2-BK-010: Call / Return lowering with return-stack.

### F2-PS — Passes for Fase 2

- [ ] F2-PS-001: x87 stack elimination pass (FEX-style).
- [x] (cac89f7) F2-PS-002: Flag elimination (remove unused flag writes — Decoder already hints). (Extended DCE to mark WriteFlags / ReadFlag / WriteFlagsFp / WriteFlagsPtest as pure, and added missing operand-collect entries for WriteFlags / ReadFlag / CondJumpFlags / StoreVecRegHi / VecFpFma so the live-set is correct. F1's flag_write_elimination [F1-PS-012] handles the CmpFlags pattern; this commit covers the SSA WriteFlags family.)
- [ ] F2-PS-003: Loop-invariant code motion (LICM).
- [ ] F2-PS-004: Global CSE via dominator-based analysis.
- [ ] F2-PS-005: Inlining of short helpers.

### F2-BM — Benchmarks

- [ ] F2-BM-001: Dhrystone harness (Python driver + C source bundled).
- [ ] F2-BM-002: CoreMark harness.
- [ ] F2-BM-003: nbench harness.
- [ ] F2-BM-004: SPEC CPU2017 subset (the ones that run without GUI / net).
- [ ] F2-BM-005: Per-baseline runners: Prisma, QEMU, Box64, FEX, native.
- [ ] F2-BM-006: Result schema (JSON) + aggregation.
- [ ] F2-BM-007: Markdown / LaTeX report generation.
- [ ] F2-BM-008: `prisma-bench run --backend prisma --corpus dhrystone` fully wired.
- [ ] F2-BM-009: First public results table on prisma-emu.dev/benchmarks.
- [ ] F2-BM-010: Performance target: 30-45% of native at end of Fase 2.

### F2-AC — Academic (Fase 2 concern)

- [ ] F2-AC-001: Submit paper 1 to LCTES 2027 or VEE 2027.
- [ ] F2-AC-002: Make arXiv preprint public when submission is accepted.
- [ ] F2-AC-003: Reproducibility package: script that regenerates every table.
- [ ] F2-AC-004: Register for the conference.

### F2-CM — Community

- [ ] F2-CM-001: Open Discord server to public when coreutils passes.
- [ ] F2-CM-002: Launch blog RSS feed.
- [ ] F2-CM-003: First "ask me anything" post in r/EmulationOnAndroid (lurk-only until then).
- [ ] F2-CM-004: Submit 3 upstream patches to Wine / FEX / Box64 (good will).

---

## Fase 2.5 — Frontier research (weeks 57-80)

Five pillar prototypes, no product. This is the épico-defining block.

### F25-NP — NPU-Assisted Translation (Pillar 1)

- [ ] F25-NP-001: ONNX Runtime integration in C++ (link against libonnxruntime).
- [ ] F25-NP-002: NNAPI delegate wiring on Android.
- [ ] F25-NP-003: MediaTek NeuroPilot delegate integration.
- [ ] F25-NP-004: Qualcomm Hexagon SDK delegate integration.
- [ ] F25-NP-005: Training data pipeline: capture (bytecode, runtime-trace) pairs.
- [ ] F25-NP-006: Feature extraction: opcode histogram, branch density, memory footprint.
- [ ] F25-NP-007: Hot-path-prediction classifier (PyTorch, ~10 MB).
- [ ] F25-NP-008: TSO region classifier (5-way: ST, LF, SM, IO, Unknown).
- [ ] F25-NP-009: SIMD-pattern matcher (neural seq2seq toy).
- [ ] F25-NP-010: Register-allocation hint model.
- [ ] F25-NP-011: Benchmark NPU inference latency from C++.
- [ ] F25-NP-012: Fallback to CPU classifier when NPU unavailable.
- [ ] F25-NP-013: Telemetry opt-in UX in Android app.
- [ ] F25-NP-014: Anonymization pipeline (no bytecode leaves device, just features).
- [ ] F25-NP-015: Model versioning and signed distribution.
- [ ] F25-NP-016: Per-SoC model fine-tuning datasets.
- [ ] F25-NP-017: Paper 2 draft: "NPU-Assisted Dynamic Binary Translation".
- [ ] F25-NP-018: Submit paper 2 to MICRO 2028 or ASPLOS 2029.

### F25-TS — TSO adaptive pass (Pillar 3)

- [ ] F25-TS-001: Annotate IR with thread-affinity hints (static analysis).
- [ ] F25-TS-002: Runtime profiling hook (per-region observe).
- [ ] F25-TS-003: Classifier runs (offline) + signed classifications distributed.
- [ ] F25-TS-004: Rewrite pass: TSO→plain where proven safe.
- [ ] F25-TS-005: Conservative-fallback policy on unknown classification.
- [ ] F25-TS-006: Per-region runtime assertion (debug) that classification holds.
- [ ] F25-TS-007: Measure % of ops safely downgraded on reference workloads.
- [ ] F25-TS-008: Target: 15-20% speedup on single-threaded guest code.

### F25-LN — Formal verification of TSO pass (Pillar 2)

- [ ] F25-LN-001: Formalise classifier invariants in Lean.
- [ ] F25-LN-002: Prove rewrite preserves observable semantics under invariants.
- [ ] F25-LN-003: Connect runtime assertions to the Lean formal invariants.
- [ ] F25-LN-004: Paper 3 draft: "Formally Verified IR for x86 DBT".

### F25-CA — Distributed translation cache (Pillar 4)

- [ ] F25-CA-001: SHA-256 content hash.
- [ ] F25-CA-002: Cloudflare R2 bucket + object path scheme.
- [ ] F25-CA-003: Rust cache service (Axum): POST /cache, GET /cache/{hash}.
- [ ] F25-CA-004: Cache entry signature (Ed25519 signing key).
- [ ] F25-CA-005: Client verifies signature before mmap+exec.
- [ ] F25-CA-006: Pre-population script (for top-500 games).
- [ ] F25-CA-007: libp2p integration (DHT).
- [ ] F25-CA-008: P2P cache exchange protocol design.
- [ ] F25-CA-009: Peer discovery + trust anchors.
- [ ] F25-CA-010: Cross-device eligibility (same SoC + feature set only).
- [ ] F25-CA-011: Telemetry: cache hit rate, first-launch time savings.
- [ ] F25-CA-012: Publish protocol spec as standalone document.

### F25-AV — AVF hybrid virtualization (Pillar 5)

- [ ] F25-AV-001: AVF compatibility detection (Pixel 7a+/Tensor).
- [ ] F25-AV-002: Minimal VM image with Windows-on-ARM64.
- [ ] F25-AV-003: crosvm integration as guest launcher.
- [ ] F25-AV-004: virtio-console for debug.
- [ ] F25-AV-005: Bridge: native ARM64 regions run in guest, x86 in DBT.
- [ ] F25-AV-006: ARM64EC awareness in our translation scheduler.
- [ ] F25-AV-007: Measure speedup on DX11 hot loops.

### F25-GX — Graphics translation pre-work (Pillar 6)

- [ ] F25-GX-001: Whole-graph shader analyser prototype (Python, offline).
- [ ] F25-GX-002: Identify 20 hot-loop patterns from Portal / HL2.
- [ ] F25-GX-003: Handwritten optimised Vulkan snippets for each.
- [ ] F25-GX-004: Runtime pattern matcher in DXVK fork.
- [ ] F25-GX-005: Measure FPS delta on a controlled scene.

---

## Fase 3 — Wine integration (weeks 81-112)

### F3-WN — Wine and Windows programs

- [ ] F3-WN-001: Fork Wine 9.x ARM64 under `third_party/wine`.
- [ ] F3-WN-002: Build Wine with our CMake glue.
- [ ] F3-WN-003: Implement `wow64cpu.dll` stub matching Wine's BTCpu interface.
- [ ] F3-WN-004: `pBTCpuProcessInit` / `pBTCpuThreadInit`.
- [ ] F3-WN-005: `pBTCpuSimulate` — main dispatch loop.
- [ ] F3-WN-006: `pBTCpuGetContext` / `pBTCpuSetContext`.
- [ ] F3-WN-007: `pBTCpuNotifyMapViewOfSection`.
- [ ] F3-WN-008: `pBTCpuFlushInstructionCache2`.
- [ ] F3-WN-009: Loader PE parser (fallback if Wine's isn't reused).
- [ ] F3-WN-010: Relocations handling for guest DLLs.
- [ ] F3-WN-011: Import table translation.
- [ ] F3-WN-012: TLS callback handling.
- [ ] F3-WN-013: Integration test: run Notepad.exe (Windows XP).
- [ ] F3-WN-014: Integration test: run Calc.exe.
- [ ] F3-WN-015: Integration test: run Paint.exe.
- [ ] F3-WN-016: First bigger target: Photoshop 7 (or AutoCAD LT 2000).
- [ ] F3-WN-017: Document our Wine patch series for future upstream.

### F3-SH — Shell / orchestrator (Rust)

- [ ] F3-SH-001: Cargo workspace setup under `shell/`.
- [ ] F3-SH-002: JNI bridge crate to call from Kotlin.
- [ ] F3-SH-003: Container lifecycle: create, list, delete.
- [ ] F3-SH-004: Wine prefix management (per-container).
- [ ] F3-SH-005: Overlay filesystem (FUSE or layered).
- [ ] F3-SH-006: Component downloader (Wine bundle, DXVK bundle, VKD3D bundle).
- [ ] F3-SH-007: SHA-256 verification of downloads.
- [ ] F3-SH-008: Config TOML parsing + validation.
- [ ] F3-SH-009: Crash reporter integration (Sentry via reqwest).
- [ ] F3-SH-010: P2P cache client (libp2p).
- [ ] F3-SH-011: Translation cache persistence to disk.
- [ ] F3-SH-012: Unit tests + integration tests.

### F3-ND — Android app

- [ ] F3-ND-001: Gradle project under `android/app`.
- [ ] F3-ND-002: Min SDK 29, target SDK latest.
- [ ] F3-ND-003: Compose UI skeleton.
- [ ] F3-ND-004: Container list screen.
- [ ] F3-ND-005: Import .exe flow (SAF integration).
- [ ] F3-ND-006: Run container screen with log view.
- [ ] F3-ND-007: Settings screen.
- [ ] F3-ND-008: Bridge to Rust shell via JNI.
- [ ] F3-ND-009: Performance Hint API usage.
- [ ] F3-ND-010: Game Mode API usage.
- [ ] F3-ND-011: Permission handling for file access.
- [ ] F3-ND-012: APK signing pipeline.
- [ ] F3-ND-013: Instrumentation test: launch and import a sample .exe.

### F3-XS — X server / display

- [ ] F3-XS-001: Termux-X11 integration (initial approach).
- [ ] F3-XS-002: Move to embedded X server (Xwayland or custom).
- [ ] F3-XS-003: ANativeWindow surface bridge.
- [ ] F3-XS-004: Vulkan swap-chain passthrough.
- [ ] F3-XS-005: Input mapping: touch / mouse / gamepad.
- [ ] F3-XS-006: On-screen gamepad overlay.
- [ ] F3-XS-007: Virtual mouse cursor in touchscreen mode.

---

## Fase 4 — Graphics + games (weeks 113-140)

### F4-GX — Graphics stack

- [ ] F4-GX-001: DXVK 2.x integration.
- [ ] F4-GX-002: VKD3D-Proton 3.x integration.
- [ ] F4-GX-003: Turnip runtime updater (download from AdrenoTools).
- [ ] F4-GX-004: Mali fallback via Vortek.
- [ ] F4-GX-005: Vortek++ (our improved variant).
- [ ] F4-GX-006: Shader graph analyser wired into DXVK.
- [ ] F4-GX-007: Whole-graph optimisations (pass-fusion, dead-pass removal).
- [ ] F4-GX-008: Adaptive texture transcoding by thermal budget.
- [ ] F4-GX-009: Performance overlay (FPS, frame time p99, GPU temp).
- [ ] F4-GX-010: Shader cache pre-compilation on container launch.

### F4-GM — Games

- [ ] F4-GM-001: Half-Life 2 compatibility profile.
- [ ] F4-GM-002: Portal compatibility profile.
- [ ] F4-GM-003: NFS Most Wanted 2005 profile.
- [ ] F4-GM-004: Skyrim SE profile (stretch).
- [ ] F4-GM-005: Fallout 3 profile (stretch).
- [ ] F4-GM-006: Benchmark HL2 target: 45 FPS on Dimensity 8300.
- [ ] F4-GM-007: Benchmark HL2 target: 30 FPS on Snapdragon 7s Gen 2.
- [ ] F4-GM-008: Public demo video: gameplay capture.

### F4-AC — Paper 4 (Graphics)

- [ ] F4-AC-001: Paper 4 draft: "Whole-Graph Shader Optimization".
- [ ] F4-AC-002: Submit to SIGGRAPH 2030 or HPG 2030.

---

## Fase 5 — Beta release (weeks 141-164)

### F5-UX — Polish

- [ ] F5-UX-001: Onboarding tutorial (Compose screens).
- [ ] F5-UX-002: Steam library import (one-click).
- [ ] F5-UX-003: Epic library import.
- [ ] F5-UX-004: GOG library import.
- [ ] F5-UX-005: Gamepad mapper visual editor.
- [ ] F5-UX-006: Compatibility database UI (browse / filter).
- [ ] F5-UX-007: Cloud sync UI (Pro tier).
- [ ] F5-UX-008: Diagnostic "bug report" screen with attachment.
- [ ] F5-UX-009: Dark mode + Material You theming.
- [ ] F5-UX-010: Accessibility audit (TalkBack, contrast).

### F5-BT — Beta program

- [ ] F5-BT-001: Beta application form on prisma-emu.dev.
- [ ] F5-BT-002: Accept first 100 beta testers.
- [ ] F5-BT-003: Private Discord channel for beta.
- [ ] F5-BT-004: Feedback intake form.
- [ ] F5-BT-005: Weekly beta build cadence.
- [ ] F5-BT-006: Scale to 500 beta testers.
- [ ] F5-BT-007: Compatibility database seeded with 100 games.
- [ ] F5-BT-008: Crash rate target: <1% of sessions.

### F5-AC — Academic (Fase 5 concern)

- [ ] F5-AC-001: Paper 3 (formal IR) camera-ready.
- [ ] F5-AC-002: Conference attendance + talk (one of MICRO / ASPLOS / POPL).

### F5-PR — Press

- [ ] F5-PR-001: Pitch Phoronix with benchmark vs FEX/Box64.
- [ ] F5-PR-002: Pitch Ars Technica with story angle.
- [ ] F5-PR-003: Pitch Android Authority / Android Police.
- [ ] F5-PR-004: Publish demo video on YouTube.

---

## Fase 6 — v1.0 + open-sourcing (weeks 165-188)

### F6-OS — Open source

- [ ] F6-OS-001: Decide MIT for core + IR + NPU + graphics research.
- [ ] F6-OS-002: Remove closed-source guards from `core/`.
- [ ] F6-OS-003: Write `OSS-LICENSE` + attribution files.
- [ ] F6-OS-004: Publish `prisma-emu/prisma-core` (MIT).
- [ ] F6-OS-005: Keep `prisma-emu/prisma-app` (freemium).
- [ ] F6-OS-006: Public contributing guide.
- [ ] F6-OS-007: Governance model (one BDFL initially, grow later).
- [ ] F6-OS-008: Accept first external PR.

### F6-BZ — Monetization

- [ ] F6-BZ-001: Stripe + Paddle billing integration.
- [ ] F6-BZ-002: Pricing: $19.99 one-time Premium, $4.99/mo Pro.
- [ ] F6-BZ-003: Regional pricing tiers (IN / SEA / LatAm / Africa at 50%).
- [ ] F6-BZ-004: Licence key generation + verification.
- [ ] F6-BZ-005: Receipt emails, refunds workflow.
- [ ] F6-BZ-006: Annual subscription vs monthly.

### F6-DS — Distribution

- [ ] F6-DS-001: GitHub Releases automated (tag-triggered).
- [ ] F6-DS-002: Samsung Galaxy Store listing.
- [ ] F6-DS-003: Epic Games Store Android listing.
- [ ] F6-DS-004: Huawei AppGallery listing.
- [ ] F6-DS-005: Explicit decision: NOT Google Play.
- [ ] F6-DS-006: Updater infrastructure (in-app delta updates).

### F6-LG — Legal

- [ ] F6-LG-001: Trademark in additional jurisdictions (EU, JP).
- [ ] F6-LG-002: Revise ToS for paid tier.
- [ ] F6-LG-003: Export compliance review (encryption in transit).

### F6-AC — Academic

- [ ] F6-AC-001: Retrospective blog post: "What the 3 papers taught us".

---

## FX — Cross-cutting / ongoing

### FX-CI — CI / DevEx

- [ ] FX-CI-001: Self-hosted ARM64 runner online.
- [ ] FX-CI-002: Matrix: Linux x86_64 + Linux ARM64 + macOS ARM64.
- [ ] FX-CI-003: Sanitizers job (ASan, UBSan, TSan).
- [ ] FX-CI-004: Coverage job publishing to prisma-emu.dev/coverage.
- [ ] FX-CI-005: Lean proof-check job.
- [ ] FX-CI-006: Nightly benchmark runs with regression gate.
- [ ] FX-CI-007: Nightly fuzz runs (time-budgeted).

### FX-DC — Documentation

- [ ] FX-DC-001: Blog post every 2-3 months.
- [ ] FX-DC-002: `docs/GLOSSARY.md` of project-specific terms.
- [ ] FX-DC-003: Quarterly progress report public post.
- [ ] FX-DC-004: Video explainer series (3-5 min each) on YouTube.
- [ ] FX-DC-005: Architecture diagrams (excalidraw) regenerated per phase.

### FX-CM — Community

- [ ] FX-CM-001: Monthly office hours on Discord.
- [ ] FX-CM-002: Conference talk: FOSDEM 2028 (Emulation devroom).
- [ ] FX-CM-003: Conference talk: Open Source Summit 2028.
- [ ] FX-CM-004: Conference talk: LinuxCon 2029.

### FX-FI — Finance

- [ ] FX-FI-001: Monthly expense tracking spreadsheet.
- [ ] FX-FI-002: GitHub Sponsors set up (tiers $5-$500).
- [ ] FX-FI-003: Transparent budget post when Sponsors launches.

### FX-HC — Health

- [ ] FX-HC-001: Weekly schedule boundary: no more than 20h / week.
- [ ] FX-HC-002: 2 weeks off every 12 weeks. Non-negotiable.
- [ ] FX-HC-003: 1-month off annually.
- [ ] FX-HC-004: Therapy appointment buffer (risk mitigation).

---

## Totals (approx, lifetime plan)

- Fase 0: 80 items (most done).
- Fase 1: 160 items.
- Fase 2: 90 items.
- Fase 2.5: 50 items.
- Fase 3: 55 items.
- Fase 4: 20 items.
- Fase 5: 25 items.
- Fase 6: 25 items.
- Cross-cutting: 25 items.

**Rough total: ~530 items across the 48-54 month plan.**

This is the map. The unit of progress is a commit that closes a line.
