/-
  PrismaIR.TSO — a Total Store Order (TSO) weak-memory model.

  This is the F1-LN-014 skeleton the `MachineState` header anticipates: the
  state and steps needed to reason about Prisma's TSO-adaptive rewrite
  (Pillar 3), which preserves x86/TSO ordering only where a relaxed ARM64
  rewrite would be observable, and drops it everywhere else.

  TSO is the x86-64 memory model (Sewell et al., "x86-TSO"): sequential
  consistency relaxed by per-core FIFO store buffers.

    * a `store` appends to the issuing core's buffer — not yet globally
      visible;
    * a `load` returns the most recent buffered store to its address by the
      SAME core (store forwarding), else shared memory;
    * buffered stores drain (`propagate`) to shared memory in FIFO / program
      order, nondeterministically interleaved across cores;
    * the ONLY relaxation is Write -> Read: a later load can complete before
      an earlier store of the same core has drained. Write->Write,
      Read->Read and Read->Write stay in program order.

  This module defines the state + the four primitive steps and proves the
  characteristic TSO facts (store forwarding, private buffering, FIFO
  publication, and the SB litmus that exhibits the non-SC outcome). The TSO
  axioms-as-lemmas (F1-LN-015) and the adaptive-rewrite soundness statement
  (F1-LN-016) build on this skeleton.
-/

namespace PrismaIR

/-- Word-addressed locations for the TSO layer. (The byte-addressed
    `Memory` in `MachineState` refines this once the model is wired into the
    full step relation.) -/
abbrev Addr := Nat
/-- A stored value (one machine word). -/
abbrev Val  := UInt64
/-- A hardware thread / core identifier. -/
abbrev Tid  := Nat

/-- Pointwise update of a total function at one point (the codebase avoids a
    Mathlib dependency, so we roll our own instead of `Function.update`). -/
def upd {α : Type} [DecidableEq α] {β : Type} (f : α → β) (x : α) (y : β) : α → β :=
  fun x' => if x' = x then y else f x'

@[simp] theorem upd_same {α : Type} [DecidableEq α] {β : Type}
    (f : α → β) (x : α) (y : β) : upd f x y x = y := by simp [upd]

@[simp] theorem upd_noteq {α : Type} [DecidableEq α] {β : Type}
    (f : α → β) {x x' : α} (y : β) (h : x' ≠ x) : upd f x y x' = f x' := by
  simp [upd, h]

@[simp] theorem upd_idem {α : Type} [DecidableEq α] {β : Type}
    (f : α → β) (x : α) (a b : β) : upd (upd f x a) x b = upd f x b := by
  funext x'; by_cases h : x' = x <;> simp [upd, h]

/-- A core's FIFO store buffer: oldest entry at the head, newest appended at
    the tail. Each entry is a pending `(address, value)` store. -/
abbrev StoreBuffer := List (Addr × Val)

/-- The most recent pending store to `a` in a buffer (newest wins), or
    `none`. Folds oldest -> newest, keeping the last address match — so the
    tail (newest) store shadows older ones, exactly like store forwarding. -/
def sbLatest (buf : StoreBuffer) (a : Addr) : Option Val :=
  buf.foldl (fun acc e => if e.1 = a then some e.2 else acc) none

/-- A TSO machine: coherent shared memory plus a per-core store buffer. -/
structure TSO where
  mem : Addr → Val
  sb  : Tid → StoreBuffer

namespace TSO

/-- TSO load with store forwarding: core `t` sees its own most recent
    buffered store to `a`, otherwise shared memory. -/
def load (s : TSO) (t : Tid) (a : Addr) : Val :=
  (sbLatest (s.sb t) a).getD (s.mem a)

/-- TSO store: append `(a, v)` to core `t`'s buffer. Shared memory is
    untouched until the entry drains. -/
def store (s : TSO) (t : Tid) (a : Addr) (v : Val) : TSO :=
  { s with sb := upd s.sb t (s.sb t ++ [(a, v)]) }

/-- Drain core `t`'s oldest buffered store to shared memory. The
    nondeterministic choice of which core drains when is what admits the
    Write -> Read relaxation. No-op on an empty buffer. -/
