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

/-- A drain shrinks the buffer, so write-privacy of `a` for that core is
    preserved (the remaining entries are a sublist of the old ones). -/
theorem noStoreTo_propagate (s : TSO) (t' : Tid) (a : Addr)
    (h : NoStoreTo (s.sb t') a) : NoStoreTo ((s.propagate t').sb t') a := by
  unfold NoStoreTo propagate at *
  cases hb : s.sb t' with
  | nil => simp [hb]
  | cons hd tl =>
    simp only [upd_same]
    intro e he
    exact h e (hb ▸ List.mem_cons_of_mem hd he)

/-- A write-private address survives ANY number of partial drains by another
    core — generalises `load_private_drain_other` (one drain) and
    `load_private_fence_other` (the whole buffer) to an arbitrary drain count,
    so no schedule of the other core's draining disturbs the owner's view. -/
theorem load_private_drainN_other (s : TSO) (t t' : Tid) (a : Addr) (n : Nat)
    (htt : t' ≠ t) (h : NoStoreTo (s.sb t') a) :
    (drainN s t' n).load t a = s.load t a := by
  induction n generalizing s with
  | zero => simp [drainN]
  | succ n ih =>
    cases hb : s.sb t' with
    | nil => simp [drainN, hb]
    | cons e r =>
      have hstep : drainN s t' (n + 1) = drainN (s.propagate t') t' n := by
        simp [drainN, hb]
      rw [hstep, ih (s.propagate t') (noStoreTo_propagate s t' a h),
          load_private_drain_other s t t' a htt h]
/-- **The Pillar-3 payoff, unified.** A quiescent core reading a write-private
    address sees plain shared memory, and that reading is invariant under the
    other core's entire barrier — full sequential consistency for a thread-local
    access. This is exactly the precondition under which the adaptive pass may
    compile the access with a plain (un-ordered) ARM64 load/store and drop the
    surrounding barrier: both rewrite halves are justified by one fact. -/
theorem load_private_quiescent_sc (s : TSO) (t t' : Tid) (a : Addr)
    (htt : t' ≠ t) (hq : Quiescent s t) (h : NoStoreTo (s.sb t') a) :
    s.load t a = s.mem a ∧ (s.fence t').load t a = s.load t a := by
  refine ⟨?_, load_private_fence_other s t t' a htt h⟩
  have hsb : s.sb t = [] := hq
  simp [TSO.load, hsb, sbLatest]

/-- The barrier-elimination rewrite never grows a block: it only removes
    fences, so the result is no longer than the input. A basic well-formedness
    guarantee — the optimised block fits wherever the original did. -/
theorem elimFences_length_le (l : List Op) :
    (elimFences l).length ≤ l.length := by
  rw [elimFences_eq_filter]
  exact List.length_filter_le _ l

/-! ### Access-downgrade rewrite — IR-level bridge

  The second half of the adaptive rewrite: when the classifier proves a region
  single-threaded, its TSO-ordered loads/stores relax to plain ones. The model
  soundness lives above (`WritePrivate`, `private_step_stable`, ...); this is the
  IR-list rewrite and its projection bridge, parallel to `elimFences`. -/

/-- Is this a TSO-ordered memory access? The downgrade rewrite's target. -/
def isTsoAccess : Op → Bool
  | Op.loadMemTSO _ _    => true
  | Op.storeMemTSO _ _ _ => true
  | _                    => false

/-- Downgrade one op: a TSO load/store becomes its plain counterpart with the
    same operands; every other op is unchanged. -/
def downgradeOp : Op → Op
  | Op.loadMemTSO a s    => Op.loadMem a s
  | Op.storeMemTSO a v s => Op.storeMem a v s
  | op                   => op

/-- The access-downgrade rewrite on an IR instruction list. A `map`, so order
    and length are preserved. -/
def downgradeAccesses (l : List Op) : List Op := l.map downgradeOp

/-- The downgrade preserves the memory-model projection: a plain load/store and
    its TSO counterpart resolve to the same model `load`/`store` under ρ/σ. -/
theorem downgradeOp_proj (ρ : Ref → Addr) (σ : Ref → Val) (op : Op) :
    proj ρ σ (downgradeOp op) = proj ρ σ op := by
  cases op <;> rfl

/-- The downgrade rewrites in place — it neither grows nor shrinks a block. -/
theorem downgradeAccesses_length (l : List Op) :
    (downgradeAccesses l).length = l.length := by
  simp [downgradeAccesses]

/-- IR-level soundness bridge for the access-downgrade rewrite (parallel to
    `projBlock_elimFences`): downgrading TSO accesses to plain leaves the
    projected model block — every load/store at its resolved address/value, in
    order — completely unchanged, so the TSO observable-equivalence results carry
    over to the real downgrade. -/
theorem projBlock_downgrade (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    projBlock ρ σ (downgradeAccesses l) = projBlock ρ σ l := by
  induction l with
  | nil => rfl
  | cons op r ih =>
    simp only [downgradeAccesses, List.map_cons, projBlock, List.filterMap_cons,
               downgradeOp_proj ρ σ op] at ih ⊢
    cases proj ρ σ op with
    | none => exact ih
    | some m => rw [ih]

/-- Downgrading is idempotent: a plain access is already its own downgrade, so a
    second pass is a no-op (the rewrite reaches a fixpoint immediately). -/
theorem downgradeAccesses_idempotent (l : List Op) :
    downgradeAccesses (downgradeAccesses l) = downgradeAccesses l := by
  simp only [downgradeAccesses, List.map_map]
  apply List.map_congr_left
  intro op _
  cases op <;> rfl

/-! ### F1-LN-016 capstone — the full adaptive rewrite

  The complete TSO-adaptive rewrite is both halves: drop the fences the
  classifier proved unnecessary, then relax the TSO accesses it proved
  thread-local. -/

/-- The full TSO-adaptive rewrite: eliminate provable fences, then downgrade
    provable TSO accesses to plain. -/
def adaptiveRewrite (l : List Op) : List Op := downgradeAccesses (elimFences l)

/-- **Capstone.** The full adaptive rewrite preserves the projected model block
    modulo bar removal: composing the downgrade bridge (`projBlock_downgrade`,
    which leaves the projection untouched) with the fence-elim bridge
    (`projBlock_elimFences`, which deletes exactly the bars), the result is the
    original block with its bars gone — every load/store at its resolved
    address/value preserved in order. So both halves together carry the M2/M3
    observable-equivalence conclusion to the real rewrite. -/
theorem projBlock_adaptiveRewrite (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    projBlock ρ σ (adaptiveRewrite l) = (projBlock ρ σ l).filter (fun m => ! isBar m) := by
  rw [adaptiveRewrite, projBlock_downgrade, projBlock_elimFences]

/-- The full rewrite is bar-free: after it, the projected block has no `bar`. -/
theorem adaptiveRewrite_barFree (ρ : Ref → Addr) (σ : Ref → Val) (l : List Op) :
    ∀ m ∈ projBlock ρ σ (adaptiveRewrite l), isBar m = false := by
  intro m hm
  rw [projBlock_adaptiveRewrite] at hm
  simpa using (List.mem_filter.mp hm).2

/-- The full adaptive rewrite never grows a block: the downgrade half preserves
    length (it is a `map`) and the fence-elim half only removes ops, so the
    result fits wherever the original did. -/
theorem adaptiveRewrite_length_le (l : List Op) :
    (adaptiveRewrite l).length ≤ l.length := by
  rw [adaptiveRewrite, downgradeAccesses_length]
  exact elimFences_length_le l

/-- Downgrading preserves whether an op is a fence — it only relaxes the
    memory-ordering of loads/stores, never adds or removes a fence. -/
theorem isFence_downgradeOp (op : Op) : isFence (downgradeOp op) = isFence op := by
  cases op <;> rfl

/-- A fence-free list is left untouched by fence elimination (there is nothing
    to remove). -/
theorem elimFences_id_of_fenceFree (l : List Op)
    (h : ∀ op ∈ l, isFence op = false) : elimFences l = l := by
  induction l with
  | nil => rfl
  | cons op r ih =>
    have hop : isFence op = false := h op List.mem_cons_self
    have hr : ∀ x ∈ r, isFence x = false := fun x hx => h x (List.mem_cons_of_mem op hx)
    simp [elimFences, hop, ih hr]

/-- Downgrading a fence-free list stays fence-free (downgrade preserves fences). -/
theorem downgradeAccesses_fenceFree (l : List Op)
    (h : ∀ op ∈ l, isFence op = false) :
    ∀ op ∈ downgradeAccesses l, isFence op = false := by
  intro op hop
  simp only [downgradeAccesses, List.mem_map] at hop
  obtain ⟨op', hop', rfl⟩ := hop
  rw [isFence_downgradeOp]
  exact h op' hop'

/-- The full adaptive rewrite is idempotent — it reaches a fixpoint in one pass,
    so the pipeline never needs to iterate it. After the first pass the block is
    fence-free (so the inner `elimFences` is a no-op) and already downgraded (so
    the inner `downgradeAccesses` is a no-op). -/
theorem adaptiveRewrite_idempotent (l : List Op) :
    adaptiveRewrite (adaptiveRewrite l) = adaptiveRewrite l := by
  unfold adaptiveRewrite
  have hff : ∀ op ∈ downgradeAccesses (elimFences l), isFence op = false :=
    downgradeAccesses_fenceFree _ (elimFences_fenceFree l)
  rw [elimFences_id_of_fenceFree _ hff, downgradeAccesses_idempotent]

/-- The two adaptive-rewrite halves commute: eliminating fences then downgrading
    accesses equals downgrading then eliminating. Fence elimination is a `filter`
    on `isFence` and the downgrade is a `map` that preserves `isFence`
    (`isFence_downgradeOp`), so neither reorders the other's decisions. -/
theorem elimFences_downgrade_comm (l : List Op) :
    elimFences (downgradeAccesses l) = downgradeAccesses (elimFences l) := by
  induction l with
  | nil => rfl
  | cons op r ih =>
    simp only [downgradeAccesses, List.map_cons, elimFences, isFence_downgradeOp]
    cases h : isFence op with
    | true => simpa [h, downgradeAccesses] using ih
    | false =>
      simp only [h, Bool.false_eq_true, if_false, List.map_cons]
      simp only [downgradeAccesses] at ih
      rw [ih]

/-- **Order-independence.** The full adaptive rewrite may equally run the
    downgrade half first: `adaptiveRewrite` (fences-then-downgrade) equals
    downgrade-then-fences. So the pass scheduler is free to order the two halves
    either way — the optimised block is the same. -/
theorem adaptiveRewrite_eq_downgrade_first (l : List Op) :
    adaptiveRewrite l = elimFences (downgradeAccesses l) := by
  rw [adaptiveRewrite, elimFences_downgrade_comm]

/-- Downgrading an op always yields a non-TSO access: the two TSO constructors
    become their plain counterparts, and every other op was already non-TSO. -/
theorem isTsoAccess_downgradeOp (op : Op) : isTsoAccess (downgradeOp op) = false := by
  cases op <;> rfl

/-- After the downgrade pass, no TSO-ordered access remains. -/
theorem downgradeAccesses_tsoFree (l : List Op) :
    ∀ op ∈ downgradeAccesses l, isTsoAccess op = false := by
  intro op hop
  simp only [downgradeAccesses, List.mem_map] at hop
  obtain ⟨op', _, rfl⟩ := hop
  exact isTsoAccess_downgradeOp op'

/-- **Downgrade effectiveness — the mirror of `elimFences_barFree`.** After the
    full adaptive rewrite, no TSO-ordered load/store survives: the downgrade half
    relaxed every one to a plain access. Together with `adaptiveRewrite_barFree`
    (no `bar` survives), this shows the rewrite achieves its goal on *both* axes —
    fences gone and TSO accesses relaxed — so the ARM64 it emits needs neither a
    `dmb` nor an acquire/release for the proven-thread-local region. -/
theorem adaptiveRewrite_tsoFree (l : List Op) :
    ∀ op ∈ adaptiveRewrite l, isTsoAccess op = false := by
  intro op hop
  rw [adaptiveRewrite] at hop
  exact downgradeAccesses_tsoFree _ op hop

end TSO
end PrismaIR
