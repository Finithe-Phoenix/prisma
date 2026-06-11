# Review record — VPGATHER/VGATHER family completion (F2-IR-059)

Branch: `claude/vpgather-family` (5 commits over `7dbfe0c`).
Author: Claude (solo) — decoder + IR + lowering territory, so this
record provides the second set of eyes per the two-eyes rule
(CONTRIBUTING.md; mechanism anticipated by REVIEW_F2_SESSION.md
Blocker B option 1).

## Scope

| Commit | What |
|---|---|
| `a3c39e6` | `parse_modrm` vsib mode — xmm4 index expressible, xmm12 pinned |
| `185cef3` | `VecGather` lane descriptor + generalized lowering + serialize v2 |
| `b061255` | VPGATHERDQ / VPGATHERQQ / VPGATHERQD xmm |
| `f287af7` | VGATHERDPS / DPD / QPS / QPD |
| `ae3b82e` | All eight ymm forms (allowlist + lo/hi split geometries) |

## Validation

- Debug suite: 932/932 (5227 assertions), x86_64 container.
- ASan + UBSan: 932/932 (5227 assertions), no reports
  (`~signal_handler*`, mirrors CI).
- 9 ARM64-gated e2e gathers execute for real on the
  `core-build-arm64` CI job, each proving masked-off lanes never
  touch memory via poisoned indices (dword and qword poison).

## External reviews

Both reviewers received the full unified diff vs `main` with a
checklist covering per-form architectural semantics, ARM64 lowering
widths, and the serialization version bump.

- **Codex** (gpt-5.5, reasoning xhigh, read-only sandbox,
  session `019eb3a8-0051-7ed3-b3a3-d734afe0eb4f`): **No findings.**
  Independently checked that the parallel legacy serializer
  (`core/src/ir/serialization.cpp`, separate tag table without
  vector ops) is unaffected by the tag-88 change, and that no
  exact-output pretty-print test pins the old `vgather.s4` form.
- **Gemini** (gemini-cli, non-interactive): 6 findings, triaged
  below. None required a code change.

### Gemini triage

| # | Sev claimed | Verdict | Why |
|---|---|---|---|
| 1 | BLOCKER | False positive | Claimed the handler lost the register extraction, #UD overlap check and base computation. Those lines are unchanged context the unified diff fragmented; the branch compiles and all positive + #UD decode tests pass (932/932). |
| 2 | MAJOR | Known limitation, pre-existing | The 0x67 address-size override reject was already in the original VPGATHERDD handler (PR #9). Rejecting (not mis-decoding) is the decoder-wide conservative posture; 32-bit VSIB addressing stays queued. |
| 3 | MAJOR | False positive | `vumov_w_from_lane` / `vins_lane_from_w` special-case `VecLane::D2` with X-form Umov/Mov (emitter.cpp). The lowering tests assert `ldr x`, `.d[` and `#63`, which only an X-form path can produce. |
| 4 | MINOR | Confirmed correct | Reviewer itself notes the QD/QPS ymm mask is correctly loaded as one xmm. |
| 5 | NIT | Deliberate | `lane_count = 4` default keeps the three pre-existing 5-field aggregate-init sites meaning the original DD xmm shape; new decode sites initialize explicitly. |
| 6 | NIT | Already pinned | VEX.X populates `rex.x` at the C4 parse; the xmm12 VSIB decode test fails if that plumbing breaks. |

## Verdicts

- Codex: PASS (no findings).
- Gemini: PASS after triage (no real findings; one pre-existing
  limitation documented).

## Notes

- The stale in-code comment claiming xmm12 was inexpressible as a
  VSIB index was wrong (xmm12 worked via the GPR index math);
  `a3c39e6` fixes the comment and pins the behavior with a test.
- `kSerializeVersion` 1 → 2 is safe today: RFC 0009 blobs have no
  cache/production consumer; the RFC 0007 persistent cache stores
  machine code, not IR.
- Lean mirror of `VecGather` remains queued (pre-existing debt,
  playbook §5).