def propagate (s : TSO) (t : Tid) : TSO :=
  match s.sb t with
  | []          => s
  | (a, v) :: r => { mem := upd s.mem a v, sb := upd s.sb t r }

/-- A barrier / locked op on core `t`: drain its whole buffer to memory in
    program order, atomically (mfence / `lock`-prefixed semantics). -/
def fence (s : TSO) (t : Tid) : TSO :=
  { mem := (s.sb t).foldl (fun m e => upd m e.1 e.2) s.mem
    sb  := upd s.sb t [] }

/-- One step of the TSO operational semantics. A core either issues a store
    (append to its own buffer) or a buffered store nondeterministically
    drains to shared memory. Loads are pure observations (`load`) that do not
    change state, so they are not steps; a `fence` is the finite sequence of
    `drain` steps that empties a buffer. The interleaving of `drain` across
    cores — which core drains when — is the SOLE source of relaxation, and it
    only ever reorders a later load ahead of an earlier (still-buffered) store
    of the same core (W->R). No constructor reorders two loads, two stores, or
    a load before a store of the same core, so R->R / W->W / R->W program
    order is structurally preserved. -/
inductive Step : TSO → TSO → Prop where
  | issue (s : TSO) (t : Tid) (a : Addr) (v : Val) : Step s (s.store t a v)
  | drain (s : TSO) (t : Tid)                       : Step s (s.propagate t)

/-- Reflexive-transitive closure: reachability under the operational
    semantics. -/
