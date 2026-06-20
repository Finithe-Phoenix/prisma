# Review — TSO weak-memory model skeleton (F1-LN-014)

Date: 2026-06-12
Author: claude
Reviewer: codex (gpt) — external CLI review ([[codex-gemini-reviewers]]).
(gemini CLI failed to run this round — tool/flag error; not re-run.)

## Change

New `ir-spec/PrismaIR/TSO.lean` — a Total Store Order weak-memory model
(x86-TSO, Sewell et al.), the F1-LN-014 skeleton the `MachineState` header
anticipated. Mathlib-free, sorry-free, `lake build` green (Lean v4.30.0-rc2).

Contents:

- State `TSO` = coherent shared `mem : Addr → Val` + per-core FIFO store
  buffers `sb : Tid → StoreBuffer`.
- Primitive steps: `load` (store-forwarding via `sbLatest`), `store` (buffer
  append), `propagate` (drain oldest = head), `fence` (drain whole buffer in
  order). Own `upd` helper (no Mathlib `Function.update`).
- Operational semantics: `Step` (issue | drain) + reflexive-transitive
  closure `Steps`. Relaxation is confined to drain interleaving (only W→R).
- Theorems: `load_store_self` (store forwarding), `load_store_other` (private
  buffering), `store_mem`, `propagate_publishes` + general `propagate_cons`
  (FIFO head publication), `fence_sb`, `fence_two_same_addr` (newest of two
  distinct values reaches memory — program order), `sb_litmus` and
  `sb_litmus_reachable` (the non-SC store-buffering outcome, as genuine
  reachability through `Steps`).

## Review history

**First pass — codex: CHANGES-NEEDED.** Confirmed the model is "conceptually
right for TSO" (store-forward, FIFO drain, W→R as the only relaxation).
Triage of the 7 issues:

- *#1 (sb_litmus "malformed"), #2 (fence binds "same value v? v?"),
  #6 (placeholders `?`)* — **false positives from Unicode transcoding** over
  the CLI pipe: `∃`, `≠`, and the subscripts `v₁`/`v₂` rendered as `?` in the
  prompt. The actual file uses two distinct values and well-formed
  quantifiers, and **builds with zero errors / zero sorry**, so these are not
  real defects.
- *#3, #5 (lemmas too narrow), #4 (no step relation / implicit reordering)* —
  **legitimate**; addressed below.

**Response (committed):**

- Added the `Step` / `Steps` operational layer so relaxation is structurally
  confined to drain interleaving — R→R / W→W / R→W program order is preserved
  by construction (no constructor reorders them). [#4]
- Generalised publication: `propagate_cons` proves head-drain from an
  arbitrary non-empty buffer. [#5]
- Reframed SB as `sb_litmus_reachable` — a reachability statement through
  `Steps`, not a hand-built state. [#1]
- `fence_two_same_addr` already used two distinct values `v₁ ≠`-shaped; left
  as is (the #2 concern was the encoding artifact).

**Second pass — codex: _(verdict pending; recorded on completion)_.**

## Scope

This is the F1-LN-014 *skeleton*. F1-LN-015 (TSO axioms as lemmas over this
model — e.g. fence-orders-later-ops, lock atomicity, general FIFO publish for
arbitrary buffers) and F1-LN-016 (TSO-adaptive rewrite soundness, statement
only) build on it and remain future work, as the backlog scopes them.
