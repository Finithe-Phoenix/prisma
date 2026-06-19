# Prisma — Active Work Queue

> Externalised state for the autonomous F2 follow-on session.
> Mirrors / supersedes the agent's mental list. When a row flips to
> ✅, the agent updates the `Completed at` column with the landing
> SHA and a one-line note in `Notes`. Multi-commit items list every
> commit in order under `SHAs`.

Last updated: 2026-06-18 (Rust-migration batch E1-E9 merged to main, red CI on
main fixed, and robustness-fuzz set landed across decoder/cache/passes/backend —
all green on main).

## Session 2026-06-18 — secure WIP + CI parity (branch `claude/rust-migration-popcnt-decoder-batch`)

Native `cargo` (1.96.0) is now installed on the Windows host, so the long-running
`shell/` migration WIP could finally be built, tested, linted, and committed
(previously toolchain-absent). All pure-Rust crates: build + `cargo test` (443
tests) + `cargo clippy --all-targets -D warnings` (pedantic+nursery) + `cargo fmt`
green. FFI-bridge / ARM64 paths still need CI (cannot run locally).

- `e710fa2` shell/ir,passes: `WriteFlagsPopcnt` IR op + 6-pass plumbing.
- `bc948b5` shell/cache: limits/LRU/invalidate management API + 9 tests (E3).
- `695af1e` shell/decoder: broad one/two-byte opcode coverage (E2).
- `6e76052` shell/backend: lower expanded IR op set to ARM64 (E6).
- `3f63bb1` shell/runtime: decoder->backend smoke path + differential tests (E5/E7).
- `a3d7b72` shell/backend: lower OR/XOR logical flags (AluFlags/WriteFlags) — closes
  a hard `UnsupportedOp` gap; AND/OR/XOR/TEST share x86 logical-flag semantics.
- `3acc9ac` shell: satisfy clippy -D warnings across the pure-Rust workspace.

Verified: decoder emits 31 distinct `Op`s, all handled by the 42-op lowerer (no
decoder->backend integration gap). Next confident widening needs the C++ FFI
differential (new decoder families) and belongs to the codex-decoder lane.

### Landed via PRs (all merged, full CI green incl. ffi-link x86/arm64/windows)

- **PR #28** — the E1-E9 batch above + OR/XOR flags + clippy parity, **plus** the
  fixes that the (previously skipped) Windows gate had masked: i64->usize cast in
  `cfg(unix)`, a test asserting a non-existent `CoreError.guest_addr`, cond-jump
  forward byte drift (B.cond imm19 offset missing from the FFI differential
  matrix + 24 fixtures), and a non-portable `/tmp` in the orchestrator test.
- **CI fix (in #28)** — `ffi-link-windows` pinned to `windows-2022`; the
  `windows-latest` image had rolled off Visual Studio 2022, leaving the whole
  ffi-bridge workflow red on main since 2026-06-15. PR #29 (standalone fix)
  closed as superseded.
- **PR #30** — decoder proptest robustness fuzz + **cache deserialization DoS
  fix** (untrusted `count` no longer drives `HashMap::with_capacity`).
- **PR #31** — passes pipeline robustness fuzz (valid-SSA generator; never
  panics on shift>=64 / div0 / mod0; deterministic).
- **PR #32** — backend IR->ARM64 lowerer robustness fuzz.

The four shell processing stages (decoder, cache, passes, backend) now each have
a proptest robustness harness. KEY: the FFI crates' build.rs tolerates a missing
`PRISMA_CORE_LIB_DIR`, so `cargo clippy --workspace --all-targets -D warnings`
runs the full gate locally without the C++ DLL (only the test *link* needs it).

### Feature arc — `prisma-translator`, the integrated Rust engine (PRs #34-#38)

New crate `shell/prisma-translator` — the first end-to-end Rust translation
pipeline (the facade the plan/C++ describe), combining decoder + passes +
backend + cache:

- **#34** `Translator::translate(addr, bytes)` = decode -> default_pipeline ->
  lower to ARM64 -> cache (FNV-1a keyed). First Rust path to run the optimizer
  and the translation cache end to end.
- **#35** `translate_block(addr, bytes, max_insns)` — translate a straight-line
  run to the next control transfer, caching each instruction independently
  (`is_terminator` over control-flow ops).
- **#36** `TranslatorStats` (cache_hits/misses) + `set_cache_limits`.
- **#37** end-to-end proptest robustness fuzz (5th fuzz surface).
- **#38** SMC support — `invalidate(addr)` + `clear_cache()`.

