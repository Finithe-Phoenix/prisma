# Review record — guest feature discovery (XGETBV / VZERO* / CPUID 0-1-7)

Branch: `claude/guest-feature-discovery` (4 commits over `a6effcf`,
stacked on `claude/sha-followups` / PR #13).
Author: Claude (solo) — decoder + lowering + runtime + tests
territory; this record is the two-eyes mechanism per CONTRIBUTING.md.

## Scope

| Commit | What |
|---|---|
| `b61b865` | HostFeatures: FEAT_AES detection + AES e2e execution gate |
| `bc0fe74` | XGETBV (tag 90), VZEROUPPER/VZEROALL, CPUID leaves 0/1/7 + vendor |
| `fbe0c5d` | Self-review: drop TSC / CX8 / POPCNT / SSE4.2 bits |
| `2b5a7b2` | Codex BLOCKER: VEX scalar vvvv merge (CVTSS2SD / MOVSS / ROUNDSS) |

## Key decisions

- Vendor string is "GenuineIntel" with a Sandy-Bridge-era signature
  (0x206A7) — the Rosetta 2 precedent. Feature bits, not the vendor,
  are the compatibility contract; recorded for Danny to veto.
- OSXSAVE is advertised without XSAVE (bit 26): the XSAVE/XRSTOR
  instructions and CPUID leaf 0xD are unmodelled, and the canonical
  AVX gates (Intel's sequence, MSVC `__isa_available`, glibc) check
  OSXSAVE + XGETBV only. Codex rates the inconsistency MAJOR with
  the opposite trade-off (some guests may *skip* AVX); skipping is
  the safe direction, so deliberate — revisit when XSAVE lands.
- Family bits with known thin spots (SSE3 ADDSUBPS/HSUBPS, SSSE3
  PMADDUBSW/PMULHRSW/PSIGN, SSE4.1 INSERTPS/BLENDPS-imm/DPPS/
  PACKUSDW/MOVNTDQA) stay advertised: every gap is an UNDECODED
  opcode → loud TranslateFailed, which is the honest-failure data
  the project wants; all queued.
- Bits whose canonical use-case is broken or hazardous were dropped
  in `fbe0c5d` (self-review, independently confirmed by BOTH
  external reviewers afterwards): TSC (RDTSC returns constant zero —
  calibration loops would hang), CX8 (CMPXCHG8B without REX.W does
  not decode), POPCNT (decoder is REX.W-only; the 32-bit form
  compilers emit for `int` is missing), SSE4.2 (PCMPISTRI/PCMPESTRI
  not decoded — the reason string functions check the bit).

## Validation

- Debug suite: 973/973 (x86_64 container); ASan/UBSan green.
- New coverage: decoder (XGETBV, VZEROUPPER, VZEROALL, vvvv reject,
  3× VEX-scalar vvvv-merge pins), lowering (XGETBV flag-free shape),
  serialize/cfg/profiler/zydis for the new op, e2e (XGETBV EDX:EAX +
  RBX survival, VZERO xmm/ymm_hi matrix, CPUID vendor + leaf-1 bits
  + AESNI gating + BMI2).

## External reviews

- **Codex**: 2 BLOCKERs, 2 MAJORs. BLOCKER 1 (TSC/CX8/POPCNT lies)
  was already fixed in `fbe0c5d` before the review returned —
  three-way convergence (self-review + both externals) on the same
  bit set. BLOCKER 2 was real and unique: VEX-encoded scalar
  converts / moves / rounds kept legacy merge semantics, silently
  reading dst instead of vvvv for the upper lanes — the silent-wrong
  class. Fixed in `2b5a7b2` with decoder pins; the scalar arithmetic
  family already honoured vvvv since F2-IR-048. MAJORs (OSXSAVE/
  XSAVE inconsistency, family-bit thin spots) recorded as deliberate
  above. Codex verified XGETBV and VZERO* fidelity as sound.
- **Gemini**: 1 BLOCKER + 3 MAJORs — exactly the four capability
  lies already fixed in `fbe0c5d` (CX8, TSC, SSE4.2, POPCNT-32).
  1 MINOR: XGETBV with ECX!=0 should #GP rather than return zeros —
  deferred with reason: the runtime has no guest exception delivery
  yet (Trap is itself a placeholder); documented in-code.
  Gemini confirmed VZEROUPPER placement/synthesis and the IR-recipe
  completeness for tag 90.

## Verdicts

- Codex: PASS after fix (1 real blocker, applied + pinned).
- Gemini: PASS (its blocker/majors were already fixed pre-return;
  minor deferred with documented reason).

## Queued follow-ups

- 32-bit forms of POPCNT / LZCNT / TZCNT / BSF / BSR (decoder is
  REX.W-only across the family) — then re-advertise POPCNT.
- PCMPISTRI / PCMPESTRI / PCMPISTRM / PCMPESTRM — then re-advertise
  SSE4.2.
- RDTSC via CNTVCT_EL0 — then re-advertise TSC.
- CVTSI2SS/SD upper-lane merge (IntToFpScalar needs a merge
  operand; legacy preserves dst, VEX merges vvvv).
- CMPXCHG8B (0F C7 /1 without REX.W) — then re-advertise CX8.
- SSE3/SSSE3/SSE4.1 thin spots listed above.
- XSAVE/XRSTOR + CPUID leaf 0xD modelling, then set XSAVE bit 26.
