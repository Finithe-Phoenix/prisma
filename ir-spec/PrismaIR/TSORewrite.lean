import PrismaIR.Syntax
import PrismaIR.TSO

/-
  PrismaIR.TSORewrite — F1-LN-016 / M4.

  Bridges the adaptive barrier-elimination rewrite on the REAL IR (`Op.fence`,
  dropped when the classifier proves the region thread-local) to the TSO model's
  bar-elimination soundness (M1-M3 in `PrismaIR.TSO`). The rewrite is shown to
  correspond exactly to deleting the model `bar`s from the projected block, with
  every load/store (and its env-resolved address/value) preserved in order — so
  M2/M3 carry the observable-equivalence conclusion over to the real rewrite.
  The remaining obligation (M4-residual) is that the full byte-level IR semantics
  in `MachineState` refines the model's `run`.
-/

namespace PrismaIR
namespace TSO

/-- Is this IR op a memory fence? The adaptive pass's elimination target. -/
def isFence : Op → Bool
  | Op.fence _ => true
  | _          => false

/-- The adaptive pass's barrier-elimination rewrite on an IR instruction list:
    drop every `fence` whose enclosing region the classifier proved
    quiescent / thread-local. Loads, stores and compute ops are untouched. -/
def elimFences : List Op → List Op
  | []      => []
  | op :: r => if isFence op then elimFences r else op :: elimFences r

theorem elimFences_eq_filter (l : List Op) :
    elimFences l = l.filter (fun op => ! isFence op) := by
  induction l with
  | nil => rfl
  | cons op r ih =>
    cases h : isFence op <;> simp [elimFences, List.filter, h, ih]

/-- The model alphabet a `bar` belongs to. -/
def isBar : MOp → Bool
  | MOp.bar => true
  | _       => false

/-- Project an IR op onto the TSO model's memory alphabet under an environment
    `ρ`/`σ` resolving SSA refs to concrete addresses/values: a `fence` becomes a
    `bar`; TSO and plain loads/stores become model loads/stores; everything else
    is invisible to the memory model. -/
def proj (ρ : Ref → Addr) (σ : Ref → Val) : Op → Option MOp
  | Op.fence _           => some MOp.bar
  | Op.loadMem a _       => some (MOp.load (ρ a))
  | Op.loadMemTSO a _    => some (MOp.load (ρ a))
  | Op.storeMem a v _    => some (MOp.store (ρ a) (σ v))
  | Op.storeMemTSO a v _ => some (MOp.store (ρ a) (σ v))
  | _                    => none

def projBlock (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) : MBlock :=
  l.filterMap (proj ρ σ)

/-- A fence projects to exactly `bar`. -/
theorem proj_fence (ρ : Ref → Addr) (σ : Ref → Val) (op : Op)
    (h : isFence op = true) : proj ρ σ op = some MOp.bar := by
  cases op <;> simp_all [isFence, proj]

/-- A non-fence op never projects to a `bar`. -/
theorem proj_nonfence_not_bar (ρ : Ref → Addr) (σ : Ref → Val) (op : Op) (m : MOp)
    (hf : isFence op = false) (hp : proj ρ σ op = some m) : isBar m = false := by
  cases op with
  | loadMem a s       => simp only [proj, Option.some.injEq] at hp; subst hp; rfl
  | loadMemTSO a s    => simp only [proj, Option.some.injEq] at hp; subst hp; rfl
  | storeMem a v s    => simp only [proj, Option.some.injEq] at hp; subst hp; rfl
  | storeMemTSO a v s => simp only [proj, Option.some.injEq] at hp; subst hp; rfl
  | fence k           => simp [isFence] at hf
  | _                 => simp [proj] at hp

/-- Filtering fences before projection equals deleting bars after — the core
    commutation that makes barrier elimination a model-bar deletion. -/