- **#40** `translate_fused_block` + `prisma_ir::Op::map_refs` — **DONE**. Fuses a
  straight-line run into a SINGLE optimized SSA region so the pipeline optimizes
  ACROSS instruction boundaries. The blocker (the decoder numbers refs per
  instruction) was solved with a function-global SSA ref-visitor over all 94
  `Op` variants: exhaustive match (new variant = compile error), every shifted
  field is `Ref`-typed (compiler rejects non-ref fields), and the 112 operand
  assignments equal the 112 `Ref` fields. Each instruction's refs are renumbered
  into a disjoint range before concatenation. Verified without execution: a
  single-instruction fused block byte-matches the plain path (identity-renumber
  correctness) and a fused block is never larger than the separately-translated
  one (cross-instruction optimization can only shrink it).

The shell processing chain is now a complete, optimizing Rust translation engine:
decode -> renumber+fuse a block -> optimize across instructions -> lower -> cache,
all behind `prisma-translator`, with robustness fuzzing on every stage.

## Currently active

Branch: `main`. Decoder-gap-sweep merged (7294673), branches cleaned up.
CI verde on all workflows (core-build, asan-ubsan, tsan, ir-spec-build,
markdownlint, check-rfc-frontmatter, shell-check, benchmarks-smoke).
Test count: 34 new test cases across signal_handler (4→7), host_features
(4→6), property (5→10), smc_guard (7→11) — commit `3ed9a38`.

## Queue (priority order)

