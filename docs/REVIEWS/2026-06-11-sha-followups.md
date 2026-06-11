# Review record — SHA-NI follow-ups (F2-IR-060 completion)

Branch: `claude/sha-followups` (6 commits over `8ed9a54`, stacked on
`claude/sha-ni` / PR #12).
Author: Claude (solo) — lowering + runtime + tests + ir-spec
territory; this record is the two-eyes mechanism per CONTRIBUTING.md.

## Scope

| Commit | What |
|---|---|
| `5f048f5` | FIPS 180-4 full-digest KATs: canonical SHA-NI loops, host mirror + ARM64 JIT |
| `f5b863e` | HostFeatures: FEAT_SHA1 / FEAT_SHA256 detection (sysctl / HWCAP) |
| `1967a92` | Guest CPUID leaf model (leaf 0 + leaf 7.0), SHA bit 29 gated on host crypto |
| `2b5407a` | Lean constructor mirrors `vecGather` / `vecSha` + DCE/CP proof arms |
| `ae97518` | KATs assert `translate()` success on non-ARM64 hosts (decode+lower coverage) |
| `ce6869b` | Review findings applied (see below) |

## Key decisions

- The KAT loops follow the Intel whitepaper register dance exactly;
  a host-side mirror built from the `sha_e2e::ref_*` SDM models runs
  the identical sequence and is asserted against the FIPS digests on
  every host, so loop-orchestration bugs cannot hide behind the
  ARM64 gate. The JIT half re-runs it on hardware.
- CPUID values are baked at translation time from
  `runtime::host_features()` (first real consumer). The leaf
  dispatch is flag-free (orr/eor/lsr + cbz/cbnz) because the SDM
  says CPUID affects no flags; W-forms give EAX/ECX (not RAX/RCX)
  comparison. Fase 2.5 caveat recorded in-code: baked values make
  cached blocks host-feature-dependent; the P2P trust envelope must
  carry the feature set.
- Lean mirrors are constructor-only (repStos precedent): the Lean
  model has no 128-bit carrier yet, so semantics stay deferred and
  the sorry budget is untouched (3/3, all pre-existing).

## Validation

- Debug suite: 963/963 (5401 assertions), x86_64 container.
- ASan + UBSan: 963/963 (5401 assertions), no reports.
- `lake build` green; CI sorry-budget grep unchanged.
- Host-mirror KAT halves (39 assertions) + translate-only halves
  (6 assertions) pass on x86_64; JIT halves pending the ARM64 CI
  runner as usual.

## External reviews

- **Codex** (gpt-5.5, read-only sandbox): **no blockers, 1 MAJOR**
  — the SHA e2e execution tests gated only on `is_arm64`; an ARM64
  host without the optional crypto extensions would SIGILL inside
  JIT code (no `ScopedProtected` scope in the dispatcher run loop).
  Fixed in `ce6869b`: all 9 SHA execution tests now skip via
  `sha_e2e::host_has_sha_crypto()` — a gate this branch's own
  FEAT_SHA1/SHA256 detection makes possible. Codex explicitly
  verified the SHA-256 schedule choreography, SHA-1 E ping-pong and
  finalize trick, the in-test x86 encoders, CPUID masking and
  flag-freedom, HWCAP_SHA2 naming, scratch allocation, and Lean arm
  arity as sound.
- **Gemini** (gemini-cli, non-interactive): **no blockers**, 1
  MINOR + 3 NITs. MINOR (applied in `ce6869b`): the SDM clamps
  basic leaves above the max to the highest basic leaf instead of
  returning zeros — implemented flag-free with `lsr #3` (≥8) and a
  `lsr #31` carve-out for the unmodelled extended range, with new
  e2e sections. NITs declined and recorded: zero vendor string
  stays deliberate until leaf 1 is modelled; the `eor` constant
  temporary is fine for a low-frequency op; the `jmp +0` block
  splits are intentional translated-block-size hygiene.

## Verdicts

- Codex: PASS after fix (1 major test-gating gap, applied).
- Gemini: PASS (1 minor fidelity improvement, applied; nits
  recorded as deliberate).

## Notes

- The 7 per-instruction SHA tests from PR #12 inherited the same
  crypto gate in `ce6869b` (they predate the detection primitive).
- Queued follow-ups: leaf 1 CPUID modelling (SSE/AVX feature bits +
  vendor string decision), Lean 128-bit value carrier for semantic
  vector mirrors, `XGETBV`/OSXSAVE story before any AVX
  advertisement.
