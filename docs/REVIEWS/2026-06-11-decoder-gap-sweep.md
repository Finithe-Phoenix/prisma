# Review record — decoder gap sweep

Branch: `claude/decoder-gap-sweep` (2 commits over `609693f`,
stacked on `claude/guest-feature-discovery` / PR #14).
Author: Claude (solo) — decoder + emitter + lowering + validator
territory; this record is the two-eyes mechanism per CONTRIBUTING.md.

## Scope

| Commit | What |
|---|---|
| `d4508f4` | Atomics widths + CMPXCHG8B + 16B-ZF fix + real BSF/BSR + real RDTSC + CVTSI2 merge + SSE3 completion + TSC/CX8/POPCNT re-advertised |
| (next)    | Codex review fixes: rm-aliases-RAX store order, POPCNT result-flags, validator Rdtsc |

## Latent silent-wrong bugs fixed (found by reading, pinned by tests)

- BSF/BSR decoded to a placeholder that stored ZERO to the
  destination — every bit-scan in guest code returned 0 silently.
  Now real (Tzcnt / width-1 − Lzcnt) with the src==0 path preserving
  the destination and ZF = (src==0).
- CMPXCHG16B left guest ZF INVERTED (compare of the eq-flag against
  0): `jz/jnz` after `lock cmpxchg16b` branched backwards. Fixed by
  comparing against 1 with Selects keyed on Eq; e2e pins the `jz`
  direction on the 8B form.
- CMPXCHG with the r/m operand aliasing RAX let the unconditional
  accumulator writeback overwrite the destination write (Codex
  finding, pre-existing): store order is now accumulator-then-DEST
  per the SDM; same fix for XADD's same-register aliasing
  (SRC←DEST first, DEST←TEMP last). e2e pins `cmpxchg rax, rcx`.

## External reviews

- **Codex**: 1 BLOCKER + 3 MAJORs + 1 MINOR.
  - BLOCKER (atomicity, shared with Gemini — see below).
  - MAJOR rm-aliases-RAX: real, fixed (store order + e2e).
  - MAJOR POPCNT SF: real, fixed with Codex's own suggestion —
    compare the RESULT against zero (same ZF, and N = sign(count) =
    0 matches the architectural SF clear).
  - MAJOR CMPXCHG8B/16B flags beyond ZF: CmpFlags rewrites C/N/V
    where the SDM says "unaffected". Recorded divergence (in-code
    comment); a ZF-only flag-write model is queued. Compiler-emitted
    code tests ZF.
  - MINOR validator: real — `Stmt{ref, Rdtsc{}}` would be rejected
    as ImpureHasResult; Rdtsc added to the result-bearing list.
  - Verified sound: CNTVCT encoding, RDTSC zero-extension, BSF/BSR
    NZCV ordering, CVTSI2 lane merge, ADDSUB/HSUB lane geometry.
- **Gemini**: 1 BLOCKER (atomicity, same as Codex) + 1 MAJOR + 1
  MINOR + 1 NIT.
  - MAJOR I32/I64 mixing in Selects/Shl: triaged false positive by
    IR convention — every ref lives in a 64-bit register and narrow
    loads/ops produce zero-extended values (the whole 32-bit x86
    surface relies on this); the mixed-size statements execute in
    the 988-case suite.
  - MINOR POPCNT SF: same as Codex's, already fixed.
  - NIT RDTSCP serialization not modelled: documented in the decode
    comment (blocks execute sequentially in this DBT today).
  - Verified sound: encoding, zext, blend masks and shuffle controls
    lane-by-lane, CVTSI2 merge, BSF corner.

## The atomicity BLOCKER — disposition (flagged for Danny)

Both reviewers flag that the CMPXCHG/CMPXCHG8B/16B/XADD decode
(TSO load → Select → TSO store) is not a true CAS: a concurrent
guest writer between the load and store can be lost. This is the
established model of the ENTIRE atomics family since Fase 1 (0F B1
CMPXCHG shipped this way; CX16 was advertised in PR #14 with the
same property). Today's guests are single-threaded — there is no
guest thread layer or syscall surface yet — so the model is
internally consistent, and the real fix (a first-class `Cas` IR op
lowered to LSE `casal`, which the emitter already exposes) is the
Pillar-3-era work item now explicitly queued. The in-code comment
on CMPXCHG8B states the limitation. If Danny prefers conservatism,
dropping the CX8/CX16 bits is a 2-line change.

## Validation

- Debug + ASan/UBSan: 989/989 in the x86_64 container.
- New ARM64 e2e: RDTSC monotonic+nonzero, CMPXCHG8B success/failure
  with ZF direction via `jz`, CMPXCHG rax-alias, BSF/BSR values with
  dst-preserve and the 32-bit form, POPCNT 32-bit with ZF.

## Queued follow-ups

- First-class `Cas` IR op → LSE `casal` lowering; then revisit the
  16B form (needs CASP) and the whole LOCK family.
- ZF-only flag-write model for CMPXCHG8B/16B (and POPCNT's full
  flag-clear contract).
- LZCNT/TZCNT CF = (src==0); XADD arithmetic flags.
- 16-bit atomics (preserve-upper StoreReg semantics first).
- PCMPISTRI/PCMPESTRI → re-advertise SSE4.2 (next decoder arc).
- SSSE3 PMADDUBSW/PMULHRSW/PSIGN; SSE4.1 INSERTPS/BLENDPS-imm/DPPS/
  PACKUSDW.