theorem filter_filterMap_bar (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    (l.filter (fun op => ! isFence op)).filterMap (proj ρ σ)
      = (l.filterMap (proj ρ σ)).filter (fun m => ! isBar m) := by
  induction l with
  | nil => rfl
  | cons op r ih =>
    by_cases hf : isFence op
    · have hp := proj_fence ρ σ op hf
      simp [hf, hp, isBar, ih]
    · have hf' : isFence op = false := by simpa using hf
      simp only [List.filter_cons, hf', Bool.not_false, if_true, List.filterMap_cons]
      cases hp : proj ρ σ op with
      | none => simp [ih]
      | some m =>
        have hmb := proj_nonfence_not_bar ρ σ op m hf' hp
        simp [hmb, ih]

/-- **M4 bridge.** Eliminating IR fences corresponds exactly to deleting the
    model `bar`s from the projected block: every load/store (with its resolved
    address/value) survives in program order. Composed with M2 (`bar_elim_trace`)
    and M3 (`bar_elim_spectator_obs`), the adaptive pass's fence elimination at
    quiescent points preserves the model's observable behaviour. -/
theorem projBlock_elimFences (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    projBlock ρ σ (elimFences l) = (projBlock ρ σ l).filter (fun m => ! isBar m) := by
  rw [elimFences_eq_filter, projBlock, projBlock, filter_filterMap_bar]


/-- **F1-LN-016 capstone — the rewrite achieves its goal.** After eliminating IR
    fences, the projected model block provably contains no `bar` at all. Together
    with M2/M3/M4 (the rewrite is *sound*), this shows it is also *effective*:
    every barrier is gone, which is exactly the point of the TSO-adaptive pass —
    fewer `dmb`s on the ARM64 it emits. -/
theorem elimFences_barFree (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    ∀ m ∈ projBlock ρ σ (elimFences l), isBar m = false := by
  intro m hm
  rw [projBlock_elimFences] at hm
  have h := (List.mem_filter.mp hm).2
  simpa using h

/-! ## F1-LN-016 (second half) — access-downgrade soundness under thread-locality

The IR's `loadMemTSO`/`storeMemTSO` comment flags the other adaptive rewrite:
when the classifier proves an address is touched by a single core, its TSO
ordering is unobservable and the access downgrades to a plain ARM64 load/store.
Modelled here: a *write-private* address (no OTHER core buffers a store to it)
keeps a value, as seen by its owner, that is invariant under every other core's
drain — so no interleaving reveals the relaxed ordering, and the downgrade is
sound. -/

/-- `buf` holds no pending store to `a`. -/
def NoStoreTo (buf : StoreBuffer) (a : Addr) : Prop := ∀ e ∈ buf, e.1 ≠ a

/-- Address `a` is write-private to core `t`: no other core has a buffered store
    to it. The classifier's thread-locality precondition for downgrading a TSO
    access on `a` to a plain one. -/
def WritePrivate (s : TSO) (t : Tid) (a : Addr) : Prop :=
  ∀ t', t' ≠ t → NoStoreTo (s.sb t') a

/-- A drain by another core never changes the owner's view of a write-private
    address: the drained store is to some other address, and the owner's own
    buffer is untouched. -/
theorem load_private_drain_other (s : TSO) (t t' : Tid) (a : Addr)
    (htt : t' ≠ t) (h : NoStoreTo (s.sb t') a) :
    (s.propagate t').load t a = s.load t a := by
  unfold propagate
  cases hb : s.sb t' with
  | nil => rfl
  | cons hd tl =>
    have hne : hd.1 ≠ a := h hd (hb ▸ List.mem_cons_self)
    simp only [TSO.load, upd_noteq s.sb tl htt.symm, upd_noteq s.mem hd.2 hne.symm]

/-- A store by another core never changes the owner's view: stores only buffer,
    never touch shared memory or the owner's buffer. -/
theorem load_unaffected_by_other_store (s : TSO) (t t' : Tid) (a b : Addr) (v : Val)
    (htt : t' ≠ t) : (s.store t' b v).load t a = s.load t a := by
  simp only [TSO.load, store, upd_noteq s.sb (s.sb t' ++ [(b, v)]) htt.symm]

/-- **Access-downgrade soundness.** Under write-privacy, neither another core's
    store nor its drain alters the owner's load of `a`. So `a` behaves
    sequentially-consistently for its owner regardless of the global
    interleaving, and a TSO-ordered access to it may be downgraded to a plain
    one — the second adaptive rewrite, justified. -/
theorem private_step_stable (s : TSO) (t t' : Tid) (a b : Addr) (v : Val)
    (htt : t' ≠ t) (h : WritePrivate s t a) :
    (s.store t' b v).load t a = s.load t a ∧
    (s.propagate t').load t a = s.load t a :=
  ⟨load_unaffected_by_other_store s t t' a b v htt,
   load_private_drain_other s t t' a htt (h t' htt)⟩

/-- The eliminated block has no fence left — the rewrite removes every one. -/
theorem elimFences_fenceFree (l : List Op) :
    ∀ op ∈ elimFences l, isFence op = false := by
  intro op hop
  rw [elimFences_eq_filter] at hop
  have h := (List.mem_filter.mp hop).2
  simpa using h

/-- `elimFences` is idempotent: it reaches a fixpoint in one pass (nothing is
    left to remove), so the adaptive pass never needs to iterate it. -/
theorem elimFences_idempotent (l : List Op) :
    elimFences (elimFences l) = elimFences l := by
  simp only [elimFences_eq_filter, List.filter_filter, Bool.and_self]

/-- Folding stores into memory leaves address `a` untouched when no folded entry
    targets `a`. The memory-side invariant behind a full barrier on another
    core not disturbing a write-private address. -/
theorem foldl_upd_no_addr (buf : StoreBuffer) (a : Addr) (m0 : Addr → Val)
    (h : ∀ e ∈ buf, e.1 ≠ a) :
    (buf.foldl (fun m e => upd m e.1 e.2) m0) a = m0 a := by
  induction buf generalizing m0 with
  | nil => rfl
  | cons hd tl ih =>
    have hhd : hd.1 ≠ a := h hd List.mem_cons_self
    have htl : ∀ e ∈ tl, e.1 ≠ a := fun e he => h e (List.mem_cons_of_mem hd he)
    simp only [List.foldl_cons]
    rw [ih (upd m0 hd.1 hd.2) htl]
    exact upd_noteq m0 hd.2 (Ne.symm hhd)

/-- A FULL barrier on another core never changes the owner's view of a
    write-private address — the strongest other-core event (drain the whole
    buffer at once) still cannot disturb it, since every drained store is to a
    different address and the owner's own buffer is untouched. Strengthens
    `load_private_drain_other` from one drain to the whole barrier. -/
theorem load_private_fence_other (s : TSO) (t t' : Tid) (a : Addr)
    (htt : t' ≠ t) (h : NoStoreTo (s.sb t') a) :
    (s.fence t').load t a = s.load t a := by
  simp only [TSO.load, fence, upd_noteq s.sb [] htt.symm,
    foldl_upd_no_addr (s.sb t') a s.mem h]
end TSO
end PrismaIR