| # | Item | Scope | SHAs | Status | Notes |
|---|------|-------|------|--------|-------|
| E1 | F25-RS-001 Rust IR parity | `shell/prisma-ir`, `ir-spec/` | - | in progress | Owner: `claude-ir-core`. Keep enum/data layout aligned with C++/Lean; run Rust tests plus Lean cross-check before widening. |
| E2 | F25-RS-002 Rust decoder | `shell/prisma-decoder` | - | in progress | Owner: `codex-decoder`. Current smoke tests pass; Rust and C++ now decode `83 /0..7` imm8 ADD/OR/ADC/SBB/AND/SUB/XOR/CMP for the register-direct `rax` smoke set, and live FFI differential includes matching fixtures. The same path now includes non-`rax` probes for `add rbx, imm8`, REX.B `cmp r11, imm8`, base-memory probes for `add [rbx], imm8`, `cmp [rbx], imm8`, `or/and/xor [rbx], imm8`, SIB+disp8 `add [rax + rcx*4 + disp8], imm8`, SIB+disp8 `cmp [rax + rcx*4 + disp8], imm8`, disp32 `add [rbx + disp32], imm8`, REX.X/B SIB `add [r8 + r9*4 + disp8], imm8`, RIP-relative `add/or/and/xor/cmp [rip + disp32], imm8`, and sign-extension coverage for negative disp8 plus negative imm8 memory forms. RIP-relative uses the Rust decoder's new `decode_one_at(..., instruction_guest_pc)` API. ADC/SBB are parity placeholders matching existing C++ register-register behavior: `/2` as Add and `/3` as Sub without carry/borrow materialization; `/7` lowers to `CmpFlags`. Next: broaden beyond Group 1 memory smoke coverage. |
| E3 | F25-RS-003 Rust cache | `shell/prisma-cache` | - | in progress | Owner: `codex-cache`. Rebuild real cache implementation after scaffold churn; include SHA-256 envelope and persistence tests. |
| E4 | F25-RS-004 Rust passes | `shell/prisma-passes` | - | in progress | Owner: `claude-passes`. Implement `PassPipeline`, then const-prop/DCE/CSE with focused unit tests. |
| E5 | F25-RS-005 Rust runtime | `shell/prisma-runtime` | - | in progress | Owner: `claude-runtime`. Align dispatcher/JIT/signal/SMC API with current C API. Dispatcher now exposes cache probe/install helpers over `TranslationCache`; `SmcGuard` tracks page-to-cache-key state, invalidates matching pages, drains pending keys/pages, and `apply_smc_invalidations` applies drained pages to `TranslationCache::invalidate_page`. `GuestFetcher`/`GuestTranslator` now define the no-execute dispatch boundary, `Dispatcher::run_with_adapters` uses that boundary, and `RustSmokeTranslator` wires Rust decoder -> backend for NOP, `mov rax, imm64`, `mov rax, rcx`, `add/sub/and/or/xor rax, rcx`, full `83 /0..7` `rax` imm8 coverage, `add rbx, imm8`, REX.B `cmp r11, imm8`, base-memory `add/cmp/or/and/xor [rbx], imm8`, SIB+disp8 `add [rax + rcx*4 + disp8], imm8`, disp32 `add [rbx + disp32], imm8`, REX.X/B SIB `add [r8 + r9*4 + disp8], imm8`, and RIP-relative `add/or/and/xor [rip + disp32], imm8` without JIT execution; RIP-relative CMP is covered at Rust/C++ decoder level. `shell/prisma-runtime/tests/smoke_differential.rs` pins Rust backend bytes for those one-instruction paths, and `shell/core/tests/smoke_differential.rs` uses live C++ FFI `Translator::translate_with_code` to assert the C++ core emits byte-matched backend output for Rust-pinned fixtures and caches repeated input. Next bounded task: broaden the differential smoke corpus beyond Group 1 memory and verify `CmpFlags`/`CondJumpFlags` parity. |
| E6 | F25-RS-006 Rust backend | `shell/prisma-backend` | - | in progress | Owner: `codex-backend`. Concrete assembler/ABI/lowering slices landed locally: `NOP`, `RET`, `B`, `B.cond`, `BR/BLR X`, `CBZ/CBNZ X`, branch labels/fixups, `MOV Xd,Xn`, `MOVZ/MOVK`, `ADD/SUB Xd,Xn,#imm12`, `ADD/SUB Xd,Xn,Xm`, `AND/ORR/EOR Xd,Xn,Xm`, `LSLV/LSRV/ASRV/RORV Xd,Xn,Xm`, `MUL/UMULH/SMULH/UDIV/SDIV/MSUB Xd,Xn,Xm,Xa`, `CMP Xn,Xm`, `CSET Xd,cond`, unsigned-offset `LDR/STR` for `I8`/`I16`/`I32`/`I64`, stack pair `STP/LDP`, little-endian byte finalization, block prologue/epilogue, patchable-tail offsets, and `Function` lowering for wide constants, `LoadReg`/`StoreReg` over `CpuStateFrame::gpr[]`, `Add/Sub` with register RHS or 12-bit RHS immediates, logical `And/Or/Xor`, variable `Shl/Shr/Sar/Ror`, `Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`, `Compare`, `CmpFlags` with C++-matching sub-64-bit operand alignment, sized `LoadMem`/`StoreMem`, direct `Jump`, boolean `CondJump`, `CondJumpFlags`, and `JumpReg`. Next: differential tests against the C++ backend for register state writes plus flags/ALU slices, then widen the unsupported scalar op set. |
| E7 | F25-RS-007 Rust integration harness | `shell`, `core` boundary | - | review | Owner: `codex-integracion`. `scripts/validate-rust-workspace.ps1` runs the Windows/MSVC gate: fmt, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -D warnings` with the C++ bridge DLL path configured. `.github/workflows/ffi-bridge.yml` now includes a `ffi-link-windows` job that builds `prisma_core_tests`, runs Windows CTest smoke, and executes that script. |
| E8 | F25-RS-008 Windows C++ test runner unblock | `core/build`, `core/tests`, runtime DLL loading | - | review | Owner: `codex-integracion`. Fixed C API exports; Debug builds, `prisma_core_tests.exe --list-tests` exits 0, and direct Catch2 smoke filters `smc_guard:*`, `capi:*`, `JitBuffer*` pass. Remaining CTest execution issue moved to E9. |
| E9 | F25-RS-009 Windows CTest Unicode discovery | `core/CMakeLists.txt`, Catch2 discovery | - | review | Owner: `codex-integracion`. Windows now bypasses broken Unicode discovery and registers 3 ASCII smoke tests (`smc_guard`, `capi`, `JitBuffer`); `ctest -C Debug` passes 3/3. Full per-test Catch2 discovery remains a follow-up if Windows needs granular test reporting. |
| 1 | F2-PS-004 Global CSE via dominators | 1 commit, `core/src/passes/` | `0396c19` | ✅ done | FunctionPassManager + `global_cse`. Wiring into translator deferred (still single-block today). |
| 2 | F2-PS-003 LICM (loop-invariant code motion) | 1 commit | `ff41e83` | ✅ done | Iterates to fixed point; skips multi-entry loops conservatively. |
| 3 | docs(playbook): consolidate agent flows | 1 commit | `78868aa` | ✅ done | AGENT_PLAYBOOK.md landed. |
| 4 | VPERMQ ymm (lane-crossing qword permute) | 1 commit | `fbd714a` | ✅ done | New `VecTbl2` IR op + `vtbl2_q` emitter primitive. Reusable for VPERMD / VPGATHER followups. |
| 5 | F2-IR-052 VPERMD ymm | 1 commit | `ed505a8` | ✅ done | Runtime byte-index build via And + Shl + Pshufb + Add, then VecTbl2. |
| 6 | feat(translator): wire function pipeline | 1 commit | `a5fc152` | ✅ done | translate() now runs build_cfg → function_pipeline_ → flatten → stmt pipeline. Single-block early-out. |
| 7 | BMI2 shift family (SHLX/SARX/SHRX/RORX) | 2 commits | `1824167`, `eb31777` | ✅ done | Variable-count + imm-count rotates, all flag-preserving. |
| 8 | BMI2 MULX | 1 commit | `b0b589b` | ✅ done | Two-dest unsigned multiply, reuses MUL + UMulHi. |
| 9 | BMI2 BZHI | 1 commit | `47cf67d` | ✅ done | Count-saturation via CmpFlags + Select(Ult). |
| 9b | BMI2 PDEP / PEXT | 1 commit | `d9f12b5` | ✅ done | New Pdep/Pext BinOps, VEX decoder, const-folding, software ARM64 bit-loop lowering, and ARM-only e2e coverage. |
| 10 | F2-BK-010 Call/Ret return-stack predictor | 1 commit | `d9f12b5` | ✅ done | CallRel/CallReg/RetAdjusted decoder path, guest-stack lowering, translator exit metadata, dispatcher RAS stats. |
| 11 | Real CALL/RET semantics (opt-in) | 1 commit | `9787f25` | ✅ done | Threaded via `decode_one`'s 4th param + `Translator::set_real_call_ret()`. Decoder emits push/pop sequences when on. Default off keeps the 86 legacy e2e tests untouched. |
| 11b | Migrate e2e corpus to real CALL/RET by default | done in batch | `710ae71` | ✅ done | All 79 e2e tests migrated to `install_halt_return_stack()` AND Translator default flipped to `real_call_ret=true`. Programs with function calls now translate with real semantics by default. |
| 12 | F2-IR-007/008 x87 baseline | 6-8 commits, new domain | `d9f12b5` | ✅ done | Reduced-F64 x87 bridge, decoder/backend coverage, and F2-PS-001 stack forwarding landed; precision divergence documented in RFC 0013. |
| 13 | VPGATHER {D,Q}{PS,PD,D,Q} family | 6-8 commits | `a8a19c0`, `a3c39e6`, `185cef3`, `b061255`, `f287af7`, `ae3b82e` | ✅ done | F2-IR-059 complete: all 16 forms (DD/DQ/QD/QQ + DPS/DPD/QPS/QPD, xmm + ymm) via the VecGather lane descriptor; VSIB xmm4 fixed (xmm12 pinned); kSerializeVersion 1→2. Lean mirror still queued (pre-existing debt). |
| 14 | AES hardware crypto opcodes (AESENC/AESENCLAST/AESDEC/AESDECLAST/AESIMC) | 1 commit | `5811568` | ✅ done | New `VecAes` IR op + `vaes` emitter primitive (5-way switch). AESKEYGENASSIST queued separately. |
| 14b | SHA-NI crypto opcodes | 3-4 commits | `db335b6`, `76b8d16`, `2eaf407`, `558fe51` | ✅ done | F2-IR-060: VecSha op (tag 89), NP decoder chains (implicit xmm0 = LoadVecReg{0}), ARMv8 crypto lowering (SHA256RNDS2 proven realizable on the 4-round SHA256H/H2 with WK2/3=0; SHA1SU0 deliberately avoided), 7 e2e vs lane-exact SDM references. Follow-up queued: full-digest KATs (SHA-1/SHA-256 "abc"). |
| 14c | AESKEYGENASSIST | 1 commit | `4ee4297` | ✅ done | F2-IR-058 landed with `VecAesKeygenAssist`, decoder, ARM64 AESE/TBL lowering, serialization/profiler/DCE plumbing, and tests. |
| 14d | MOVBE (`0F 38 F0 / F1`) | 1 commit | `4e4828c` | ✅ done | New `Bswap` IR op; REV / REV16 ARM64 mapping. |
| 14e | CRC32 SSE4.2 (`F2 0F 38 F0 / F1`) | 1 commit | `de95485` | ✅ done | New `Crc32c` IR op; direct ARM64 CRC32C{B/H/W/X}. |
| 14f | SHA full-digest KATs (FIPS 180-4) | 2 commits | `5f048f5`, `ae97518` | ✅ done | Canonical SHA-NI loops (unrolled, in-test x86 encoder) over empty/"abc"/two-block messages; host-side ref_* mirror asserts on every host, translate() on non-ARM64, JIT half on ARM64 gated on host SHA crypto. |
| 14g | HostFeatures FEAT_SHA1/SHA256 + guest CPUID SHA bit | 3 commits | `f5b863e`, `1967a92`, `ce6869b` | ✅ done | First real `host_features()` consumer. CPUID leaf model baked at translate time (leaf 0 max-leaf, leaf 7.0 EBX, SDM >max clamp); flag-free dispatch (CPUID affects no flags per SDM); bit 29 gated on FEAT_SHA1 && FEAT_SHA256. Codex+Gemini review in docs/REVIEWS/2026-06-11. |
| 14i | Guest feature discovery: XGETBV + VZEROUPPER/ALL + CPUID 0/1/7 | 4 commits | `b61b865`, `bc0fe74`, `fbe0c5d`, `2b5a7b2` | ✅ done | FEAT_AES detection + AES e2e gate; Xgetbv IR op (tag 90) with baked XCR0=0x7; VZEROUPPER/ALL synthesized from VecConstant+StoreVecReg(Hi); vendor GenuineIntel + leaf 1 honest bits (FPU/CMOV/SSE/SSE2 + SSE3/SSSE3/FMA/CX16/SSE4.1/MOVBE/OSXSAVE/AVX, AESNI host-gated) + leaf 7 BMI2. Self-review dropped TSC/CX8/POPCNT/SSE4.2 (decoder can't back them); Codex caught VEX scalar vvvv merge bug (CVTSS2SD/MOVSS/ROUNDSS) — fixed + pinned. Queued: 32-bit POPCNT/LZCNT/TZCNT/BSF/BSR forms, PCMPxSTRx, RDTSC via CNTVCT_EL0, CVTSI2SS/SD upper-lane merge, SSE3/SSSE3/SSE4.1 thin spots. |
| 14j | Decoder gap sweep (atomics widths, BSF/BSR real, RDTSC real, CMPXCHG8B, 16B-ZF fix, CVTSI2 merge, SSE3 completion) | 1 commit | `d4508f4` | ✅ done | Two latent silent-wrong bugs fixed (BSF/BSR stored zero; CMPXCHG16B guest ZF inverted). TSC/CX8/POPCNT re-advertised. ir::Rdtsc (tag 91) via mrs CNTVCT_EL0. Follow-ups still queued: PCMPxSTRx (re-advertise SSE4.2), true-atomic CAS lowering (LoadMemTSO/StoreMemTSO split is not a real CAS under guest MT), SSSE3/SSE4.1 thin spots. LZCNT/TZCNT CF flag and 16-bit atomics are now implemented in the current session. |
| 14k | XADD arithmetic flags | 1 commit | `96fde35` | ✅ done | Added side-effecting `AluFlags` for implicit ALU/NZCV flag writes, lowered Add/Sub/And via existing flag-setting ARM64 ops, taught passes/serialization/profiler/validation about the op, and made XADD emit flags from `old_dst + src` before writeback. Includes decoder/pipeline tests plus ARM64 e2e `XADD` + `JC`. |
| 14h | Lean mirrors: vecGather + vecSha | 1 commit | `2b5407a` | ✅ done | Constructor-only mirrors (repStos precedent): no 128-bit carrier in the Lean model yet, semantics deferred; DCE + ConstProp case-splits stay exhaustive; sorry budget unchanged at 3. |
| 15 | Direct branch threading | 4-7 commits | `b1e112f`, `194fcca`, `b7e4cfd`, `2ad2e26`, `9a84fba`, `6e3a541`, `a25bbbc` | 🟡 partial | Stage 2 threads direct CallRel and REP clamp re-entry/fallthrough successors through the hash-checked dispatcher path. Stage 3 adds the safe AArch64 branch patch primitive in `JitSlabPool`; Stage 4 emits patchable tail slots + Translator metadata for JumpRel/CallRel; Stage 5 adds Translator patch/unpatch lifecycle + stale incoming unpatch on SMC retranslation; Stage 6 enables one-hop dispatcher patches with target hash-check, halt/max-step guards, and chain rejection; Stage 6b gates auto-patching to sources with no guest-memory writes; Stage 6c exposes direct JIT patch attempt/apply/reject/unpatch/execute counters through runtime stats plus C/Rust ABI v2. Multi-hop and CallRel auto-patching remain queued behind SmcGuard/page invalidation. |
| 16a | RFC 0014 — C-ABI FFI boundary core↔shell | 1 commit | `09efbc4` | ✅ done | Contract: pure C ABI, opaque handles, status codes, panic/exception firewall, `PRISMA_CAPI_VERSION`. |
| 16b | `prisma_core_c` C API (header + impl + tests) | 1-2 commits | `ea37029` | ✅ done | `capi.h` + shared lib target + 8 Catch2 cases (905/905 suite green in container). |
| 16c | Rust bridge crates `core-sys` + `core` | 1-2 commits | `9f68ca6` | ✅ done | Hand-written extern decls + safe RAII wrapper + 9 cross-language integration tests. |
| 16d | Hybrid e2e: PE loader → DBT | 1-2 commits | `ef542f4` | ✅ done | `map_image()` with checked bounds + pe_e2e.rs: PE → Rust parse/map → C++ translate (all hosts) / execute (ARM64). |
| 16e | CI: `ffi-link` workflow | 1 commit | `bf86cca` | ✅ done | ffi-link (x86_64) + ffi-link-arm64 (real ARM64 JIT execution in CI, public-repo runner). |
| 17a | CI: full C++ suite on ARM64 runner | 1 commit | `0ff071e` | ✅ done | `core-build-arm64` — the whole e2e JIT corpus now executes in CI, not just on Apple Silicon. |
| 17b | Proptest decoder fuzzing via bridge | 1 commit | `0e8a1e2` | ✅ done | 3 properties (noise, instruction-shaped, determinism+cache), 1,280+ cases/run; complements AFL++. |

## Completed (current session — CI recovery + test strengthening)

| # | Item | SHA | Note |
|---|------|-----|------|
| – | `feat(ir,decoder): model LZCNT TZCNT flags` | `156a664` | Danny's commit: flag write for LZCNT/TZCNT CF+ZF. |
| – | `fix(lint): remove trailing spaces in bug_report.md` | `9447037` | Claude: markdownlint MD009. |
| – | `test(cache): align upsert/compact tests with documented semantics` | `78e5d84` | Danny: fixed invented expectations. |
| – | `ci(clang-format): scope to changed files + advisory` | `ef14fff` | Danny: unblock CI by making clang-format advisory. |
| – | `test: fix invented expectations in parallel-landed test batch` | `bca6320` | Danny: sha256, ARCH_SET_FS/GS, FNV-1a avalanche fixes. |
| – | `test: strengthen signal_handler, host_features, property, smc_guard coverage` | `3ed9a38` | Claude: 34 new test cases across 4 files (262 insertions). |
| – | `rfc(0015): Rust core migration roadmap + prisma-ir crate` | `dac9b6a` | Claude: RFC 0015 (467 lines), Fase 0 IR types (620 lines), 5 crate skeletons, agent playbook, differential testing guide, workspace update. |

## Completed (this session)

| # | Item | SHA | Note |
|---|------|-----|------|
| – | RFC 0011 — AVX-256 pair-of-Vec128 + FMA | `143a330` | Post-hoc, resolves Blocker B option 2 for AVX/FMA commits. |
| – | RFC 0012 — wide-form BinOps + REP string ops | `9acb5e6` | Surfaces Blocker A in §1 with the canonical clamp sketch. |
| – | REVIEW: Blocker B resolved note | `5829cb7` | — |
| – | `fix(runtime): #include <cstdlib>` | `08c6cf8` | Unblocks scaffolding-check on clang-17/libstdc++. |
| – | `ci(ir-spec)`: tolerate zero sorries in budget check | `03f4d88` | grep+pipefail false-positive. |
| – | **Blocker A — REP DoS bounded + PC-aware re-entry** | `5756084` | RepStos/RepMovs are now block terminators; 16 MiB clamp. |
| – | REVIEW: Blocker A resolved | `553ee46` | — |
| – | `docs(rfc)`: MD004/MD026 markdownlint nits | `e2fc8a7` | — |
| – | `test(zydis-diff)`: SUCCEED instead of !shouldfail+WARN | `115e69b` | — |
| – | `ci(shell-stub)`: split clippy/rustfmt `--component` | `bf91c38` | rustup syntax. |
| – | `fix(shell)`: cargo fmt + clippy-clean orchestrator | `b8d74c6` | 20 warnings → 0, 22/22 tests verde. |
| – | F2-IR-049 — VPTEST ymm + `WriteFlagsPtestYmm` | `67a7336` | New emitter primitive `vptest_ymm`. |
| – | F2-IR-050 — VPBLENDVB / VBLENDVPS / VBLENDVPD VEX | `6a21ba5` | xmm + ymm; reuses `VecBlend`. |
| – | spec(lean): wide-form BinOps + RepStos/RepMovs | `b7a8f31` | `.sorry-budget` 0 → 3 for signed corner cases. |
| – | REVIEW: VPTEST/VBLEND/Lean follow-ons + two-eyes tally | `b9b3e7e` | — |
| – | spec(lean): DCE + ConstProp case-splits for new ops | `9d1660a` | Closes Lean exhaustive-match failure. |
| – | docs: add WORK_QUEUE.md | `59ac4c0` | This file. |
| 1 | feat(passes): F2-PS-004 — FunctionPassManager + global_cse | `0396c19` | 780/780 verde Debug + ASan/UBSan. |
| 2 | feat(passes): F2-PS-003 — loop_invariant_motion | `ff41e83` | 788/788 verde Debug + ASan/UBSan. |
| 3 | docs: AGENT_PLAYBOOK.md | `78868aa` | Container, 13-file IR-op recipe, function-pass authoring, two-eyes, Lean, CI, WORK_QUEUE contract. |
| 4 | feat(ir,decoder,backend): F2-IR-051 — VPERMQ ymm via VecTbl2 | `fbd714a` | 790/790 verde Debug + ASan/UBSan. |
| 5 | feat(decoder): F2-IR-052 — VPERMD ymm | `ed505a8` | 792/792 verde Debug + ASan/UBSan. Runtime byte-index build. |
| 6 | feat(translator): wire function pipeline into translate() | `a5fc152` | 792/792 verde; GCSE/LICM now live on multi-block decoded regions. |
| 7 | feat(decoder): F2-IR-053 — SHLX / SARX / SHRX | `1824167` | 795/795 verde Debug + ASan/UBSan. |
| 8 | feat(decoder): F2-IR-053 followup — RORX | `eb31777` | 796/796 verde. |
| 9 | feat(decoder): F2-IR-053 followup — MULX | `b0b589b` | 797/797 verde Debug + ASan/UBSan. |
| 10 | docs(queue): mark BMI2 done + queue follow-ups | `a82043d` | — |
| 11 | feat(decoder,translator): F2-IR-054 — real CALL/RET (opt-in) | `9787f25` | 802/802 verde Debug + ASan/UBSan; flag default off for back-compat. |
| 12 | feat(decoder): F2-IR-053 followup — BMI2 BZHI | `47cf67d` | 803/803 verde Debug + ASan/UBSan. |
| 13 | feat(decoder): F2-IR-054 followup — RET imm16 real-mode | `9e6ed9c` | 804/804 verde. |
| 14 | feat(decoder): F2-IR-054 followup — CALL r/m64 real-mode | `779fb17` | 806/806 verde. |
| 15 | feat(runtime): Dispatcher::install_halt_return_stack() | `dd94171` | 806/806 verde. POC migration of 1 test. |
| 16 | feat(ir,decoder,backend): F2-IR-055 — AES-NI primitives | `5811568` | 810/810 verde Debug + ASan/UBSan. |
| 17 | docs(queue + playbook + architecture) — checkpoints | `a2c2f7c` `78868aa` `fee97f6` | — |
| 18 | feat(ir,decoder,backend): F2-IR-056 — MOVBE / Bswap | `4e4828c` | 813/813 verde Debug + ASan/UBSan. |
| 19 | feat(translator): flip real_call_ret default + migrate corpus | `710ae71` | 813/813 verde Debug + ASan/UBSan. **The real CALL/RET unlock.** |
| 20 | feat(ir,decoder,backend): F2-IR-057 — CRC32 / Crc32c | `de95485` | 816/816 verde Debug + ASan/UBSan. |
| 21 | feat(ir,decoder,backend): BMI2 PDEP / PEXT | `d9f12b5` | 848/848 verde Debug + ASan/UBSan + Zydis; software ARM64 loop lowering + const-folding + ARM-only e2e. |
| 22 | feat(runtime,backend): F2-BK-010 call/ret return-stack | `d9f12b5` | 848/848 verde Debug + ASan/UBSan + Zydis; first-class call/ret terminators + dispatcher RAS hit/miss counters. |
| 23 | feat(core): x87 reduced-F64 bridge + stack forwarding | `d9f12b5` | 848/848 verde Debug + ASan/UBSan + Zydis; RFC 0013 documents precision scope. |
| 24 | feat(runtime): dispatcher direct-thread cache | `5a4fb7e` | Direct branch successors can run from the executable cache without another translate() call; SMC hash checks preserved. |
| 24b | runtime: add JIT branch patch primitive | `194fcca` | Adds `JitSlabPool::patch_aarch64_branch()` with ownership/alignment/range checks, W^X-aware write, icache invalidation, and focused unit coverage. 992/992 (`~signal_handler*`) green in mounted container. |
| 24c | translator: add patchable direct-exit tail slots | `b7e4cfd` | Adds an ABI tail epilogue with an unpatched `b fallback` slot, Emitter cursor offsets, and Translator patch-site metadata for JumpRel/CallRel only. CondJumpRel remains unpatchable until dual-path/SMC policy lands. 994/994 (`~signal_handler*`) green in mounted container. |
| 24d | translator: add direct-exit patch lifecycle | `2ad2e26` | Translator now keeps the owning `JitBlock`, can patch/unpatch direct-exit slots to already-translated targets, rejects invalid/self targets, and unpatches stale incoming edges on SMC retranslation. 997/997 (`~signal_handler*`) green in mounted container. |
| 24e | runtime: enable one-hop direct JIT patches | `9a84fba` | Dispatcher now patches JumpRel/CallRel tail slots after hash-checked successor install/lookup, verifies patched targets before entry, unpatches on stale/halt/budget conflicts, and accounts both source and target blocks. Multi-hop chains are rejected. 999/999 (`~signal_handler*`) green in mounted container. |
| 24f | runtime: gate direct JIT patches on guest writes | `6e3a541` | Auto-patching now requires `DirectPatchSite::auto_patch_safe`, false for CallRel and any source with guest-memory writes/InlineAsm/REP. Fixes halt-PC priority at exact max_steps. 1001/1001 (`~signal_handler*`) green in mounted container. |
| 24g | runtime: expose direct JIT patch stats | `a25bbbc` | Adds attempt/apply/reject/unpatch/execute counters to `DispatchStats`, appends them to `prisma_dispatch_stats` with `PRISMA_CAPI_VERSION=2`, updates Rust bindings/wrapper structs, and pins stale-target unpatch accounting. 1003/1003 (`~signal_handler*`) green in mounted container. |
| 25 | feat(ir,decoder,backend): F2-IR-058 - AESKEYGENASSIST | `4ee4297` | 897/897 Debug + ASan/UBSan (`~signal_handler*`) + Zydis 897/897. Gemini review caught the RIP-relative test gap; fixed before commit. |
| 26 | feat(ir,decoder,backend): F2-IR-059 complete - VPGATHER/VGATHER family | `a3c39e6` `185cef3` `b061255` `f287af7` `ae3b82e` | 932/932 (5227 assertions) Debug + ASan/UBSan in container. VSIB xmm4 fix, VecGather lane descriptor, Q widths, FP forms, ymm lo/hi (incl. index_lane_base=2 split-index and QD chained-halves geometries). 9 ARM64 e2e gathers with poisoned-masked-index proofs. Codex + Gemini external review (see docs/REVIEWS/). |