inductive Steps : TSO → TSO → Prop where
  | refl (s : TSO)                                : Steps s s
  | tail {s s' s'' : TSO} (h : Steps s s') (e : Step s' s'') : Steps s s''

-- A store to `a` appended at the tail is the newest, so it always forwards.
@[simp] theorem sbLatest_append_self (buf : StoreBuffer) (a : Addr) (v : Val) :
    sbLatest (buf ++ [(a, v)]) a = some v := by
  simp [sbLatest]

-- A store to a different address than the query never shadows it.
@[simp] theorem sbLatest_append_other (buf : StoreBuffer) (a b : Addr) (v : Val)
    (hab : a ≠ b) : sbLatest (buf ++ [(a, v)]) b = sbLatest buf b := by
  simp [sbLatest, List.foldl_append, hab]

/-- **Store forwarding.** A core reads back its own most recent store
    immediately, before it has drained to shared memory. -/
theorem load_store_self (s : TSO) (t : Tid) (a : Addr) (v : Val) :
    (s.store t a v).load t a = v := by
  simp [load, store]

/-- **Write buffering is private.** A store buffered by core `t` is invisible
    to any other core `t' ≠ t` until it drains — the root of the W->R
    relaxation and of the SB litmus below. -/
theorem load_store_other (s : TSO) (t t' : Tid) (a : Addr) (v : Val)
    (h : t' ≠ t) :
    (s.store t a v).load t' a = s.load t' a := by
  simp only [load, store, upd]
  rw [if_neg h]

/-- **Stores are address-local.** A store to `a` is invisible to a load of any
    other address `b ≠ a`, on any core — the buffered entry never shadows a
    different location. -/
theorem load_store_diff_addr (s : TSO) (t t' : Tid) (a b : Addr) (v : Val)
    (hab : a ≠ b) : (s.store t a v).load t' b = s.load t' b := by
  by_cases ht : t' = t
  · subst ht
    simp [load, store, sbLatest_append_other, hab]
  · simp only [load, store, upd]
    rw [if_neg ht]

/-- **Independent stores commute.** Two same-core stores to distinct addresses
    leave the core observing identical values regardless of their order — the
    formal licence for a rewrite to reorder writes to disjoint locations
    (TSO keeps W→W program order, but on disjoint addresses that order is
    unobservable). -/
theorem store_store_commute (s : TSO) (t : Tid) (a b : Addr) (v w : Val)
    (hab : a ≠ b) (c : Addr) :
    ((s.store t a v).store t b w).load t c
      = ((s.store t b w).store t a v).load t c := by
  simp only [load, store, upd_same, sbLatest, List.foldl_append, List.foldl_cons,
    List.foldl_nil]
  by_cases hca : c = a
  · subst hca; simp [Ne.symm hab]
  · by_cases hcb : c = b
    · subst hcb; simp [hab]
    · simp [Ne.symm hca, Ne.symm hcb]

/-- A store never modifies shared memory. -/
@[simp] theorem store_mem (s : TSO) (t : Tid) (a : Addr) (v : Val) :
    (s.store t a v).mem = s.mem := rfl

/-- **Draining publishes.** After draining a single buffered store, the value
    is in shared memory (and thus visible to every core whose own buffer has
    no shadowing entry). -/
theorem propagate_publishes (s : TSO) (t : Tid) (a : Addr) (v : Val)
    (h : s.sb t = [(a, v)]) :
    (s.propagate t).mem a = v := by
  unfold propagate
  rw [h]
  simp

/-- **Program order to memory (no W->W reordering).** Draining two same-core
    stores to the same address via a fence leaves the newer value: the buffer
    is FIFO, so stores reach memory in program order. -/
theorem fence_two_same_addr (s : TSO) (t : Tid) (a : Addr) (v₁ v₂ : Val)
    (h : s.sb t = [(a, v₁), (a, v₂)]) :
    (s.fence t).mem a = v₂ := by
  simp [fence, h]

/-- **The store-buffering litmus (SB).** TSO admits the non-sequentially-
    consistent outcome where both cores miss each other's just-issued store.
    From zeroed memory, core 0 stores `x := 1` (addr 0) and core 1 stores
    `y := 1` (addr 1); each then loads the other's location *before either
    store drains* — and both read the stale `0`. Under sequential
    consistency at least one load must see `1`; under TSO both buffered
    stores are still private, so `(0, 0)` is reachable. -/
theorem sb_litmus :
    ∃ s : TSO,
      let s' := (s.store 0 0 1).store 1 1 1
      s'.load 0 1 = 0 ∧ s'.load 1 0 = 0 := by
  refine ⟨{ mem := fun _ => 0, sb := fun _ => [] }, ?_⟩
  refine ⟨?_, ?_⟩ <;>
    simp [store, load, sbLatest, upd]

/-- **General one-step publication** (generalises `propagate_publishes` from
    the singleton buffer to any non-empty buffer). A `drain` publishes the
    OLDEST entry — the head — to memory and pops exactly it, leaving the tail
    intact, so drains happen in FIFO / program order. -/
theorem propagate_cons (s : TSO) (t : Tid) (a : Addr) (v : Val) (r : StoreBuffer)
    (h : s.sb t = (a, v) :: r) :
    (s.propagate t).mem a = v ∧ (s.propagate t).sb t = r := by
  refine ⟨?_, ?_⟩ <;> simp [propagate, h]

/-- A fence empties the issuing core's buffer, so its subsequent loads read
    only shared memory — the post-fence ordering F1-LN-015 builds on. -/
@[simp] theorem fence_sb (s : TSO) (t : Tid) : (s.fence t).sb t = [] := by
  simp [fence]

/-- With an empty buffer a core's load reads shared memory directly — the base
    case the forwarding/soundness lemmas reduce to. -/
theorem load_no_buffer (s : TSO) (t : Tid) (a : Addr) (h : s.sb t = []) :
    s.load t a = s.mem a := by
  simp [load, h, sbLatest]

/-- **A fence subsumes one drain step.** Propagating one buffered store and then
    fencing reaches the same shared memory as fencing directly — the fence is
    the closed form of the step-wise `propagate` drain. A building block for
    lifting the single-state soundness lemmas to operational `Steps` traces
    (RFC 0016, F1-LN-016 / M3). -/
theorem fence_eq_propagate_fence (s : TSO) (t : Tid) :
    (s.fence t).mem = ((s.propagate t).fence t).mem := by
  unfold propagate fence
  cases h : s.sb t with
  | nil => simp [h]
  | cons e r => simp [upd_same, List.foldl_cons]

/-- Drain core `t`'s buffer with `n` fuel units (one `propagate` each). -/
def drainN (s : TSO) (t : Tid) : Nat → TSO
  | 0 => s
  | n + 1 => match s.sb t with
             | [] => s
             | _ :: _ => drainN (s.propagate t) t n

/-- **A fence equals a full step-wise drain.** With fuel covering the buffer,
    repeatedly propagating reaches the same shared memory the atomic fence does
    — closing the gap between the abstract `fence` and the operational
    `propagate` steps the rewrite reasons about (RFC 0016 / M3). -/
theorem fence_eq_drainN (s : TSO) (t : Tid) (n : Nat) (hn : (s.sb t).length ≤ n) :
    (s.fence t).mem = (drainN s t n).mem := by
  induction n generalizing s with
  | zero =>
    have h : s.sb t = [] := by
      cases hb : s.sb t with
      | nil => rfl
      | cons _ _ => rw [hb] at hn; simp at hn
    simp [drainN, fence, h]
  | succ n ih =>
    cases hb : s.sb t with
    | nil => simp [drainN, hb, fence]
    | cons e r =>
      have hps : (s.propagate t).sb t = r := by simp [propagate, hb, upd_same]
      rw [hb] at hn
      have hlen : ((s.propagate t).sb t).length ≤ n := by
        rw [hps]; simp [List.length_cons] at hn; omega
      have hstep : drainN s t (n + 1) = drainN (s.propagate t) t n := by
        simp [drainN, hb]
      rw [fence_eq_propagate_fence, hstep]
      exact ih (s.propagate t) hlen

/-- Prepend a step to a reachability chain (the `head` companion to `tail`). -/
theorem Steps.head_step {s s' s'' : TSO} (e : Step s s') (h : Steps s' s'') :
    Steps s s'' := by
  induction h with
  | refl => exact Steps.tail (Steps.refl _) e
  | tail _ e' ih => exact Steps.tail ih e'

/-- **A drain sequence is operationally reachable.** `drainN` is repeated
    `drain` (`propagate`) steps, so the drained state is reachable from the
    start under the operational `Steps` semantics. With `fence_eq_drainN` this
    means a fence's memory effect is realised by a real Steps trace — the bridge
    from the abstract barrier to the operational model the rewrite uses
    (RFC 0016 / M3). -/
theorem drainN_reachable (s : TSO) (t : Tid) (n : Nat) : Steps s (drainN s t n) := by
  induction n generalizing s with
  | zero => simpa [drainN] using Steps.refl s
  | succ n ih =>
    cases hb : s.sb t with
    | nil => simpa [drainN, hb] using Steps.refl s
    | cons e r =>
      have hstep : drainN s t (n + 1) = drainN (s.propagate t) t n := by simp [drainN, hb]
      rw [hstep]
      exact Steps.head_step (Step.drain s t) (ih (s.propagate t))


/-- A fence is core-local: it drains only the issuing core's buffer, leaving
    every other core's buffer untouched. The structural complement to the
    cross-core visibility lemmas. -/
@[simp] theorem fence_other_sb (s : TSO) (t t' : Tid) (h : t' ≠ t) :
    (s.fence t).sb t' = s.sb t' := by
  simp [fence, h]

/-- A store is core-local for the buffer too: it appends only to the issuing
    core's buffer, so any other core's buffer is unchanged (the structural
    root of private buffering). -/
@[simp] theorem store_other_sb (s : TSO) (t t' : Tid) (a : Addr) (v : Val)
    (h : t' ≠ t) : (s.store t a v).sb t' = s.sb t' := by
  simp [store, h]

/-- A `propagate` step is core-local too: draining `t`'s oldest store touches
    only `t`'s buffer (and shared memory), never another core's buffer. With
    `store_other_sb` and `fence_other_sb` this gives full core isolation of the
    buffers — the foundation for reasoning about independent threads. -/
@[simp] theorem propagate_other_sb (s : TSO) (t t' : Tid) (h : t' ≠ t) :
    (s.propagate t).sb t' = s.sb t' := by
  unfold propagate
  cases s.sb t <;> simp [h]

/-- **A second fence is redundant.** Fencing an already-fenced core is a no-op
    (its buffer is already empty), so a rewrite may drop back-to-back barriers
    on the same core. -/
theorem fence_idempotent (s : TSO) (t : Tid) : (s.fence t).fence t = s.fence t := by
  simp [fence]

/-- **F1-LN-015 — a fence restores the Write→Read order.** After a fence the
    issuing core's buffer is empty (`fence_sb`), so a subsequent load reads
    shared memory directly: no earlier store of `t` can still be observed
    *after* this load. This is exactly the guarantee `mfence` / a `lock`
    prefix provides, and the reason the TSO-adaptive rewrite (Pillar 3) must
    keep a barrier wherever the W→R relaxation would otherwise be observable. -/
theorem load_after_fence (s : TSO) (t : Tid) (a : Addr) :
    (s.fence t).load t a = (s.fence t).mem a := by
  simp [load, sbLatest]

/-- **F1-LN-015 — same-core stores publish in program order (W→W).** From an
    empty buffer, two stores by `t` to distinct addresses both reach shared
    memory after a fence, each carrying its own value: the buffer drains in
    FIFO program order, so neither store is lost nor reordered past the other.
    W→W is never relaxed under TSO, and the fence makes that publication
    globally visible. -/
theorem fence_publishes_two (s : TSO) (t : Tid) (a b : Addr) (v w : Val)
    (hne : a ≠ b) (hempty : s.sb t = []) :
    (((s.store t a v).store t b w).fence t).mem a = v ∧
    (((s.store t a v).store t b w).fence t).mem b = w := by
  refine ⟨?_, ?_⟩
  · simp [store, fence, upd, hempty, hne]
  · simp [store, fence, upd, hempty]

/-- The `sbLatest` fold is independent of its starting accumulator up to
    `orElse`: a later (newer) matching store overrides whatever came before. -/
private theorem sbLatest_acc (buf : StoreBuffer) (a : Addr) (acc : Option Val) :
    (buf.foldl (fun acc e => if e.1 = a then some e.2 else acc) acc)
      = (sbLatest buf a).orElse (fun _ => acc) := by
  induction buf generalizing acc with
  | nil => simp [sbLatest]
  | cons e r ih =>
    have lhs : (e :: r).foldl (fun acc e => if e.1 = a then some e.2 else acc) acc
             = (sbLatest r a).orElse (fun _ => if e.1 = a then some e.2 else acc) := by
      rw [List.foldl_cons, ih]
    have rhs : sbLatest (e :: r) a
             = (sbLatest r a).orElse (fun _ => if e.1 = a then some e.2 else none) := by
      rw [sbLatest, List.foldl_cons]; exact ih (if e.1 = a then some e.2 else none)
    rw [lhs, rhs]
    rcases sbLatest r a with _ | v
    · by_cases he : e.1 = a <;> simp [he]
    · simp

/-- **The two views of the same writes agree.** Draining a store buffer into
    memory (`fence`'s fold) and reading `a` gives exactly what the buffered
    `load` would forward: the buffer's latest store to `a`, else memory. -/
private theorem foldl_upd_apply (buf : StoreBuffer) (m : Addr → Val) (a : Addr) :
    (buf.foldl (fun m e => upd m e.1 e.2) m) a = (sbLatest buf a).getD (m a) := by
  induction buf generalizing m with
  | nil => simp [sbLatest]
  | cons e r ih =>
    have hrec : sbLatest (e :: r) a
              = (sbLatest r a).orElse (fun _ => if e.1 = a then some e.2 else none) := by
      rw [sbLatest, List.foldl_cons]; exact sbLatest_acc r a _
    rw [List.foldl_cons, ih, hrec]
    rcases sbLatest r a with _ | v
    · by_cases he : e.1 = a
      · simp [upd, he]
      · simp [upd, he, Ne.symm he]
    · simp

/-- **F1-LN-016 (single-thread soundness core) — a core's own load is
    unaffected by its own fence.** Store forwarding already lets `t` observe
    its latest store, so draining the buffer (a fence) changes nothing `t`
    itself can see: `(s.fence t).load t a = s.load t a`. This is the formal
    justification for dropping a barrier in a region where only the issuing
    core's observations matter (the single-threaded case the TSO-adaptive
    rewrite, Pillar 3, classifies as safe to relax). -/
theorem load_unaffected_by_fence (s : TSO) (t : Tid) (a : Addr) :
    (s.fence t).load t a = s.load t a := by
  rw [load_after_fence]
  simp only [fence, load]
  exact foldl_upd_apply (s.sb t) s.mem a

/-- **Draining is unobservable to the issuing core.** A `propagate` step (drain
    the oldest buffered store of `t` to memory) does not change what `t` itself
    loads: store forwarding already gives `t` its latest store, whether that
    store is still buffered or has just drained. This is why the
    nondeterministic *timing* of a core's drains is sound for that core's own
    observations — the basis of relaxed single-threaded execution. -/
theorem load_unaffected_by_propagate (s : TSO) (t : Tid) (a : Addr) :
    (s.propagate t).load t a = s.load t a := by
  unfold propagate load
  cases h : s.sb t with
  | nil => simp [h]
  | cons e r =>
    have hrec : sbLatest (e :: r) a
              = (sbLatest r a).orElse (fun _ => if e.1 = a then some e.2 else none) := by
      rw [sbLatest, List.foldl_cons]; exact sbLatest_acc r a _
    simp only [upd_same, hrec]
    rcases sbLatest r a with _ | v
    · by_cases he : e.1 = a
      · simp [upd, he]
      · simp [upd, he, Ne.symm he]
    · simp

/-- **Cross-core publication.** Once core `t` fences (drains its buffer), its
    store is in shared memory, so any other core `t'` whose own buffer is empty
    observes it. The complement of private buffering: a *buffered* store is
    invisible across cores, a *fenced* (drained) one is visible — together they
    characterise inter-core visibility under TSO. -/
theorem store_visible_to_idle_core (s : TSO) (t t' : Tid) (a : Addr) (v : Val)
    (htt : t ≠ t') (hs : s.sb t = []) (hs' : s.sb t' = []) :
    ((s.store t a v).fence t).load t' a = v := by
  simp [store, fence, load, upd, sbLatest, hs, hs', Ne.symm htt,
    List.foldl_cons, List.foldl_nil]

/-- **SB via the operational semantics.** From a zeroed machine, two `issue`
    steps — core 0 stores `x := 1` (addr 0), core 1 stores `y := 1` (addr 1) —
    reach, through `Steps`, a state in which each core's load of the OTHER
    location still reads the stale `0`, because both stores are still
    buffered. This is the canonical outcome separating TSO from sequential
    consistency, here a genuine reachability statement (not just a hand-built
    state). -/
theorem sb_litmus_reachable :
    ∃ f : TSO,
      Steps { mem := fun _ => 0, sb := fun _ => [] } f ∧
      f.load 0 1 = 0 ∧ f.load 1 0 = 0 := by
  refine ⟨((({ mem := fun _ => 0, sb := fun _ => [] } : TSO).store 0 0 1).store 1 1 1),
          ?_, ?_, ?_⟩
  · exact Steps.tail (Steps.tail (Steps.refl _) (Step.issue _ 0 0 1)) (Step.issue _ 1 1 1)
  · simp [store, load, sbLatest, upd]
  · simp [store, load, sbLatest, upd]

/-- **A full drain is unobservable to the issuing core.** Draining any number of
    `t`'s buffered stores leaves `t`'s own loads unchanged (each step is, by
    `load_unaffected_by_propagate`). The operational form of single-threaded
    soundness: the *timing and amount* a core drains never changes what it
    itself sees (RFC 0016 / M3, F1-LN-016). -/
theorem load_unaffected_by_drainN (s : TSO) (t : Tid) (a : Addr) (n : Nat) :
    (drainN s t n).load t a = s.load t a := by
  induction n generalizing s with
  | zero => simp [drainN]
  | succ n ih =>
    cases hb : s.sb t with
    | nil => simp [drainN, hb]
    | cons e r =>
      have hstep : drainN s t (n + 1) = drainN (s.propagate t) t n := by simp [drainN, hb]
      rw [hstep, ih (s.propagate t), load_unaffected_by_propagate]


/-! ## F1-LN-016 — TSO-adaptive rewrite soundness (state level)

The translator's adaptive pass replaces a TSO barrier (a DMB / drained `fence`)
with nothing whenever it can statically prove the executing core is *quiescent*
— it has no pending buffered stores at that program point (e.g. right after a
prior barrier, or in a region the pass proved store-free). The lemmas below are
the soundness obligation of that rewrite at the machine-state level: the elided
barrier is the identity transition, so no core at any address can observe its
removal. RFC 0016 (M1) tracks the connection to the IR rewrite itself. -/

/-- A core is *quiescent* when its store buffer is empty: the adaptive pass's
    barrier-elision precondition. -/
def Quiescent (s : TSO) (t : Tid) : Prop := s.sb t = []

/-- **Barrier elision soundness.** A `fence` on a quiescent core is the identity
    transition — its only effect (draining the buffer, in program order, to
    shared memory) is already accomplished. The pass may drop it. -/
theorem fence_elim_quiescent (s : TSO) (t : Tid) (h : Quiescent s t) :
    s.fence t = s := by
  have h' : s.sb t = [] := h
  have hsb : upd s.sb t [] = s.sb := by
    funext x'
    by_cases hx : x' = t
    · subst hx; simp [upd, h']
    · simp [upd, hx]
  simp only [fence, h', List.foldl_nil, hsb]

/-- **Observational soundness of the elision.** No core, at any address, can
    distinguish the elided-barrier machine from the kept-barrier one — the two
    states are literally equal, so every `load` agrees. -/
theorem fence_elim_obs (s : TSO) (t : Tid) (h : Quiescent s t) :
    ∀ (t'' : Tid) (a : Addr), (s.fence t).load t'' a = s.load t'' a := by
  intro t'' a; rw [fence_elim_quiescent s t h]

/-- Quiescence of core `t` is stable under a *different* core issuing a store:
    `t` keeps an empty buffer, so a barrier the pass proved elidable on `t`
    stays elidable across other cores' writes. -/
theorem quiescent_stable_issue_other (s : TSO) (t t' : Tid) (a : Addr) (v : Val)
    (htt : t ≠ t') (h : Quiescent s t) : Quiescent (s.store t' a v) t := by
  have h' : s.sb t = [] := h
  unfold Quiescent store
  simpa [upd, htt] using h'

/-- Quiescence of core `t` is stable under a *different* core draining its
    buffer to shared memory. Together with `quiescent_stable_issue_other`, a
    core the pass proved store-free stays quiescent across every step of the
    other cores — so all its barriers in that span elide. -/
theorem quiescent_stable_drain_other (s : TSO) (t t' : Tid)
    (htt : t ≠ t') (h : Quiescent s t) : Quiescent (s.propagate t') t := by
  have h' : s.sb t = [] := h
  unfold Quiescent propagate
  cases hb : s.sb t' with
  | nil => simpa [hb] using h'
  | cons hd tl => simpa [hb, upd, htt] using h'

/-- The adaptive rewrite is sound against any reachable state with the same
    quiescence witness: if a later state `s'` still has core `t` quiescent, its
    barrier on `t` elides too. The bridge from "proved quiescent once" to
    "elidable here". -/
theorem adaptive_fence_elim (s' : TSO) (t : Tid) (h : Quiescent s' t) :
    s'.fence t = s' :=
  fence_elim_quiescent s' t h

/-! ### F1-LN-016 / M2 — barrier elision preserves the whole-block trace

M1 above shows a single elided `fence` is the identity *transition*. M2 lifts
that to a *block*: a translated basic block is a sequence of the executing
core's memory ops, and its observable behaviour is the list of values its loads
return (its trace) together with its final state. The adaptive pass removes a
barrier whose program point the analysis proved quiescent; the theorem here is
that this removal changes neither the trace nor the final state of the entire
block — the rewrite is observationally sound at the block level, which is the
granularity the pass actually operates on. -/

/-- One memory op of a single core's translated block. (`MOp`, not `Op`, to
    avoid colliding with the IR opcode type.) -/
inductive MOp
  | load  (a : Addr)
  | store (a : Addr) (v : Val)
  | bar
  deriving DecidableEq

abbrev MBlock := List MOp

/-- Execute a block on core `t`: thread the TSO state through each op and
    collect the value every `load` observes. Returns the final state paired
    with the trace (loads in program order). A `bar` drains the buffer; a
    `store` buffers; a `load` is a pure observation appended to the trace. -/
def run (t : Tid) : MBlock → TSO → TSO × List Val
  | [],                 s => (s, [])
  | MOp.load a    :: r, s => let p := run t r s; (p.1, s.load t a :: p.2)
  | MOp.store a v :: r, s => run t r (s.store t a v)
  | MOp.bar       :: r, s => run t r (s.fence t)

/-- Running a concatenated block is running the first half, then the second
    from the resulting state, with the traces concatenated. The compositional
    backbone that lets a local rewrite be reasoned about inside a larger block. -/
theorem run_append (t : Tid) (xs ys : MBlock) (s : TSO) :
    run t (xs ++ ys) s =
      ((run t ys (run t xs s).1).1,
       (run t xs s).2 ++ (run t ys (run t xs s).1).2) := by
  induction xs generalizing s with
  | nil => simp [run]
  | cons op r ih =>
    cases op with
    | load a => simp [run, ih]
    | store a v => simp [run, ih]
    | bar => simp [run, ih]

/-- **M2 — block-level barrier-elision soundness.** If the prefix `pre` leaves
    core `t`'s buffer empty, then deleting the `bar` that follows it changes
    neither the final state nor the trace of the whole block `pre ++ bar ::
    post`. The adaptive pass may drop a barrier at any quiescent program point
    without altering observable block behaviour. -/
theorem bar_elim_trace (t : Tid) (pre post : MBlock) (s : TSO)
    (h : Quiescent (run t pre s).1 t) :
    run t (pre ++ MOp.bar :: post) s = run t (pre ++ post) s := by
  rw [run_append, run_append]
  have hf : (run t pre s).1.fence t = (run t pre s).1 :=
    fence_elim_quiescent _ t h
  simp only [run, hf]

/-- A block of pure loads (no store, no barrier) never changes the machine: its
    final state is the start state. Such a block is trivially quiescent-stable,
    so every barrier the pass might consider inserting into it is elidable — the
    read-only-region case of the adaptive analysis. -/
theorem run_loads_state (t : Tid) (addrs : List Addr) (s : TSO) :
    (run t (addrs.map MOp.load) s).1 = s := by
  induction addrs generalizing s with
  | nil => simp [run]
  | cons a r ih => simp [run, ih]

/-! ### F1-LN-016 / M3 — elision is sound under multi-core interleaving

M1/M2 reason about the rewriting core's own view. M3 connects the elision to the
operational `Steps` relation — the arbitrary interleaving of every core's
issue/drain — and shows a *spectator* core can never tell the barrier was
removed. Because an elided barrier at a quiescent point yields a *literally
equal* machine (M1, `fence_elim_quiescent`), the two programs have identical
reachable-state sets; hence every value a spectator could observe after the
kept barrier it can also observe after the elided one, and conversely. This is
the cross-core soundness obligation of the adaptive rewrite. -/

/-- The elided and kept-barrier machines reach exactly the same states under
    any interleaving: their reachable-state sets coincide. -/
theorem bar_elim_reachable_iff (s : TSO) (t : Tid) (h : Quiescent s t) (s' : TSO) :
    Steps (s.fence t) s' ↔ Steps s s' := by
  rw [fence_elim_quiescent s t h]

/-- **M3 — cross-core observational equivalence.** Every value a spectator core
    `t''` can observe at address `a`, in some state reachable under the full
    issue/drain interleaving, is identical whether the quiescent-point barrier
    is kept or elided. No interleaving distinguishes the rewrite. -/
theorem bar_elim_spectator_obs (s : TSO) (t t'' : Tid) (a : Addr) (v : Val)
    (h : Quiescent s t) :
    (∃ s', Steps (s.fence t) s' ∧ s'.load t'' a = v) ↔
    (∃ s', Steps s s' ∧ s'.load t'' a = v) := by
  rw [fence_elim_quiescent s t h]

/-- A drain by any core is itself one legal `Step`, so a single elided barrier
    never removes a behaviour: anything the kept-barrier machine could reach by
    draining, the elided one reaches too (same state). The reflexive bridge used
    when composing the elision into a larger interleaved schedule. -/
theorem bar_elim_drain_step (s : TSO) (t t' : Tid) (h : Quiescent s t) :
    Step (s.fence t) ((s.fence t).propagate t') ∧
    (s.fence t).propagate t' = s.propagate t' := by
  refine ⟨Step.drain _ _, ?_⟩
  rw [fence_elim_quiescent s t h]
end TSO

end PrismaIR
