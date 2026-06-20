# Review — decoder gap close-out (BSF/BSR, CMPXCHG, PUSHFQ/POPFQ, BT)

Date: 2026-06-19. Reviewer: **codex** (gpt-5.5, read-only sandbox). Gemini was
unavailable (`IneligibleTierError` — "Gemini Code Assist for individuals" tier
retired; needs migration to Antigravity), so this round is codex-only.

Coordinator: Claude. Each diff was implemented by a worktree-isolated agent,
then codex-reviewed before merge; findings triaged against the C++ reference and
the byte-level `smoke_differential` (which compares emitted ARM64 bytes, not IR
ref numbers).

## PR #49 — CMPXCHG r/m,r (0F B1)

codex: REQUEST-CHANGES → all real findings fixed before merge.

- **Dropped 0F B0** (CMPXCHG r/m8,r8): the C++ dispatch only routes 0F B1, so
  0F B0 stays Unsupported (keeps the differential). This also removed an
  incorrect I8 failure-writeback that wrote full RAX instead of AL.
- **Added prefix guards** for 0F B1: reject F2/F3 without LOCK, reject REX.W+0x66.
- Documented the one-CmpFlags/two-Selects pattern (matches C++; correct because
  the first Select lowers to `b.cond`+`mov`, which don't clobber NZCV).
- Added e2e: 32-bit failure zero-extends RAX; RAX-aliased dst proves the
  accumulator-then-DEST store order.

## PR #50 — PUSHFQ/POPFQ placeholders + BT/BTS/BTR/BTC (0F BA /4../7)

codex: REQUEST-CHANGES → one real finding fixed; the rest triaged as
non-blocking and disproven.

- **REAL (fixed):** PUSHFQ/POPFQ accepted prefixes the C++ rejects. Added guards
  to reject the 0x66 override, F3, and any REX, with a rejection unit test.
- **False-positive (re: byte-differential):** BT IR ref *numbering* differs from
  C++. `smoke_differential.rs` compares EMITTED ARM64 BYTES, not IR ref indices;
  every merged decoder family uses the same allocate-before-push convention and
  passes `ffi-link` (#50's `ffi-link` job passed too). Not changed.
- **Acknowledged (codex agreed):** isolated plain BT lowers to empty code — fine
  under the current implicit-flag model; the flag effect is kept when a consumer
  (jc/select) follows.
- **Follow-up:** adding `smoke_differential` fixtures for these families needs
  the live C++ DLL + C++-exact ref matching; deferred.

## Blocked families (NOT implemented — for codex / C++ coordination)

These cannot be done Rust-only without breaking the byte-differential, because
the C++ reference itself uses placeholders or lacks the flag infrastructure:

- **ADC/SBB real carry** — C++ decodes them as ADD/SUB placeholders; the Rust
  decoder already matches. Real carry needs `ReadFlag(Carry)` plumbing AND a
  coordinated C++ change (codex territory + a core rebuild). Doing it in Rust
  alone would diverge from C++.
- **RCL/RCR** — the Rust decoder Group2 and the Rust backend do not handle
  `Rcl`/`Rcr` (only Rol/Ror/Shl/Shr/Sar); rotate-through-carry needs CF and a
  coordinated decoder+backend+C++ effort.

These are the honest stopping point for the Rust-only decoder gap sweep.