## Standing decisions (carry across items)

- **Recipe for new IR ops**: HANDOFF.md §4 lists 11 mandatory file touches.
  Two more files in practice (`core/src/translator/translator.cpp::is_block_terminator`
  if the op terminates blocks; `core/src/ir/cfg.cpp::is_terminator` likewise).
  Plus tests in 3 places (`test_decoder.cpp`, `test_lowering.cpp`, `test_profiler.cpp`).
- **Sanitizer validation is local**: `cmake --build core/build-asan` in the
  `prisma-build` container runs the full suite under ASan+UBSan in ~3 min.
  Use this before every push to catch UB / memory issues that the
  default Debug build misses.
- **`.sorry-budget` is the tripwire**: bump only when a real sorry is added.
  CI fails on > budget. Drop the budget back down once a proof lands.
- **Two-eyes ledger**: every solo Claude commit in IR / decoder / lowering /
  emitter territory needs eventual co-sign. Tally lives in
  `docs/REVIEW_F2_SESSION.md` under the "Two-eyes tally" entry.
- **Container lifecycle**: `prisma-build` container persists across
  ScheduleWakeup-style restarts. `apt-get install` + `wget` of cmake
  has been done once; rebuilds are seconds. `~/.cargo` + rustup also installed.

## Skipped / out-of-scope (this session)

- **F2-CLAUDE-WAIVER.md** authoring — Danny's call to write, not the agent.
- **Apple Silicon E2E adversarial test** of the REP clamp — needs the
  other machine; the wired test SUCCEED-skips on x86_64.
- **Lean signed-arithmetic proofs** for sMulHi/sDiv/sMod — tracked
  under F1-LN-014/015/016 placeholders; not blocking F2 merge.
