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

end TSO

end PrismaIR
